//! Verifikationer: rader, motpart, validering och den bokförda formen.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::account::AccountNumber;
use crate::canonical::{CanonicalWriter, sha256_hex};
use crate::money::{Money, checked_sum};
use crate::series::{Series, VoucherRef};

/// Längsta tillåtna text i en beskrivning eller radtext.
pub const MAX_TEXT_LEN: usize = 1000;

/// Version och domän för verifikationens kanoniska form.
pub const CANONICAL_DOMAIN: &str = "bokis-verifikation-v1";

/// En konteringsrad. Exakt en av debet och kredit är större än noll.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    #[serde(rename = "konto")]
    pub account: AccountNumber,
    #[serde(rename = "debet", default)]
    pub debit: Money,
    #[serde(rename = "kredit", default)]
    pub credit: Money,
    #[serde(rename = "momskod", default, skip_serializing_if = "Option::is_none")]
    pub vat_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

impl Line {
    pub fn debit(account: AccountNumber, amount: Money) -> Self {
        Line {
            account,
            debit: amount,
            credit: Money::ZERO,
            vat_code: None,
            text: None,
        }
    }

    pub fn credit(account: AccountNumber, amount: Money) -> Self {
        Line {
            account,
            debit: Money::ZERO,
            credit: amount,
            vat_code: None,
            text: None,
        }
    }

    /// Positivt belopp blir debet, negativt kredit.
    pub fn from_signed(account: AccountNumber, amount: Money) -> Self {
        if amount.is_negative() {
            Line::credit(account, -amount)
        } else {
            Line::debit(account, amount)
        }
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    /// Debet minus kredit.
    pub fn net(&self) -> Money {
        self.debit - self.credit
    }
}

/// Motpart som den kopieras in på verifikationen vid bokföring.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counterparty {
    #[serde(rename = "namn")]
    pub name: String,
    #[serde(rename = "orgnr", default, skip_serializing_if = "Option::is_none")]
    pub org_number: Option<String>,
    #[serde(rename = "momsnr", default, skip_serializing_if = "Option::is_none")]
    pub vat_number: Option<String>,
    /// Postadressen, på en rad. Namn och adress räcker för att identifiera
    /// en motpart som saknar organisationsnummer (BFNAR 2013:2, kommentaren
    /// till punkt 5.6).
    #[serde(rename = "adress", default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum VoucherError {
    #[error("verifikationen måste ha minst två rader")]
    TooFewLines,
    #[error("rad {line} har varken debet eller kredit")]
    EmptyLine { line: usize },
    #[error("rad {line} har både debet och kredit")]
    BothSides { line: usize },
    #[error("rad {line} har ett negativt belopp; använd motsatt sida i stället")]
    NegativeAmount { line: usize },
    #[error("debet ({debit}) och kredit ({credit}) balanserar inte")]
    Unbalanced { debit: Money, credit: Money },
    #[error("summan av raderna är för stor")]
    Overflow,
    #[error("verifikationen måste beskriva vad affärshändelsen avser")]
    MissingDescription,
    #[error("texten i {field} är längre än {MAX_TEXT_LEN} tecken")]
    TextTooLong { field: &'static str },
    #[error("en verifikation i serie R måste peka på verifikationen den rättar")]
    SeriesRWithoutOriginal,
    #[error("en rättelse måste läggas i serie R")]
    CorrectionOutsideSeriesR,
    #[error("motparten måste ha ett namn")]
    MissingCounterpartyName,
    /// Motpartens organisations- eller personnummer går inte igenom
    /// kontrollen; texten säger varför.
    #[error("motpartens nummer: {0}")]
    InvalidPartyNumber(String),
    #[error(
        "ingående balans (serie I) får bara innehålla balanskonton; {account} är ett resultatkonto"
    )]
    ResultAccountInOpeningBalance { account: AccountNumber },
}

fn check_text(field: &'static str, text: &str) -> Result<(), VoucherError> {
    if text.chars().count() > MAX_TEXT_LEN {
        Err(VoucherError::TextTooLong { field })
    } else {
        Ok(())
    }
}

/// Kontrollerar raderna och ger summan av debet och kredit.
/// Balanskontrollen är flyttad från `bokforing-core::ledger`.
pub fn validate_lines(lines: &[Line]) -> Result<(Money, Money), VoucherError> {
    if lines.len() < 2 {
        return Err(VoucherError::TooFewLines);
    }
    for (i, line) in lines.iter().enumerate() {
        let n = i + 1;
        if line.debit.is_negative() || line.credit.is_negative() {
            return Err(VoucherError::NegativeAmount { line: n });
        }
        match (line.debit.is_zero(), line.credit.is_zero()) {
            (true, true) => return Err(VoucherError::EmptyLine { line: n }),
            (false, false) => return Err(VoucherError::BothSides { line: n }),
            _ => {}
        }
        if let Some(t) = &line.text {
            check_text("radtexten", t)?;
        }
        if let Some(v) = &line.vat_code {
            check_text("momskoden", v)?;
        }
    }
    let debit = checked_sum(lines.iter().map(|l| l.debit)).ok_or(VoucherError::Overflow)?;
    let credit = checked_sum(lines.iter().map(|l| l.credit)).ok_or(VoucherError::Overflow)?;
    if debit != credit {
        return Err(VoucherError::Unbalanced { debit, credit });
    }
    Ok((debit, credit))
}

pub fn validate_description(description: &str) -> Result<(), VoucherError> {
    if description.trim().is_empty() {
        return Err(VoucherError::MissingDescription);
    }
    check_text("beskrivningen", description)
}

/// Kontrollerar motparten: namnet måste finnas, och ett svenskt
/// organisations- eller personnummer måste ha rätt form och kontrollsiffra.
/// Ett utländskt nummer anges med landskoden först (till exempel DK12345678).
pub fn validate_counterparty(c: &Counterparty) -> Result<(), VoucherError> {
    if c.name.trim().is_empty() {
        return Err(VoucherError::MissingCounterpartyName);
    }
    check_text("motpartens namn", &c.name)?;
    if let Some(a) = &c.address {
        check_text("motpartens adress", a)?;
    }
    if let Some(number) = c.org_number.as_deref().filter(|n| !n.trim().is_empty()) {
        check_text("motpartens nummer", number)?;
        crate::id_number::normalize_party_number(number)
            .map_err(|e| VoucherError::InvalidPartyNumber(e.to_string()))?;
    }
    Ok(())
}

/// Motparten på normalform: namnet utan omgivande blanksteg, numret som
/// NNNNNN-NNNN (eller med landskod) och tomma fält borttagna. Anropas efter
/// [`validate_counterparty`].
pub fn normalize_counterparty(c: &Counterparty) -> Counterparty {
    let clean = |s: Option<&str>| {
        s.map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
    };
    Counterparty {
        name: c.name.trim().to_string(),
        org_number: clean(c.org_number.as_deref())
            .map(|n| crate::id_number::normalize_party_number(&n).unwrap_or(n)),
        vat_number: clean(c.vat_number.as_deref()),
        address: c.address.as_deref().and_then(one_line_address),
    }
}

/// En adress på en rad: varje rad utan överflödiga blanksteg, åtskilda med
/// kommatecken. Ingenting kvar ger `None`.
pub fn one_line_address(text: &str) -> Option<String> {
    let lines: Vec<String> = text
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .map(|l| l.trim_matches(',').trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    (!lines.is_empty()).then(|| lines.join(", "))
}

/// Serie R och en hänvisning till originalet hör ihop: varje rättelse ligger
/// i R, och allt i R är en rättelse.
pub fn validate_series(series: Series, corrects_something: bool) -> Result<(), VoucherError> {
    match (series == Series::R, corrects_something) {
        (true, false) => Err(VoucherError::SeriesRWithoutOriginal),
        (false, true) => Err(VoucherError::CorrectionOutsideSeriesR),
        _ => Ok(()),
    }
}

/// Ingående balans (serie I) gäller bara balanskonton (klass 1–2).
pub fn validate_series_lines(series: Series, lines: &[Line]) -> Result<(), VoucherError> {
    if series == Series::I
        && let Some(line) = lines.iter().find(|l| l.account.is_result_account())
    {
        return Err(VoucherError::ResultAccountInOpeningBalance {
            account: line.account,
        });
    }
    Ok(())
}

/// Originalet som en rättelse pekar på. Hashen binder rättelsen till exakt
/// det innehåll som rättas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorrectedOriginal {
    pub reference: VoucherRef,
    pub fiscal_year_start: NaiveDate,
    pub canonical_hash: String,
}

/// En bokförd verifikation med alla uppgifter som signaturen täcker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostedVoucher {
    pub series: Series,
    pub number: i64,
    pub fiscal_year: (NaiveDate, NaiveDate),
    pub period: (NaiveDate, NaiveDate),
    /// När affärshändelsen inträffade.
    pub date: NaiveDate,
    /// När verifikationen sammanställdes.
    pub compiled_on: NaiveDate,
    /// Registreringstid i UTC, lagrad ordagrant så att formen går att räkna om.
    pub registered_at: String,
    pub description: String,
    pub counterparty: Option<Counterparty>,
    pub proposal_id: i64,
    /// SHA-256 för underlagen som länkades vid bokföringen.
    pub attachments: Vec<String>,
    pub corrects: Option<CorrectedOriginal>,
    pub lines: Vec<Line>,
}

fn date_range(r: (NaiveDate, NaiveDate)) -> String {
    format!("{}..{}", r.0, r.1)
}

impl PostedVoucher {
    pub fn reference(&self) -> VoucherRef {
        VoucherRef {
            series: self.series,
            number: self.number,
        }
    }

    /// Den kanoniska formen som signeras. Den täcker raderna, så även en rad
    /// som läggs till förbi programmet syns vid verifiering.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut w = CanonicalWriter::new(CANONICAL_DOMAIN);
        w.field("serie", self.series.as_str());
        w.int("nummer", self.number);
        w.field("rakenskapsar", &date_range(self.fiscal_year));
        w.field("period", &date_range(self.period));
        w.field("datum", &self.date.to_string());
        w.field("sammanstallningsdag", &self.compiled_on.to_string());
        w.field("registrerad", &self.registered_at);
        w.field("beskrivning", &self.description);
        let cp = self.counterparty.as_ref();
        w.opt("motpart.namn", cp.map(|c| c.name.as_str()));
        w.opt("motpart.orgnr", cp.and_then(|c| c.org_number.as_deref()));
        w.opt("motpart.momsnr", cp.and_then(|c| c.vat_number.as_deref()));
        // Adressen kom i schemaversion 5. Den skrivs bara när den finns, så
        // att verifikationer utan adress behåller sin kanoniska form.
        if let Some(a) = cp.and_then(|c| c.address.as_deref()) {
            w.field("motpart.adress", a);
        }
        w.int("forslag", self.proposal_id);
        let mut attachments = self.attachments.clone();
        attachments.sort();
        w.int("underlag.antal", attachments.len() as i64);
        for a in &attachments {
            w.field("underlag", a);
        }
        match &self.corrects {
            Some(o) => {
                w.field("rattar.ref", &o.reference.to_string());
                w.field("rattar.rakenskapsar", &o.fiscal_year_start.to_string());
                w.field("rattar.hash", &o.canonical_hash);
            }
            None => w.opt("rattar.ref", None),
        }
        w.int("rader.antal", self.lines.len() as i64);
        for (i, line) in self.lines.iter().enumerate() {
            w.int("rad.nr", i as i64 + 1);
            w.field("rad.konto", &line.account.to_string());
            w.int("rad.debet", line.debit.ore());
            w.int("rad.kredit", line.credit.ore());
            w.opt("rad.momskod", line.vat_code.as_deref());
            w.opt("rad.text", line.text.as_deref());
        }
        w.finish()
    }

    pub fn canonical_hash(&self) -> String {
        sha256_hex(&self.canonical_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acc(n: i64) -> AccountNumber {
        AccountNumber::new(n).unwrap()
    }

    fn kr(ore: i64) -> Money {
        Money::from_ore(ore)
    }

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn sample() -> PostedVoucher {
        PostedVoucher {
            series: Series::A,
            number: 12,
            fiscal_year: (d(2026, 1, 1), d(2026, 12, 31)),
            period: (d(2026, 3, 1), d(2026, 3, 31)),
            date: d(2026, 3, 15),
            compiled_on: d(2026, 3, 16),
            registered_at: "2026-03-16T10:22:33.123456Z".into(),
            description: "Kontorsmaterial".into(),
            counterparty: Some(Counterparty {
                name: "Papper & Penna AB".into(),
                org_number: Some("556000-0000".into()),
                vat_number: None,
                address: None,
            }),
            proposal_id: 7,
            attachments: vec!["b".repeat(64), "a".repeat(64)],
            corrects: None,
            lines: vec![
                Line::debit(acc(5410), kr(8000)),
                Line::debit(acc(2641), kr(2000)),
                Line::credit(acc(1930), kr(10000)),
            ],
        }
    }

    #[test]
    fn balanced_lines_pass() {
        let lines = sample().lines;
        assert_eq!(validate_lines(&lines), Ok((kr(10000), kr(10000))));
    }

    #[test]
    fn line_rules() {
        assert_eq!(
            validate_lines(&[Line::debit(acc(1930), kr(1))]),
            Err(VoucherError::TooFewLines)
        );
        let unbalanced = [
            Line::debit(acc(5410), kr(100)),
            Line::credit(acc(1930), kr(99)),
        ];
        assert!(matches!(
            validate_lines(&unbalanced),
            Err(VoucherError::Unbalanced { .. })
        ));
        let empty = [
            Line::debit(acc(5410), kr(0)),
            Line::credit(acc(1930), kr(0)),
        ];
        assert_eq!(
            validate_lines(&empty),
            Err(VoucherError::EmptyLine { line: 1 })
        );
        let mut both = Line::debit(acc(5410), kr(5));
        both.credit = kr(5);
        assert_eq!(
            validate_lines(&[both, Line::credit(acc(1930), kr(0))]),
            Err(VoucherError::BothSides { line: 1 })
        );
        let negative = [
            Line::debit(acc(5410), kr(-5)),
            Line::credit(acc(1930), kr(-5)),
        ];
        assert_eq!(
            validate_lines(&negative),
            Err(VoucherError::NegativeAmount { line: 1 })
        );
        let huge = [
            Line::debit(acc(5410), kr(i64::MAX)),
            Line::debit(acc(5410), kr(1)),
            Line::credit(acc(1930), kr(1)),
        ];
        assert_eq!(validate_lines(&huge), Err(VoucherError::Overflow));
    }

    #[test]
    fn series_r_and_corrections_go_together() {
        assert!(validate_series(Series::R, true).is_ok());
        assert!(validate_series(Series::A, false).is_ok());
        assert_eq!(
            validate_series(Series::R, false),
            Err(VoucherError::SeriesRWithoutOriginal)
        );
        assert_eq!(
            validate_series(Series::A, true),
            Err(VoucherError::CorrectionOutsideSeriesR)
        );
        assert_eq!(
            validate_description("  "),
            Err(VoucherError::MissingDescription)
        );
        let ib = [
            Line::debit(acc(1930), kr(100)),
            Line::credit(acc(2081), kr(100)),
        ];
        assert!(validate_series_lines(Series::I, &ib).is_ok());
        let wrong = [
            Line::debit(acc(1930), kr(100)),
            Line::credit(acc(3010), kr(100)),
        ];
        assert!(matches!(
            validate_series_lines(Series::I, &wrong),
            Err(VoucherError::ResultAccountInOpeningBalance { .. })
        ));
        assert!(validate_series_lines(Series::A, &wrong).is_ok());
    }

    #[test]
    fn canonical_form_is_deterministic() {
        let a = sample();
        let mut b = sample();
        b.attachments.reverse();
        assert_eq!(a.canonical_hash(), b.canonical_hash());
        assert_eq!(a.canonical_hash().len(), 64);
        let text = String::from_utf8(a.canonical_bytes()).unwrap();
        assert!(text.starts_with("bokis-verifikation-v1\nserie:1:A\nnummer:2:12\n"));
        assert!(text.contains("motpart.namn:17:Papper & Penna AB\n"));
        assert!(text.contains("rad.konto:4:2641\nrad.debet:4:2000\nrad.kredit:1:0\n"));
    }

    #[test]
    fn canonical_hash_covers_every_field() {
        let base = sample().canonical_hash();
        let mut changes: Vec<PostedVoucher> = Vec::new();
        let mut v = sample();
        v.number = 13;
        changes.push(v);
        let mut v = sample();
        v.registered_at = "2026-03-16T10:22:33.123457Z".into();
        changes.push(v);
        let mut v = sample();
        v.lines[0].debit = kr(8001);
        changes.push(v);
        let mut v = sample();
        v.lines.push(Line::debit(acc(6570), kr(0)));
        changes.push(v);
        let mut v = sample();
        v.counterparty = None;
        changes.push(v);
        let mut v = sample();
        v.attachments.pop();
        changes.push(v);
        let mut v = sample();
        v.lines[2].text = Some("bank".into());
        changes.push(v);
        let mut v = sample();
        v.corrects = Some(CorrectedOriginal {
            reference: "A1".parse().unwrap(),
            fiscal_year_start: d(2026, 1, 1),
            canonical_hash: "c".repeat(64),
        });
        changes.push(v);
        for changed in changes {
            assert_ne!(changed.canonical_hash(), base);
        }
    }

    #[test]
    fn lines_round_trip_through_json_in_ore() {
        let line = Line::credit(acc(1930), kr(1)).with_text("x");
        let json = serde_json::to_string(&Line::debit(acc(1930), kr(12550))).unwrap();
        assert_eq!(json, r#"{"konto":1930,"debet":12550,"kredit":0}"#);
        let back: Line = serde_json::from_str(r#"{"konto":5410,"kredit":5}"#).unwrap();
        assert_eq!(back, Line::credit(acc(5410), kr(5)));
        assert!(serde_json::from_str::<Line>(r#"{"konto":99,"debet":5}"#).is_err());
        assert_eq!(line.text.as_deref(), Some("x"));
        assert_eq!(
            Line::from_signed(acc(1930), kr(-1)),
            Line::credit(acc(1930), kr(1))
        );
    }

    #[test]
    fn counterparty_numbers_are_checked_and_normalized() {
        let cp = |n: &str| Counterparty {
            name: " Exempel AB ".into(),
            org_number: Some(n.into()),
            vat_number: Some(" ".into()),
            address: None,
        };
        assert!(validate_counterparty(&cp("5560000100")).is_ok());
        assert!(validate_counterparty(&cp("")).is_ok());
        assert!(validate_counterparty(&cp("DK12345678")).is_ok());
        assert!(matches!(
            validate_counterparty(&cp("559000-0000")),
            Err(VoucherError::InvalidPartyNumber(_))
        ));
        let n = normalize_counterparty(&cp("5560000100"));
        assert_eq!(n.name, "Exempel AB");
        assert_eq!(n.org_number.as_deref(), Some("556000-0100"));
        assert_eq!(n.vat_number, None);
    }

    #[test]
    fn the_address_is_in_the_canonical_form_only_when_given() {
        let without = sample();
        let text = String::from_utf8(without.canonical_bytes()).unwrap();
        assert!(!text.contains("motpart.adress"), "{text}");
        let mut with = sample();
        with.counterparty.as_mut().unwrap().address =
            Some("Storgatan 1, 123 45 Exempelstad".into());
        let text = String::from_utf8(with.canonical_bytes()).unwrap();
        assert!(
            text.contains(
                "motpart.momsnr:-\nmotpart.adress:31:Storgatan 1, 123 45 Exempelstad\nforslag:"
            ),
            "{text}"
        );
        assert_ne!(with.canonical_hash(), without.canonical_hash());
    }

    #[test]
    fn addresses_end_up_on_one_line() {
        assert_eq!(
            one_line_address("  Storgatan  1,\n\n123 45   Exempelstad \n").as_deref(),
            Some("Storgatan 1, 123 45 Exempelstad")
        );
        assert_eq!(one_line_address(" \n "), None);
        let cp = Counterparty {
            name: "Anna Exempel".into(),
            org_number: None,
            vat_number: None,
            address: Some("Storgatan 1\n123 45 Exempelstad".into()),
        };
        assert_eq!(
            normalize_counterparty(&cp).address.as_deref(),
            Some("Storgatan 1, 123 45 Exempelstad")
        );
    }
}
