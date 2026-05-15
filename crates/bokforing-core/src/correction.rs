use chrono::{NaiveDate, Utc};
use uuid::Uuid;

use crate::error::CorrectionError;
use crate::types::VoucherStatus;
use crate::voucher::{Voucher, VoucherLine};

pub struct CorrectionLink {
    pub id: Uuid,
    pub original_voucher_id: Uuid,
    pub reversal_voucher_id: Uuid,
    pub replacement_voucher_id: Option<Uuid>,
    pub reason: String,
    pub created_by: Uuid,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub fn create_reversal_lines(original_lines: &[VoucherLine], reversal_voucher_id: Uuid) -> Vec<VoucherLine> {
    original_lines
        .iter()
        .enumerate()
        .map(|(i, line)| VoucherLine {
            id: Uuid::new_v4(),
            voucher_id: reversal_voucher_id,
            line_number: (i + 1) as i32,
            account_id: line.account_id,
            account_number: line.account_number,
            debit_amount: line.credit_amount,
            credit_amount: line.debit_amount,
            description: line.description.clone(),
            counterparty_id: line.counterparty_id,
            cost_center: line.cost_center.clone(),
            project: line.project.clone(),
        })
        .collect()
}

pub fn create_reversal_voucher(
    original: &Voucher,
    reason: &str,
    user_id: Uuid,
    new_voucher_number: i64,
    correction_date: NaiveDate,
    period_id: Uuid,
) -> Result<Voucher, CorrectionError> {
    if original.status != VoucherStatus::Posted {
        return Err(CorrectionError::NotPosted);
    }
    if reason.is_empty() {
        return Err(CorrectionError::MissingReason);
    }

    Ok(Voucher {
        id: Uuid::new_v4(),
        company_id: original.company_id,
        fiscal_year_id: original.fiscal_year_id,
        period_id,
        voucher_series: original.voucher_series.clone(),
        voucher_number: new_voucher_number,
        voucher_date: correction_date,
        description: format!(
            "Rättelse av ver {}{}: {}",
            original.voucher_series, original.voucher_number, reason
        ),
        status: VoucherStatus::Draft,
        counterparty_id: original.counterparty_id,
        created_by: user_id,
        created_at: Utc::now(),
        posted_at: None,
        posted_by: None,
    })
}

pub fn build_correction_link(
    original_id: Uuid,
    reversal_id: Uuid,
    replacement_id: Option<Uuid>,
    reason: &str,
    user_id: Uuid,
) -> CorrectionLink {
    CorrectionLink {
        id: Uuid::new_v4(),
        original_voucher_id: original_id,
        reversal_voucher_id: reversal_id,
        replacement_voucher_id: replacement_id,
        reason: reason.to_string(),
        created_by: user_id,
        created_at: Utc::now(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Money;

    fn posted_voucher() -> Voucher {
        Voucher {
            id: Uuid::new_v4(),
            company_id: Uuid::new_v4(),
            fiscal_year_id: Uuid::new_v4(),
            period_id: Uuid::new_v4(),
            voucher_series: "A".to_string(),
            voucher_number: 5,
            voucher_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            description: "Köp kontorsmaterial".to_string(),
            status: VoucherStatus::Posted,
            counterparty_id: None,
            created_by: Uuid::new_v4(),
            created_at: Utc::now(),
            posted_at: Some(Utc::now()),
            posted_by: Some(Uuid::new_v4()),
        }
    }

    fn sample_lines(voucher_id: Uuid) -> Vec<VoucherLine> {
        vec![
            VoucherLine {
                id: Uuid::new_v4(),
                voucher_id,
                line_number: 1,
                account_id: Uuid::new_v4(),
                account_number: 5410,
                debit_amount: Money::from_ore(10000),
                credit_amount: Money::from_ore(0),
                description: Some("Kontorsmaterial".to_string()),
                counterparty_id: None,
                cost_center: None,
                project: None,
            },
            VoucherLine {
                id: Uuid::new_v4(),
                voucher_id,
                line_number: 2,
                account_id: Uuid::new_v4(),
                account_number: 2440,
                debit_amount: Money::from_ore(0),
                credit_amount: Money::from_ore(10000),
                description: Some("Leverantörsskuld".to_string()),
                counterparty_id: None,
                cost_center: None,
                project: None,
            },
        ]
    }

    #[test]
    fn reversal_swaps_debit_credit() {
        let v = posted_voucher();
        let lines = sample_lines(v.id);
        let reversal_id = Uuid::new_v4();
        let reversed = create_reversal_lines(&lines, reversal_id);

        assert_eq!(reversed.len(), 2);
        assert_eq!(reversed[0].debit_amount.ore(), 0);
        assert_eq!(reversed[0].credit_amount.ore(), 10000);
        assert_eq!(reversed[1].debit_amount.ore(), 10000);
        assert_eq!(reversed[1].credit_amount.ore(), 0);
    }

    #[test]
    fn reversal_voucher_references_original() {
        let v = posted_voucher();
        let user_id = Uuid::new_v4();
        let date = NaiveDate::from_ymd_opt(2026, 1, 20).unwrap();
        let result = create_reversal_voucher(&v, "Fel belopp", user_id, 6, date, v.period_id);
        let rv = result.unwrap();
        assert!(rv.description.contains("Rättelse av ver A5"));
        assert!(rv.description.contains("Fel belopp"));
        assert_eq!(rv.status, VoucherStatus::Draft);
    }

    #[test]
    fn cannot_reverse_draft() {
        let mut v = posted_voucher();
        v.status = VoucherStatus::Draft;
        let result = create_reversal_voucher(
            &v,
            "Test",
            Uuid::new_v4(),
            6,
            NaiveDate::from_ymd_opt(2026, 1, 20).unwrap(),
            v.period_id,
        );
        assert!(matches!(result, Err(CorrectionError::NotPosted)));
    }

    #[test]
    fn empty_reason_rejected() {
        let v = posted_voucher();
        let result = create_reversal_voucher(
            &v,
            "",
            Uuid::new_v4(),
            6,
            NaiveDate::from_ymd_opt(2026, 1, 20).unwrap(),
            v.period_id,
        );
        assert!(matches!(result, Err(CorrectionError::MissingReason)));
    }
}
