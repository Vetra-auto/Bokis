//! Kanoniska former för signerade beslut utöver bokföringen: registrering
//! av en nyckel, stängning eller låsning av en period och ändring av
//! kontoplanen. Signaturen görs över formens SHA-256 (hex).

use chrono::NaiveDate;

use crate::account::AccountNumber;
use crate::canonical::{CanonicalWriter, sha256_hex};
use crate::period::PeriodStatus;

pub fn key_registration_hash(
    name: &str,
    public_key: &str,
    fingerprint: &str,
    registered_at: &str,
) -> String {
    let mut w = CanonicalWriter::new("bokis-nyckel-v1");
    w.field("namn", name);
    w.field("nyckel", public_key);
    w.field("fingeravtryck", fingerprint);
    w.field("registrerad", registered_at);
    sha256_hex(&w.finish())
}

pub fn period_decision_hash(
    period_start: NaiveDate,
    period_end: NaiveDate,
    from: PeriodStatus,
    to: PeriodStatus,
    decided_at: &str,
) -> String {
    let mut w = CanonicalWriter::new("bokis-periodbeslut-v1");
    w.field("period", &format!("{period_start}..{period_end}"));
    w.field("fran", from.as_db_str());
    w.field("till", to.as_db_str());
    w.field("tid", decided_at);
    sha256_hex(&w.finish())
}

/// Ett konto i en signerad ändring av kontoplanen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChartEntry<'a> {
    pub number: AccountNumber,
    pub name: &'a str,
    pub active: bool,
    pub sru: Option<&'a str>,
    pub vat_code: Option<&'a str>,
}

/// Hashen för en ändring av kontoplanen. Ordningen i indata spelar ingen
/// roll; kontona sorteras.
pub fn chart_change_hash(valid_from: NaiveDate, entries: &[ChartEntry<'_>]) -> String {
    let mut sorted = entries.to_vec();
    sorted.sort_by_key(|e| e.number);
    let mut w = CanonicalWriter::new("bokis-kontoplan-v1");
    w.field("giltig_fran", &valid_from.to_string());
    w.int("antal", sorted.len() as i64);
    for e in &sorted {
        w.field("konto", &e.number.to_string());
        w.field("namn", e.name);
        w.field("aktiv", if e.active { "1" } else { "0" });
        w.opt("sru", e.sru);
        w.opt("momskod", e.vat_code);
    }
    sha256_hex(&w.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chart_hash_ignores_input_order_but_not_content() {
        let d = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let a = ChartEntry {
            number: AccountNumber::new(1930).unwrap(),
            name: "Bank",
            active: true,
            sru: None,
            vat_code: None,
        };
        let b = ChartEntry {
            number: AccountNumber::new(2440).unwrap(),
            name: "Leverantörsskulder",
            active: true,
            sru: None,
            vat_code: None,
        };
        assert_eq!(chart_change_hash(d, &[a, b]), chart_change_hash(d, &[b, a]));
        let renamed = ChartEntry {
            name: "Företagskonto",
            ..a
        };
        assert_ne!(
            chart_change_hash(d, &[a, b]),
            chart_change_hash(d, &[renamed, b])
        );
        let inactive = ChartEntry { active: false, ..a };
        assert_ne!(
            chart_change_hash(d, &[a]),
            chart_change_hash(d, &[inactive])
        );
    }

    #[test]
    fn period_hash_depends_on_transition_and_time() {
        let s = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let e = NaiveDate::from_ymd_opt(2026, 3, 31).unwrap();
        let close =
            period_decision_hash(s, e, PeriodStatus::UnderReview, PeriodStatus::Closed, "t1");
        let lock = period_decision_hash(s, e, PeriodStatus::Closed, PeriodStatus::Locked, "t1");
        let later =
            period_decision_hash(s, e, PeriodStatus::UnderReview, PeriodStatus::Closed, "t2");
        assert_ne!(close, lock);
        assert_ne!(close, later);
    }
}
