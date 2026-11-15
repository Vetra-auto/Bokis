//! Rättelser (BFL 5 kap. 5 §, BFNAR 2013:2 punkt 2.17–2.18).
//!
//! Ett fel rättas aldrig i den gamla verifikationen. Rättelsen är en ny
//! verifikation i serie R som pekar på originalet; originalet visar att det
//! är rättat. Omvändningen av raderna är flyttad från
//! `bokforing-core::correction`.

use thiserror::Error;

use crate::period::PeriodStatus;
use crate::series::VoucherRef;
use crate::voucher::Line;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CorrectionError {
    #[error("en rättelse måste ha en orsak")]
    MissingReason,
}

/// Rader som tar ut originalet: debet och kredit byter plats.
pub fn reversal_lines(original: &[Line]) -> Vec<Line> {
    original
        .iter()
        .map(|line| Line {
            account: line.account,
            debit: line.credit,
            credit: line.debit,
            vat_code: line.vat_code.clone(),
            text: line.text.clone(),
        })
        .collect()
}

pub fn validate_reason(reason: &str) -> Result<(), CorrectionError> {
    if reason.trim().is_empty() {
        Err(CorrectionError::MissingReason)
    } else {
        Ok(())
    }
}

pub fn correction_description(original: VoucherRef, reason: &str) -> String {
    format!("Rättelse av {original}: {}", reason.trim())
}

/// Var en godkänd rättelse bokförs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// I originalets period, med originalets datum.
    OriginalPeriod,
    /// I den period som är öppen nu, med dagens datum.
    CurrentPeriod,
}

/// I originalets period om den är öppen, annars i den period som är öppen nu.
pub fn placement(original_period: PeriodStatus) -> Placement {
    if original_period.allows_posting() {
        Placement::OriginalPeriod
    } else {
        Placement::CurrentPeriod
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AccountNumber, Money};

    #[test]
    fn reversal_swaps_sides_and_keeps_balance() {
        let original = vec![
            Line::debit(AccountNumber::new(5410).unwrap(), Money::from_ore(10000))
                .with_text("Material"),
            Line::credit(AccountNumber::new(2440).unwrap(), Money::from_ore(10000)),
        ];
        let reversed = reversal_lines(&original);
        assert_eq!(reversed[0].credit.ore(), 10000);
        assert_eq!(reversed[0].debit.ore(), 0);
        assert_eq!(reversed[0].text.as_deref(), Some("Material"));
        assert_eq!(reversed[1].debit.ore(), 10000);
        assert!(crate::voucher::validate_lines(&reversed).is_ok());
    }

    #[test]
    fn reason_is_required_and_described() {
        assert_eq!(validate_reason(" "), Err(CorrectionError::MissingReason));
        let r: VoucherRef = "A5".parse().unwrap();
        assert_eq!(
            correction_description(r, " Fel konto "),
            "Rättelse av A5: Fel konto"
        );
    }

    #[test]
    fn placement_follows_the_original_period() {
        assert_eq!(placement(PeriodStatus::Open), Placement::OriginalPeriod);
        assert_eq!(
            placement(PeriodStatus::UnderReview),
            Placement::CurrentPeriod
        );
        assert_eq!(placement(PeriodStatus::Closed), Placement::CurrentPeriod);
        assert_eq!(placement(PeriodStatus::Locked), Placement::CurrentPeriod);
    }
}
