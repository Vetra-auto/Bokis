use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FiscalYear {
    pub id: Uuid,
    pub company_id: Uuid,
    pub name: String,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub is_closed: bool,
}

pub fn generate_monthly_periods(start: NaiveDate, end: NaiveDate) -> Vec<(NaiveDate, NaiveDate, String)> {
    let mut periods = Vec::new();
    let mut current = start;

    while current <= end {
        let year = current.year();
        let month = current.month();
        let period_start = NaiveDate::from_ymd_opt(year, month, 1).unwrap();

        let next_month = if month == 12 {
            NaiveDate::from_ymd_opt(year + 1, 1, 1).unwrap()
        } else {
            NaiveDate::from_ymd_opt(year, month + 1, 1).unwrap()
        };
        let period_end = next_month.pred_opt().unwrap();
        let period_end = if period_end > end { end } else { period_end };

        let year_month = format!("{:04}-{:02}", year, month);
        periods.push((period_start, period_end, year_month));

        current = next_month;
    }

    periods
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_12_periods_for_calendar_year() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 12, 31).unwrap();
        let periods = generate_monthly_periods(start, end);
        assert_eq!(periods.len(), 12);
        assert_eq!(periods[0].2, "2026-01");
        assert_eq!(periods[11].2, "2026-12");
        assert_eq!(periods[0].0, NaiveDate::from_ymd_opt(2026, 1, 1).unwrap());
        assert_eq!(periods[0].1, NaiveDate::from_ymd_opt(2026, 1, 31).unwrap());
        assert_eq!(periods[1].0, NaiveDate::from_ymd_opt(2026, 2, 1).unwrap());
        assert_eq!(periods[1].1, NaiveDate::from_ymd_opt(2026, 2, 28).unwrap());
    }

    #[test]
    fn handles_broken_fiscal_year() {
        let start = NaiveDate::from_ymd_opt(2025, 7, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 6, 30).unwrap();
        let periods = generate_monthly_periods(start, end);
        assert_eq!(periods.len(), 12);
        assert_eq!(periods[0].2, "2025-07");
        assert_eq!(periods[11].2, "2026-06");
    }
}
