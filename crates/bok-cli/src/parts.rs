//! Kommandona för inkorgen, kopiorna och bankfilerna (steg 2 och 4).
//!
//! - `bok inkorg <mapp>`: läser nya filer, sparar dem oförändrade i arkivet,
//!   registrerar dem som underlag och flyttar dem först därefter till
//!   `<mapp>/inlasta`. Avbryts något ligger filen kvar och läses nästa gång.
//! - `bok kopia skapa|kontrollera|lista|las-in`: kopior till en extern plats.
//!   `las-in` skriver bara till en ny, tom katalog och verifierar kopian där;
//!   den levande bokföringen rörs aldrig.
//! - `bok bank importera|rader|forslag|matcha`: kontoutdraget tas emot som
//!   underlag och raderna läses in (en gång, även om filen läses igen).
//!   Avstämningen görs av dig, mot en bokförd verifikation med rätt belopp
//!   eller med en förklaring. En period med rader som inte är avstämda kan
//!   inte stängas.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand, ValueEnum};

use bok_store::bok_core::{AccountNumber, Money, VoucherRef};
use bok_store::{
    Actor, AttachmentVerdict, CaseKind, Channel, MatchTarget, NewAttachment, NewBankRow, Store,
    StoreError, SuggestedMatch,
};

use crate::{Ctx, PROGRAM, render};

#[derive(Args)]
pub struct InboxArgs {
    /// Mappen som bevakas, till exempel en synkad katalog från telefonen.
    folder: PathBuf,
    /// Hit flyttas inlästa filer [standard: <mapp>/inlasta].
    #[arg(long)]
    done: Option<PathBuf>,
    /// Filer som ändrats senare än så här många sekunder sedan väntar till nästa gång.
    #[arg(long, default_value_t = 2)]
    min_age: u64,
}

#[derive(Subcommand)]
pub enum BackupCmd {
    /// Skriver en konsistent kopia av databasen och nya underlag till målet, med manifest.
    #[command(visible_alias = "skapa")]
    Create {
        /// Målet, till exempel en monterad USB-disk.
        target: PathBuf,
    },
    /// Kontrollerar en kopia mot dess manifest och databasens integritet.
    #[command(visible_alias = "kontrollera")]
    Check {
        /// Kopians katalog, `<mål>/kopior/<tid>`.
        backup: PathBuf,
    },
    /// Visar kopiorna under ett mål, äldst först.
    #[command(visible_alias = "lista")]
    List { target: PathBuf },
    /// Läser in en kopia i en ny, tom katalog och verifierar bokföringen där.
    #[command(visible_alias = "las-in")]
    Readback {
        /// Kopians katalog, `<mål>/kopior/<tid>`.
        backup: PathBuf,
        /// En katalog som inte finns eller är tom.
        new_dir: PathBuf,
    },
}

#[derive(Subcommand)]
pub enum BankCmd {
    /// Tar emot ett kontoutdrag (camt.053 eller CSV) som underlag och läser in raderna.
    #[command(visible_alias = "importera")]
    Import {
        file: PathBuf,
        /// För CSV: bankkontot. En export från Lunar Business känns igen på rubrikraden; annars är
        /// formatet datum;text;belopp;saldo med rubrikrad.
        #[arg(long)]
        csv_account: Option<String>,
        /// Bokföringskontot som banken motsvarar.
        #[arg(long, default_value = "1930")]
        account: AccountNumber,
        /// För en fil med utdrag för flera bankkonton: BANKKONTO=KONTO för vart och ett.
        #[arg(long = "account-for", value_name = "BANKKONTO=KONTO")]
        account_for: Vec<String>,
    },
    /// Visar bankraderna som inte är avstämda, eller alla.
    #[command(visible_alias = "rader")]
    Rows {
        #[arg(long)]
        all: bool,
    },
    /// Stämmer av en bankrad mot en bokförd verifikation, eller med en förklaring.
    #[command(visible_alias = "matcha")]
    Match {
        /// Bankradens nummer (se `bok bank rader`).
        row: i64,
        /// Verifikationen, till exempel A12.
        voucher: Option<VoucherRef>,
        /// Räkenskapsåret för verifikationen [standard: året för bankradens dag].
        #[arg(long)]
        year: Option<String>,
        /// En förklaring i stället för en verifikation.
        #[arg(long, conflicts_with = "voucher")]
        explanation: Option<String>,
    },
    /// Visar möjliga verifikationer och agentens förslag för raderna som inte är avstämda.
    #[command(visible_alias = "forslag")]
    Suggestions {
        /// Bara den här bankraden.
        row: Option<i64>,
        /// Hur många dagar från bankradens dag en verifikation får ligga.
        #[arg(long, default_value_t = 14)]
        days: i64,
    },
}

fn register(store: &mut Store, actor: &Actor, a: &NewAttachment) -> Result<(i64, bool)> {
    match store.register_attachment(actor, a) {
        Ok(id) => Ok((id, true)),
        Err(StoreError::DuplicateAttachment(id)) => Ok((id, false)),
        Err(e) => Err(e.into()),
    }
}

pub fn inbox(ctx: &Ctx, args: InboxArgs) -> Result<()> {
    let mut store = ctx.open()?;
    let archive = ctx.archive().open()?;
    let done = args.done.unwrap_or_else(|| args.folder.join("inlasta"));
    let options = bok_intake::ScanOptions {
        min_age: Duration::from_secs(args.min_age),
        done_dir: None,
    };
    let received = bok_intake::scan_folder(&args.folder, &archive, &options)
        .with_context(|| format!("kan inte läsa {}", args.folder.display()))?;
    if received.is_empty() {
        println!("Inga nya filer i {}.", args.folder.display());
        return Ok(());
    }
    let actor = Actor::system(format!("inkorg ({})", ctx.user));
    let (mut new, mut known) = (0, 0);
    for r in received {
        let data = archive.read(&r.sha256)?;
        let channel = if bok_intake::looks_like_camt053(&data) {
            Channel::Bank
        } else {
            Channel::Folder
        };
        let attachment = NewAttachment {
            sha256: r.sha256.clone(),
            size: i64::try_from(r.size).context("filen är för stor")?,
            channel,
            sender: None,
            original_name: Some(r.file_name.clone()),
            file_type: Some(r.media_type.clone()),
        };
        let (id, is_new) = register(&mut store, &actor, &attachment)?;
        // Flyttas först när filen är både arkiverad och registrerad.
        let moved = bok_intake::move_to(&r.path_after, &done)?;
        if is_new {
            new += 1;
            println!(
                "Underlag {id}: {} ({}, {})",
                render::safe(&r.file_name),
                r.media_type,
                &r.sha256[..12]
            );
        } else {
            known += 1;
            println!(
                "{}: samma innehåll finns redan som underlag {id}; flyttad till {}",
                render::safe(&r.file_name),
                moved.display()
            );
        }
    }
    println!(
        "{new} nya underlag, {known} redan mottagna. Filerna ligger nu i {}.",
        done.display()
    );
    Ok(())
}

/// Underlagspoolen hör till målet: `<mål>/underlag` bredvid `<mål>/kopior`.
fn pool_for(backup: &Path) -> Result<PathBuf> {
    let target = backup
        .parent()
        .and_then(Path::parent)
        .with_context(|| format!("{} är ingen kopia (<mål>/kopior/<tid>)", backup.display()))?;
    Ok(target.join("underlag"))
}

pub fn backup(ctx: &Ctx, cmd: BackupCmd) -> Result<()> {
    match cmd {
        BackupCmd::Create { target } => {
            let mut store = ctx.open()?;
            let chain_head = store.events(Some(1))?.last().map(|e| e.hash.clone());
            let meta = bok_backup::BackupMeta {
                created: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                program_version: PROGRAM.to_string(),
                chain_head,
            };
            let archive = ctx.archive().open()?;
            let report = bok_backup::create_backup(&ctx.db, &archive, &target, &meta)?;
            println!("Kopia skriven till {}.", report.dir.display());
            println!(
                "Databasen {} byte, {} nya underlag kopierade, {} fanns redan på målet.",
                report.database.size, report.documents_copied, report.documents_existing
            );
            println!("Manifestets SHA-256: {}", report.manifest_sha256);
            match store.fiscal_year_for_date(store.today())? {
                Some(year) => store.record_export(
                    &ctx.actor(),
                    "kopia",
                    year.id,
                    &report.dir.display().to_string(),
                    &report.manifest_sha256,
                )?,
                None => println!(
                    "Obs: inget räkenskapsår omfattar i dag; kopian loggades inte i historiken."
                ),
            }
        }
        BackupCmd::Check { backup } => {
            let report = bok_backup::verify_backup(&backup, &pool_for(&backup)?)?;
            for e in &report.errors {
                println!("FEL  {e}");
            }
            if !report.is_intact() {
                bail!("kopian {} är skadad", backup.display());
            }
            println!(
                "Kopian är hel: databasen och {} underlag stämmer med manifestet.",
                report.documents_ok
            );
        }
        BackupCmd::List { target } => {
            let list = bok_backup::list_backups(&target)?;
            if list.is_empty() {
                println!("Inga kopior under {}.", target.display());
            }
            for b in list {
                println!("{}", b.display());
            }
        }
        BackupCmd::Readback { backup, new_dir } => {
            let restored = bok_backup::restore_into_empty(&backup, &pool_for(&backup)?, &new_dir)?;
            println!(
                "Kopian är inläst i {} med {} underlag.",
                new_dir.display(),
                restored.documents
            );
            let store = Store::open(&restored.database, PROGRAM)?;
            let mut report = store.verify()?;
            let archive = bok_archive::Archive::open(&restored.archive)?;
            let mut files = bok_store::Check {
                name: "Underlagen i kopian är oförändrade",
                ..Default::default()
            };
            for a in store.attachments()? {
                files.checked += 1;
                if let Err(e) = archive.read(&a.sha256) {
                    files.problems.push(format!("underlag {}: {e}", a.id));
                }
            }
            report.checks.push(files);
            render::report(&report);
            if !report.is_ok() {
                bail!("den inlästa kopian har fel");
            }
            println!(
                "Den inlästa bokföringen är intakt. Öppna den med --db {}.",
                restored.database.display()
            );
        }
    }
    Ok(())
}

fn money(ore: i64) -> String {
    render::amount(Money::from_ore(ore))
}

pub fn bank(ctx: &Ctx, cmd: BankCmd) -> Result<()> {
    match cmd {
        BankCmd::Import {
            file,
            csv_account,
            account,
            account_for,
        } => bank_import(ctx, &file, csv_account, account, &account_for),
        BankCmd::Rows { all } => {
            let store = ctx.open()?;
            let rows = store.bank_rows(!all)?;
            if rows.is_empty() {
                println!(
                    "{}",
                    if all {
                        "Inga bankrader är inlästa."
                    } else {
                        "Alla bankrader är avstämda."
                    }
                );
            }
            for r in &rows {
                println!("{}", row_line(&store, r)?);
            }
            Ok(())
        }
        BankCmd::Match {
            row,
            voucher,
            year,
            explanation,
        } => {
            let mut store = ctx.open()?;
            let target = match (voucher, explanation) {
                (Some(reference), None) => {
                    let bank_row = store.bank_row(row)?;
                    let fiscal_year = match year {
                        Some(y) => crate::resolve_year(&store, Some(&y))?,
                        None => store
                            .fiscal_year_for_date(bank_row.booking_date)?
                            .with_context(|| {
                                format!(
                                    "inget räkenskapsår omfattar {}; ange --year",
                                    bank_row.booking_date
                                )
                            })?,
                    };
                    let v = store.voucher_by_ref(fiscal_year.id, reference)?;
                    MatchTarget::Voucher(v.id)
                }
                (None, Some(text)) => MatchTarget::Explanation(text),
                _ => bail!("ange en verifikation, till exempel A12, eller --explanation"),
            };
            store.reconcile_bank_row(&ctx.actor(), row, &target)?;
            println!("Bankrad {row} är avstämd.");
            Ok(())
        }
        BankCmd::Suggestions { row, days } => {
            let store = ctx.open()?;
            let rows: Vec<_> = match row {
                Some(id) => vec![store.bank_row(id)?],
                None => store.bank_rows(true)?,
            };
            for r in &rows {
                println!("{}", row_line(&store, r)?);
                let candidates = store.match_candidates(r.id, days)?;
                if candidates.is_empty() {
                    println!("      ingen bokförd verifikation med samma belopp inom {days} dagar");
                }
                for c in candidates.iter().take(3) {
                    println!(
                        "      {:<6} {}  {}  ({} dagar, poäng {})",
                        c.reference.to_string(),
                        c.date,
                        render::safe(&c.description),
                        c.days_apart,
                        c.score
                    );
                }
                for s in store.match_suggestions(Some(r.id))? {
                    let target = match &s.target {
                        SuggestedMatch::Voucher(id) => {
                            format!("verifikation {}", store.voucher(*id)?.reference)
                        }
                        SuggestedMatch::Proposal(id) => {
                            match store.proposal(*id)?.decision.and_then(|d| d.voucher_id) {
                                Some(v) => format!(
                                    "förslag {id}, bokfört som {}",
                                    store.voucher(v)?.reference
                                ),
                                None => format!("förslag {id} (inte bokfört än)"),
                            }
                        }
                        SuggestedMatch::Explanation(t) => {
                            format!("förklaring: {}", render::safe(t))
                        }
                    };
                    println!(
                        "      {} föreslår {target}{}",
                        render::safe(&s.actor),
                        s.rationale
                            .as_deref()
                            .map(|r| format!(" – {}", render::safe(r)))
                            .unwrap_or_default()
                    );
                }
            }
            Ok(())
        }
    }
}

fn row_line(store: &Store, r: &bok_store::BankRowRecord) -> Result<String> {
    let status = match &r.reconciliation {
        None => "ej avstämd".to_string(),
        Some(rec) => match (rec.voucher_id, &rec.explanation) {
            (Some(v), _) => format!("avstämd mot {}", store.voucher(v)?.reference),
            (None, Some(e)) => format!("förklarad: {}", render::safe(e)),
            (None, None) => "avstämd".to_string(),
        },
    };
    Ok(format!(
        "{:>4}  {}  {:>14}  {:<40}  {}",
        r.id,
        r.booking_date,
        render::amount(r.amount),
        render::safe(&r.text),
        status
    ))
}

fn bank_import(
    ctx: &Ctx,
    file: &Path,
    csv_account: Option<String>,
    account: AccountNumber,
    account_for: &[String],
) -> Result<()> {
    let data = std::fs::read(file).with_context(|| format!("kan inte läsa {}", file.display()))?;
    let (statements, file_type) = if bok_intake::looks_like_camt053(&data) {
        (bok_bank::parse_camt053(&data)?, "application/xml")
    } else if let Some(bank_account) = csv_account {
        let format = bok_bank::CsvFormat::detect(&data, bank_account.clone());
        println!("{} läses som {}.", file.display(), format.name);
        let rows = bok_bank::parse_csv(&data, &format)?;
        let balances = bok_bank::balances_from_rows(&rows, format.order);
        let statement = bok_bank::Statement {
            account: bank_account,
            currency: format.currency.clone(),
            opening_balance_ore: balances.map(|(opening, _)| opening),
            closing_balance_ore: balances.map(|(_, closing)| closing),
            rows,
        };
        (vec![statement], "text/csv")
    } else {
        bail!(
            "{} är inget camt.053-utdrag; ange --csv-account för en CSV-fil",
            file.display()
        );
    };

    // Vilket bokföringskonto varje bankkonto i filen motsvarar. Har filen utdrag
    // för flera bankkonton måste vart och ett anges; annars hamnar allt på ett konto.
    let mut mapping: Vec<(String, AccountNumber)> = Vec::new();
    for spec in account_for {
        let (bank, ledger) = spec
            .split_once('=')
            .with_context(|| format!("--account-for {spec}: skriv BANKKONTO=KONTO"))?;
        let ledger: AccountNumber = ledger
            .trim()
            .parse()
            .with_context(|| format!("--account-for {spec}"))?;
        mapping.push((bank.trim().to_string(), ledger));
    }
    let bank_accounts: std::collections::BTreeSet<&str> =
        statements.iter().map(|s| s.account.as_str()).collect();
    let unmapped: Vec<&str> = bank_accounts
        .iter()
        .copied()
        .filter(|b| !mapping.iter().any(|(m, _)| m == b))
        .collect();
    if bank_accounts.len() > 1 && !unmapped.is_empty() {
        bail!(
            "filen har utdrag för flera bankkonton ({}); ange bokföringskonto för vart och ett, till exempel --account-for {}=1930",
            bank_accounts.iter().copied().collect::<Vec<_>>().join(", "),
            unmapped[0]
        );
    }
    let ledger_for = |bank: &str| {
        mapping
            .iter()
            .find(|(m, _)| m == bank)
            .map_or(account, |(_, l)| *l)
    };

    let mut store = ctx.open()?;
    let sha256 = ctx.archive().store(&data)?;
    let attachment = NewAttachment {
        sha256: sha256.clone(),
        size: i64::try_from(data.len()).context("filen är för stor")?,
        channel: Channel::Bank,
        sender: None,
        original_name: file.file_name().map(|n| n.to_string_lossy().into_owned()),
        file_type: Some(file_type.into()),
    };
    let (id, is_new) = register(&mut store, &ctx.actor(), &attachment)?;
    if is_new {
        println!(
            "Kontoutdraget är mottaget som underlag {id} ({}).",
            &sha256[..12]
        );
    } else {
        println!("Kontoutdraget finns redan som underlag {id}.");
    }

    let mut groups: Vec<(AccountNumber, Vec<NewBankRow>)> = Vec::new();
    for s in &statements {
        let ledger = ledger_for(&s.account);
        if !groups.iter().any(|(l, _)| *l == ledger) {
            groups.push((ledger, Vec::new()));
        }
        let sum: i64 = s.rows.iter().map(|r| r.amount_ore).sum();
        println!();
        println!(
            "Konto {} ({}): {} rader, summa {}",
            render::safe(&s.account),
            s.currency,
            s.rows.len(),
            money(sum)
        );
        if let (Some(o), Some(c)) = (s.opening_balance_ore, s.closing_balance_ore) {
            println!("Ingående saldo {}, utgående saldo {}", money(o), money(c));
        }
        match s.balances_match() {
            Some(true) => {
                println!("Saldokontroll: ingående saldo plus raderna ger utgående saldo.")
            }
            Some(false) => println!("Saldokontroll: STÄMMER INTE. Kontrollera att filen är hel."),
            None if s.rows.is_empty() => println!("Filen har inga rader för perioden."),
            None => println!("Saldokontroll: filen saknar saldon."),
        }
        for r in &s.rows {
            println!(
                "  {}  {:>14}  {}{}",
                r.booking_date,
                money(r.amount_ore),
                render::safe(&r.text),
                r.reference
                    .as_deref()
                    .map(|x| format!("  ref {}", render::safe(x)))
                    .unwrap_or_default()
            );
            let group = groups
                .iter_mut()
                .find(|(l, _)| *l == ledger)
                .map(|(_, rows)| rows)
                .context("intern fel: saknad grupp")?;
            group.push(NewBankRow {
                post_id: r.entry_id.clone(),
                bank_account: r.account.clone(),
                booking_date: r.booking_date,
                value_date: r.value_date,
                amount: Money::from_ore(r.amount_ore),
                currency: r.currency.clone(),
                text: r.text.clone(),
                reference: r.reference.clone(),
                counterparty: r.counterparty.clone(),
                balance: r.balance_ore.map(Money::from_ore),
            });
        }
    }
    println!();
    for (ledger, rows) in &groups {
        let imported = store.import_bank_rows(&ctx.actor(), id, *ledger, rows)?;
        println!(
            "{} nya bankrader inlästa mot konto {ledger}, {} fanns redan.",
            imported.new, imported.existing
        );
    }
    println!("Stäm av med `bok bank forslag` och `bok bank matcha`.");
    Ok(())
}

#[derive(Subcommand)]
pub enum CaseCmd {
    /// Visar agentens frågor och flaggor som väntar på svar, eller alla.
    #[command(visible_alias = "lista")]
    List {
        #[arg(long)]
        all: bool,
    },
    /// Besvarar ett ärende. Svaret på en flagga kan också ändra underlagets status.
    #[command(visible_alias = "svara")]
    Answer {
        id: i64,
        /// Svaret, som sparas i historiken.
        answer: String,
        /// Ändra underlagets status (bara för flaggor på underlag som inte är bokförda).
        #[arg(long, value_enum)]
        mark: Option<Mark>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Mark {
    /// Underlaget är inte räkenskapsinformation.
    #[value(name = "ej-rakenskapsinformation")]
    NotAccountingInformation,
    /// Underlaget är en dubblett av ett annat.
    #[value(name = "dubblett")]
    Duplicate,
}

pub fn case(ctx: &Ctx, cmd: CaseCmd) -> Result<()> {
    match cmd {
        CaseCmd::List { all } => {
            let store = ctx.open()?;
            let cases = store.cases(!all)?;
            if cases.is_empty() {
                println!(
                    "{}",
                    if all {
                        "Inga ärenden."
                    } else {
                        "Inga ärenden väntar på svar."
                    }
                );
            }
            for c in cases {
                let what = match &c.kind {
                    CaseKind::Question => "fråga".to_string(),
                    CaseKind::Flag { reason } => format!("flagga ({reason})"),
                };
                let on = c
                    .attachment_id
                    .map(|a| format!("underlag {a}"))
                    .unwrap_or_else(|| "allmän".into());
                println!(
                    "{:>4}  {what:<36} {on:<12} {} {}",
                    c.id,
                    render::safe(&c.actor),
                    bok_store::display_timestamp(&c.created_at)
                );
                println!("      {}", render::safe(&c.text));
                if let Some(a) = c.answer {
                    println!(
                        "      svar av {}: {}{}",
                        render::safe(&a.actor),
                        render::safe(&a.text),
                        a.status
                            .map(|s| format!(" [underlaget: {s}]"))
                            .unwrap_or_default()
                    );
                }
            }
            Ok(())
        }
        CaseCmd::Answer { id, answer, mark } => {
            let mut store = ctx.open()?;
            let verdict = mark.map(|m| match m {
                Mark::NotAccountingInformation => AttachmentVerdict::NotAccountingInformation,
                Mark::Duplicate => AttachmentVerdict::Duplicate,
            });
            store.answer_case(&ctx.actor(), id, &answer, verdict)?;
            match verdict {
                Some(v) => println!(
                    "Ärende {id} är besvarat och underlaget markerat som {}.",
                    v.as_db_str()
                ),
                None => println!("Ärende {id} är besvarat."),
            }
            Ok(())
        }
    }
}

/// Läser en fil med KONTO;RUTA per rad. Tomma rader och rader som börjar
/// med `//` eller `#` hoppas över. RUTA måste vara en ruta i momsdeklarationen.
pub fn read_vat_codes(file: &Path) -> Result<Vec<(AccountNumber, String)>> {
    let text = std::fs::read_to_string(file)
        .with_context(|| format!("kan inte läsa {}", file.display()))?;
    let mut out = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('#') {
            continue;
        }
        let (number, code) = line
            .split_once([';', '\t', ' '])
            .with_context(|| format!("rad {}: skriv KONTO;RUTA", i + 1))?;
        let number: AccountNumber = number
            .trim()
            .parse()
            .with_context(|| format!("rad {}", i + 1))?;
        let code = code.trim();
        if bok_store::box_title(code).is_none() {
            bail!("rad {}: {code} är ingen ruta i momsdeklarationen", i + 1);
        }
        out.push((number, code.to_string()));
    }
    Ok(out)
}

/// Sätter momskoderna på konton som läses in. Konton i filen som inte läses
/// in hoppas över.
pub fn apply_vat_codes(
    changes: &mut [bok_store::AccountChange],
    codes: &[(AccountNumber, String)],
) -> usize {
    let mut n = 0;
    for c in changes.iter_mut() {
        if let Some((_, code)) = codes.iter().find(|(a, _)| *a == c.number) {
            c.vat_code = Some(code.clone());
            n += 1;
        }
    }
    n
}

/// Behåller kontoplanens momskod för konton som läses in utan momskod, så att
/// en ny version av kontot inte tyst faller ur momsrapporten. Ger antalet.
pub fn carry_vat_codes(
    changes: &mut [bok_store::AccountChange],
    current: &[bok_store::AccountDef],
) -> usize {
    let mut n = 0;
    for c in changes.iter_mut().filter(|c| c.vat_code.is_none()) {
        if let Some(code) = current
            .iter()
            .find(|a| a.number == c.number)
            .and_then(|a| a.vat_code.clone())
        {
            c.vat_code = Some(code);
            n += 1;
        }
    }
    n
}

/// Ändrar kontonas momskod med en signerad ändring av kontoplanen, från
/// `from` (standard i dag). Namn och övriga uppgifter behålls. Momskoder som
/// ska gälla från början läses in med kontoplanen (`bok konto importera
/// --momskoder`), eftersom en ändring bara gäller verifikationer från sitt
/// datum.
pub fn vat_codes(ctx: &Ctx, file: &Path, from: Option<&str>) -> Result<()> {
    let codes = read_vat_codes(file)?;
    let mut store = ctx.open()?;
    let from = match from {
        Some(s) => crate::input::parse_date(s)?,
        None => store.today(),
    };
    let chart = store.chart(from)?;
    let mut changes = Vec::new();
    let mut missing = Vec::new();
    for (number, code) in codes {
        match chart.iter().find(|a| a.number == number) {
            Some(a) => changes.push(bok_store::AccountChange {
                number,
                name: a.name.clone(),
                active: a.active,
                sru: a.sru.clone(),
                vat_code: Some(code),
            }),
            None => missing.push(number.to_string()),
        }
    }
    if changes.is_empty() {
        bail!(
            "inga av kontona i {} finns i kontoplanen per {from}",
            file.display()
        );
    }
    let n = store.change_chart(&ctx.actor(), from, &changes, &ctx.signer())?;
    println!("Momskod satt på {n} konton, gäller från {from}.");
    if !missing.is_empty() {
        println!(
            "Finns inte i kontoplanen och hoppades över: {}",
            missing.join(", ")
        );
    }
    Ok(())
}

/// `2026-03`, `2026-01:2026-03` eller `2026`.
fn vat_period(spec: &str) -> Result<(chrono::NaiveDate, chrono::NaiveDate)> {
    use chrono::{Datelike, NaiveDate};
    let month = |s: &str| -> Result<(NaiveDate, NaiveDate)> {
        let start = NaiveDate::parse_from_str(&format!("{}-01", s.trim()), "%Y-%m-%d")
            .with_context(|| format!("{s} är ingen månad (ÅÅÅÅ-MM)"))?;
        let next = if start.month() == 12 {
            NaiveDate::from_ymd_opt(start.year() + 1, 1, 1)
        } else {
            NaiveDate::from_ymd_opt(start.year(), start.month() + 1, 1)
        }
        .context("ogiltig månad")?;
        Ok((start, next.pred_opt().context("ogiltig månad")?))
    };
    let spec = spec.trim();
    if let Some((a, b)) = spec.split_once(':') {
        let (from, _) = month(a)?;
        let (_, to) = month(b)?;
        if from > to {
            bail!("perioden {spec} slutar före den börjar");
        }
        return Ok((from, to));
    }
    if spec.len() == 4 && spec.chars().all(|c| c.is_ascii_digit()) {
        let year: i32 = spec.parse()?;
        return Ok((
            NaiveDate::from_ymd_opt(year, 1, 1).context("ogiltigt år")?,
            NaiveDate::from_ymd_opt(year, 12, 31).context("ogiltigt år")?,
        ));
    }
    month(spec)
}

pub fn vat_report(ctx: &Ctx, period: &str) -> Result<()> {
    let (from, to) = vat_period(period)?;
    let store = ctx.open()?;
    let report = store.vat_report(from, to)?;
    println!(
        "Momsrapport {from} – {to}, {}",
        crate::company_line(&store)?
    );
    println!("Bokförda verifikationer, per ruta enligt kontoplanens momskoder.");
    println!();
    for b in &report.boxes {
        println!(
            "Ruta {}  {:<58} {:>14}",
            b.code,
            b.title,
            render::amount(b.amount)
        );
        for (account, amount) in &b.accounts {
            println!("           konto {account} {:>48}", render::amount(*amount));
        }
    }
    println!();
    if report.to_pay.is_negative() {
        println!("Moms att få tillbaka: {}", render::amount(-report.to_pay));
    } else {
        println!("Moms att betala: {}", render::amount(report.to_pay));
    }
    if report.settlements > 0 {
        println!(
            "{} verifikation(er) med momsavräkning (konto med momskod 49) räknades inte med.",
            report.settlements
        );
    }
    for note in &report.notes {
        println!("Obs: {note}");
    }
    for (code, account) in &report.unknown_codes {
        println!(
            "Obs: konto {account} har momskod {code}, som inte är någon ruta; det räknades inte med."
        );
    }
    if report.boxes.len() == 1 {
        println!(
            "Obs: inga belopp. Har kontona momskoder? Läs in dem med `bok konto importera <kontoplan> --vat-codes <fil>`."
        );
    }
    Ok(())
}
