//! Byggstenar för sidorna: escaping, belopp och sidramen med pärmens flikar.

use std::fmt::Write as _;

use bok_store::bok_core::Money;
use bok_store::FiscalYear;

use crate::{year_dir, Insyn, Mode};

/// Escapar text för HTML-innehåll och attributvärden. Styrtecken och
/// osynliga riktningsmarkörer visas som `\u{…}`, så att en text från ett
/// underlag eller en agent inte kan se ut att säga något annat än den gör.
pub fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            '\n' | '\t' | '\r' => out.push(c),
            c if hidden(c) => {
                let _ = write!(out, "\\u{{{:x}}}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

fn hidden(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{061c}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}'
        )
}

/// Ett belopp som `-1 234,56`: hårt mellanslag mellan tusentalen och vanligt
/// minustecken, så att beloppet går att klistra in i ett kalkylark.
pub fn amount(m: Money) -> String {
    let (negative, kronor, ore) = m.split();
    let digits = kronor.to_string();
    let mut out = String::with_capacity(digits.len() + 8);
    if negative {
        out.push('-');
    }
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('\u{a0}');
        }
        out.push(c);
    }
    let _ = write!(out, ",{ore:02}");
    out
}

/// Tomt för noll, annars beloppet. För debet- och kreditkolumner.
pub fn amount_or_blank(m: Money) -> String {
    if m.is_zero() {
        String::new()
    } else {
        amount(m)
    }
}

/// En tidsstämpel som den lagras (RFC 3339, UTC) visad i lokal tid.
pub fn time(stored: &str) -> String {
    esc(&bok_store::display_timestamp(stored))
}

/// Flikarna i pärmen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tab {
    Overview,
    Journal,
    Ledger,
    TrialBalance,
    Annual,
    Vat,
    Bank,
    Assets,
    Declaration,
    Attachments,
    History,
    SystemDoc,
    Export,
}

const YEAR_TABS: [(Tab, &str, &str); 8] = [
    (Tab::Journal, "grundbok.html", "Grundbok"),
    (Tab::Ledger, "huvudbok.html", "Huvudbok"),
    (Tab::TrialBalance, "saldobalans.html", "Saldobalans"),
    (Tab::Annual, "arsbokslut.html", "Årsbokslut"),
    (Tab::Vat, "moms.html", "Moms"),
    (Tab::Bank, "bank.html", "Bank"),
    (Tab::Assets, "inventarier.html", "Inventarier"),
    (Tab::Declaration, "deklaration.html", "Deklaration"),
];

const COMMON_TABS: [(Tab, &str, &str); 4] = [
    (Tab::Attachments, "underlag.html", "Underlag"),
    (Tab::History, "historik.html", "Behandlingshistorik"),
    (
        Tab::SystemDoc,
        "systemdokumentation.html",
        "Systemdokumentation",
    ),
    (Tab::Export, "export.html", "Export"),
];

/// En sida i pärmen: rubrik, flik och år.
pub(crate) struct Frame<'a> {
    /// Sidans namn i fönstrets titel.
    pub title: String,
    pub tab: Tab,
    /// Året sidan gäller, för sidor i årets katalog.
    pub year: Option<&'a FiscalYear>,
    /// Sidans fil i årskatalogen, för länkarna till samma sida andra år.
    pub year_page: Option<&'static str>,
}

impl<'a> Frame<'a> {
    pub fn root(title: impl Into<String>, tab: Tab) -> Self {
        Self {
            title: title.into(),
            tab,
            year: None,
            year_page: None,
        }
    }

    pub fn year(
        title: impl Into<String>,
        tab: Tab,
        year: &'a FiscalYear,
        page: &'static str,
    ) -> Self {
        Self {
            title: title.into(),
            tab,
            year: Some(year),
            year_page: Some(page),
        }
    }

    /// En sida i årskatalogen som inte finns för andra år, till exempel en verifikation.
    pub fn in_year(title: impl Into<String>, tab: Tab, year: &'a FiscalYear) -> Self {
        Self {
            title: title.into(),
            tab,
            year: Some(year),
            year_page: None,
        }
    }

    /// Vägen till sajtens rot från sidan.
    pub fn up(&self) -> &'static str {
        if self.year.is_some() {
            "../"
        } else {
            ""
        }
    }
}

impl Insyn<'_> {
    /// Hela sidan med flikar, sidhuvud och sidfot runt `body`.
    pub(crate) fn page(&self, frame: &Frame, body: &str) -> String {
        let up = frame.up();
        let company = esc(&self.company.name);
        let org = esc(&self.company.org_number);
        let mut s = String::with_capacity(body.len() + 4096);
        let _ = write!(
            s,
            "<!doctype html>\n<html lang=\"sv\">\n<head>\n<meta charset=\"utf-8\">\n\
             <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
             <meta name=\"referrer\" content=\"no-referrer\">\n\
             <title>{} – {company}</title>\n<link rel=\"stylesheet\" href=\"{up}insyn.css\">\n</head>\n<body>\n\
             <div class=\"parm\">\n<nav class=\"flikar\" aria-label=\"Pärmens flikar\">\n\
             <p class=\"marke\"><strong>{company}</strong>{org}</p>\n",
            esc(&frame.title)
        );
        tab_link(
            &mut s,
            up,
            "index.html",
            "Översikt",
            frame.tab == Tab::Overview,
        );
        let year = frame.year.or_else(|| self.current_year());
        if let Some(y) = year {
            let dir = year_dir(y);
            let _ = writeln!(s, "<p class=\"aret\">Räkenskapsår {}</p>", esc(&y.label()));
            for (tab, file, name) in YEAR_TABS {
                tab_link(&mut s, up, &format!("{dir}/{file}"), name, frame.tab == tab);
            }
        }
        s.push_str("<p class=\"aret\">Hela bokföringen</p>\n");
        for (tab, file, name) in COMMON_TABS {
            tab_link(&mut s, up, file, name, frame.tab == tab);
        }
        s.push_str("</nav>\n<main class=\"blad\">\n<header class=\"huvud\">\n");
        let _ = writeln!(s, "<p class=\"foretag\">{company}, {org}</p>");
        if let (Some(y), Some(page)) = (frame.year, frame.year_page) {
            if self.years.len() > 1 {
                s.push_str("<p class=\"ar\">Räkenskapsår");
                for other in &self.years {
                    let current = other.id == y.id;
                    let _ = write!(
                        s,
                        " <a href=\"{up}{}/{page}\"{}>{}</a>",
                        year_dir(other),
                        if current {
                            " aria-current=\"page\""
                        } else {
                            ""
                        },
                        esc(&other.label())
                    );
                }
                s.push_str("</p>\n");
            }
        }
        s.push_str("</header>\n");
        s.push_str(body);
        let mode = match self.mode {
            Mode::Live => "Visas live ur databasen",
            Mode::Export => "Exporterad",
        };
        let _ = write!(
            s,
            "<footer class=\"fot\"><p>{mode} {} med {}. Sidan visar bokföringen och kan inte ändra den.</p></footer>\n\
             </main>\n</div>\n</body>\n</html>\n",
            esc(&self.generated),
            esc(self.program)
        );
        s
    }
}

fn tab_link(s: &mut String, up: &str, href: &str, name: &str, current: bool) {
    let _ = writeln!(
        s,
        "<a href=\"{up}{href}\"{}>{name}</a>",
        if current {
            " aria-current=\"page\""
        } else {
            ""
        }
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_markup_and_shows_hidden_characters() {
        assert_eq!(
            esc("<a href=\"x\">'&'</a>"),
            "&lt;a href=&quot;x&quot;&gt;&#39;&amp;&#39;&lt;/a&gt;"
        );
        assert_eq!(esc("faktura\u{202e}fdp.exe"), "faktura\\u{202e}fdp.exe");
        assert_eq!(esc("a\u{0}b\u{7}"), "a\\u{0}b\\u{7}");
        assert_eq!(esc("rad 1\nrad 2"), "rad 1\nrad 2");
    }

    #[test]
    fn formats_amounts_with_groups_and_sign() {
        assert_eq!(amount(Money::from_ore(0)), "0,00");
        assert_eq!(amount(Money::from_ore(-50)), "-0,50");
        assert_eq!(amount(Money::from_ore(123_456)), "1\u{a0}234,56");
        assert_eq!(
            amount(Money::from_ore(-123_456_789)),
            "-1\u{a0}234\u{a0}567,89"
        );
        assert_eq!(amount(Money::from_ore(100_000)), "1\u{a0}000,00");
        assert_eq!(amount_or_blank(Money::ZERO), "");
    }
}
