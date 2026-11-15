//! Tolkning av indata: rader på kommandoraden, förslag i JSON, kontoplaner
//! och räkenskapsår.

use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use chrono::{Datelike, NaiveDate};
use serde::Deserialize;

use bok_store::bok_core::voucher::{Counterparty, Line};
use bok_store::bok_core::{AccountNumber, Money, Series};
use bok_store::{AccountChange, NewProposal, ProposalCounterparty, Provenance};

/// `KONTO=BELOPP[:TEXT]`. Positivt belopp är debet, negativt kredit:
/// `5410=80,00:Papper`, `1930=-100`.
pub fn parse_line(spec: &str) -> Result<Line> {
    let (account, rest) = spec.split_once('=').ok_or_else(|| {
        anyhow!("raden \"{spec}\" ska skrivas KONTO=BELOPP, till exempel 5410=80,00")
    })?;
    let account: AccountNumber = account.parse()?;
    let (amount, text) = match rest.split_once(':') {
        Some((a, t)) => (a, Some(t.trim().to_string()).filter(|t| !t.is_empty())),
        None => (rest, None),
    };
    let amount = Money::parse(amount)?;
    if amount.is_zero() {
        bail!("raden \"{spec}\" har beloppet noll");
    }
    let mut line = Line::from_signed(account, amount);
    line.text = text;
    Ok(line)
}

/// Ett förslag i JSON, i samma form som agenten kommer att använda:
///
/// ```json
/// {"serie": "A", "datum": "2026-03-15", "beskrivning": "Papper",
///  "motpart": {"namn": "Papper AB", "orgnr": "556000-0000"},
///  "underlag": [1],
///  "rader": [{"konto": 5410, "debet": 8000}, {"konto": 1930, "kredit": 8000}]}
/// ```
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalFile {
    #[serde(default = "default_series")]
    pub serie: Series,
    pub datum: NaiveDate,
    pub beskrivning: String,
    #[serde(default)]
    pub motpart: Option<FileCounterparty>,
    #[serde(default)]
    pub underlag: Vec<i64>,
    pub rader: Vec<Line>,
    #[serde(default)]
    pub idempotensnyckel: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum FileCounterparty {
    Register { id: i64 },
    Inline(Counterparty),
}

fn default_series() -> Series {
    Series::A
}

impl ProposalFile {
    pub fn read(path: &Path) -> Result<ProposalFile> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("kan inte läsa {}", path.display()))?;
        serde_json::from_str(&text)
            .with_context(|| format!("{} är inget giltigt förslag", path.display()))
    }

    pub fn into_proposal(self) -> NewProposal {
        NewProposal {
            series: self.serie,
            date: self.datum,
            description: self.beskrivning,
            counterparty: self.motpart.map(|m| match m {
                FileCounterparty::Register { id } => ProposalCounterparty::Register(id),
                FileCounterparty::Inline(c) => ProposalCounterparty::Inline(c),
            }),
            lines: self.rader,
            attachments: self.underlag,
            idempotency_key: self.idempotensnyckel,
            provenance: Provenance::default(),
        }
    }
}

/// Läser en kontoplan. Antingen en SIE-fil (`#KONTO`, `#SRU`), som läses
/// med sie-parser, eller en textfil med en rad per konto:
/// `1930;Företagskonto` eller `1930 Företagskonto`, och med semikolon även
/// `2641;Debiterad ingående moms;48` där tredje kolumnen är momskoden.
/// Tomma rader och rader som börjar med `//` hoppas över.
pub fn read_chart(path: &Path) -> Result<Vec<AccountChange>> {
    let text =
        sie_parser::read_file(path).with_context(|| format!("kan inte läsa {}", path.display()))?;
    if text.lines().any(|l| l.trim_start().starts_with("#KONTO")) {
        let doc = sie_parser::document::read(&text)
            .map_err(|e| anyhow!("{} går inte att läsa som SIE: {e}", path.display()))?;
        return doc
            .accounts
            .values()
            .map(|a| {
                Ok(AccountChange {
                    number: AccountNumber::new(i64::from(a.no))?,
                    name: a.name.clone(),
                    active: true,
                    sru: a.sru.map(|s| s.to_string()),
                    vat_code: None,
                })
            })
            .collect();
    }
    let mut out = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        let (number, rest) = line
            .split_once([';', '\t', ' '])
            .ok_or_else(|| anyhow!("rad {}: skriv KONTO;NAMN", i + 1))?;
        // KONTO;NAMN;RUTA: en tredje kolumn är momskoden (rutan i momsdeklarationen).
        let (name, vat_code) = match line.contains(';').then(|| rest.rsplit_once(';')).flatten() {
            Some((name, code)) if !code.trim().is_empty() => {
                let code = code.trim();
                if bok_store::box_title(code).is_none() {
                    bail!("rad {}: {code} är ingen ruta i momsdeklarationen", i + 1);
                }
                (name, Some(code.to_string()))
            }
            Some((name, _)) => (name, None),
            None => (rest, None),
        };
        let name = name.trim().trim_matches('"').trim();
        if name.is_empty() {
            bail!("rad {}: konto {number} saknar namn", i + 1);
        }
        out.push(AccountChange {
            number: number
                .trim()
                .parse()
                .with_context(|| format!("rad {}", i + 1))?,
            name: name.to_string(),
            active: true,
            sru: None,
            vat_code,
        });
    }
    if out.is_empty() {
        bail!("{} innehåller inga konton", path.display());
    }
    Ok(out)
}

/// Ett räkenskapsår: `2026` för kalenderåret, eller `2025-07-01:2026-06-30`.
pub fn parse_year_spec(spec: &str) -> Result<(NaiveDate, NaiveDate)> {
    let spec = spec.trim();
    if let Ok(year) = spec.parse::<i32>() {
        let start =
            NaiveDate::from_ymd_opt(year, 1, 1).ok_or_else(|| anyhow!("ogiltigt år {year}"))?;
        let end =
            NaiveDate::from_ymd_opt(year, 12, 31).ok_or_else(|| anyhow!("ogiltigt år {year}"))?;
        return Ok((start, end));
    }
    let (a, b) = spec
        .split_once(':')
        .ok_or_else(|| anyhow!("skriv räkenskapsåret som 2026 eller 2025-07-01:2026-06-30"))?;
    let start = parse_date(a)?;
    let end = parse_date(b)?;
    if start.year() > end.year() + 1 {
        bail!("räkenskapsåret {spec} är för långt");
    }
    Ok((start, end))
}

pub fn parse_date(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
        .map_err(|_| anyhow!("\"{s}\" är inget datum (skriv ÅÅÅÅ-MM-DD)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_from_the_command_line() {
        let l = parse_line("5410=80,00:Papper").unwrap();
        assert_eq!(
            (l.account.value(), l.debit.ore(), l.credit.ore()),
            (5410, 8000, 0)
        );
        assert_eq!(l.text.as_deref(), Some("Papper"));
        let l = parse_line("1930=-0,50").unwrap();
        assert_eq!((l.debit.ore(), l.credit.ore()), (0, 50));
        assert!(parse_line("1930").is_err());
        assert!(parse_line("1930=0").is_err());
        assert!(parse_line("193=5").is_err());
    }

    #[test]
    fn year_specs() {
        let (s, e) = parse_year_spec("2026").unwrap();
        assert_eq!(
            (s.to_string(), e.to_string()),
            ("2026-01-01".into(), "2026-12-31".into())
        );
        let (s, e) = parse_year_spec("2025-07-01:2026-06-30").unwrap();
        assert_eq!(
            (s.to_string(), e.to_string()),
            ("2025-07-01".into(), "2026-06-30".into())
        );
        assert!(parse_year_spec("i år").is_err());
    }

    #[test]
    fn proposal_file_uses_ore_and_swedish_keys() {
        let json = r#"{"datum": "2026-03-15", "beskrivning": "Papper",
            "motpart": {"namn": "Papper AB", "orgnr": "556000-0000"},
            "rader": [{"konto": 5410, "debet": 8000}, {"konto": 1930, "kredit": 8000}]}"#;
        let p: ProposalFile = serde_json::from_str(json).unwrap();
        let p = p.into_proposal();
        assert_eq!(p.series, Series::A);
        assert_eq!(p.lines[1].credit.ore(), 8000);
        assert!(matches!(
            p.counterparty,
            Some(ProposalCounterparty::Inline(_))
        ));
        let by_id: ProposalFile = serde_json::from_str(
            r#"{"datum": "2026-03-15", "beskrivning": "x", "motpart": {"id": 3}, "rader": []}"#,
        )
        .unwrap();
        assert!(matches!(
            by_id.into_proposal().counterparty,
            Some(ProposalCounterparty::Register(3))
        ));
        assert!(
            serde_json::from_str::<ProposalFile>(
                r#"{"datum": "2026-03-15", "beskrivning": "x", "rader": [], "nummer": 5}"#
            )
            .is_err()
        );
    }
}
