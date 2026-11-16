//! När ett inventarium lämnar företaget: försäljning, utrangering eller
//! felregistrering.
//!
//! Vid försäljning och utrangering skrivs inventariet av till och med
//! månaden före avgången. Verifikationen tar då bort anskaffningsvärdet och
//! de ackumulerade avskrivningarna, bokför årets avskrivning fram till
//! avgången och för skillnaden mellan pris och bokfört värde till vinst
//! eller förlust (i BAS 3973 och 7973). Moms på försäljningen räknas aldrig
//! fram här; den anges från fakturan.
//!
//! En felregistrering stryker inventariet ur registret, som om det aldrig
//! hade funnits, och ger ingen verifikation.

use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};

use bok_core::voucher::Line;
use bok_core::{AccountNumber, Money};

use crate::asset::{AssetError, NewAsset};
use crate::plan::Plan;

/// Vad som hände med inventariet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "typ", rename_all = "snake_case")]
pub enum DisposalKind {
    /// Sålt för ett pris utan moms.
    #[serde(rename = "forsaljning")]
    Sold {
        /// Priset utan moms.
        #[serde(rename = "pris")]
        price: Money,
    },
    /// Kasserat, stulet eller förstört utan ersättning.
    #[serde(rename = "utrangering")]
    Scrapped,
    /// Registrerat av misstag; strykes ur registret.
    #[serde(rename = "felregistrering")]
    Void,
}

impl DisposalKind {
    /// Namnet i databasen och i användartexter.
    pub fn as_str(&self) -> &'static str {
        match self {
            DisposalKind::Sold { .. } => "forsaljning",
            DisposalKind::Scrapped => "utrangering",
            DisposalKind::Void => "felregistrering",
        }
    }
}

/// En avgång ur registret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Disposal {
    /// Dagen inventariet såldes, kasserades eller ströks.
    pub date: NaiveDate,
    /// Vad som hände.
    #[serde(flatten)]
    pub kind: DisposalKind,
    /// En förklaring, till exempel köparen eller varför det kasserades.
    pub note: Option<String>,
}

impl Disposal {
    /// Sant för en felregistrering.
    pub fn is_void(&self) -> bool {
        self.kind == DisposalKind::Void
    }
}

/// Kontona för en avyttring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisposalAccounts {
    /// Dit betalningen kommer, till exempel 1930 eller 1510.
    pub payment: AccountNumber,
    /// Utgående moms på försäljningen, till exempel 2611.
    pub output_vat: AccountNumber,
    /// Vinst vid avyttring, till exempel 3973.
    pub gain: AccountNumber,
    /// Förlust vid avyttring, till exempel 7973.
    pub loss: AccountNumber,
}

/// Verifikationen för en avyttring, med beloppen som förklarar den.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisposalVoucher {
    /// Raderna, i balans.
    pub lines: Vec<Line>,
    /// Årets avskrivning fram till avgången.
    pub depreciation_this_year: Money,
    /// Ackumulerade avskrivningar vid avgången.
    pub accumulated: Money,
    /// Bokfört värde vid avgången.
    pub book_value: Money,
    /// Pris minus bokfört värde: positivt är vinst.
    pub result: Money,
}

fn add(a: Money, b: Money) -> Result<Money, AssetError> {
    a.checked_add(b).ok_or(AssetError::Overflow)
}

fn sub(a: Money, b: Money) -> Result<Money, AssetError> {
    a.checked_sub(b).ok_or(AssetError::Overflow)
}

/// Raderna för en försäljning eller utrangering i räkenskapsåret som börjar
/// `year_start`. Avskrivningar för tidigare år antas vara bokförda enligt
/// plan. `vat` är den utgående momsen enligt fakturan (noll vid utrangering).
/// En felregistrering ger `Ok(None)`.
pub fn disposal_voucher(
    asset: &NewAsset,
    disposal: &Disposal,
    year_start: NaiveDate,
    vat: Money,
    accounts: &DisposalAccounts,
) -> Result<Option<DisposalVoucher>, AssetError> {
    let price = match disposal.kind {
        DisposalKind::Void => return Ok(None),
        DisposalKind::Sold { price } => price,
        DisposalKind::Scrapped => Money::ZERO,
    };
    if disposal.date < asset.acquired {
        return Err(AssetError::DisposalBeforeAcquisition(disposal.date));
    }
    if price.is_negative() || vat.is_negative() {
        return Err(AssetError::Overflow);
    }
    let plan = Plan::of(asset);
    let accumulated = plan.accumulated_before(disposal.date);
    let prior = plan.accumulated_before(year_start.max(asset.depreciation_start));
    let prior = prior.min(accumulated);
    let this_year = sub(accumulated, prior)?;
    let book_value = sub(asset.cost, accumulated)?;
    let result = sub(price, book_value)?;

    let a = &asset.accounts;
    let what = &asset.description;
    let mut lines = Vec::new();
    if !this_year.is_zero() {
        let last = disposal
            .date
            .with_day(1)
            .and_then(|d| d.pred_opt())
            .map(|d| d.format("%Y-%m").to_string())
            .unwrap_or_default();
        lines.push(
            Line::debit(a.expense, this_year)
                .with_text(format!("Avskrivning t.o.m. {last}: {what}")),
        );
    }
    if !prior.is_zero() {
        lines.push(
            Line::debit(a.accumulated, prior)
                .with_text(format!("Tidigare års avskrivningar: {what}")),
        );
    }
    lines.push(Line::credit(a.asset, asset.cost).with_text(format!("Anskaffningsvärde: {what}")));
    let received = add(price, vat)?;
    if !received.is_zero() {
        lines.push(
            Line::debit(accounts.payment, received).with_text(format!("Försäljning: {what}")),
        );
    }
    if !vat.is_zero() {
        lines.push(Line::credit(accounts.output_vat, vat).with_text("Utgående moms"));
    }
    if result.is_negative() {
        lines.push(
            Line::debit(accounts.loss, -result).with_text(format!("Förlust vid avyttring: {what}")),
        );
    } else if !result.is_zero() {
        lines.push(
            Line::credit(accounts.gain, result).with_text(format!("Vinst vid avyttring: {what}")),
        );
    }
    Ok(Some(DisposalVoucher {
        lines,
        depreciation_this_year: this_year,
        accumulated,
        book_value,
        result,
    }))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::testdata::{acc, d, kr, laptop};

    fn accounts() -> DisposalAccounts {
        DisposalAccounts {
            payment: acc(1930),
            output_vat: acc(2611),
            gain: acc(3973),
            loss: acc(7973),
        }
    }

    fn balance(lines: &[Line]) -> i64 {
        lines
            .iter()
            .map(|l| l.debit.ore() - l.credit.ore())
            .sum::<i64>()
    }

    #[test]
    fn sale_with_gain() {
        // Bokförda avskrivningar 2026: 8 000. Säljs 15 mars 2027: jan–feb
        // 2027 skrivs av (2 000), bokfört värde 26 000, pris 30 000.
        let disposal = Disposal {
            date: d(2027, 3, 15),
            kind: DisposalKind::Sold { price: kr(30_000) },
            note: None,
        };
        let v = disposal_voucher(&laptop(), &disposal, d(2027, 1, 1), kr(7_500), &accounts())
            .unwrap()
            .unwrap();
        assert_eq!(v.depreciation_this_year, kr(2_000));
        assert_eq!(v.accumulated, kr(10_000));
        assert_eq!(v.book_value, kr(26_000));
        assert_eq!(v.result, kr(4_000));
        assert_eq!(balance(&v.lines), 0);
        let gain = v.lines.iter().find(|l| l.account == acc(3973)).unwrap();
        assert_eq!(gain.credit, kr(4_000));
        let bank = v.lines.iter().find(|l| l.account == acc(1930)).unwrap();
        assert_eq!(bank.debit, kr(37_500));
    }

    #[test]
    fn scrapping_is_a_loss_of_the_book_value() {
        let disposal = Disposal {
            date: d(2026, 9, 3),
            kind: DisposalKind::Scrapped,
            note: Some("Tappad".into()),
        };
        let v = disposal_voucher(
            &laptop(),
            &disposal,
            d(2026, 1, 1),
            Money::ZERO,
            &accounts(),
        )
        .unwrap()
        .unwrap();
        // Maj–augusti: 4 000 avskrivet, 32 000 förlust.
        assert_eq!(v.depreciation_this_year, kr(4_000));
        assert_eq!(v.result, kr(-32_000));
        assert_eq!(balance(&v.lines), 0);
        assert!(v.lines.iter().all(|l| l.account != acc(1930)));
        assert!(v.lines.iter().all(|l| l.account != acc(1229)));
    }

    #[test]
    fn a_void_registration_books_nothing() {
        let disposal = Disposal {
            date: d(2026, 6, 1),
            kind: DisposalKind::Void,
            note: Some("dubbelregistrerad".into()),
        };
        assert!(disposal.is_void());
        assert_eq!(
            disposal_voucher(
                &laptop(),
                &disposal,
                d(2026, 1, 1),
                Money::ZERO,
                &accounts()
            ),
            Ok(None)
        );
    }

    #[test]
    fn disposal_before_acquisition_is_refused() {
        let disposal = Disposal {
            date: d(2026, 1, 1),
            kind: DisposalKind::Scrapped,
            note: None,
        };
        assert!(
            disposal_voucher(
                &laptop(),
                &disposal,
                d(2026, 1, 1),
                Money::ZERO,
                &accounts()
            )
            .is_err()
        );
    }

    #[test]
    fn kinds_have_stable_names() {
        assert_eq!(DisposalKind::Scrapped.as_str(), "utrangering");
        assert_eq!(DisposalKind::Void.as_str(), "felregistrering");
        assert_eq!(DisposalKind::Sold { price: kr(1) }.as_str(), "forsaljning");
    }
}
