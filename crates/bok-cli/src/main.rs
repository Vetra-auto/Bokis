//! `bok` – kommandoradsverktyget för den som granskar och bokför.
//!
//! TILLFÄLLIGT (milstolpe 1): `bok` öppnar databasen och anropar `bok-store`
//! direkt. Enligt arkitekturen ska bara `bokd` äga databasen; när `bokd`
//! finns (milstolpe 2) går kommandona i stället via dess Unix-socket, och
//! direktanropen här tas bort.
//!
//! Det finns medvetet inga kommandon som raderar, nollställer eller
//! återställer en databas. `bok init` skapar bara nya filer.

mod archive;
mod archive_plan_cmd;
mod closing;
mod declaration_cmd;
mod input;
mod insyn_cmd;
mod inventory_cmd;
mod parts;
mod render;
mod rules_cmd;
mod signer;
mod sysdoc;
mod year_archive;
mod year_end_cmd;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, anyhow, bail};
use chrono::NaiveDate;
use clap::{Args, Parser, Subcommand};

use bok_store::bok_core::canonical::sha256_hex;
use bok_store::bok_core::security::SecurityLevel;
use bok_store::bok_core::signature::parse_public_key;
use bok_store::bok_core::voucher::Counterparty;
use bok_store::bok_core::{AccountNumber, PeriodStatus, Series, VoucherRef};
use bok_store::{
    AccountChange, Actor, Channel, FiscalYear, NewAttachment, NewBook, NewProposal,
    ProposalCounterparty, Provenance, Store, StoreError,
};
use bokforing_export::sie4::{
    Sie4Account, Sie4Company, Sie4Export, Sie4Voucher, Sie4VoucherLine, Sie4VoucherRecord,
};

use archive::{Archive, FileCheck};
use signer::SshKeygenSigner;

const PROGRAM: &str = concat!("bok ", env!("CARGO_PKG_VERSION"));

#[derive(Parser)]
#[command(
    name = "bok",
    version,
    about = "Bokis: granska förslag, bokför med din nyckel och verifiera bokföringen",
    after_help = "Agenten föreslår, du bokför. Varje bokföring, periodstängning och ändring av kontoplanen \
                  signeras med nyckeln (ed25519-sk, till exempel en YubiKey)."
)]
struct Cli {
    /// Databasen [standard: ~/.local/share/bokis/bokis.db]
    #[arg(long, env = "BOKIS_DB", global = true)]
    db: Option<PathBuf>,
    /// Nyckeln som signerar, en privat nyckelfil för ssh-keygen [standard: ~/.ssh/id_ed25519_sk]
    #[arg(long, env = "BOKIS_KEY", global = true)]
    key: Option<PathBuf>,
    /// Ditt namn i behandlingshistoriken [standard: $USER]
    #[arg(long, env = "BOKIS_USER", global = true)]
    user: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Skapar en ny, tom bokföring och registrerar din nyckel. Skriver aldrig över en befintlig.
    Init(InitArgs),
    /// Nycklar som får signera.
    #[command(subcommand, visible_alias = "nyckel")]
    Key(KeyCmd),
    /// Räkenskapsår och deras perioder.
    #[command(subcommand, visible_alias = "ar")]
    Year(YearCmd),
    /// Kontoplanen. Varje ändring signeras med nyckeln.
    #[command(subcommand, visible_alias = "konto")]
    Account(AccountCmd),
    /// Motpartsregistret.
    #[command(subcommand, visible_alias = "motpart")]
    Counterparty(CounterpartyCmd),
    /// Underlag: kvitton, fakturor och andra filer som verifikationerna bygger på.
    #[command(subcommand, visible_alias = "underlag")]
    Attachment(AttachmentCmd),
    /// Läser in nya filer från en bevakad mapp som underlag och flyttar dem till <mapp>/inlasta.
    #[command(visible_alias = "inkorg")]
    Inbox(parts::InboxArgs),
    /// Kopior till en extern plats: skapa, kontrollera, lista och läsa in i en tom katalog.
    #[command(subcommand, visible_alias = "kopia")]
    Backup(parts::BackupCmd),
    /// Kontoutdrag från banken (camt.053 eller CSV) och avstämning.
    #[command(subcommand)]
    Bank(parts::BankCmd),
    /// Agentens frågor och flaggor, som du besvarar.
    #[command(subcommand, visible_alias = "arende")]
    Case(parts::CaseCmd),
    /// Regler som agenten följer. Varje version signeras med nyckeln.
    #[command(subcommand, visible_alias = "regel")]
    Rule(rules_cmd::RuleCmd),
    /// Momsrapporten för en period: 2026-03, 2026-01:2026-03 eller 2026.
    #[command(visible_alias = "moms")]
    Vat { period: String },
    /// Skriver systemdokumentationen, genererad ur bokföringen, till en ny fil.
    #[command(visible_alias = "systemdokumentation")]
    SystemDoc { out: PathBuf },
    /// Arkivplanen: vad som arkiveras, var, hur ofta och hur det kontrolleras.
    #[command(subcommand, visible_alias = "arkivplan")]
    ArchivePlan(archive_plan_cmd::ArchivePlanCmd),
    /// Bokslut: förslag om avskrivningar, periodiseringar, skatt och årets resultat (serie B)
    /// och ingående balans för nästa år (serie I).
    #[command(subcommand, visible_alias = "bokslut")]
    Closing(closing::ClosingCmd),
    /// Inventarieregistret: anskaffningar, avskrivningsplan och avyttringar.
    #[command(subcommand, visible_alias = "inventarie")]
    Inventory(inventory_cmd::InventoryCmd),
    /// Deklarationer ur bokföringen: inkomstdeklaration 2 som SRU-filer.
    #[command(subcommand, visible_alias = "deklaration")]
    Declaration(declaration_cmd::DeclarationCmd),
    /// Stämmer av ett konto mot ett externt saldo, till exempel skattekontot 1630 mot Skatteverket.
    #[command(visible_alias = "avstam")]
    Reconcile {
        account: AccountNumber,
        /// Dagen saldot gäller.
        #[arg(long)]
        date: String,
        /// Det externa saldot, med bokföringens tecken (tillgodohavande positivt): 1234,56.
        #[arg(long, allow_hyphen_values = true)]
        balance: String,
        #[arg(long)]
        note: Option<String>,
    },
    /// Insyn för revisor och Skatteverket: bokföringen som läsbara sidor, live eller som export.
    #[command(visible_alias = "insyn")]
    Inspect(insyn_cmd::InsynArgs),
    /// Skriver årets arkivpaket: SIE, rapporter, underlag, historik, systemdokumentation och programmet.
    #[command(visible_alias = "arkivpaket")]
    YearArchive {
        /// Katalogen som paketet läggs i, till exempel en extern disk.
        target: PathBuf,
        /// Räkenskapsåret [standard: året som omfattar i dag].
        #[arg(long)]
        year: Option<String>,
    },
    /// Lägger ett förslag för hand. Förslaget får nummer först när det bokförs.
    #[command(visible_alias = "foresla")]
    Propose(ProposeArgs),
    /// Visar förslagen som väntar på beslut, eller ett enskilt förslag.
    #[command(visible_alias = "forslag")]
    Proposals {
        /// Ett förslag att visa i detalj.
        id: Option<i64>,
        /// Visa även beslutade förslag.
        #[arg(long)]
        all: bool,
    },
    /// Godkänner och bokför förslag. Varje bokföring signeras med nyckeln.
    #[command(visible_alias = "godkann")]
    Approve {
        #[arg(required = true)]
        ids: Vec<i64>,
    },
    /// Avvisar ett förslag. Inget nummer förbrukas.
    #[command(visible_alias = "avvisa")]
    Reject {
        id: i64,
        /// Varför förslaget avvisas.
        #[arg(long)]
        reason: String,
    },
    /// Föreslår en rättelse: en ny verifikation i serie R som tar ut originalet.
    #[command(visible_alias = "ratta")]
    Correct(CorrectArgs),
    /// Periodernas status: granska, öppna igen, stänga och låsa.
    #[command(subcommand)]
    Period(PeriodCmd),
    /// Grundboken: verifikationerna i registreringsordning.
    #[command(visible_alias = "grundbok")]
    Journal {
        #[arg(long)]
        year: Option<String>,
        #[arg(long)]
        series: Option<Series>,
    },
    /// Huvudboken per konto, med ingående balans.
    #[command(visible_alias = "huvudbok")]
    Ledger {
        #[arg(long)]
        year: Option<String>,
        #[arg(long)]
        account: Option<AccountNumber>,
    },
    /// Saldobalans för ett räkenskapsår.
    #[command(visible_alias = "saldobalans")]
    TrialBalance {
        #[arg(long)]
        year: Option<String>,
    },
    /// Visar en verifikation med signatur och hashar.
    #[command(visible_alias = "visa")]
    Show {
        voucher: VoucherRef,
        #[arg(long)]
        year: Option<String>,
    },
    /// Behandlingshistoriken.
    #[command(visible_alias = "historik")]
    History {
        /// Bara de senaste händelserna.
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Kontrollerar serier, historikens kedja, signaturer och underlagsarkivet.
    #[command(visible_alias = "verifiera")]
    Verify,
    /// Klockan. Bokis skriver inget när klockan står före den senaste händelsen i historiken.
    #[command(subcommand, visible_alias = "klocka")]
    Clock(ClockCmd),
    /// Säkerhetsnivån: vilka signaturer bokföringen godtar.
    #[command(subcommand, visible_alias = "sakerhet")]
    Security(SecurityCmd),
    /// Exporterar bokföringen.
    #[command(subcommand, visible_alias = "exportera")]
    Export(ExportCmd),
}

#[derive(Subcommand)]
enum SecurityCmd {
    /// Visar nivån som gäller och alla beslut om den.
    #[command(visible_alias = "visa")]
    Show,
    /// Byter nivå: mjukvara, hardvara eller hardvara-pin. Den som höjer nivån signerar med en
    /// nyckel som klarar den nya nivån, den som sänker den med en nyckel som klarar den gamla.
    #[command(visible_alias = "satt")]
    Set {
        #[arg(value_name = "NIVÅ")]
        level: SecurityLevel,
    },
}

#[derive(Args)]
struct InitArgs {
    /// Företagets namn.
    #[arg(long)]
    company: String,
    /// Organisationsnummer (NNNNNN-NNNN), eller personnummer för en enskild
    /// firma. Kontrollsiffran prövas.
    #[arg(long)]
    orgnr: String,
    /// Namnet på den som äger nyckeln [standard: --user].
    #[arg(long)]
    name: Option<String>,
    /// Den publika nyckeln [standard: nyckelfilen + .pub].
    #[arg(long)]
    public_key: Option<PathBuf>,
    /// Första räkenskapsåret: 2026 eller 2025-07-01:2026-06-30.
    #[arg(long)]
    year: Option<String>,
    /// Säkerhetsnivån. mjukvara: alla nycklar godtas, också en nyckelfil. hardvara: bara
    /// hårdvarunycklar, som måste röras. hardvara-pin: dessutom nyckelns PIN. Kan ändras
    /// senare med `bok sakerhet satt`.
    #[arg(
        long,
        visible_alias = "sakerhet",
        value_name = "NIVÅ",
        default_value = "hardvara"
    )]
    security: SecurityLevel,
}

#[derive(Subcommand)]
enum KeyCmd {
    /// Registrerar en ny nyckel. En redan registrerad nyckel signerar.
    #[command(visible_alias = "lagg-till")]
    Add {
        public_key: PathBuf,
        #[arg(long)]
        name: String,
    },
    /// Visar de registrerade nycklarna.
    #[command(visible_alias = "lista")]
    List,
}

#[derive(Subcommand)]
enum YearCmd {
    /// Lägger till ett räkenskapsår: 2026 eller 2025-07-01:2026-06-30.
    #[command(visible_alias = "lagg-till")]
    Add { year: String },
    #[command(visible_alias = "lista")]
    List,
}

#[derive(Subcommand)]
enum AccountCmd {
    /// Läser in konton från en SIE-fil (#KONTO) eller en textfil med KONTO;NAMN[;RUTA] per rad.
    #[command(visible_alias = "importera")]
    Import {
        file: PathBuf,
        /// Gäller från [standard: första räkenskapsårets start].
        #[arg(long)]
        from: Option<String>,
        /// Momskoder att sätta samtidigt, KONTO;RUTA per rad (se docs/momskoder-bas.txt).
        #[arg(long)]
        vat_codes: Option<PathBuf>,
    },
    /// Lägger till eller döper om ett konto.
    #[command(visible_alias = "lagg-till")]
    Add {
        number: AccountNumber,
        name: String,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        sru: Option<String>,
        #[arg(long)]
        vat_code: Option<String>,
    },
    /// Gör ett konto inaktivt från ett datum.
    #[command(visible_alias = "inaktivera")]
    Deactivate {
        number: AccountNumber,
        #[arg(long)]
        from: Option<String>,
    },
    /// Visar kontoplanen som den ser ut ett visst datum.
    #[command(visible_alias = "lista")]
    List {
        #[arg(long)]
        date: Option<String>,
    },
    /// Ändrar momskoder (rutan i momsdeklarationen) från en fil med KONTO;RUTA per rad.
    #[command(visible_alias = "momskoder")]
    VatCodes {
        file: PathBuf,
        /// Gäller från [standard: i dag]. Koder som ska gälla från början läses in med
        /// `bok konto importera --vat-codes`.
        #[arg(long)]
        from: Option<String>,
    },
}

#[derive(Subcommand)]
enum CounterpartyCmd {
    #[command(visible_alias = "lagg-till")]
    Add {
        #[arg(long)]
        name: String,
        #[arg(long)]
        orgnr: Option<String>,
        #[arg(long)]
        vatnr: Option<String>,
        /// Postadressen, till exempel "Storgatan 1, 123 45 Exempelstad".
        #[arg(long, visible_alias = "adress")]
        address: Option<String>,
    },
    /// Ändrar en motpart. Redan bokförda verifikationer behåller sin kopia.
    #[command(visible_alias = "andra")]
    Update {
        id: i64,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        orgnr: Option<String>,
        #[arg(long)]
        vatnr: Option<String>,
        /// Ny postadress.
        #[arg(long, visible_alias = "adress")]
        address: Option<String>,
    },
    #[command(visible_alias = "lista")]
    List,
}

#[derive(Subcommand)]
enum AttachmentCmd {
    /// Tar emot en fil: sparar den oförändrad i arkivet och registrerar dess SHA-256.
    #[command(visible_alias = "lagg-till")]
    Add {
        file: PathBuf,
        /// Vem som skickade underlaget.
        #[arg(long)]
        sender: Option<String>,
    },
    #[command(visible_alias = "lista")]
    List,
}

#[derive(Args)]
struct ProposeArgs {
    /// Förslaget som JSON (samma form som agenten använder). Ersätter övriga flaggor.
    #[arg(long, conflicts_with_all = ["date", "text", "lines"])]
    file: Option<PathBuf>,
    /// Affärshändelsens datum.
    #[arg(long, required_unless_present = "file")]
    date: Option<String>,
    /// Vad affärshändelsen avser.
    #[arg(long, required_unless_present = "file")]
    text: Option<String>,
    /// KONTO=BELOPP[:TEXT]; positivt belopp är debet, negativt kredit. Upprepa för varje rad.
    #[arg(long = "line", required_unless_present = "file")]
    lines: Vec<String>,
    #[arg(long, default_value = "A")]
    series: Series,
    /// En motpart ur registret.
    #[arg(long, conflicts_with = "counterparty_name")]
    counterparty: Option<i64>,
    /// En motpart som inte finns i registret.
    #[arg(long)]
    counterparty_name: Option<String>,
    #[arg(long, requires = "counterparty_name")]
    orgnr: Option<String>,
    #[arg(long, requires = "counterparty_name")]
    vatnr: Option<String>,
    /// Motpartens postadress; identifierar den tillsammans med namnet när
    /// organisationsnummer saknas.
    #[arg(long, visible_alias = "motpart-adress", requires = "counterparty_name")]
    counterparty_address: Option<String>,
    /// Underlag (id) som verifikationen bygger på. Upprepa för flera.
    #[arg(long = "attachment")]
    attachments: Vec<i64>,
    /// Bokför förslaget direkt.
    #[arg(long)]
    approve: bool,
}

#[derive(Args)]
struct CorrectArgs {
    /// Verifikationen som ska rättas, till exempel A12.
    voucher: VoucherRef,
    #[arg(long)]
    reason: String,
    #[arg(long)]
    year: Option<String>,
    /// Rätta rader att lägga till efter omvändningen: KONTO=BELOPP[:TEXT].
    #[arg(long = "line")]
    lines: Vec<String>,
    /// Bokför rättelsen direkt.
    #[arg(long)]
    approve: bool,
}

#[derive(Subcommand)]
enum PeriodCmd {
    /// Visar perioderna och deras status.
    #[command(visible_alias = "lista")]
    List {
        #[arg(long)]
        year: Option<String>,
    },
    /// Öppen → under avstämning.
    #[command(visible_alias = "granska")]
    Review { period: String },
    /// Under avstämning → öppen. En stängd period kan aldrig öppnas.
    #[command(visible_alias = "oppna")]
    Reopen { period: String },
    /// Under avstämning → stängd. Signeras med nyckeln.
    #[command(visible_alias = "stang")]
    Close { period: String },
    /// Stängd → låst. Signeras med nyckeln.
    #[command(visible_alias = "las")]
    Lock { period: String },
}

#[derive(Subcommand)]
enum ClockCmd {
    /// Godtar att klockan står före den senaste händelsen. Gör det bara när klockan nu går rätt
    /// och det är den senaste händelsen som fick fel tid. Båda tiderna och förklaringen hamnar i
    /// historiken.
    #[command(visible_alias = "godta")]
    Accept {
        /// Varför, till exempel "datorns klocka gick en timme före den 20 november".
        #[arg(long)]
        reason: String,
    },
}

#[derive(Subcommand)]
enum ExportCmd {
    /// SIE 4 med #IB, #UB, #RES och verifikationer med registreringsdatum och signatur.
    Sie {
        #[arg(long)]
        year: Option<String>,
        /// Filen som skapas. En befintlig fil skrivs aldrig över.
        #[arg(long)]
        out: PathBuf,
    },
}

struct Ctx {
    db: PathBuf,
    key: PathBuf,
    user: String,
}

impl Ctx {
    fn from(cli: &Cli) -> Result<Ctx> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let db = match &cli.db {
            Some(p) => p.clone(),
            None => {
                let data = std::env::var_os("XDG_DATA_HOME")
                    .map(PathBuf::from)
                    .or_else(|| home.as_ref().map(|h| h.join(".local/share")))
                    .ok_or_else(|| anyhow!("ange databasen med --db eller BOKIS_DB"))?;
                data.join("bokis").join("bokis.db")
            }
        };
        let key = match &cli.key {
            Some(k) => k.clone(),
            None => home
                .map(|h| h.join(".ssh/id_ed25519_sk"))
                .unwrap_or_else(|| PathBuf::from("id_ed25519_sk")),
        };
        let user = cli
            .user
            .clone()
            .or_else(|| std::env::var("USER").ok())
            .filter(|u| !u.trim().is_empty())
            .unwrap_or_else(|| "granskare".into());
        Ok(Ctx { db, key, user })
    }

    fn open(&self) -> Result<Store> {
        Store::open(&self.db, PROGRAM)
            .with_context(|| format!("kan inte öppna bokföringen {}", self.db.display()))
    }

    fn signer(&self) -> SshKeygenSigner {
        SshKeygenSigner::new(self.key.clone())
    }

    fn actor(&self) -> Actor {
        Actor::human(self.user.clone())
    }

    fn archive(&self) -> Archive {
        Archive::beside(&self.db)
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("fel: {e:#}");
            let clock_behind = e
                .chain()
                .any(|c| matches!(c.downcast_ref(), Some(StoreError::ClockBehind { .. })));
            if clock_behind {
                eprintln!(
                    "Ställ datorns klocka. Går den redan rätt, och det är den senaste händelsen som \
                     fick fel tid, godta det med bok klocka godta --reason \"…\"."
                );
            }
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    let ctx = Ctx::from(&cli)?;
    match cli.command {
        Command::Init(args) => init(&ctx, args)?,
        Command::Key(cmd) => key(&ctx, cmd)?,
        Command::Year(cmd) => year(&ctx, cmd)?,
        Command::Account(cmd) => account(&ctx, cmd)?,
        Command::Counterparty(cmd) => counterparty(&ctx, cmd)?,
        Command::Attachment(cmd) => attachment(&ctx, cmd)?,
        Command::Inbox(args) => parts::inbox(&ctx, args)?,
        Command::Backup(cmd) => parts::backup(&ctx, cmd)?,
        Command::Bank(cmd) => parts::bank(&ctx, cmd)?,
        Command::Case(cmd) => parts::case(&ctx, cmd)?,
        Command::Rule(cmd) => rules_cmd::rule(&ctx, cmd)?,
        Command::Vat { period } => parts::vat_report(&ctx, &period)?,
        Command::SystemDoc { out } => sysdoc::write(&ctx, &out)?,
        Command::ArchivePlan(cmd) => archive_plan_cmd::archive_plan(&ctx, cmd)?,
        Command::Closing(cmd) => closing::closing(&ctx, cmd)?,
        Command::Inventory(cmd) => inventory_cmd::inventory(&ctx, cmd)?,
        Command::Declaration(cmd) => declaration_cmd::declaration(&ctx, cmd)?,
        Command::Inspect(args) => insyn_cmd::insyn(&ctx, args)?,
        Command::YearArchive { target, year } => {
            year_archive::write(&ctx, year.as_deref(), &target)?
        }
        Command::Reconcile {
            account,
            date,
            balance,
            note,
        } => {
            let mut store = ctx.open()?;
            let date = input::parse_date(&date)?;
            let external = bok_store::bok_core::Money::parse(&balance)
                .with_context(|| format!("{balance} är inget belopp"))?;
            let r = store.record_reconciliation(
                &ctx.actor(),
                account,
                date,
                external,
                note.as_deref(),
            )?;
            println!("Konto {account} per {date}");
            println!("  bokfört saldo  {:>16}", render::amount(r.booked));
            println!("  externt saldo  {:>16}", render::amount(r.external));
            println!("  skillnad       {:>16}", render::amount(r.difference));
            if r.difference.is_zero() {
                println!("Kontot stämmer. Avstämningen är loggad i historiken.");
            } else {
                println!(
                    "Kontot stämmer INTE; utred skillnaden. Avstämningen är loggad i historiken."
                );
            }
        }
        Command::Propose(args) => propose(&ctx, args)?,
        Command::Proposals { id, all } => proposals(&ctx, id, all)?,
        Command::Approve { ids } => {
            let mut store = ctx.open()?;
            for id in ids {
                approve(&ctx, &mut store, id)?;
            }
        }
        Command::Reject { id, reason } => {
            let mut store = ctx.open()?;
            store.reject(&ctx.actor(), id, &reason)?;
            println!("Förslag {id} avvisat. Inget verifikationsnummer förbrukades.");
        }
        Command::Correct(args) => correct(&ctx, args)?,
        Command::Period(cmd) => period(&ctx, cmd)?,
        Command::Journal { year, series } => journal(&ctx, year.as_deref(), series)?,
        Command::Ledger { year, account } => {
            let store = ctx.open()?;
            let y = resolve_year(&store, year.as_deref())?;
            println!("Huvudbok {} – {}", y.label(), company_line(&store)?);
            println!();
            render::ledger(&y, &store.ledger(y.id, account)?);
        }
        Command::TrialBalance { year } => {
            let store = ctx.open()?;
            let y = resolve_year(&store, year.as_deref())?;
            println!("Saldobalans {} – {}", y.label(), company_line(&store)?);
            println!(
                "Bara bokförda verifikationer i räkenskapsåret {} – {}.",
                y.start, y.end
            );
            println!();
            render::trial_balance(&store.trial_balance(y.id)?);
        }
        Command::Show { voucher, year } => {
            let store = ctx.open()?;
            let y = resolve_year(&store, year.as_deref())?;
            render::voucher(&store, &store.voucher_by_ref(y.id, voucher)?, true)?;
        }
        Command::History { limit } => render::events(&ctx.open()?.events(limit)?),
        Command::Verify => return verify(&ctx),
        Command::Clock(ClockCmd::Accept { reason }) => {
            let mut store = ctx.open()?;
            let (last, now) = store.accept_clock(&ctx.actor(), &reason)?;
            println!(
                "Godtaget. Den senaste händelsen är från {}, och klockan visar {}.",
                bok_store::display_timestamp(&last),
                bok_store::display_timestamp(&now)
            );
            println!("Det står i historiken, och nya händelser jämförs med den här.");
        }
        Command::Security(cmd) => security(&ctx, cmd)?,
        Command::Export(ExportCmd::Sie { year, out }) => export_sie(&ctx, year.as_deref(), &out)?,
    }
    Ok(ExitCode::SUCCESS)
}

fn company_line(store: &Store) -> Result<String> {
    let c = store.company()?;
    Ok(format!("{} ({})", c.name, c.org_number))
}

/// Räkenskapsåret som `arg` pekar på ("2026", "2025/26" eller ett datum i
/// året). Utan `arg`: året som innehåller dagens datum, annars det senaste.
fn resolve_year(store: &Store, arg: Option<&str>) -> Result<FiscalYear> {
    let years = store.fiscal_years()?;
    let Some(last) = years.last().copied() else {
        bail!("det finns inget räkenskapsår; lägg till ett med `bok year add 2026`");
    };
    let Some(arg) = arg else {
        let today = store.today();
        return Ok(years
            .iter()
            .copied()
            .find(|y| y.contains(today))
            .unwrap_or(last));
    };
    if let Some(y) = years.iter().find(|y| y.label() == arg.trim()) {
        return Ok(*y);
    }
    if let Ok(d) = input::parse_date(arg)
        && let Some(y) = years.iter().find(|y| y.contains(d))
    {
        return Ok(*y);
    }
    let labels: Vec<String> = years.iter().map(FiscalYear::label).collect();
    bail!(
        "hittar inget räkenskapsår \"{arg}\" (det finns {})",
        labels.join(", ")
    )
}

fn date_or(store: &Store, arg: Option<&str>) -> Result<NaiveDate> {
    match arg {
        Some(s) => input::parse_date(s),
        None => Ok(store
            .fiscal_years()?
            .first()
            .map(|y| y.start)
            .unwrap_or_else(|| store.today())),
    }
}

fn init(ctx: &Ctx, args: InitArgs) -> Result<()> {
    let public_key_path = args.public_key.unwrap_or_else(|| {
        let mut p = ctx.key.clone().into_os_string();
        p.push(".pub");
        PathBuf::from(p)
    });
    let public_key = std::fs::read_to_string(&public_key_path).with_context(|| {
        format!(
            "kan inte läsa den publika nyckeln {}",
            public_key_path.display()
        )
    })?;
    let info = parse_public_key(&public_key)?;
    let level = args.security;
    if !level.admits_key(info.hardware) {
        bail!(
            "{} är en nyckelfil, och säkerhetsnivån {} kräver en hårdvarunyckel (ed25519-sk). \
             Använd YubiKeyn, eller välj --security mjukvara om en nyckelfil räcker.",
            public_key_path.display(),
            level.title()
        );
    }
    let year = args
        .year
        .as_deref()
        .map(input::parse_year_spec)
        .transpose()?;
    if let Some(dir) = ctx.db.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("kan inte skapa {}", dir.display()))?;
    }
    let name = args.name.unwrap_or_else(|| ctx.user.clone());
    let signer = ctx.signer();
    let mut store = Store::create(
        &ctx.db,
        PROGRAM,
        NewBook {
            company_name: &args.company,
            org_number: &args.orgnr,
            key_name: &name,
            public_key: &public_key,
            signer: &signer,
            security_level: level,
        },
    )?;
    let company = store.company()?;
    println!(
        "Skapade bokföringen {} för {} ({}).",
        ctx.db.display(),
        company.name,
        company.org_number
    );
    println!("Nyckel {} registrerad för {name}.", info.fingerprint);
    println!("Säkerhetsnivå: {}. {}", level.title(), level.meaning());
    if let Some((start, end)) = year {
        let y = store.add_fiscal_year(&Actor::human(name), start, end)?;
        println!(
            "Räkenskapsår {} ({} – {}) med perioder och serierna A, B, I och R.",
            y.label(),
            y.start,
            y.end
        );
    }
    Ok(())
}

fn security(ctx: &Ctx, cmd: SecurityCmd) -> Result<()> {
    match cmd {
        SecurityCmd::Show => {
            let store = ctx.open()?;
            let level = store.security_level()?;
            let decisions = store.security_decisions()?;
            match decisions.last() {
                Some(d) => println!(
                    "Säkerhetsnivå: {}, sedan {}.",
                    level.title(),
                    bok_store::display_timestamp(&d.decided)
                ),
                None => println!(
                    "Säkerhetsnivå: {}. Bokföringen skapades före nivåerna och har inget beslut.",
                    level.title()
                ),
            }
            println!("{}", level.meaning());
            if !decisions.is_empty() {
                println!();
                for d in &decisions {
                    let change = match d.previous {
                        Some(p) => format!("{} → {}", p.title(), d.level.title()),
                        None => format!("{} när bokföringen skapades", d.level.title()),
                    };
                    println!(
                        "  {}  {change}  ({}, händelse {})",
                        bok_store::display_timestamp(&d.decided),
                        d.actor,
                        d.event
                    );
                }
            }
        }
        SecurityCmd::Set { level } => {
            let mut store = ctx.open()?;
            store.set_security_level(&ctx.actor(), level, &ctx.signer())?;
            println!(
                "Säkerhetsnivån är nu {}. {}",
                level.title(),
                level.meaning()
            );
            println!("Beslutet står i historiken.");
        }
    }
    Ok(())
}

fn key(ctx: &Ctx, cmd: KeyCmd) -> Result<()> {
    match cmd {
        KeyCmd::Add { public_key, name } => {
            let mut store = ctx.open()?;
            let text = std::fs::read_to_string(&public_key)
                .with_context(|| format!("kan inte läsa {}", public_key.display()))?;
            let key = store.add_key(&ctx.actor(), &name, &text, &ctx.signer())?;
            println!("Nyckel {} registrerad för {}.", key.fingerprint, key.name);
        }
        KeyCmd::List => {
            for k in ctx.open()?.keys()? {
                let kind = if k.hardware { "hårdvara" } else { "mjukvara" };
                println!(
                    "{:>3}  {:<12} {}  {} ({kind})",
                    k.id, k.name, k.fingerprint, k.algorithm
                );
            }
        }
    }
    Ok(())
}

fn year(ctx: &Ctx, cmd: YearCmd) -> Result<()> {
    match cmd {
        YearCmd::Add { year } => {
            let (start, end) = input::parse_year_spec(&year)?;
            let mut store = ctx.open()?;
            let y = store.add_fiscal_year(&ctx.actor(), start, end)?;
            println!(
                "Räkenskapsår {} ({} – {}) med perioder och serierna A, B, I och R.",
                y.label(),
                y.start,
                y.end
            );
        }
        YearCmd::List => {
            let store = ctx.open()?;
            for y in store.fiscal_years()? {
                let periods = store.periods(y.id)?;
                let open = periods
                    .iter()
                    .filter(|p| p.status == PeriodStatus::Open)
                    .count();
                println!(
                    "{:<8} {} – {}  {} perioder, {open} öppna",
                    y.label(),
                    y.start,
                    y.end,
                    periods.len()
                );
            }
        }
    }
    Ok(())
}

fn account(ctx: &Ctx, cmd: AccountCmd) -> Result<()> {
    match cmd {
        AccountCmd::VatCodes { file, from } => parts::vat_codes(ctx, &file, from.as_deref())?,
        AccountCmd::Import {
            file,
            from,
            vat_codes,
        } => {
            let mut changes = input::read_chart(&file)?;
            if let Some(path) = &vat_codes {
                parts::apply_vat_codes(&mut changes, &parts::read_vat_codes(path)?);
            }
            let mut store = ctx.open()?;
            let from = date_or(&store, from.as_deref())?;
            // Ett konto som redan har momskod behåller den om filen inte anger någon.
            let carried = parts::carry_vat_codes(&mut changes, &store.chart(from)?);
            let with_codes = changes.iter().filter(|c| c.vat_code.is_some()).count();
            let n = store.change_chart(&ctx.actor(), from, &changes, &ctx.signer())?;
            println!("{n} konton inlästa, gäller från {from}.");
            if with_codes > 0 {
                println!("{with_codes} av dem har momskod ({carried} behållna från kontoplanen).");
            }
        }
        AccountCmd::Add {
            number,
            name,
            from,
            sru,
            vat_code,
        } => {
            let mut store = ctx.open()?;
            let from = date_or(&store, from.as_deref())?;
            let mut changes = [AccountChange {
                number,
                name,
                active: true,
                sru,
                vat_code,
            }];
            parts::carry_vat_codes(&mut changes, &store.chart(from)?);
            store.change_chart(&ctx.actor(), from, &changes, &ctx.signer())?;
            println!("Konto {number} gäller från {from}.");
        }
        AccountCmd::Deactivate { number, from } => {
            let mut store = ctx.open()?;
            let from = match from {
                Some(s) => input::parse_date(&s)?,
                None => store.today(),
            };
            let current = store
                .chart(from)?
                .into_iter()
                .find(|a| a.number == number)
                .ok_or_else(|| anyhow!("konto {number} finns inte i kontoplanen per {from}"))?;
            let change = AccountChange {
                number,
                name: current.name,
                active: false,
                sru: current.sru,
                vat_code: current.vat_code,
            };
            store.change_chart(&ctx.actor(), from, &[change], &ctx.signer())?;
            println!("Konto {number} är inaktivt från {from}.");
        }
        AccountCmd::List { date } => {
            let store = ctx.open()?;
            let at = match date {
                Some(s) => input::parse_date(&s)?,
                None => store.today(),
            };
            let chart = store.chart(at)?;
            if chart.is_empty() {
                println!("Kontoplanen är tom per {at}. Läs in den med `bok account import`.");
            }
            for a in chart {
                let inactive = if a.active { "" } else { "  (inaktivt)" };
                println!("{} {}{inactive}", a.number, a.name);
            }
        }
    }
    Ok(())
}

fn counterparty(ctx: &Ctx, cmd: CounterpartyCmd) -> Result<()> {
    match cmd {
        CounterpartyCmd::Add {
            name,
            orgnr,
            vatnr,
            address,
        } => {
            let mut store = ctx.open()?;
            let id = store.add_counterparty(
                &ctx.actor(),
                &Counterparty {
                    name,
                    org_number: orgnr,
                    vat_number: vatnr,
                    address,
                },
            )?;
            println!("Motpart {id} tillagd.");
        }
        CounterpartyCmd::Update {
            id,
            name,
            orgnr,
            vatnr,
            address,
        } => {
            let mut store = ctx.open()?;
            let current = store
                .counterparties()?
                .into_iter()
                .find(|r| r.id == id)
                .ok_or_else(|| anyhow!("motpart {id} finns inte"))?
                .counterparty;
            let updated = Counterparty {
                name: name.unwrap_or(current.name),
                org_number: orgnr.or(current.org_number),
                vat_number: vatnr.or(current.vat_number),
                address: address.or(current.address),
            };
            store.update_counterparty(&ctx.actor(), id, &updated)?;
            println!("Motpart {id} ändrad. Bokförda verifikationer behåller sin kopia.");
        }
        CounterpartyCmd::List => {
            for r in ctx.open()?.counterparties()? {
                println!("{:>4}  {}", r.id, render::counterparty(&r.counterparty));
            }
        }
    }
    Ok(())
}

fn attachment(ctx: &Ctx, cmd: AttachmentCmd) -> Result<()> {
    match cmd {
        AttachmentCmd::Add { file, sender } => {
            let mut store = ctx.open()?;
            let bytes = std::fs::read(&file)
                .with_context(|| format!("kan inte läsa {}", file.display()))?;
            let sha = ctx.archive().store(&bytes)?;
            let attachment = NewAttachment {
                sha256: sha.clone(),
                size: bytes.len() as i64,
                channel: Channel::Manual,
                sender,
                original_name: file.file_name().map(|n| n.to_string_lossy().into_owned()),
                file_type: file.extension().map(|e| e.to_string_lossy().to_lowercase()),
            };
            match store.register_attachment(&ctx.actor(), &attachment) {
                Ok(id) => println!("Underlag {id} mottaget: {} ({sha}).", file.display()),
                Err(StoreError::DuplicateAttachment(id)) => {
                    println!("Filen är redan mottagen som underlag {id}.");
                }
                Err(e) => return Err(e.into()),
            }
        }
        AttachmentCmd::List => {
            for a in ctx.open()?.attachments()? {
                println!(
                    "{:>4}  {}  {:<28} {}  {}",
                    a.id,
                    bok_store::display_timestamp(&a.received_at),
                    a.original_name.as_deref().unwrap_or("-"),
                    &a.sha256[..12],
                    a.status_text.as_deref().unwrap_or(&a.status)
                );
            }
        }
    }
    Ok(())
}

fn propose(ctx: &Ctx, args: ProposeArgs) -> Result<()> {
    let proposal = match &args.file {
        Some(path) => input::ProposalFile::read(path)?.into_proposal(),
        None => {
            let lines = args
                .lines
                .iter()
                .map(|s| input::parse_line(s))
                .collect::<Result<Vec<_>>>()?;
            let counterparty = match (args.counterparty, args.counterparty_name.clone()) {
                (Some(id), _) => Some(ProposalCounterparty::Register(id)),
                (None, Some(name)) => Some(ProposalCounterparty::Inline(Counterparty {
                    name,
                    org_number: args.orgnr.clone(),
                    vat_number: args.vatnr.clone(),
                    address: args.counterparty_address.clone(),
                })),
                (None, None) => None,
            };
            NewProposal {
                series: args.series,
                date: input::parse_date(args.date.as_deref().unwrap_or_default())?,
                description: args.text.clone().unwrap_or_default(),
                counterparty,
                lines,
                attachments: args.attachments.clone(),
                idempotency_key: None,
                provenance: Provenance::default(),
            }
        }
    };
    let mut store = ctx.open()?;
    let id = store.create_proposal(&ctx.actor(), &proposal)?;
    println!("Förslag {id} lagt. Det får ett verifikationsnummer först när det bokförs.");
    if args.approve {
        approve(ctx, &mut store, id)?;
    } else {
        println!("Bokför med `bok approve {id}` eller avvisa med `bok reject {id} --reason …`.");
    }
    Ok(())
}

fn proposals(ctx: &Ctx, id: Option<i64>, all: bool) -> Result<()> {
    let store = ctx.open()?;
    if let Some(id) = id {
        return render::proposal(&store, &store.proposal(id)?);
    }
    let rows = store.proposals(!all)?;
    if rows.is_empty() {
        println!("Inga förslag väntar på beslut.");
    }
    for p in &rows {
        render::proposal(&store, p)?;
        println!();
    }
    Ok(())
}

fn approve(ctx: &Ctx, store: &mut Store, id: i64) -> Result<()> {
    let p = store.proposal(id)?;
    render::proposal(store, &p)?;
    let posted = store.approve(&ctx.actor(), id, &ctx.signer())?;
    println!(
        "Bokförd som {} i perioden {} ({}). Kanonisk hash {}.",
        posted.reference, posted.period, posted.date, posted.canonical_hash
    );
    Ok(())
}

fn correct(ctx: &Ctx, args: CorrectArgs) -> Result<()> {
    let mut store = ctx.open()?;
    let y = resolve_year(&store, args.year.as_deref())?;
    let original = store.voucher_by_ref(y.id, args.voucher)?;
    let replacement = args
        .lines
        .iter()
        .map(|s| input::parse_line(s))
        .collect::<Result<Vec<_>>>()?;
    let id = store.propose_correction(&ctx.actor(), original.id, &args.reason, &replacement)?;
    println!(
        "Rättelseförslag {id} lagt för {} ({}).",
        original.reference,
        y.label()
    );
    if args.approve {
        approve(ctx, &mut store, id)?;
    } else {
        render::proposal(&store, &store.proposal(id)?)?;
        println!("Bokför med `bok approve {id}`.");
    }
    Ok(())
}

fn period(ctx: &Ctx, cmd: PeriodCmd) -> Result<()> {
    let (label, to) = match cmd {
        PeriodCmd::List { year } => {
            let store = ctx.open()?;
            let y = resolve_year(&store, year.as_deref())?;
            println!("Perioder {}", y.label());
            for p in store.periods(y.id)? {
                println!("  {}  {} – {}  {}", p.label(), p.start, p.end, p.status);
            }
            return Ok(());
        }
        PeriodCmd::Review { period } => (period, PeriodStatus::UnderReview),
        PeriodCmd::Reopen { period } => (period, PeriodStatus::Open),
        PeriodCmd::Close { period } => (period, PeriodStatus::Closed),
        PeriodCmd::Lock { period } => (period, PeriodStatus::Locked),
    };
    let mut store = ctx.open()?;
    let period = store.period_by_label(&label)?;
    let signer = ctx.signer();
    let signer_ref: Option<&dyn bok_store::bok_core::signature::Signer> = if to.requires_signature()
    {
        Some(&signer)
    } else {
        None
    };
    let t = store.transition_period(&ctx.actor(), period.id, to, signer_ref)?;
    println!("Perioden {} är nu {} (var {}).", t.period, t.to, t.from);
    if t.pending_proposals > 0 && to != PeriodStatus::Open {
        println!(
            "Obs: {} obeslutade förslag har datum i perioden. De kan bara bokföras där medan perioden är öppen.",
            t.pending_proposals
        );
    }
    Ok(())
}

fn journal(ctx: &Ctx, year: Option<&str>, series: Option<Series>) -> Result<()> {
    let store = ctx.open()?;
    let y = resolve_year(&store, year)?;
    println!("Grundbok {} – {}", y.label(), company_line(&store)?);
    println!(
        "Registreringsordning. Inom varje serie är det nummerordning; numret sätts vid bokföringen."
    );
    println!();
    for v in store.journal(y.id)? {
        if series.is_some_and(|s| s != v.reference.series) {
            continue;
        }
        render::voucher(&store, &v, false)?;
        println!();
    }
    Ok(())
}

fn verify(ctx: &Ctx) -> Result<ExitCode> {
    let store = ctx.open()?;
    let mut report = store.verify()?;
    let archive = ctx.archive();
    let mut files = bok_store::Check {
        name: "Underlagen i arkivet är oförändrade",
        ..Default::default()
    };
    for a in store.attachments()? {
        files.checked += 1;
        match archive.check(&a.sha256)? {
            FileCheck::Ok => {}
            FileCheck::Missing => files
                .problems
                .push(format!("underlag {}: filen saknas i arkivet", a.id)),
            FileCheck::Changed => files
                .problems
                .push(format!("underlag {}: filen har ändrats", a.id)),
        }
    }
    report.checks.push(files);
    let mut foreign = bok_store::Check {
        name: "Arkivet innehåller inga främmande filer",
        ..Default::default()
    };
    let (entries, unknown) = archive.unknown_files()?;
    foreign.checked = entries;
    for path in unknown {
        foreign
            .problems
            .push(format!("{} hör inte hemma i arkivet", path.display()));
    }
    report.checks.push(foreign);
    render::report(&report);
    if report.is_ok() {
        println!("Bokföringen är intakt.");
        Ok(ExitCode::SUCCESS)
    } else {
        println!("Verifieringen hittade problem.");
        Ok(ExitCode::FAILURE)
    }
}

fn sie_date(d: NaiveDate) -> String {
    d.format("%Y%m%d").to_string()
}

/// SIE 4 för ett räkenskapsår: #IB, #UB, #RES och verifikationer med
/// registreringsdatum och signatur.
fn sie_bytes(store: &Store, y: &FiscalYear) -> Result<Vec<u8>> {
    let company = store.company()?;
    let opening_balances = store
        .trial_balance(y.id)?
        .into_iter()
        .filter(|r| !r.opening.is_zero())
        .map(|r| (r.account.value(), r.opening.ore()))
        .collect();
    let mut vouchers = Vec::new();
    for v in store.journal(y.id)? {
        // Ingående balans skrivs som #IB, inte som verifikationer.
        if v.reference.series == Series::I {
            continue;
        }
        let mut description = v.description.clone();
        if !v.corrected_by.is_empty() {
            let refs = v
                .corrected_by
                .iter()
                .map(|l| render::link(store, l, v.fiscal_year_id))
                .collect::<Result<Vec<_>>>()?;
            description.push_str(&format!(" (rättad genom {})", refs.join(", ")));
        }
        vouchers.push(Sie4VoucherRecord {
            voucher: Sie4Voucher {
                series: v.reference.series.to_string(),
                number: v.reference.number,
                date: sie_date(v.date),
                description,
                lines: v
                    .lines
                    .iter()
                    .map(|l| Sie4VoucherLine {
                        account_number: l.account.value(),
                        amount: bokforing_core::types::Money::from_ore(l.net().ore()),
                        date: sie_date(v.date),
                        description: l.text.clone().unwrap_or_default(),
                    })
                    .collect(),
            },
            reg_date: Some(sie_date(bok_store::local_date(
                chrono::DateTime::parse_from_rfc3339(&v.registered_at)?.to_utc(),
            ))),
            sign: Some(v.key_name.clone()),
        });
    }
    let export = Sie4Export {
        company: Sie4Company {
            name: company.name,
            org_number: company.org_number,
            fiscal_year_start: sie_date(y.start),
            fiscal_year_end: sie_date(y.end),
        },
        program_name: "Bokis".into(),
        program_version: env!("CARGO_PKG_VERSION").into(),
        generated: sie_date(store.today()),
        accounts: store
            .chart(y.end)?
            .into_iter()
            .map(|a| Sie4Account {
                number: a.number.value(),
                name: a.name,
                sru_code: a.sru,
            })
            .collect(),
        opening_balances,
        vouchers,
    };
    Ok(bokforing_export::sie4::export_year(&export))
}

fn export_sie(ctx: &Ctx, year: Option<&str>, out: &Path) -> Result<()> {
    let mut store = ctx.open()?;
    let y = resolve_year(&store, year)?;
    let bytes = sie_bytes(&store, &y)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(out)
        .with_context(|| {
            format!(
                "kan inte skapa {} (en befintlig fil skrivs aldrig över)",
                out.display()
            )
        })?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    let sha = sha256_hex(&bytes);
    store.record_export(
        &ctx.actor(),
        "SIE 4",
        y.id,
        &out.display().to_string(),
        &sha,
    )?;
    println!(
        "SIE 4 för {} skriven till {} ({sha}).",
        y.label(),
        out.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::Cli;

    /// Gapanalysen: `restore_backup` och `restore_from_file` skrev över den
    /// levande databasen. `bok` har inga kommandon som återställer,
    /// nollställer eller raderar.
    #[test]
    fn no_command_restores_resets_or_deletes() {
        let forbidden = [
            "restore",
            "reset",
            "delete",
            "remove",
            "drop",
            "purge",
            "wipe",
            "truncate",
            "overwrite",
            "import-db",
            "aterstall",
            "nollstall",
            "radera",
            "ta-bort",
            "rensa",
        ];
        fn walk(cmd: &clap::Command, path: &str, forbidden: &[&str], found: &mut Vec<String>) {
            for sub in cmd.get_subcommands() {
                let names: Vec<&str> = std::iter::once(sub.get_name())
                    .chain(sub.get_all_aliases())
                    .collect();
                for n in names {
                    if forbidden.iter().any(|f| n.contains(f)) {
                        found.push(format!("{path} {n}"));
                    }
                }
                walk(sub, &format!("{path} {}", sub.get_name()), forbidden, found);
            }
        }
        let cli = Cli::command();
        cli.clone().debug_assert();
        let mut found = Vec::new();
        walk(&cli, "bok", &forbidden, &mut found);
        assert!(found.is_empty(), "förbjudna kommandon: {found:?}");
    }
}
