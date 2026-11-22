//! Systemdokumentationen, genererad ur bokföringen själv.
//!
//! Dokumentet beskriver det som faktiskt gäller i databasen: spärrarna som
//! de står i schemat, agentens och granskarens operationer ur
//! operationsregistret, nycklarna, perioderna, kontoplanens momskoder,
//! företagets arkivplan och resultatet av en full verifiering vid
//! framtagandet. `bok
//! systemdokumentation` skriver det som Markdown, och insynsvyn visar det.

use std::fmt::Write as _;
use std::path::Path;

use bok_archive::Archive;
use bok_mcp::registry::{self, Risk, Role};
use bok_store::bok_core::Series;
use bok_store::{Store, VerifyReport, APPEND_ONLY_TABLES};

use crate::Result;

fn risk(r: Risk) -> &'static str {
    match r {
        Risk::Read => "läser",
        Risk::Propose => "föreslår",
        Risk::Post => "bokför eller ändrar",
    }
}

/// Dokumentet som Markdown.
pub fn render(
    store: &Store,
    archive: &Archive,
    db: &Path,
    program: &str,
    generated: &str,
) -> Result<String> {
    let report = crate::verify_all(store, archive)?;
    render_with(store, db, program, generated, &report, generated)
}

/// Dokumentet med en kontroll som redan är gjord.
pub fn render_with(
    store: &Store,
    db: &Path,
    program: &str,
    generated: &str,
    report: &VerifyReport,
    checked: &str,
) -> Result<String> {
    let mut d = String::new();
    let company = store.company()?;
    let w = &mut d;
    writeln!(w, "# Systemdokumentation för Bokis")?;
    writeln!(w)?;
    writeln!(w, "{} ({})", company.name, company.org_number)?;
    writeln!(w)?;
    writeln!(
        w,
        "Framtagen {generated} av {program}, schemaversion {}, ur databasen `{}`. \
         Dokumentet genereras ur bokföringen och beskriver det som gäller i den, inte hur \
         programmet är tänkt att fungera.",
        bok_store::SCHEMA_VERSION,
        db.display()
    )?;

    writeln!(w)?;
    writeln!(w, "## Ansvar och arbetsfördelning")?;
    writeln!(w)?;
    writeln!(
        w,
        "Bokföringen förs enligt bokföringslagen (1999:1078) och BFNAR 2013:2. En AI-agent får läsa \
         underlag, kontoplan, regler och tidigare konteringar och lägga förslag, frågor och flaggor. \
         Bara en människa bokför: varje bokföring, periodstängning och ändring av kontoplanen \
         signeras med en registrerad nyckel, och säkerhetsnivån avgör vilka nycklar som godtas \
         (se Nycklar som får signera). Ett förslag har inget verifikationsnummer; numret tas ur \
         serien i samma transaktion som verifikationen skrivs, så serien får inga luckor."
    )?;

    writeln!(w)?;
    writeln!(w, "## Programdelar")?;
    writeln!(w)?;
    writeln!(w, "| Del | Uppgift |")?;
    writeln!(w, "| --- | --- |")?;
    for (part, task) in [
        (
            "bok-store",
            "Databasen (SQLite) med spärrar; varje skrivning i en transaktion med en hashkedjad händelse",
        ),
        (
            "bok",
            "Kommandoradsverktyget för den som granskar, bokför, stämmer av, stänger perioder och tar kopior",
        ),
        (
            "bok-archive",
            "Underlagsarkivet: varje fil byte för byte under sin SHA-256, skrivskyddad",
        ),
        (
            "bok-mcp",
            "MCP-servern som ger agenten sina verktyg; den har inga verktyg för att bokföra",
        ),
        (
            "bok-review",
            "Granskningsvyn på 127.0.0.1, där ett godkännande signeras med nyckeln",
        ),
        (
            "bok-insyn",
            "Insynen för revisor och Skatteverket: bokföringen som läsbara sidor, live eller som fristående export",
        ),
        (
            "bok-fjarr",
            "Bokis på distans: agenten ansluter över HTTPS med OAuth och kopplingskod, och telefonen skickar underlag med en personlig länk",
        ),
        (
            "bok-backup",
            "Kopior till extern plats med manifest, och inläsning bara till en tom katalog",
        ),
    ] {
        writeln!(w, "| `{part}` | {task} |")?;
    }

    writeln!(w)?;
    writeln!(w, "## Behörigheter")?;
    writeln!(w)?;
    writeln!(
        w,
        "Operationerna är definierade i ett register. Agenten når bara sina egna; övriga finns inte för den."
    )?;
    writeln!(w)?;
    writeln!(w, "| Operation | Vem | Vad den gör |")?;
    writeln!(w, "| --- | --- | --- |")?;
    for op in registry::all() {
        let who = match op.role {
            Role::Agent => "agenten",
            Role::Reviewer => "granskaren",
        };
        writeln!(
            w,
            "| `{}` | {who} ({}) | {} |",
            op.name,
            risk(op.risk),
            op.description
        )?;
    }

    writeln!(w)?;
    writeln!(w, "## Nycklar som får signera")?;
    writeln!(w)?;
    let level = store.security_level()?;
    let since = match store.security_decisions()?.last() {
        Some(d) => format!("beslutad {}", bok_store::display_timestamp(&d.decided)),
        None => "bokföringen skapades före nivåerna och har inget beslut".to_string(),
    };
    writeln!(
        w,
        "Säkerhetsnivå: {} ({since}). {} Varje beslut om nivån är signerat; den som \
         höjer den signerar med en nyckel för den nya nivån och den som sänker den med en nyckel \
         för den gamla.",
        level.title(),
        level.meaning()
    )?;
    writeln!(w)?;
    for k in store.keys()? {
        writeln!(
            w,
            "- {}: {} ({}{}), registrerad {}",
            k.name,
            k.fingerprint,
            k.algorithm,
            if k.hardware {
                ", hårdvarunyckel"
            } else {
                ", mjukvarunyckel"
            },
            bok_store::display_timestamp(&k.registered_at)
        )?;
    }

    writeln!(w)?;
    writeln!(w, "## Räkenskapsår, perioder och serier")?;
    writeln!(w)?;
    for y in store.fiscal_years()? {
        let periods = store.periods(y.id)?;
        let mut counts: Vec<(String, usize)> = Vec::new();
        for p in &periods {
            let s = p.status.to_string();
            match counts.iter_mut().find(|(k, _)| *k == s) {
                Some((_, n)) => *n += 1,
                None => counts.push((s, 1)),
            }
        }
        let summary: Vec<String> = counts.iter().map(|(s, n)| format!("{n} {s}")).collect();
        writeln!(
            w,
            "- {} ({} – {}): {}",
            y.label(),
            y.start,
            y.end,
            summary.join(", ")
        )?;
    }
    writeln!(w)?;
    for s in Series::ALL {
        writeln!(w, "- Serie {}: {}", s.as_str(), s.describe())?;
    }
    writeln!(w)?;
    writeln!(
        w,
        "En stängd eller låst period tar inte emot poster och öppnas aldrig igen. En period kan inte \
         stängas så länge en bankrad i den inte är avstämd."
    )?;

    writeln!(w)?;
    writeln!(w, "## Kontoplan och momskoder")?;
    writeln!(w)?;
    let chart = store.chart(store.today())?;
    let active = chart.iter().filter(|a| a.active).count();
    let with_vat: Vec<String> = chart
        .iter()
        .filter_map(|a| {
            a.vat_code
                .as_ref()
                .map(|c| format!("{} → ruta {c}", a.number))
        })
        .collect();
    writeln!(
        w,
        "{} konton, varav {active} aktiva. Kontoplanen ändras bara genom signerade versioner med giltighetsdag.",
        chart.len()
    )?;
    if with_vat.is_empty() {
        writeln!(
            w,
            "Inga konton har momskod; momsrapporten är tom tills de har det."
        )?;
    } else {
        writeln!(w)?;
        writeln!(
            w,
            "Momskod (ruta i momsdeklarationen) per konto: {}.",
            with_vat.join(", ")
        )?;
    }

    writeln!(w)?;
    writeln!(w, "## Spärrar i databasen")?;
    writeln!(w)?;
    let missing = store.missing_guards()?;
    if missing.is_empty() {
        writeln!(
            w,
            "Alla tabeller som bara ska kunna växa har spärrar mot ändring och radering: {}.",
            APPEND_ONLY_TABLES.join(", ")
        )?;
    } else {
        writeln!(w, "**SAKNAS:** {}", missing.join(", "))?;
    }
    writeln!(w)?;
    writeln!(w, "| Tabell | Händelse | Spärr |")?;
    writeln!(w, "| --- | --- | --- |")?;
    for g in store.guards()? {
        let text = if g.messages.is_empty() {
            g.trigger.clone()
        } else {
            g.messages.join("; ")
        };
        writeln!(
            w,
            "| `{}` | {} | {} |",
            g.table,
            g.event,
            text.replace('|', "\\|")
        )?;
    }

    writeln!(w)?;
    writeln!(w, "## Underlag, bank och ärenden")?;
    writeln!(w)?;
    let attachments = store.attachments()?;
    writeln!(
        w,
        "{} underlag är mottagna. Varje fil sparas oförändrad i `underlag/<ab>/<sha256>` bredvid \
         databasen innan någon verifikation finns, och kontrolleras mot sin hash vid varje läsning \
         och vid `bok verify`.",
        attachments.len()
    )?;
    let bank = store.bank_rows(false)?;
    let open = bank.iter().filter(|r| r.reconciliation.is_none()).count();
    writeln!(
        w,
        "{} bankrader är inlästa, varav {open} inte avstämda. Avstämning görs av en människa, mot en \
         verifikation med exakt belopp på bankens konto eller med en skriftlig förklaring.",
        bank.len()
    )?;
    let cases = store.cases(false)?;
    writeln!(
        w,
        "{} ärenden (agentens frågor och flaggor), varav {} obesvarade. Bara granskarens svar på en \
         flagga kan ändra ett underlags status.",
        cases.len(),
        cases.iter().filter(|c| c.answer.is_none()).count()
    )?;

    let events = store.events(None)?;
    writeln!(w)?;
    writeln!(w, "## Fjärråtkomst")?;
    writeln!(w)?;
    let count = |action: &str| events.iter().filter(|e| e.action == action).count();
    writeln!(
        w,
        "Agenter kan ansluta på distans genom `bok-fjarr`: MCP över HTTPS, inloggning med OAuth 2.1 \
         (PKCE) och ett godkännande med en kopplingskod som bara visas på datorn med bokföringen. En \
         anslutning får samma verktyg som agenten lokalt och kan aldrig bokföra. Underlag kan skickas \
         från en telefon med en personlig länk; de sparas oförändrade och registreras med länkens namn \
         som avsändare. Nycklar och länkar sparas bara som SHA-256 i `fjarr.json` bredvid databasen \
         och kan återkallas med `bok-fjarr aterkalla`."
    )?;
    writeln!(w)?;
    writeln!(
        w,
        "Historiken visar {} godkända och {} återkallade agentanslutningar, och {} skapade och {} \
         återkallade uppladdningslänkar.",
        count("fjarr.anslutning.godkand"),
        count("fjarr.anslutning.aterkallad"),
        count("fjarr.lank.skapad"),
        count("fjarr.lank.aterkallad")
    )?;
    writeln!(w)?;
    writeln!(
        w,
        "Insynen (`bok insyn`) visar bokföringen och kan inte ändra den. Som tjänst får varje person \
         en egen namngiven länk (`bok insyn lank --namn`) med ett slutdatum; den kan återkallas \
         (`bok insyn aterkalla`) och slutar då gälla direkt, också för öppna sessioner. En session \
         gäller högst 12 timmar och aldrig längre än länken. Varje inloggning loggas med vem och när \
         innan den släpps in. Historiken visar {} skapade och {} återkallade insynslänkar och {} \
         inloggningar.",
        count(bok_store::INSYN_LINK_CREATED),
        count(bok_store::INSYN_LINK_REVOKED),
        count(bok_store::INSYN_LOGIN)
    )?;

    writeln!(w)?;
    writeln!(w, "## Behandlingshistorik och kopior")?;
    writeln!(w)?;
    let backups = events
        .iter()
        .filter(|e| {
            e.action == "export.skapad"
                && e.new_values
                    .as_deref()
                    .is_some_and(|v| v.contains("\"kopia\""))
        })
        .count();
    writeln!(
        w,
        "Historiken har {} händelser i en obruten hashkedja; den senaste har hashen `{}`. Varje \
         händelse visar vem, roll, vad, när, gamla och nya värden och programversion.",
        events.len(),
        events.last().map(|e| e.hash.as_str()).unwrap_or("–")
    )?;
    writeln!(
        w,
        "Ingen händelse får en tid före den föregående. Står datorns klocka före den senaste \
         händelsen vägrar Bokis att skriva, tills klockan är ställd. Har den senaste händelsen fått \
         fel tid kan en människa godta det med `bok klocka godta` och en förklaring; det har skett {} \
         gånger.",
        events
            .iter()
            .filter(|e| e.action == bok_store::CLOCK_ACCEPTED)
            .count()
    )?;
    writeln!(
        w,
        "Kopior skrivs med `bok kopia skapa <mål>` till en plats utanför datorn och kontrolleras med \
         `bok kopia kontrollera`; inläsning görs bara till en tom katalog (`bok kopia las-in`). \
         Historiken visar {backups} kopior."
    )?;

    writeln!(w)?;
    w.push_str(&crate::sysdoc_archive::section(store, &events)?);

    writeln!(w)?;
    if checked == generated {
        writeln!(w, "## Kontroller vid framtagandet")?;
    } else {
        writeln!(w, "## Kontroller gjorda {checked}")?;
    }
    writeln!(w)?;
    for c in &report.checks {
        let mark = if c.problems.is_empty() { "OK" } else { "FEL" };
        writeln!(w, "- {mark}: {} ({} kontrollerade)", c.name, c.checked)?;
        for p in &c.problems {
            writeln!(w, "  - {p}")?;
        }
    }
    Ok(d)
}
