//! Räkenskapsår och dess månadsperioder.

use chrono::{Datelike, Months, NaiveDate};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FiscalYearError {
    #[error("räkenskapsåret måste börja den första i en månad")]
    StartNotFirstOfMonth,
    #[error("räkenskapsåret måste sluta den sista i en månad")]
    EndNotLastOfMonth,
    #[error("räkenskapsåret måste vara 1–18 månader långt (BFL 3 kap.)")]
    BadLength,
}

/// Kontrollerar att ett räkenskapsår börjar den första i en månad, slutar
/// den sista i en månad och är högst 18 månader (BFL 3 kap. 1–3 §§).
pub fn validate(start: NaiveDate, end: NaiveDate) -> Result<(), FiscalYearError> {
    if start.day() != 1 {
        return Err(FiscalYearError::StartNotFirstOfMonth);
    }
    validate_first(start, end)
}

/// Som [`validate`], men för bokföringens första räkenskapsår: det börjar
/// när bokföringsskyldigheten inträder, till exempel den dag ett aktiebolag
/// bildas, och får därför börja vilken dag som helst (BFL 2 kap. 1 §,
/// 3 kap. 3 §). Det slutar ändå den sista i en månad och är högst 18
/// månader; den första, bruten månaden räknas som en hel.
pub fn validate_first(start: NaiveDate, end: NaiveDate) -> Result<(), FiscalYearError> {
    if end.succ_opt().is_none_or(|next| next.day() != 1) {
        return Err(FiscalYearError::EndNotLastOfMonth);
    }
    let months =
        (end.year() * 12 + end.month() as i32) - (start.year() * 12 + start.month() as i32) + 1;
    if end < start || !(1..=18).contains(&months) {
        return Err(FiscalYearError::BadLength);
    }
    Ok(())
}

/// En period per kalendermånad, från `start` till och med `end`.
/// Flyttad från `bokforing-core::fiscal_year`.
pub fn monthly_periods(start: NaiveDate, end: NaiveDate) -> Vec<(NaiveDate, NaiveDate)> {
    let mut periods = Vec::new();
    let mut current = start;
    while current <= end {
        let first_of_month = current.with_day(1).unwrap_or(current);
        let next = first_of_month.checked_add_months(Months::new(1));
        let last = next.and_then(|n| n.pred_opt()).unwrap_or(end).min(end);
        periods.push((current, last));
        match next {
            Some(n) => current = n,
            None => break,
        }
    }
    periods
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn calendar_year_has_twelve_periods() {
        let p = monthly_periods(d(2026, 1, 1), d(2026, 12, 31));
        assert_eq!(p.len(), 12);
        assert_eq!(p[0], (d(2026, 1, 1), d(2026, 1, 31)));
        assert_eq!(p[1], (d(2026, 2, 1), d(2026, 2, 28)));
        assert_eq!(p[11], (d(2026, 12, 1), d(2026, 12, 31)));
    }

    #[test]
    fn broken_and_extended_years() {
        let p = monthly_periods(d(2025, 7, 1), d(2026, 6, 30));
        assert_eq!(p.len(), 12);
        assert_eq!(p[0].0, d(2025, 7, 1));
        assert_eq!(p[11].1, d(2026, 6, 30));
        assert!(validate(d(2025, 7, 1), d(2026, 6, 30)).is_ok());
        assert!(validate(d(2025, 7, 1), d(2026, 12, 31)).is_ok());
        assert_eq!(monthly_periods(d(2025, 7, 1), d(2026, 12, 31)).len(), 18);
    }

    #[test]
    fn the_first_year_may_start_on_any_day() {
        assert!(validate_first(d(2026, 5, 11), d(2026, 12, 31)).is_ok());
        let p = monthly_periods(d(2026, 5, 11), d(2026, 12, 31));
        assert_eq!(p.len(), 8);
        assert_eq!(p[0], (d(2026, 5, 11), d(2026, 5, 31)));
        assert_eq!(
            validate_first(d(2026, 5, 11), d(2026, 12, 30)),
            Err(FiscalYearError::EndNotLastOfMonth)
        );
        assert_eq!(
            validate_first(d(2026, 5, 11), d(2027, 11, 30)),
            Err(FiscalYearError::BadLength)
        );
        assert!(validate_first(d(2026, 5, 11), d(2027, 10, 31)).is_ok());
        assert_eq!(
            validate(d(2026, 5, 11), d(2026, 12, 31)),
            Err(FiscalYearError::StartNotFirstOfMonth)
        );
    }

    #[test]
    fn invalid_years_are_refused() {
        assert_eq!(
            validate(d(2026, 1, 2), d(2026, 12, 31)),
            Err(FiscalYearError::StartNotFirstOfMonth)
        );
        assert_eq!(
            validate(d(2026, 1, 1), d(2026, 12, 30)),
            Err(FiscalYearError::EndNotLastOfMonth)
        );
        assert_eq!(
            validate(d(2026, 1, 1), d(2027, 7, 31)),
            Err(FiscalYearError::BadLength)
        );
        assert_eq!(
            validate(d(2026, 2, 1), d(2026, 1, 31)),
            Err(FiscalYearError::BadLength)
        );
    }
}
