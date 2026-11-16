//! Sidorna i insynen. Varje funktion ger en hel HTML-sida, och allt som
//! kommer ur bokföringen går genom [`esc`].

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;

use bok_store::bok_core::{Money, PeriodStatus, Series, VoucherRef};
use bok_store::{
    AttachmentRow, CorrectionLink, EventRow, FiscalYear, Outcome, Period, TrialBalanceRow,
    VerifyReport, VoucherView,
};

use crate::html::{amount, amount_or_blank, esc, time, Frame, Tab};
use crate::{extension, year_dir, Insyn, Mode, Result};

/// Från en sida i årets katalog till sajtens rot.
const UP: &str = "../";

/// Största textunderlag som visas direkt på verifikationen.
const MAX_INLINE_TEXT: usize = 64 * 1024;

fn money(ore: i64) -> Money {
    Money::from_ore(ore)
}

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn size(bytes: usize) -> String {
    const MB: usize = 1024 * 1024;
    if bytes < 1024 {
        format!("{bytes} byte")
    } else if bytes < MB {
        format!("{} kB", bytes.div_ceil(1024))
    } else {
        format!("{},{} MB", bytes / MB, (bytes % MB) * 10 / MB)
    }
}

fn short_hash(h: &str) -> &str {
    h.get(..16).unwrap_or(h)
}

fn pretty_json(s: &str) -> String {
    serde_json::from_str::<serde_json::Value>(s)
        .ok()
        .and_then(|v| serde_json::to_string_pretty(&v).ok())
        .unwrap_or_else(|| s.to_string())
}

fn period_summary(periods: &[Period]) -> String {
    let mut parts = Vec::new();
    for (status, one, many) in [
        (PeriodStatus::Open, "öppen", "öppna"),
        (
            PeriodStatus::UnderReview,
            "under avstämning",
            "under avstämning",
        ),
        (PeriodStatus::Closed, "stängd", "stängda"),
        (PeriodStatus::Locked, "låst", "låsta"),
    ] {
        let n = periods.iter().filter(|p| p.status == status).count();
        if n > 0 {
            parts.push(count(n, one, many));
        }
    }
    if parts.is_empty() {
        "inga perioder".into()
    } else {
        parts.join(", ")
    }
}

/// Årets resultat ur saldobalansen: klass 3–8 utan 8990–8999, där resultatet
/// förs över till eget kapital. Positivt är vinst.
fn result_of(tb: &[TrialBalanceRow]) -> Money {
    money(
        -tb.iter()
            .filter(|r| (3000..=8989).contains(&r.account.value()))
            .map(|r| r.closing.ore())
            .sum::<i64>(),
    )
}

fn action_label(action: &str) -> &str {
    match action {
        "databas.skapad" => "Bokföringen skapad",
        "nyckel.registrerad" => "Nyckel registrerad",
        "rakenskapsar.skapat" => "Räkenskapsår upplagt",
        "kontoplan.andrad" => "Kontoplanen ändrad",
        "motpart.skapad" => "Motpart registrerad",
        "motpart.andrad" => "Motpart ändrad",
        "underlag.mottaget" => "Underlag mottaget",
        "forslag.skapat" => "Förslag lagt",
        "forslag.avvisat" => "Förslag avvisat",
        "verifikation.bokford" => "Verifikation bokförd",
        "period.oppnad" => "Period öppnad",
        "period.under_avstamning" => "Period under avstämning",
        "period.stangd" => "Period stängd",
        "period.last" => "Period låst",
        "bankrader.inlasta" => "Bankrader inlästa",
        "matchning.foreslagen" => "Matchning föreslagen",
        "bankrad.avstamd" => "Bankrad avstämd",
        "bankrad.forklarad" => "Bankrad förklarad",
        "konto.avstamt" => "Konto avstämt",
        "arende.oppnat" => "Ärende öppnat",
        "arende.besvarat" => "Ärende besvarat",
        "regel.andrad" => "Regel ändrad",
        "export.skapad" => "Export tagen",
        "fjarr.anslutning.godkand" => "Agentanslutning godkänd",
        "fjarr.anslutning.aterkallad" => "Agentanslutning återkallad",
        "fjarr.lank.skapad" => "Uppladdningslänk skapad",
        "fjarr.lank.aterkallad" => "Uppladdningslänk återkallad",
        "klocka.godtagen" => "Tid bakåt godtagen",
        other => other,
    }
}

/// Hur underlaget kom in, som det står före tidpunkten: "inlagt för hand 2026-03-03 10:40".
fn channel_label(channel: &str) -> &str {
    match channel {
        "manuell" => "inlagt för hand",
        "mapp" => "inläst från inkorgsmappen",
        "mejl" => "mottaget med e-post",
        "bank" => "inläst som bankfil",
        other => other,
    }
}

fn status_label(status: &str) -> &str {
    match status {
        "mottagen" => "Mottaget, inte bokfört",
        "bokford" => "Bokfört",
        "komplettering" => "Komplettering",
        "ej_rakenskapsinformation" => "Inte räkenskapsinformation",
        "dubblett" => "Dubblett",
        other => other,
    }
}

fn source_label(source: &str) -> &str {
    match source {
        "agent" => "agenten",
        "manniska" => "en människa",
        "system" => "systemet",
        other => other,
    }
}

/// Vem som gjorde något, med rollen när den inte redan framgår av namnet.
fn who(e: &EventRow) -> String {
    if e.role == e.actor {
        esc(&e.actor)
    } else {
        format!(
            "{} <span class=\"tyst\">{}</span>",
            esc(&e.actor),
            esc(&e.role)
        )
    }
}

impl Insyn<'_> {
    /// Verifikationsnumret som länk, om året är med i insynen.
    fn voucher_link(
        &self,
        up: &str,
        fiscal_year_id: i64,
        reference: VoucherRef,
        own_year: Option<i64>,
    ) -> String {
        let label = esc(&self.voucher_label(fiscal_year_id, reference, own_year));
        match self.voucher_href(up, fiscal_year_id, reference) {
            Some(href) => format!("<a href=\"{href}\">{label}</a>"),
            None => label,
        }
    }

    fn correction_links(&self, up: &str, links: &[CorrectionLink], own_year: i64) -> String {
        links
            .iter()
            .map(|l| self.voucher_link(up, l.fiscal_year_id, l.reference, Some(own_year)))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Vad en händelse avser, med länk där det finns en sida.
    fn object_html(
        &self,
        up: &str,
        object: &str,
        periods: &HashMap<i64, Period>,
    ) -> Result<String> {
        let index = self.index()?;
        let Some((kind, id)) = object.split_once(':') else {
            return Ok(esc(object));
        };
        let n = id.parse::<i64>().ok();
        Ok(match (kind, n) {
            ("verifikation", Some(n)) => match index.vouchers.get(&n) {
                Some((y, r)) => format!("Verifikation {}", self.voucher_link(up, *y, *r, None)),
                None => format!("Verifikation {n}"),
            },
            ("forslag", Some(n)) => match index
                .by_proposal
                .get(&n)
                .and_then(|v| index.vouchers.get(v))
            {
                Some((y, r)) => format!(
                    "Förslag {n}, bokfört som {}",
                    self.voucher_link(up, *y, *r, None)
                ),
                None => format!("Förslag {n}"),
            },
            ("underlag", Some(n)) => format!("<a href=\"{up}underlag.html#u{n}\">Underlag {n}</a>"),
            ("period", Some(n)) => match periods.get(&n) {
                Some(p) => format!("Period {}", esc(&p.label())),
                None => format!("Period {n}"),
            },
            ("rakenskapsar", Some(n)) => match self.all_years.iter().find(|y| y.id == n) {
                Some(y) => format!("Räkenskapsår {}", esc(&y.label())),
                None => format!("Räkenskapsår {n}"),
            },
            ("bankrad", Some(n)) => format!("Bankrad {n}"),
            ("arende", Some(n)) => format!("Ärende {n}"),
            ("motpart", Some(n)) => format!("Motpart {n}"),
            ("nyckel", Some(n)) => format!("Nyckel {n}"),
            ("konto", _) => format!("Konto {}", esc(id)),
            ("kontoplan", _) => format!("Kontoplanen från {}", esc(id)),
            ("regel", _) => format!("Regeln {}", esc(id)),
            ("fjarr", _) => format!("Fjärranslutningen {}", esc(id)),
            _ => esc(object),
        })
    }

    fn all_periods(&self) -> Result<HashMap<i64, Period>> {
        let mut out = HashMap::new();
        for y in &self.all_years {
            for p in self.store.periods(y.id)? {
                out.insert(p.id, p);
            }
        }
        Ok(out)
    }

    fn attachment_row(&self, sha256: &str) -> Result<Option<&AttachmentRow>> {
        Ok(self.all_attachments()?.iter().find(|a| a.sha256 == sha256))
    }
}

/// Kontrollstämpeln: kontrollens resultat med varje delkontroll.
fn stamp(b: &mut String, report: &VerifyReport, when: &str) -> Result<()> {
    let ok = report.is_ok();
    let posts: usize = report.checks.iter().map(|c| c.checked).sum();
    writeln!(
        b,
        "<section class=\"stampel{}\" aria-label=\"Kontrollen av bokföringen\">",
        if ok { "" } else { " fel" }
    )?;
    writeln!(
        b,
        "<p class=\"utfall\">{}</p>",
        if ok {
            "Bokföringen är obruten"
        } else {
            "Kontrollen hittade fel"
        }
    )?;
    writeln!(
        b,
        "<p class=\"nar\">Kontrollerad {}: {} av {}.</p>",
        esc(when),
        count(report.checks.len(), "kontroll", "kontroller"),
        count(posts, "post", "poster")
    )?;
    b.push_str("<ul>\n");
    for c in &report.checks {
        if c.problems.is_empty() {
            writeln!(
                b,
                "<li><span>{}</span> <span class=\"tal\">{}</span></li>",
                esc(c.name),
                c.checked
            )?;
        }
        for p in &c.problems {
            writeln!(
                b,
                "<li class=\"problem\"><span>{}: {}</span></li>",
                esc(c.name),
                esc(p)
            )?;
        }
    }
    b.push_str("</ul>\n</section>\n");
    let warnings = group_warnings(report.checks.iter().flat_map(|c| &c.warnings));
    if !warnings.is_empty() {
        b.push_str("<div class=\"notis\"><p>Att se över, men inga fel:</p><ul>\n");
        for w in warnings {
            writeln!(b, "<li>{}</li>", esc(&w))?;
        }
        b.push_str("</ul></div>\n");
    }
    Ok(())
}

/// Varningar med samma innebörd i en rad: "A1 (id 1): signerad med en
/// mjukvarunyckel" för tjugo verifikationer blir en rad som räknar upp dem.
fn group_warnings<'a>(warnings: impl Iterator<Item = &'a String>) -> Vec<String> {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for w in warnings {
        let (what, why) = match w.split_once(": ") {
            Some((what, why)) => (
                what.split(" (id ").next().unwrap_or(what).to_string(),
                why.to_string(),
            ),
            None => (String::new(), w.clone()),
        };
        match groups.iter_mut().find(|(g, _)| *g == why) {
            Some((_, items)) => items.push(what),
            None => groups.push((why, vec![what])),
        }
    }
    groups
        .into_iter()
        .map(|(why, items)| {
            let items: Vec<String> = items.into_iter().filter(|i| !i.is_empty()).collect();
            match items.len() {
                0 => capital(&why),
                1 => format!("{}: {why}", items[0]),
                n if n <= 6 => format!("{} ({n} st): {}", capital(&why), items.join(", ")),
                n => format!("{}: {n} poster", capital(&why)),
            }
        })
        .collect()
}

pub(crate) fn overview(i: &Insyn) -> Result<String> {
    let report = i.verify()?;
    let index = i.index()?;
    let mut b = String::new();
    writeln!(b, "<h1>{}</h1>", esc(&i.company.name))?;
    let lead = match i.mode {
        Mode::Live => "Bokföringen som den står i Bokis just nu. Sidorna läses direkt ur databasen och kan inte ändra något i den.".to_string(),
        Mode::Export => format!(
            "En export av bokföringen i Bokis, framtagen {}. Sidorna visar bokföringen som den stod då och kan inte ändra något i den.",
            esc(&i.generated)
        ),
    };
    writeln!(b, "<p class=\"ingress\">{lead}</p>")?;
    stamp(&mut b, report, i.checked_at())?;
    writeln!(
        b,
        "<p class=\"tyst\">Kontrollen räknar om hashkedjan i behandlingshistoriken, letar luckor i \
         verifikationsserierna, prövar varje signatur mot de registrerade nycklarna och läser varje \
         underlag mot dess SHA-256. {}</p>",
        match i.mode {
            Mode::Live => "Resultatet sparas tills insynen startas om, eftersom kontrollen läser \
                           varje underlag och tar tid när de är många. \
                           <a href=\"kontrollera\">Kontrollera igen</a>.",
            Mode::Export => "Den gjordes när exporten togs fram. Kör <code>bok verify</code> mot databasen för en ny kontroll.",
        }
    )?;

    writeln!(b, "<h2>Räkenskapsår</h2>")?;
    if i.years.is_empty() {
        b.push_str("<p class=\"notis\">Inga räkenskapsår är upplagda.</p>\n");
    } else {
        b.push_str(
            "<div class=\"rull\"><table class=\"bok\">\n<thead><tr><th scope=\"col\">År</th><th scope=\"col\">Omfattar</th>\
             <th scope=\"col\" class=\"num\">Verifikationer</th><th scope=\"col\">Perioder</th>\
             <th scope=\"col\" class=\"num\">Årets resultat</th><th scope=\"col\">Bokslut</th></tr></thead>\n<tbody>\n",
        );
        for y in i.years.iter().rev() {
            let dir = year_dir(y);
            let tb = i.store.trial_balance(y.id)?;
            let n = index
                .vouchers
                .values()
                .filter(|(yid, _)| *yid == y.id)
                .count();
            let periods = i.store.periods(y.id)?;
            writeln!(
                b,
                "<tr><td><a href=\"{dir}/grundbok.html\">{}</a></td><td>{} – {}</td><td class=\"num\">{n}</td>\
                 <td>{}</td><td class=\"num\">{}</td><td><a href=\"{dir}/arsbokslut.html\">Årsbokslut {}</a></td></tr>",
                esc(&y.label()),
                y.start,
                y.end,
                esc(&period_summary(&periods)),
                amount(result_of(&tb)),
                esc(&y.label())
            )?;
        }
        b.push_str("</tbody></table></div>\n");
    }

    writeln!(b, "<h2>Så förs bokföringen</h2>")?;
    b.push_str(
        "<p>En AI-agent läser underlagen och lägger förslag till verifikationer. Ingenting bokförs \
         förrän en människa har granskat förslaget och signerat det med sin nyckel; agenten kan inte \
         bokföra själv. Varje verifikation visar underlaget, vem som föreslog, vem som godkände och \
         signaturen.</p>\n<dl class=\"fakta\">\n",
    );
    writeln!(
        b,
        "<dt>Organisationsnummer</dt><dd>{}</dd>",
        esc(&i.company.org_number)
    )?;
    writeln!(
        b,
        "<dt>Program</dt><dd>{}, schemaversion {}</dd>",
        esc(i.program),
        bok_store::SCHEMA_VERSION
    )?;
    let keys = i.store.keys()?;
    let keys_html: Vec<String> = keys
        .iter()
        .map(|k| {
            format!(
                "{} ({})",
                esc(&k.name),
                if k.hardware {
                    "hårdvarunyckel"
                } else {
                    "mjukvarunyckel"
                }
            )
        })
        .collect();
    writeln!(
        b,
        "<dt>Nycklar som får signera</dt><dd>{}</dd>",
        keys_html.join(", ")
    )?;
    let attachments = i.all_attachments()?;
    let booked = attachments.iter().filter(|a| a.status == "bokford").count();
    writeln!(
        b,
        "<dt>Underlag</dt><dd><a href=\"underlag.html\">{} mottagna</a>, varav {booked} bokförda</dd>",
        attachments.len()
    )?;
    let bank = i.store.bank_rows(false)?;
    let open_bank = bank.iter().filter(|r| r.reconciliation.is_none()).count();
    writeln!(
        b,
        "<dt>Bankrader</dt><dd>{} inlästa, varav {}</dd>",
        bank.len(),
        if open_bank == 0 {
            "alla avstämda".to_string()
        } else {
            format!("<span class=\"avvikelse\">{open_bank} inte avstämda</span>")
        }
    )?;
    let cases = i.store.cases(false)?;
    writeln!(
        b,
        "<dt>Agentens frågor och flaggor</dt><dd>{}, varav {} obesvarade</dd>",
        cases.len(),
        cases.iter().filter(|c| c.answer.is_none()).count()
    )?;
    let events = i.events()?;
    match events.last() {
        Some(last) => writeln!(
            b,
            "<dt>Behandlingshistorik</dt><dd><a href=\"historik.html\">{}</a>, den senaste {}</dd>",
            count(events.len(), "händelse", "händelser"),
            time(&last.time)
        )?,
        None => writeln!(b, "<dt>Behandlingshistorik</dt><dd>Inga händelser</dd>")?,
    }
    b.push_str("</dl>\n");
    Ok(i.page(&Frame::root("Översikt", Tab::Overview), &b))
}

fn line_text(names: &HashMap<u16, String>, account: u16, text: Option<&str>) -> String {
    let mut s = format!(
        "<a href=\"huvudbok.html#k{account}\">{account}</a> {}",
        esc(names.get(&account).map(String::as_str).unwrap_or(""))
    );
    if let Some(t) = text.filter(|t| !t.trim().is_empty()) {
        let _ = write!(s, "<br><span class=\"tyst\">{}</span>", esc(t));
    }
    s
}

pub(crate) fn journal(i: &Insyn, y: &FiscalYear) -> Result<String> {
    let vouchers = i.store.journal(y.id)?;
    let names = i.account_names(y.end)?;
    let mut b = String::new();
    writeln!(b, "<h1>Grundbok {}</h1>", esc(&y.label()))?;
    writeln!(
        b,
        "<p class=\"ingress\">{} i den ordning de registrerades, räkenskapsåret {} – {}. \
         Verifikationsnumret leder till underlag, signatur och historik.</p>",
        capital(&count(vouchers.len(), "verifikation", "verifikationer")),
        y.start,
        y.end
    )?;
    b.push_str(
        "<p class=\"utskrift-dold\"><a href=\"grundbok.csv\">Grundboken som CSV</a> för kalkylprogram \
         (semikolon och decimalkomma).</p>\n",
    );
    if vouchers.is_empty() {
        b.push_str("<p class=\"notis\">Inga verifikationer är bokförda för året.</p>\n");
        return Ok(i.page(
            &Frame::year(
                format!("Grundbok {}", y.label()),
                Tab::Journal,
                y,
                "grundbok.html",
            ),
            &b,
        ));
    }
    b.push_str(
        "<div class=\"rull\"><table class=\"bok grundbok\">\n<thead><tr><th scope=\"col\">Ver</th>\
         <th scope=\"col\">Datum</th><th scope=\"col\" class=\"reg\">Registrerad</th><th scope=\"col\">Text och kontering</th>\
         <th scope=\"col\" class=\"num\">Debet</th><th scope=\"col\" class=\"num\">Kredit</th></tr></thead>\n<tbody>\n",
    );
    let (mut debit, mut credit) = (0i64, 0i64);
    for v in &vouchers {
        let mut text = esc(&v.description);
        if let Some(c) = &v.counterparty {
            let _ = write!(text, "<br><span class=\"tyst\">{}</span>", esc(&c.name));
        }
        if !v.corrected_by.is_empty() {
            let _ = write!(
                text,
                "<br><span class=\"rattad\">Rättad genom {}</span>",
                i.correction_links(UP, &v.corrected_by, y.id)
            );
        }
        if let Some(c) = &v.corrects {
            let _ = write!(
                text,
                "<br><span class=\"rattad\">Rättar {}</span>",
                i.correction_links(UP, std::slice::from_ref(c), y.id)
            );
        }
        writeln!(
            b,
            "<tr class=\"ver\" id=\"{r}\"><td><a href=\"v-{r}.html\">{r}</a></td><td class=\"dag\">{}</td><td class=\"dag reg\">{}</td><td>{text}</td><td></td><td></td></tr>",
            v.date,
            time(&v.registered_at),
            r = v.reference
        )?;
        for l in &v.lines {
            debit += l.debit.ore();
            credit += l.credit.ore();
            writeln!(
                b,
                "<tr class=\"rad\"><td></td><td></td><td class=\"reg\"></td><td>{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
                line_text(&names, l.account.value(), l.text.as_deref()),
                amount_or_blank(l.debit),
                amount_or_blank(l.credit)
            )?;
        }
    }
    writeln!(
        b,
        "<tr class=\"summa\"><td colspan=\"2\">Summa för året</td><td class=\"reg\"></td><td></td><td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
        amount(money(debit)),
        amount(money(credit))
    )?;
    b.push_str("</tbody></table></div>\n");
    Ok(i.page(
        &Frame::year(
            format!("Grundbok {}", y.label()),
            Tab::Journal,
            y,
            "grundbok.html",
        ),
        &b,
    ))
}

/// Ett textfält i CSV: inom citattecken vid behov, och med en apostrof före
/// text som ett kalkylprogram annars skulle läsa som en formel.
fn csv_text(s: &str) -> String {
    let guarded = if s.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{s}")
    } else {
        s.to_string()
    };
    if guarded.contains([';', '"', '\n', '\r']) {
        format!("\"{}\"", guarded.replace('"', "\"\""))
    } else {
        guarded
    }
}

pub(crate) fn journal_csv(i: &Insyn, y: &FiscalYear) -> Result<String> {
    let names = i.account_names(y.end)?;
    let mut s = String::from(
        "\u{feff}serie;nummer;datum;registrerad;text;motpart;konto;kontonamn;radtext;momskod;debet;kredit;rattad_genom;rattar\n",
    );
    for v in i.store.journal(y.id)? {
        let corrected: Vec<String> = v
            .corrected_by
            .iter()
            .map(|l| i.voucher_label(l.fiscal_year_id, l.reference, Some(y.id)))
            .collect();
        let corrects = v
            .corrects
            .as_ref()
            .map(|l| i.voucher_label(l.fiscal_year_id, l.reference, Some(y.id)))
            .unwrap_or_default();
        for l in &v.lines {
            writeln!(
                s,
                "{};{};{};{};{};{};{};{};{};{};{};{};{};{}",
                v.reference.series,
                v.reference.number,
                v.date,
                csv_text(&bok_store::display_timestamp(&v.registered_at)),
                csv_text(&v.description),
                csv_text(
                    v.counterparty
                        .as_ref()
                        .map(|c| c.name.as_str())
                        .unwrap_or("")
                ),
                l.account,
                csv_text(
                    names
                        .get(&l.account.value())
                        .map(String::as_str)
                        .unwrap_or("")
                ),
                csv_text(l.text.as_deref().unwrap_or("")),
                csv_text(l.vat_code.as_deref().unwrap_or("")),
                l.debit,
                l.credit,
                csv_text(&corrected.join(" ")),
                csv_text(&corrects)
            )?;
        }
    }
    Ok(s)
}

pub(crate) fn voucher(i: &Insyn, y: &FiscalYear, v: &VoucherView) -> Result<String> {
    let names = i.account_names(v.date.max(y.start))?;
    let period = i.store.period(v.period_id)?;
    let mut b = String::new();

    b.push_str("<article class=\"blankett\">\n");
    writeln!(b, "<h1>Verifikation {}</h1>", v.reference)?;
    writeln!(b, "<p class=\"ingress\">{}</p>", esc(&v.description))?;
    b.push_str("<dl class=\"fakta\">\n");
    writeln!(b, "<dt>Affärshändelsens datum</dt><dd>{}</dd>", v.date)?;
    writeln!(b, "<dt>Sammanställd</dt><dd>{}</dd>", v.compiled_on)?;
    writeln!(b, "<dt>Registrerad</dt><dd>{}</dd>", time(&v.registered_at))?;
    writeln!(
        b,
        "<dt>Serie</dt><dd>{}, {}</dd>",
        v.reference.series,
        v.reference.series.describe()
    )?;
    writeln!(
        b,
        "<dt>Period</dt><dd>{}, {}</dd>",
        esc(&period.label()),
        period.status
    )?;
    if let Some(c) = &v.counterparty {
        let mut parts = vec![esc(&c.name)];
        if let Some(o) = &c.org_number {
            parts.push(format!("org.nr {}", esc(o)));
        }
        if let Some(m) = &c.vat_number {
            parts.push(format!("momsreg.nr {}", esc(m)));
        }
        writeln!(b, "<dt>Motpart</dt><dd>{}</dd>", parts.join(", "))?;
    }
    for l in &v.corrected_by {
        writeln!(
            b,
            "<dt>Rättad genom</dt><dd class=\"rattad\">{}: {}</dd>",
            i.voucher_link(UP, l.fiscal_year_id, l.reference, Some(y.id)),
            esc(&l.reason)
        )?;
    }
    if let Some(l) = &v.corrects {
        writeln!(
            b,
            "<dt>Rättar</dt><dd class=\"rattad\">{}: {}</dd>",
            i.voucher_link(UP, l.fiscal_year_id, l.reference, Some(y.id)),
            esc(&l.reason)
        )?;
    }
    b.push_str("</dl>\n");

    let has_text = v
        .lines
        .iter()
        .any(|l| l.text.as_deref().is_some_and(|t| !t.trim().is_empty()));
    let has_vat = v.lines.iter().any(|l| l.vat_code.is_some());
    write!(
        b,
        "<div class=\"rull\"><table class=\"bok\">\n<caption>Kontering</caption>\n<thead><tr><th scope=\"col\">Konto</th>{}{}\
         <th scope=\"col\" class=\"num\">Debet</th><th scope=\"col\" class=\"num\">Kredit</th></tr></thead>\n<tbody>\n",
        if has_text { "<th scope=\"col\">Text</th>" } else { "" },
        if has_vat { "<th scope=\"col\">Momskod</th>" } else { "" }
    )?;
    let (mut debit, mut credit) = (0i64, 0i64);
    for l in &v.lines {
        debit += l.debit.ore();
        credit += l.credit.ore();
        let text = if has_text {
            format!("<td>{}</td>", esc(l.text.as_deref().unwrap_or("")))
        } else {
            String::new()
        };
        let vat = if has_vat {
            format!("<td>{}</td>", esc(l.vat_code.as_deref().unwrap_or("")))
        } else {
            String::new()
        };
        writeln!(
            b,
            "<tr><td class=\"konto\"><a href=\"huvudbok.html#k{a}\">{a}</a> {}</td>{text}{vat}\
             <td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
            esc(names
                .get(&l.account.value())
                .map(String::as_str)
                .unwrap_or("")),
            amount_or_blank(l.debit),
            amount_or_blank(l.credit),
            a = l.account
        )?;
    }
    writeln!(
        b,
        "<tr class=\"summa\"><td colspan=\"{}\">Summa</td><td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
        1 + usize::from(has_text) + usize::from(has_vat),
        amount(money(debit)),
        amount(money(credit))
    )?;
    b.push_str("</tbody></table></div>\n</article>\n");

    // Underlaget bredvid konteringen.
    b.push_str("<section class=\"underlag\" aria-labelledby=\"rubrik-underlag\">\n<h2 id=\"rubrik-underlag\">Underlag</h2>\n");
    if v.attachments.is_empty() {
        let text = match v.reference.series {
            Series::A => "<p class=\"notis varning\">Verifikationen har inget underlag kopplat.</p>",
            Series::R => "<p class=\"notis\">Rättelsen har inget eget underlag; orsaken står ovan och originalets underlag finns på originalet.</p>",
            Series::B | Series::I => "<p class=\"notis\">Bokslutsverifikationer och ingående balanser har normalt inget eget underlag; de bygger på bokföringen själv.</p>",
        };
        writeln!(b, "{text}")?;
    }
    for sha in &v.attachments {
        let row = i.attachment_row(sha)?;
        let name = row
            .and_then(|a| a.original_name.as_deref())
            .unwrap_or("underlag utan namn");
        b.push_str("<figure>\n");
        match i.attachment(sha) {
            Ok((path, media, data)) => {
                match extension(&media) {
                    "png" | "jpg" | "gif" | "webp" => writeln!(
                        b,
                        "<a href=\"{UP}{path}\"><img src=\"{UP}{path}\" alt=\"Underlaget {}\"></a>",
                        esc(name)
                    )?,
                    "pdf" => writeln!(
                        b,
                        "<object class=\"pdf\" data=\"{UP}{path}\" type=\"application/pdf\" \
                         aria-label=\"Underlaget {}\"></object>\n\
                         <p><a href=\"{UP}{path}\">Öppna underlaget</a> (PDF, {})</p>",
                        esc(name),
                        size(data.len())
                    )?,
                    "txt" => {
                        let text = String::from_utf8_lossy(&data);
                        let shown: String = text.chars().take(MAX_INLINE_TEXT).collect();
                        writeln!(b, "<pre>{}</pre>", esc(&shown))?;
                        if shown.len() < text.len() {
                            writeln!(b, "<p><a href=\"{UP}{path}\">Hela texten</a> ({})</p>", size(data.len()))?;
                        }
                    }
                    _ => writeln!(
                        b,
                        "<p><a href=\"{UP}{path}\">Hämta underlaget</a> ({}, {}). Filtypen visas inte i webbläsaren.</p>",
                        esc(&media),
                        size(data.len())
                    )?,
                }
            }
            Err(e) => writeln!(
                b,
                "<p class=\"notis varning\">Underlaget kunde inte läsas ur arkivet: {}</p>",
                esc(&e.to_string())
            )?,
        }
        let received = match row {
            Some(a) => format!(
                ", {} {}{}",
                esc(channel_label(&a.channel)),
                time(&a.received_at),
                a.sender
                    .as_deref()
                    .map(|s| format!(" från {}", esc(s)))
                    .unwrap_or_default()
            ),
            None => String::new(),
        };
        writeln!(
            b,
            "<figcaption>{}{received}. SHA-256 <span class=\"hash\">{}</span></figcaption>\n</figure>",
            esc(name),
            esc(sha)
        )?;
    }
    b.push_str("</section>\n");

    // Signaturen.
    b.push_str(
        "<section aria-labelledby=\"rubrik-signatur\">\n<h2 id=\"rubrik-signatur\">Signatur</h2>\n",
    );
    writeln!(
        b,
        "<p class=\"signerad\">Bokförd med nyckeln {}, fingeravtryck <span class=\"hash\">{}</span>.</p>",
        esc(&v.key_name),
        esc(&v.key_fingerprint)
    )?;
    writeln!(
        b,
        "<dl class=\"fakta\"><dt>Verifikationens hash</dt><dd class=\"hash\">{}</dd>\
         <dt>Kedjehash</dt><dd class=\"hash\">{}</dd></dl>",
        esc(&v.canonical_hash),
        esc(&v.chain_hash)
    )?;
    writeln!(
        b,
        "<details><summary>Signaturen</summary><pre class=\"hash\">{}</pre></details>",
        esc(&v.signature)
    )?;
    b.push_str(
        "<p class=\"tyst\">Hashen räknas över verifikationens innehåll, och signaturen är gjord över \
         hashen. Kedjehashen binder verifikationen till den som bokfördes före den, så att en \
         borttagen eller utbytt verifikation syns. Kontrollen på översikten prövar signaturen mot \
         den registrerade nyckeln.</p>\n</section>\n",
    );

    // Förslaget och beslutet.
    let p = i.store.proposal(v.proposal_id)?;
    b.push_str("<section aria-labelledby=\"rubrik-forslag\">\n<h2 id=\"rubrik-forslag\">Förslag och beslut</h2>\n<dl class=\"fakta\">\n");
    writeln!(
        b,
        "<dt>Föreslagen av</dt><dd>{} ({}), {}</dd>",
        esc(&p.actor),
        esc(source_label(&p.source)),
        time(&p.created_at)
    )?;
    let pv = &p.provenance;
    let model: Vec<String> = [&pv.model, &pv.provider, &pv.region]
        .into_iter()
        .flatten()
        .map(|s| esc(s))
        .collect();
    if !model.is_empty() {
        writeln!(b, "<dt>Modell</dt><dd>{}</dd>", model.join(", "))?;
    }
    if let Some(c) = pv.confidence {
        let pct = if c <= 1.0 { c * 100.0 } else { c };
        writeln!(b, "<dt>Agentens säkerhet</dt><dd>{pct:.0} %</dd>")?;
    }
    if let Some(r) = &pv.rule_version {
        writeln!(b, "<dt>Regelversion</dt><dd>{}</dd>", esc(r))?;
    }
    if let Some(q) = pv.questions.as_deref().filter(|q| !q.trim().is_empty()) {
        writeln!(b, "<dt>Agentens frågor</dt><dd>{}</dd>", esc(q))?;
    }
    if let Some(f) = pv.flags.as_deref().filter(|f| !f.trim().is_empty()) {
        writeln!(b, "<dt>Agentens flaggor</dt><dd>{}</dd>", esc(f))?;
    }
    match &p.decision {
        Some(d) => {
            let outcome = match d.outcome {
                Outcome::Approved => "Godkänd",
                Outcome::Rejected => "Avvisad",
            };
            writeln!(
                b,
                "<dt>Beslut</dt><dd>{outcome} av {}, {}</dd>",
                esc(&d.actor),
                time(&d.time)
            )?;
            if let Some(r) = &d.reason {
                writeln!(b, "<dt>Motivering</dt><dd>{}</dd>", esc(r))?;
            }
            if let Some(c) = &d.changes {
                let text = serde_json::from_str::<serde_json::Value>(c)
                    .ok()
                    .and_then(|v| v["skal"].as_str().map(str::to_string))
                    .unwrap_or_else(|| c.clone());
                writeln!(b, "<dt>Ändrat vid bokföringen</dt><dd>{}</dd>", esc(&text))?;
            }
        }
        None => writeln!(b, "<dt>Beslut</dt><dd>Inget beslut registrerat</dd>")?,
    }
    b.push_str("</dl>\n");
    if let Some(raw) = pv.raw_response.as_deref().filter(|r| !r.trim().is_empty()) {
        writeln!(
            b,
            "<details><summary>Agentens fullständiga svar</summary><pre>{}</pre></details>",
            esc(&pretty_json(raw))
        )?;
    }
    if let Some(h) = &pv.prompt_hash {
        writeln!(
            b,
            "<p class=\"tyst\">Frågan till agenten har hashen <span class=\"hash\">{}</span>.</p>",
            esc(h)
        )?;
    }
    b.push_str("</section>\n");

    // Händelserna som rör verifikationen.
    let index = i.index()?;
    let events = i.events()?;
    let mut objects = vec![
        format!("verifikation:{}", v.id),
        format!("forslag:{}", v.proposal_id),
    ];
    for sha in &v.attachments {
        if let Some(a) = i.attachment_row(sha)? {
            objects.push(format!("underlag:{}", a.id));
        }
    }
    if let Some(rows) = index.bank_by_voucher.get(&v.id) {
        objects.extend(rows.iter().map(|id| format!("bankrad:{id}")));
    }
    let seqs: BTreeSet<usize> = objects
        .iter()
        .filter_map(|o| index.events_by_object.get(o))
        .flatten()
        .copied()
        .collect();
    b.push_str("<section aria-labelledby=\"rubrik-handelser\">\n<h2 id=\"rubrik-handelser\">Händelser</h2>\n");
    b.push_str(
        "<div class=\"rull\"><table class=\"bok\">\n<thead><tr><th scope=\"col\" class=\"num\">Nr</th>\
         <th scope=\"col\">Tid</th><th scope=\"col\">Vem</th><th scope=\"col\">Händelse</th></tr></thead>\n<tbody>\n",
    );
    let periods = HashMap::new();
    for n in seqs {
        let e = &events[n];
        writeln!(
            b,
            "<tr><td class=\"num\"><a href=\"{UP}historik.html#h{seq}\">{seq}</a></td><td class=\"dag\">{}</td><td>{}</td><td>{}{}<br><span class=\"tyst\">{}</span></td></tr>",
            time(&e.time),
            who(e),
            esc(action_label(&e.action)),
            if e.signed { " <span class=\"signerad\">signerad</span>" } else { "" },
            i.object_html(UP, e.object.as_deref().unwrap_or(""), &periods)?,
            seq = e.seq
        )?;
    }
    b.push_str("</tbody></table></div>\n</section>\n");

    Ok(i.page(
        &Frame::in_year(
            format!("Verifikation {} ({})", v.reference, y.label()),
            Tab::Journal,
            y,
        ),
        &b,
    ))
}

pub(crate) fn ledger(i: &Insyn, y: &FiscalYear) -> Result<String> {
    let accounts = i.store.ledger(y.id, None)?;
    let mut b = String::new();
    writeln!(b, "<h1>Huvudbok {}</h1>", esc(&y.label()))?;
    writeln!(
        b,
        "<p class=\"ingress\">Årets poster per konto, med ingående balans, löpande saldo och utgående \
         balans. {} har poster.</p>",
        capital(&count(accounts.len(), "konto", "konton"))
    )?;
    if accounts.is_empty() {
        b.push_str("<p class=\"notis\">Inga poster är bokförda för året.</p>\n");
        return Ok(i.page(
            &Frame::year(
                format!("Huvudbok {}", y.label()),
                Tab::Ledger,
                y,
                "huvudbok.html",
            ),
            &b,
        ));
    }
    b.push_str(
        "<nav class=\"utskrift-dold\" aria-label=\"Konton i huvudboken\"><ul class=\"konton\">\n",
    );
    for a in &accounts {
        writeln!(b, "<li><a href=\"#k{0}\">{0}</a></li>", a.account)?;
    }
    b.push_str("</ul></nav>\n");
    for a in &accounts {
        writeln!(
            b,
            "<section class=\"kontoblad\" id=\"k{0}\">\n<h2>{0} {1}</h2>",
            a.account,
            esc(&a.name)
        )?;
        b.push_str(
            "<div class=\"rull\"><table class=\"bok\">\n<thead><tr><th scope=\"col\">Datum</th><th scope=\"col\">Ver</th>\
             <th scope=\"col\">Text</th><th scope=\"col\" class=\"num\">Debet</th><th scope=\"col\" class=\"num\">Kredit</th>\
             <th scope=\"col\" class=\"num\">Saldo</th></tr></thead>\n<tbody>\n",
        );
        writeln!(
            b,
            "<tr class=\"del\"><td colspan=\"5\">Ingående balans</td><td class=\"num\">{}</td></tr>",
            amount(a.opening)
        )?;
        let (mut debit, mut credit) = (0i64, 0i64);
        for e in &a.entries {
            debit += e.debit.ore();
            credit += e.credit.ore();
            let corrected = if e.corrected_by.is_empty() {
                String::new()
            } else {
                format!(
                    "<br><span class=\"rattad\">Rättad genom {}</span>",
                    i.correction_links(UP, &e.corrected_by, y.id)
                )
            };
            writeln!(
                b,
                "<tr><td class=\"dag\">{}</td><td><a href=\"v-{r}.html\">{r}</a></td><td>{}{corrected}</td><td class=\"num\">{}</td>\
                 <td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
                e.date,
                esc(&e.text),
                amount_or_blank(e.debit),
                amount_or_blank(e.credit),
                amount(e.balance),
                r = e.reference
            )?;
        }
        writeln!(
            b,
            "<tr class=\"summa\"><td colspan=\"3\">Utgående balans</td><td class=\"num\">{}</td><td class=\"num\">{}</td>\
             <td class=\"num\">{}</td></tr>",
            amount(money(debit)),
            amount(money(credit)),
            amount(a.closing)
        )?;
        b.push_str("</tbody></table></div>\n</section>\n");
    }
    Ok(i.page(
        &Frame::year(
            format!("Huvudbok {}", y.label()),
            Tab::Ledger,
            y,
            "huvudbok.html",
        ),
        &b,
    ))
}

const CLASSES: [(u8, &str); 8] = [
    (1, "Tillgångar"),
    (2, "Eget kapital och skulder"),
    (3, "Rörelsens intäkter"),
    (4, "Varor, material och tjänster"),
    (5, "Övriga externa kostnader"),
    (6, "Övriga externa kostnader"),
    (7, "Personal, avskrivningar och övriga rörelsekostnader"),
    (8, "Finansiella poster, bokslutsdispositioner och skatt"),
];

pub(crate) fn trial_balance(i: &Insyn, y: &FiscalYear) -> Result<String> {
    let rows = i.store.trial_balance(y.id)?;
    let mut b = String::new();
    writeln!(b, "<h1>Saldobalans {}</h1>", esc(&y.label()))?;
    b.push_str(
        "<p class=\"ingress\">Varje konto med ingående balans, årets debet och kredit och utgående \
         balans. Debet är positivt. Kontonumret leder till kontots poster i huvudboken.</p>\n",
    );
    if rows.is_empty() {
        b.push_str("<p class=\"notis\">Inga poster är bokförda för året.</p>\n");
        return Ok(i.page(
            &Frame::year(
                format!("Saldobalans {}", y.label()),
                Tab::TrialBalance,
                y,
                "saldobalans.html",
            ),
            &b,
        ));
    }
    b.push_str(
        "<div class=\"rull\"><table class=\"bok\">\n<thead><tr><th scope=\"col\">Konto</th><th scope=\"col\">Benämning</th>\
         <th scope=\"col\" class=\"num\">Ingående balans</th><th scope=\"col\" class=\"num\">Debet</th>\
         <th scope=\"col\" class=\"num\">Kredit</th><th scope=\"col\" class=\"num\">Utgående balans</th></tr></thead>\n<tbody>\n",
    );
    let mut last_class = 0u8;
    let mut sums = [0i64; 4];
    for r in &rows {
        let class = r.account.class();
        if class != last_class {
            let title = CLASSES
                .iter()
                .find(|(c, _)| *c == class)
                .map(|(_, t)| *t)
                .unwrap_or("");
            writeln!(
                b,
                "<tr class=\"del\"><td colspan=\"6\">Klass {class}: {title}</td></tr>"
            )?;
            last_class = class;
        }
        sums[0] += r.opening.ore();
        sums[1] += r.debit.ore();
        sums[2] += r.credit.ore();
        sums[3] += r.closing.ore();
        writeln!(
            b,
            "<tr><td class=\"konto\"><a href=\"huvudbok.html#k{a}\">{a}</a></td><td>{}</td><td class=\"num\">{}</td>\
             <td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
            esc(&r.name),
            amount(r.opening),
            amount_or_blank(r.debit),
            amount_or_blank(r.credit),
            amount(r.closing),
            a = r.account
        )?;
    }
    writeln!(
        b,
        "<tr class=\"summa\"><td colspan=\"2\">Summa</td><td class=\"num\">{}</td><td class=\"num\">{}</td>\
         <td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
        amount(money(sums[0])),
        amount(money(sums[1])),
        amount(money(sums[2])),
        amount(money(sums[3]))
    )?;
    b.push_str("</tbody></table></div>\n");
    if sums[0] != 0 || sums[3] != 0 || sums[1] != sums[2] {
        b.push_str("<p class=\"notis varning\">Saldobalansen går inte jämnt ut: summan av balanserna ska vara noll och debet ska vara lika med kredit.</p>\n");
    }
    Ok(i.page(
        &Frame::year(
            format!("Saldobalans {}", y.label()),
            Tab::TrialBalance,
            y,
            "saldobalans.html",
        ),
        &b,
    ))
}

/// En rad i en uppställning efter BAS-kontogrupper.
enum Post {
    Heading(&'static str),
    Group(u16, u16, &'static str),
    Sum(&'static str, u16, u16),
}

const INCOME_STATEMENT: &[Post] = &[
    Post::Heading("Rörelsens intäkter"),
    Post::Group(3000, 3799, "Nettoomsättning"),
    Post::Group(3800, 3899, "Aktiverat arbete för egen räkning"),
    Post::Group(3900, 3999, "Övriga rörelseintäkter"),
    Post::Heading("Rörelsens kostnader"),
    Post::Group(4000, 4999, "Råvaror, förnödenheter och handelsvaror"),
    Post::Group(5000, 6999, "Övriga externa kostnader"),
    Post::Group(7000, 7699, "Personalkostnader"),
    Post::Group(7700, 7899, "Av- och nedskrivningar"),
    Post::Group(7900, 7999, "Övriga rörelsekostnader"),
    Post::Sum("Rörelseresultat", 3000, 7999),
    Post::Heading("Finansiella poster"),
    Post::Group(
        8000,
        8299,
        "Resultat från finansiella anläggningstillgångar",
    ),
    Post::Group(8300, 8399, "Ränteintäkter och liknande resultatposter"),
    Post::Group(8400, 8499, "Räntekostnader och liknande resultatposter"),
    Post::Group(8500, 8799, "Övriga finansiella poster"),
    Post::Sum("Resultat efter finansiella poster", 3000, 8799),
    Post::Group(8800, 8899, "Bokslutsdispositioner"),
    Post::Sum("Resultat före skatt", 3000, 8899),
    Post::Group(8900, 8989, "Skatt på årets resultat"),
    Post::Sum("Årets resultat", 3000, 8989),
];

const ASSETS: &[Post] = &[
    Post::Heading("Anläggningstillgångar"),
    Post::Group(1000, 1099, "Immateriella anläggningstillgångar"),
    Post::Group(1100, 1299, "Materiella anläggningstillgångar"),
    Post::Group(1300, 1399, "Finansiella anläggningstillgångar"),
    Post::Heading("Omsättningstillgångar"),
    Post::Group(1400, 1499, "Varulager"),
    Post::Group(1500, 1799, "Kortfristiga fordringar"),
    Post::Group(1800, 1899, "Kortfristiga placeringar"),
    Post::Group(1900, 1999, "Kassa och bank"),
    Post::Sum("Summa tillgångar", 1000, 1999),
];

const EQUITY_AND_LIABILITIES: &[Post] = &[
    Post::Group(2000, 2099, "Eget kapital"),
    Post::Group(2100, 2199, "Obeskattade reserver"),
    Post::Group(2200, 2299, "Avsättningar"),
    Post::Group(2300, 2399, "Långfristiga skulder"),
    Post::Group(2400, 2999, "Kortfristiga skulder"),
    Post::Sum("Summa eget kapital och skulder", 2000, 2999),
];

/// Saldon per konto, debet positivt, i öre.
type Balances = BTreeMap<u16, i64>;

fn closing(tb: &[TrialBalanceRow]) -> Balances {
    tb.iter()
        .map(|r| (r.account.value(), r.closing.ore()))
        .collect()
}

fn opening(tb: &[TrialBalanceRow]) -> Balances {
    tb.iter()
        .map(|r| (r.account.value(), r.opening.ore()))
        .collect()
}

fn range_sum(b: &Balances, lo: u16, hi: u16) -> i64 {
    b.range(lo..=hi).map(|(_, v)| *v).sum()
}

/// Resultat som inte är fört till eget kapital: allt i klass 3–8, med
/// 8999 inräknat. Noll när bokslutsverifikationen är bokförd.
fn unbooked_result(b: &Balances) -> i64 {
    -range_sum(b, 3000, 8999)
}

struct Statement<'a> {
    names: &'a HashMap<u16, String>,
    columns: Vec<(String, Balances)>,
    /// -1 där kredit visas positivt.
    sign: i64,
    /// Resultat som inte är bokfört, per kolumn, visas under eget kapital.
    unbooked: Vec<i64>,
}

impl Statement<'_> {
    /// Om något konto i gruppen har ett belopp i någon kolumn, eller om
    /// gruppen är eget kapital och resultatet inte är bokfört.
    fn has_amounts(&self, lo: u16, hi: u16) -> bool {
        self.columns
            .iter()
            .any(|(_, c)| c.range(lo..=hi).any(|(_, v)| *v != 0))
            || (lo == 2000 && self.unbooked.iter().any(|v| *v != 0))
    }

    fn render(&self, b: &mut String, posts: &[Post], caption: &str) -> Result<()> {
        let n = self.columns.len();
        writeln!(
            b,
            "<div class=\"rull\"><table class=\"bok uppstallning\">\n<caption>{caption}</caption>"
        )?;
        b.push_str("<thead><tr><th scope=\"col\">Post</th>");
        for (title, _) in &self.columns {
            write!(b, "<th scope=\"col\" class=\"num\">{}</th>", esc(title))?;
        }
        b.push_str("</tr></thead>\n<tbody>\n");
        let cells = |values: &[i64]| -> String {
            values
                .iter()
                .map(|v| format!("<td class=\"num\">{}</td>", amount(money(*v))))
                .collect()
        };
        for (k, post) in posts.iter().enumerate() {
            match post {
                Post::Heading(t) => {
                    let section = posts[k + 1..]
                        .iter()
                        .take_while(|p| matches!(p, Post::Group(..)))
                        .any(|p| matches!(p, Post::Group(lo, hi, _) if self.has_amounts(*lo, *hi)));
                    if section {
                        writeln!(
                            b,
                            "<tr class=\"del\"><td colspan=\"{}\">{t}</td></tr>",
                            n + 1
                        )?;
                    }
                }
                Post::Group(lo, hi, name) => {
                    let equity = *lo == 2000;
                    let extra: Vec<i64> = if equity {
                        self.unbooked.clone()
                    } else {
                        vec![0; n]
                    };
                    let accounts: BTreeSet<u16> = self
                        .columns
                        .iter()
                        .flat_map(|(_, c)| {
                            c.range(lo..=hi).filter(|(_, v)| **v != 0).map(|(k, _)| *k)
                        })
                        .collect();
                    if accounts.is_empty() && extra.iter().all(|v| *v == 0) {
                        continue;
                    }
                    let values: Vec<i64> = self
                        .columns
                        .iter()
                        .zip(&extra)
                        .map(|((_, c), x)| self.sign * range_sum(c, *lo, *hi) + x)
                        .collect();
                    writeln!(
                        b,
                        "<tr class=\"post\"><td>{name}</td>{}</tr>",
                        cells(&values)
                    )?;
                    for a in accounts {
                        let values: Vec<i64> = self
                            .columns
                            .iter()
                            .map(|(_, c)| self.sign * c.get(&a).copied().unwrap_or(0))
                            .collect();
                        writeln!(
                            b,
                            "<tr class=\"rad\"><td><a href=\"huvudbok.html#k{a}\">{a}</a> {}</td>{}</tr>",
                            esc(self.names.get(&a).map(String::as_str).unwrap_or("")),
                            cells(&values)
                        )?;
                    }
                    if extra.iter().any(|v| *v != 0) {
                        writeln!(
                            b,
                            "<tr class=\"rad\"><td><span class=\"avvikelse\">Årets resultat, inte bokfört mot eget kapital</span></td>{}</tr>",
                            cells(&extra)
                        )?;
                    }
                }
                Post::Sum(name, lo, hi) => {
                    let values: Vec<i64> = self
                        .columns
                        .iter()
                        .enumerate()
                        .map(|(k, (_, c))| {
                            let extra = if *lo <= 2000 && *hi >= 2099 {
                                self.unbooked.get(k).copied().unwrap_or(0)
                            } else {
                                0
                            };
                            self.sign * range_sum(c, *lo, *hi) + extra
                        })
                        .collect();
                    writeln!(
                        b,
                        "<tr class=\"summa\"><td>{name}</td>{}</tr>",
                        cells(&values)
                    )?;
                }
            }
        }
        b.push_str("</tbody></table></div>\n");
        Ok(())
    }
}

/// En punkt i bokslutskontrollen.
enum Mark {
    Done,
    Pending,
    Problem,
}

pub(crate) fn annual_accounts(i: &Insyn, y: &FiscalYear) -> Result<String> {
    let tb = i.store.trial_balance(y.id)?;
    let names = i.account_names(y.end)?;
    let previous = i.previous_year(y).copied();
    let next = i.next_year(y).copied();
    let prev_tb = match previous {
        Some(p) => i.store.trial_balance(p.id)?,
        None => Vec::new(),
    };
    let now = closing(&tb);
    let start = opening(&tb);
    let result = result_of(&tb);
    let unbooked = unbooked_result(&now);

    let mut b = String::new();
    writeln!(b, "<h1>Årsbokslut {}</h1>", esc(&y.label()))?;
    writeln!(
        b,
        "<p class=\"ingress\">Resultat- och balansräkning för räkenskapsåret {} – {}, uppställda efter \
         BAS-kontoplanens grupper direkt ur bokföringen. Årets resultat är {}.</p>",
        y.start,
        y.end,
        amount(result)
    )?;

    // Bokslutskontrollen.
    let mut checks: Vec<(Mark, String)> = Vec::new();
    let periods = i.store.periods(y.id)?;
    let open_periods = periods
        .iter()
        .filter(|p| !matches!(p.status, PeriodStatus::Closed | PeriodStatus::Locked))
        .count();
    checks.push(if open_periods == 0 {
        (
            Mark::Done,
            format!("Alla {} perioder är stängda eller låsta", periods.len()),
        )
    } else {
        (
            Mark::Pending,
            format!(
                "{} av {} perioder är inte stängda ({})",
                open_periods,
                periods.len(),
                period_summary(&periods)
            ),
        )
    });
    let bank = i.store.bank_rows(false)?;
    let year_bank: Vec<_> = bank.iter().filter(|r| y.contains(r.booking_date)).collect();
    let open_bank = year_bank
        .iter()
        .filter(|r| r.reconciliation.is_none())
        .count();
    checks.push(if open_bank == 0 {
        (
            Mark::Done,
            format!("Alla {} bankrader under året är avstämda", year_bank.len()),
        )
    } else {
        (
            Mark::Pending,
            format!(
                "{} av {} bankrader under året är inte avstämda",
                open_bank,
                year_bank.len()
            ),
        )
    });
    let journal = i.store.journal(y.id)?;
    let closing_vouchers: Vec<&VoucherView> = journal
        .iter()
        .filter(|v| v.reference.series == Series::B)
        .collect();
    checks.push(if unbooked == 0 && !closing_vouchers.is_empty() {
        (
            Mark::Done,
            "Årets resultat är bokfört mot eget kapital (serie B)".to_string(),
        )
    } else if unbooked == 0 {
        (Mark::Done, "Resultatkontona är nollställda".to_string())
    } else {
        (
            Mark::Pending,
            format!(
                "Årets resultat, {}, är inte bokfört mot eget kapital",
                amount(money(unbooked))
            ),
        )
    });
    let assets = range_sum(&now, 1000, 1999);
    let equity = -range_sum(&now, 2000, 2999) + unbooked;
    checks.push(if assets == equity {
        (Mark::Done, "Balansräkningen balanserar".to_string())
    } else {
        (Mark::Problem, format!("Balansräkningen balanserar inte: tillgångarna är {} och eget kapital och skulder {}", amount(money(assets)), amount(money(equity))))
    });
    if previous.is_some() {
        let prev = closing(&prev_tb);
        let accounts: BTreeSet<u16> = prev
            .range(1000..=2999)
            .chain(start.range(1000..=2999))
            .map(|(k, _)| *k)
            .collect();
        let differs: Vec<String> = accounts
            .into_iter()
            .filter(|a| prev.get(a).copied().unwrap_or(0) != start.get(a).copied().unwrap_or(0))
            .map(|a| a.to_string())
            .collect();
        checks.push(if differs.is_empty() {
            (
                Mark::Done,
                "Ingående balanser stämmer mot föregående års utgående balanser".to_string(),
            )
        } else {
            (
                Mark::Problem,
                format!(
                    "Ingående balans skiljer sig från föregående års utgående balans på konto {}",
                    differs.join(", ")
                ),
            )
        });
    }
    match next {
        Some(n) => {
            let has_ib = i
                .store
                .journal(n.id)?
                .iter()
                .any(|v| v.reference.series == Series::I);
            checks.push(if has_ib {
                (
                    Mark::Done,
                    format!("Ingående balanser för {} är bokförda (serie I)", n.label()),
                )
            } else {
                (
                    Mark::Pending,
                    format!("Ingående balanser för {} är inte bokförda", n.label()),
                )
            });
        }
        None => checks.push((
            Mark::Pending,
            "Nästa räkenskapsår är inte upplagt".to_string(),
        )),
    }
    let reconciled: Vec<String> = i
        .store
        .reconciliations()?
        .into_iter()
        .filter(|(_, _, r)| r.date == y.end)
        .map(|(_, _, r)| {
            if r.difference.is_zero() {
                r.account.to_string()
            } else {
                format!("{} (skillnad {})", r.account, amount(r.difference))
            }
        })
        .collect();
    checks.push(if reconciled.is_empty() {
        (
            Mark::Pending,
            format!("Inga konton är avstämda mot externa saldon per {}", y.end),
        )
    } else {
        (
            Mark::Done,
            format!("Avstämda per {}: konto {}", y.end, reconciled.join(", ")),
        )
    });

    let problems = checks.iter().any(|(m, _)| matches!(m, Mark::Problem));
    let pending = checks.iter().any(|(m, _)| matches!(m, Mark::Pending));
    let (class, verdict) = if problems {
        (" fel", "Bokslutet har avvikelser")
    } else if pending {
        (" oppen", "Året är inte stängt i bokföringen")
    } else {
        ("", "Året är stängt i bokföringen")
    };
    writeln!(b, "<section class=\"stampel{class}\" aria-label=\"Bokslutskontroll\">\n<p class=\"utfall\">{verdict}</p>")?;
    writeln!(
        b,
        "<p class=\"nar\">Bokslutskontroll {}.</p>\n<ul>",
        esc(&i.generated)
    )?;
    for (mark, text) in &checks {
        let class = match mark {
            Mark::Done => "",
            Mark::Pending => " class=\"vantar\"",
            Mark::Problem => " class=\"problem\"",
        };
        writeln!(b, "<li{class}><span>{}</span></li>", esc(text))?;
    }
    b.push_str("</ul>\n</section>\n");

    // Resultaträkningen, med föregående år om det finns.
    let mut columns = vec![(y.label(), now.clone())];
    if let Some(p) = previous {
        columns.push((p.label(), closing(&prev_tb)));
    }
    b.push_str("<h2>Resultaträkning</h2>\n");
    let rr = Statement {
        names: &names,
        unbooked: vec![0; columns.len()],
        columns,
        sign: -1,
    };
    rr.render(
        &mut b,
        INCOME_STATEMENT,
        "Belopp i kronor. Intäkter är positiva och kostnader negativa.",
    )?;

    // Balansräkningen vid årets slut och början.
    b.push_str("<h2>Balansräkning</h2>\n");
    let columns = vec![
        (y.end.to_string(), now.clone()),
        (y.start.to_string(), start.clone()),
    ];
    let unbooked_cols = vec![unbooked, unbooked_result(&start)];
    let assets_stmt = Statement {
        names: &names,
        columns: columns.clone(),
        sign: 1,
        unbooked: vec![0, 0],
    };
    assets_stmt.render(&mut b, ASSETS, "Tillgångar")?;
    let equity_stmt = Statement {
        names: &names,
        columns,
        sign: -1,
        unbooked: unbooked_cols,
    };
    equity_stmt.render(&mut b, EQUITY_AND_LIABILITIES, "Eget kapital och skulder")?;

    // Bokslutets verifikationer.
    b.push_str("<h2>Bokslutets verifikationer</h2>\n<ul class=\"lankar\">\n");
    let mut any = false;
    for v in journal
        .iter()
        .filter(|v| matches!(v.reference.series, Series::B | Series::I))
    {
        any = true;
        writeln!(
            b,
            "<li>{} {}: {}</li>",
            i.voucher_link(UP, y.id, v.reference, Some(y.id)),
            v.date,
            esc(&v.description)
        )?;
    }
    if let Some(n) = next {
        for v in i
            .store
            .journal(n.id)?
            .iter()
            .filter(|v| v.reference.series == Series::I)
        {
            any = true;
            writeln!(
                b,
                "<li>{} {}: {}</li>",
                i.voucher_link(UP, n.id, v.reference, Some(y.id)),
                v.date,
                esc(&v.description)
            )?;
        }
    }
    if !any {
        b.push_str("<li>Inga bokslutsverifikationer eller ingående balanser är bokförda.</li>\n");
    }
    b.push_str("</ul>\n");
    b.push_str(
        "<p class=\"notis\">Uppställningen är ett underlag ur bokföringen. Årsbokslutet eller \
         årsredovisningen upprättas och undertecknas av företaget enligt bokföringslagen och BFN:s \
         allmänna råd, till exempel BFNAR 2017:3 för årsbokslut, och posterna kan då behöva delas upp \
         eller justeras.</p>\n",
    );
    Ok(i.page(
        &Frame::year(
            format!("Årsbokslut {}", y.label()),
            Tab::Annual,
            y,
            "arsbokslut.html",
        ),
        &b,
    ))
}

pub(crate) fn vat(i: &Insyn, y: &FiscalYear) -> Result<String> {
    let year_report = i.store.vat_report(y.start, y.end)?;
    let names = i.account_names(y.end)?;
    let mut months = Vec::new();
    for p in i.store.periods(y.id)? {
        let r = i.store.vat_report(p.start, p.end)?;
        months.push((p, r));
    }
    let mut b = String::new();
    writeln!(b, "<h1>Moms {}</h1>", esc(&y.label()))?;
    b.push_str(
        "<p class=\"ingress\">Rutorna i momsdeklarationen ur bokförda verifikationer, för hela året och \
         per månad. Momskoden i kontoplanen avgör vilken ruta ett konto hör till. Verifikationer som \
         avräknar momsen mot redovisningskontot räknas inte med.</p>\n",
    );
    let used: Vec<&bok_store::VatBox> = year_report
        .boxes
        .iter()
        .filter(|x| x.code != "49" && (!x.amount.is_zero() || !x.accounts.is_empty()))
        .collect();
    b.push_str("<h2>Hela året</h2>\n");
    if used.is_empty() {
        b.push_str("<p class=\"notis\">Inga bokförda belopp hör till någon ruta. Kontoplanens konton behöver momskoder för att rapporten ska fyllas i.</p>\n");
    } else {
        b.push_str(
            "<div class=\"rull\"><table class=\"bok\">\n<thead><tr><th scope=\"col\">Ruta</th><th scope=\"col\">Rubrik och konton</th>\
             <th scope=\"col\" class=\"num\">Belopp</th></tr></thead>\n<tbody>\n",
        );
        for x in &used {
            writeln!(
                b,
                "<tr class=\"post\"><td>{}</td><td>{}</td><td class=\"num\">{}</td></tr>",
                esc(&x.code),
                esc(&x.title),
                amount(x.amount)
            )?;
            for (account, value) in &x.accounts {
                writeln!(
                    b,
                    "<tr class=\"rad\"><td></td><td><a href=\"huvudbok.html#k{account}\">{account}</a> {}</td><td class=\"num\">{}</td></tr>",
                    esc(names.get(&account.value()).map(String::as_str).unwrap_or("")),
                    amount(*value)
                )?;
            }
        }
        writeln!(
            b,
            "<tr class=\"summa\"><td>49</td><td>{}</td><td class=\"num\">{}</td></tr>",
            if year_report.to_pay.is_negative() {
                "Moms att få tillbaka"
            } else {
                "Moms att betala"
            },
            amount(year_report.to_pay)
        )?;
        b.push_str("</tbody></table></div>\n");
    }
    if year_report.settlements > 0 {
        writeln!(
            b,
            "<p class=\"tyst\">{} räknades inte med, eftersom de avräknar momsen.</p>",
            capital(&count(
                year_report.settlements,
                "verifikation",
                "verifikationer"
            ))
        )?;
    }
    if !year_report.unknown_codes.is_empty() {
        let list: Vec<String> = year_report
            .unknown_codes
            .iter()
            .map(|(code, account)| format!("{account} ({})", esc(code)))
            .collect();
        writeln!(
            b,
            "<p class=\"notis varning\">Konton med en momskod som inte är någon ruta i deklarationen: {}.</p>",
            list.join(", ")
        )?;
    }

    // Per månad: en rad per månad och en kolumn per ruta som används.
    let codes: Vec<&str> = bok_store::VAT_BOXES
        .iter()
        .map(|(c, _)| *c)
        .filter(|c| *c != "49")
        .filter(|c| {
            months
                .iter()
                .any(|(_, r)| r.boxes.iter().any(|x| x.code == *c && !x.amount.is_zero()))
        })
        .collect();
    b.push_str("<h2>Per månad</h2>\n");
    if codes.is_empty() {
        b.push_str("<p class=\"notis\">Inga belopp under året.</p>\n");
    } else {
        b.push_str("<div class=\"rull\"><table class=\"bok\">\n<caption>Rutornas rubriker står i tabellen för hela året.</caption>\n<thead><tr><th scope=\"col\">Månad</th>");
        for c in &codes {
            write!(b, "<th scope=\"col\" class=\"num\">Ruta {c}</th>")?;
        }
        b.push_str("<th scope=\"col\" class=\"num\">Ruta 49</th></tr></thead>\n<tbody>\n");
        let value = |r: &bok_store::VatReport, code: &str| {
            r.boxes
                .iter()
                .find(|x| x.code == code)
                .map(|x| x.amount)
                .unwrap_or(Money::ZERO)
        };
        for (p, r) in &months {
            write!(b, "<tr><td>{}</td>", esc(&p.label()))?;
            for c in &codes {
                write!(b, "<td class=\"num\">{}</td>", amount_or_blank(value(r, c)))?;
            }
            writeln!(b, "<td class=\"num\">{}</td></tr>", amount(r.to_pay))?;
        }
        write!(b, "<tr class=\"summa\"><td>Helår</td>")?;
        for c in &codes {
            write!(
                b,
                "<td class=\"num\">{}</td>",
                amount(value(&year_report, c))
            )?;
        }
        writeln!(
            b,
            "<td class=\"num\">{}</td></tr>",
            amount(year_report.to_pay)
        )?;
        b.push_str("</tbody></table></div>\n");
    }
    Ok(i.page(
        &Frame::year(format!("Moms {}", y.label()), Tab::Vat, y, "moms.html"),
        &b,
    ))
}

pub(crate) fn bank(i: &Insyn, y: &FiscalYear) -> Result<String> {
    let index = i.index()?;
    let rows: Vec<_> = i
        .store
        .bank_rows(false)?
        .into_iter()
        .filter(|r| y.contains(r.booking_date))
        .collect();
    let open = rows.iter().filter(|r| r.reconciliation.is_none()).count();
    let mut b = String::new();
    writeln!(b, "<h1>Bank {}</h1>", esc(&y.label()))?;
    writeln!(
        b,
        "<p class=\"ingress\">{} med bokföringsdag under året och hur var och en stämdes av. En period \
         kan inte stängas så länge en bankrad i den inte är avstämd.</p>",
        capital(&count(rows.len(), "inläst bankrad", "inlästa bankrader"))
    )?;
    if open > 0 {
        writeln!(
            b,
            "<p class=\"notis varning\">{} inte avstämda.</p>",
            capital(&count(open, "bankrad är", "bankrader är"))
        )?;
    }
    if !rows.is_empty() {
        b.push_str(
            "<div class=\"rull\"><table class=\"bok\">\n<thead><tr><th scope=\"col\">Bokföringsdag</th><th scope=\"col\">Konto</th>\
             <th scope=\"col\">Text</th><th scope=\"col\" class=\"num\">Belopp</th><th scope=\"col\">Avstämd mot</th></tr></thead>\n<tbody>\n",
        );
        for r in &rows {
            let mut text = esc(&r.text);
            if let Some(c) = r.counterparty.as_deref().filter(|c| !c.trim().is_empty()) {
                let _ = write!(text, "<br><span class=\"tyst\">{}</span>", esc(c));
            }
            if let Some(reference) = r.reference.as_deref().filter(|c| !c.trim().is_empty()) {
                let _ = write!(
                    text,
                    "<br><span class=\"tyst\">Referens {}</span>",
                    esc(reference)
                );
            }
            let matched = match &r.reconciliation {
                None => "<span class=\"avvikelse\">Inte avstämd</span>".to_string(),
                Some(rec) => {
                    let target = match (rec.voucher_id, &rec.explanation) {
                        (Some(vid), _) => match index.vouchers.get(&vid) {
                            Some((yid, vref)) => i.voucher_link(UP, *yid, *vref, Some(y.id)),
                            None => format!("verifikation {vid}"),
                        },
                        (None, Some(e)) => format!("Förklaring: {}", esc(e)),
                        (None, None) => "–".to_string(),
                    };
                    format!(
                        "{target}<br><span class=\"tyst\">{}, {}</span>",
                        esc(&rec.actor),
                        time(&rec.time)
                    )
                }
            };
            writeln!(
                b,
                "<tr id=\"b{}\"><td class=\"dag\">{}</td><td class=\"konto\"><a href=\"huvudbok.html#k{a}\">{a}</a><br><span class=\"tyst\">{}</span></td>\
                 <td>{text}</td><td class=\"num\">{}</td><td>{matched}</td></tr>",
                r.id,
                r.booking_date,
                esc(&r.bank_account),
                amount(r.amount),
                a = r.account
            )?;
        }
        b.push_str("</tbody></table></div>\n");
    }

    let recs: Vec<_> = i
        .store
        .reconciliations()?
        .into_iter()
        .filter(|(_, _, r)| y.contains(r.date))
        .collect();
    b.push_str("<h2>Kontoavstämningar</h2>\n");
    if recs.is_empty() {
        b.push_str(
            "<p class=\"notis\">Inga konton är avstämda mot externa saldon under året.</p>\n",
        );
    } else {
        b.push_str(
            "<div class=\"rull\"><table class=\"bok\">\n<thead><tr><th scope=\"col\">Konto</th><th scope=\"col\">Per</th>\
             <th scope=\"col\" class=\"num\">Bokfört</th><th scope=\"col\" class=\"num\">Externt</th>\
             <th scope=\"col\" class=\"num\">Skillnad</th><th scope=\"col\">Av</th></tr></thead>\n<tbody>\n",
        );
        for (when, actor, r) in &recs {
            writeln!(
                b,
                "<tr><td class=\"konto\"><a href=\"huvudbok.html#k{a}\">{a}</a></td><td>{}</td><td class=\"num\">{}</td>\
                 <td class=\"num\">{}</td><td class=\"num{}\">{}</td><td>{}, {}</td></tr>",
                r.date,
                amount(r.booked),
                amount(r.external),
                if r.difference.is_zero() { "" } else { " avvikelse" },
                amount(r.difference),
                esc(actor),
                time(when),
                a = r.account
            )?;
        }
        b.push_str("</tbody></table></div>\n");
    }
    Ok(i.page(
        &Frame::year(format!("Bank {}", y.label()), Tab::Bank, y, "bank.html"),
        &b,
    ))
}

pub(crate) fn attachments(i: &Insyn) -> Result<String> {
    let rows = i.attachments()?;
    let index = i.index()?;
    let mut b = String::new();
    b.push_str("<h1>Underlag</h1>\n");
    writeln!(
        b,
        "<p class=\"ingress\">{}: kvitton, fakturor, kontoutdrag och annat som kommit in. Varje fil är \
         sparad oförändrad under sin SHA-256 och prövas mot den varje gång den läses.</p>",
        capital(&count(rows.len(), "mottaget underlag", "mottagna underlag"))
    )?;
    if i.years.len() < i.all_years.len() {
        let labels: Vec<String> = i.years.iter().map(|y| y.label()).collect();
        writeln!(
            b,
            "<p class=\"notis\">Insynen gäller räkenskapsåret {}: här finns underlagen som årets \
             verifikationer bygger på och de som togs emot under året.</p>",
            esc(&labels.join(", "))
        )?;
    }
    if rows.is_empty() {
        return Ok(i.page(&Frame::root("Underlag", Tab::Attachments), &b));
    }
    b.push_str(
        "<div class=\"rull\"><table class=\"bok\">\n<thead><tr><th scope=\"col\" class=\"num\">Nr</th><th scope=\"col\">Mottaget</th>\
         <th scope=\"col\">Underlag</th><th scope=\"col\">Status</th><th scope=\"col\">Verifikation</th>\
         <th scope=\"col\">SHA-256</th></tr></thead>\n<tbody>\n",
    );
    for a in rows {
        let name = a.original_name.as_deref().unwrap_or("underlag utan namn");
        let file = match i.attachment(&a.sha256) {
            Ok((path, media, data)) => format!(
                "<a href=\"{path}\">{}</a><br><span class=\"tyst\">{}, {}</span>",
                esc(name),
                esc(&media),
                size(data.len())
            ),
            Err(e) => format!(
                "{}<br><span class=\"avvikelse\">{}</span>",
                esc(name),
                esc(&e.to_string())
            ),
        };
        let vouchers: Vec<String> = index
            .used_by
            .get(&a.sha256)
            .map(|v| {
                v.iter()
                    .map(|(y, r)| i.voucher_link("", *y, *r, None))
                    .collect()
            })
            .unwrap_or_default();
        let status = if a.channel == "bank" && a.status == "mottagen" {
            "Kontoutdrag<br><span class=\"tyst\">underlag för bankraderna</span>".to_string()
        } else {
            match a
                .status_text
                .as_deref()
                .filter(|t| !t.trim().is_empty() && a.status != "bokford")
            {
                Some(t) => format!(
                    "{}<br><span class=\"tyst\">{}</span>",
                    esc(status_label(&a.status)),
                    esc(t)
                ),
                None => esc(status_label(&a.status)),
            }
        };
        writeln!(
            b,
            "<tr id=\"u{id}\"><td class=\"num\">{id}</td><td class=\"dag\">{}<br><span class=\"tyst\">{}{}</span></td><td>{file}</td><td>{status}</td>\
             <td>{}</td><td class=\"hash\">{}</td></tr>",
            time(&a.received_at),
            esc(channel_label(&a.channel)),
            a.sender.as_deref().map(|s| format!(" från {}", esc(s))).unwrap_or_default(),
            vouchers.join(", "),
            esc(short_hash(&a.sha256)),
            id = a.id
        )?;
    }
    b.push_str("</tbody></table></div>\n");
    Ok(i.page(&Frame::root("Underlag", Tab::Attachments), &b))
}

pub(crate) fn history(i: &Insyn) -> Result<String> {
    let events = i.events()?;
    let periods = i.all_periods()?;
    let mut b = String::new();
    b.push_str("<h1>Behandlingshistorik</h1>\n");
    writeln!(
        b,
        "<p class=\"ingress\">Varje ändring i bokföringen som en händelse: vem, i vilken roll, vad och \
         när, med gamla och nya värden. {} i ordning.</p>",
        capital(&count(events.len(), "händelse", "händelser"))
    )?;
    b.push_str(
        "<p>Händelserna bildar en hashkedja. Varje händelses hash räknas över händelsen och hashen \
         före den, så om en händelse ändras eller tas bort går kedjan inte längre ihop, och kontrollen \
         på översikten säger ifrån. Signerade händelser är beslut som gjorts med en registrerad nyckel.</p>\n",
    );
    if events.is_empty() {
        return Ok(i.page(&Frame::root("Behandlingshistorik", Tab::History), &b));
    }
    b.push_str(
        "<div class=\"rull\"><table class=\"bok historik\">\n<thead><tr><th scope=\"col\" class=\"num\">Nr</th><th scope=\"col\">Tid</th>\
         <th scope=\"col\">Vem</th><th scope=\"col\">Händelse</th><th scope=\"col\">Hash</th></tr></thead>\n<tbody>\n",
    );
    for e in events {
        let mut details = String::new();
        if let Some(old) = &e.old_values {
            let _ = write!(details, "Före:\n{}\n\n", pretty_json(old));
        }
        if let Some(new) = &e.new_values {
            let _ = write!(details, "Efter:\n{}\n\n", pretty_json(new));
        }
        let _ = write!(
            details,
            "Programversion: {}\nHash: {}",
            e.program_version, e.hash
        );
        writeln!(
            b,
            "<tr id=\"h{seq}\"><td class=\"num\">{seq}</td><td class=\"dag\">{}</td><td>{}</td><td>{}{}<br><span class=\"tyst\">{}</span>\
             <details><summary>Detaljer</summary><pre>{}</pre></details></td><td class=\"hash\">{}</td></tr>",
            time(&e.time),
            who(e),
            esc(action_label(&e.action)),
            if e.signed { " <span class=\"signerad\">signerad</span>" } else { "" },
            i.object_html("", e.object.as_deref().unwrap_or(""), &periods)?,
            esc(&details),
            esc(short_hash(&e.hash)),
            seq = e.seq
        )?;
    }
    b.push_str("</tbody></table></div>\n");
    Ok(i.page(&Frame::root("Behandlingshistorik", Tab::History), &b))
}

pub(crate) fn system_doc(i: &Insyn) -> Result<String> {
    let report = i.verify()?;
    let md = crate::sysdoc::render_with(
        i.store,
        i.db,
        i.program,
        &i.generated,
        report,
        i.checked_at(),
    )?;
    let body = format!(
        "<article class=\"dokument\">\n{}</article>\n",
        crate::markdown::to_html(&md)
    );
    Ok(i.page(&Frame::root("Systemdokumentation", Tab::SystemDoc), &body))
}

pub(crate) fn export_page(i: &Insyn) -> Result<String> {
    let mut b = String::new();
    b.push_str("<h1>Export</h1>\n");
    match i.mode {
        Mode::Export => {
            writeln!(
                b,
                "<p class=\"ingress\">Den här katalogen är en fristående kopia av bokföringen, framtagen {}. \
                 Den öppnas i en vanlig webbläsare, utan Bokis och utan nät.</p>",
                esc(&i.generated)
            )?;
            b.push_str("<h2>Innehåll</h2>\n<ul class=\"lankar\">\n");
            b.push_str(
                "<li><a href=\"index.html\">index.html</a> och sidorna under flikarna</li>\n",
            );
            for f in &i.extra {
                writeln!(
                    b,
                    "<li><a href=\"{p}\">{p}</a>: {}</li>",
                    esc(&f.description),
                    p = esc(&f.path)
                )?;
            }
            for y in &i.years {
                writeln!(
                    b,
                    "<li><a href=\"{d}/grundbok.csv\">{d}/grundbok.csv</a>: grundboken för {} som CSV, med semikolon och decimalkomma</li>",
                    esc(&y.label()),
                    d = year_dir(y)
                )?;
            }
            b.push_str(
                "<li><code>underlag/</code>: underlagen byte för byte, namngivna efter sin SHA-256</li>\n\
                 <li><code>manifest.sha256</code>: SHA-256 för varje fil ovan</li>\n</ul>\n",
            );
            b.push_str(
                "<h2>Kontrollera att inget har ändrats</h2>\n<p>Manifestet har samma form som \
                 <code>sha256sum</code>. Kör kommandot i katalogen; varje fil ska få OK.</p>\n\
                 <p>Linux:</p>\n<pre>sha256sum -c manifest.sha256</pre>\n<p>macOS:</p>\n\
                 <pre>shasum -a 256 -c manifest.sha256</pre>\n<p>Windows PowerShell (skriver bara ut filer som avviker):</p>\n\
                 <pre>Get-Content manifest.sha256 | ForEach-Object { $h, $f = $_ -split '  ', 2; \
                 if ((Get-FileHash -Algorithm SHA256 -LiteralPath $f).Hash -ne $h) { \"AVVIKER: $f\" } }</pre>\n\
                 <p class=\"tyst\">Manifestets egen SHA-256 skrevs ut när exporten togs fram och kan antecknas \
                 separat, till exempel i ett följebrev.</p>\n",
            );
        }
        Mode::Live => {
            b.push_str(
                "<p class=\"ingress\">Ta fram en fristående kopia till revisorn eller Skatteverket. Kopian \
                 öppnas i en vanlig webbläsare utan Bokis och utan nät, och innehåller allt som syns här: \
                 sidorna, underlagen, SIE-filer och CSV, med en kontrollsumma för varje fil.</p>\n\
                 <h2>Kommandon</h2>\n<dl class=\"fakta kommandon\">\n\
                 <dt>Hela bokföringen</dt><dd><pre>bok insyn export /media/usb/bokforing</pre></dd>\n\
                 <dt>Ett räkenskapsår</dt><dd><pre>bok insyn export /media/usb/bokforing-2026 --year 2026</pre></dd>\n\
                 <dt>Arkivpaket för bevarandet</dt><dd><pre>bok arkivpaket /media/usb --year 2026</pre>\
                 SIE, rapporter som text, underlagen, historiken, systemdokumentationen och programmet.</dd>\n\
                 <dt>Bara SIE 4</dt><dd><pre>bok export sie --year 2026 --out bokis-2026.se</pre></dd>\n</dl>\n\
                 <p class=\"tyst\">En katalog eller fil som redan finns skrivs aldrig över, och varje export \
                 hamnar i behandlingshistoriken.</p>\n",
            );
            if !i.years.is_empty() {
                b.push_str("<h2>Hämta direkt</h2>\n<ul class=\"lankar\">\n");
                for y in i.years.iter().rev() {
                    writeln!(
                        b,
                        "<li><a href=\"{}/grundbok.csv\">Grundboken {} som CSV</a></li>",
                        year_dir(y),
                        esc(&y.label())
                    )?;
                }
                b.push_str("</ul>\n");
            }
        }
    }
    Ok(i.page(&Frame::root("Export", Tab::Export), &b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_fields_are_quoted_and_guarded() {
        assert_eq!(csv_text("Kaffe"), "Kaffe");
        assert_eq!(csv_text("a;b"), "\"a;b\"");
        assert_eq!(csv_text("säger \"hej\""), "\"säger \"\"hej\"\"\"");
        assert_eq!(csv_text("=HYPERLINK(\"x\")"), "\"'=HYPERLINK(\"\"x\"\")\"");
        assert_eq!(csv_text("-5"), "'-5");
    }

    #[test]
    fn sizes_and_counts_read_naturally() {
        assert_eq!(size(10), "10 byte");
        assert_eq!(size(2048), "2 kB");
        assert_eq!(size(3 * 1024 * 1024 + 512 * 1024), "3,5 MB");
        assert_eq!(count(1, "verifikation", "verifikationer"), "1 verifikation");
        assert_eq!(
            count(2, "verifikation", "verifikationer"),
            "2 verifikationer"
        );
        assert_eq!(capital("åtta"), "Åtta");
    }
}
