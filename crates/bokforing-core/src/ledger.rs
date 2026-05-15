use chrono::NaiveDate;

use crate::error::LedgerError;
use crate::types::{Money, PeriodStatus, VoucherStatus};
use crate::voucher::{Voucher, VoucherLine};

pub struct PeriodInfo {
    pub status: PeriodStatus,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
}

pub struct AccountInfo {
    pub is_active: bool,
    pub account_number: u16,
}

pub fn validate_voucher_for_posting(
    voucher: &Voucher,
    lines: &[VoucherLine],
    period: &PeriodInfo,
    accounts: &[AccountInfo],
) -> Result<(), LedgerError> {
    if voucher.status != VoucherStatus::Draft {
        return Err(LedgerError::AlreadyPosted {
            status: voucher.status,
        });
    }

    if !period.status.allows_posting() {
        return Err(LedgerError::PeriodNotOpen {
            status: period.status,
        });
    }

    if lines.len() < 2 {
        return Err(LedgerError::TooFewLines);
    }

    if voucher.voucher_date < period.start_date || voucher.voucher_date > period.end_date {
        return Err(LedgerError::DateOutsidePeriod {
            date: voucher.voucher_date.to_string(),
            period_start: period.start_date.to_string(),
            period_end: period.end_date.to_string(),
        });
    }

    let mut total_debit = Money::from_ore(0);
    let mut total_credit = Money::from_ore(0);

    for (i, line) in lines.iter().enumerate() {
        let d = line.debit_amount.ore();
        let c = line.credit_amount.ore();

        if d == 0 && c == 0 {
            return Err(LedgerError::EmptyLine { line: i + 1 });
        }
        if d > 0 && c > 0 {
            return Err(LedgerError::BothDebitAndCredit { line: i + 1 });
        }

        if let Some(acc) = accounts.iter().find(|a| a.account_number == line.account_number) {
            if !acc.is_active {
                return Err(LedgerError::InactiveAccount {
                    account: line.account_number,
                });
            }
        }

        total_debit = total_debit + line.debit_amount;
        total_credit = total_credit + line.credit_amount;
    }

    if total_debit != total_credit {
        return Err(LedgerError::Unbalanced {
            debit: total_debit.ore(),
            credit: total_credit.ore(),
        });
    }

    Ok(())
}

pub fn sum_debit(lines: &[VoucherLine]) -> Money {
    lines
        .iter()
        .fold(Money::from_ore(0), |acc, l| acc + l.debit_amount)
}

pub fn sum_credit(lines: &[VoucherLine]) -> Money {
    lines
        .iter()
        .fold(Money::from_ore(0), |acc, l| acc + l.credit_amount)
}

pub fn is_balanced(lines: &[VoucherLine]) -> bool {
    sum_debit(lines) == sum_credit(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::VoucherStatus;
    use chrono::Utc;
    use uuid::Uuid;

    fn make_voucher(date: NaiveDate) -> Voucher {
        Voucher {
            id: Uuid::new_v4(),
            company_id: Uuid::new_v4(),
            fiscal_year_id: Uuid::new_v4(),
            period_id: Uuid::new_v4(),
            voucher_series: "A".to_string(),
            voucher_number: 1,
            voucher_date: date,
            description: "Test".to_string(),
            status: VoucherStatus::Draft,
            counterparty_id: None,
            created_by: Uuid::new_v4(),
            created_at: Utc::now(),
            posted_at: None,
            posted_by: None,
        }
    }

    fn make_line(voucher_id: Uuid, num: u16, debit: i64, credit: i64) -> VoucherLine {
        VoucherLine {
            id: Uuid::new_v4(),
            voucher_id,
            line_number: 1,
            account_id: Uuid::new_v4(),
            account_number: num,
            debit_amount: Money::from_ore(debit),
            credit_amount: Money::from_ore(credit),
            description: None,
            counterparty_id: None,
            cost_center: None,
            project: None,
        }
    }

    fn open_period() -> PeriodInfo {
        PeriodInfo {
            status: PeriodStatus::Open,
            start_date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            end_date: NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
        }
    }

    fn active_accounts() -> Vec<AccountInfo> {
        vec![
            AccountInfo { is_active: true, account_number: 1910 },
            AccountInfo { is_active: true, account_number: 2440 },
        ]
    }

    #[test]
    fn valid_voucher_passes() {
        let date = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
        let v = make_voucher(date);
        let lines = vec![
            make_line(v.id, 1910, 10000, 0),
            make_line(v.id, 2440, 0, 10000),
        ];
        assert!(validate_voucher_for_posting(&v, &lines, &open_period(), &active_accounts()).is_ok());
    }

    #[test]
    fn unbalanced_voucher_rejected() {
        let date = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
        let v = make_voucher(date);
        let lines = vec![
            make_line(v.id, 1910, 10000, 0),
            make_line(v.id, 2440, 0, 5000),
        ];
        let result = validate_voucher_for_posting(&v, &lines, &open_period(), &active_accounts());
        assert!(matches!(result, Err(LedgerError::Unbalanced { .. })));
    }

    #[test]
    fn too_few_lines_rejected() {
        let date = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
        let v = make_voucher(date);
        let lines = vec![make_line(v.id, 1910, 10000, 0)];
        let result = validate_voucher_for_posting(&v, &lines, &open_period(), &active_accounts());
        assert!(matches!(result, Err(LedgerError::TooFewLines)));
    }

    #[test]
    fn closed_period_rejected() {
        let date = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
        let v = make_voucher(date);
        let lines = vec![
            make_line(v.id, 1910, 10000, 0),
            make_line(v.id, 2440, 0, 10000),
        ];
        let period = PeriodInfo {
            status: PeriodStatus::Closed,
            start_date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            end_date: NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
        };
        let result = validate_voucher_for_posting(&v, &lines, &period, &active_accounts());
        assert!(matches!(result, Err(LedgerError::PeriodNotOpen { .. })));
    }

    #[test]
    fn date_outside_period_rejected() {
        let date = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();
        let v = make_voucher(date);
        let lines = vec![
            make_line(v.id, 1910, 10000, 0),
            make_line(v.id, 2440, 0, 10000),
        ];
        let result = validate_voucher_for_posting(&v, &lines, &open_period(), &active_accounts());
        assert!(matches!(result, Err(LedgerError::DateOutsidePeriod { .. })));
    }

    #[test]
    fn already_posted_rejected() {
        let date = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
        let mut v = make_voucher(date);
        v.status = VoucherStatus::Posted;
        let lines = vec![
            make_line(v.id, 1910, 10000, 0),
            make_line(v.id, 2440, 0, 10000),
        ];
        let result = validate_voucher_for_posting(&v, &lines, &open_period(), &active_accounts());
        assert!(matches!(result, Err(LedgerError::AlreadyPosted { .. })));
    }
}
