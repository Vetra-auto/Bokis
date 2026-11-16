//! Periodisering: en kostnad eller intäkt som avser tid på båda sidor om
//! årsskiftet förs till rätt år med ett interimskonto.
//!
//! - **Förutbetald kostnad**: bokförd i år, avser delvis nästa år. Delen
//!   efter årsskiftet flyttas till en fordran: debet 1790, kredit kostnaden.
//! - **Upplupen kostnad**: avser i år men är inte bokförd (fakturan kommer
//!   senare). Årets del bokförs som skuld: debet kostnaden, kredit 2990.
//! - **Förutbetald intäkt**: bokförd i år, avser delvis nästa år: debet
//!   intäkten, kredit 2990.
//! - **Upplupen intäkt**: avser i år men är inte fakturerad: debet 1790,
//!   kredit intäkten.
//!
//! Delen räknas per dag. Enligt K2 behöver en inkomst eller utgift som
//! understiger en beloppsgräns inte periodiseras, och det är beloppet på
//! varje enskild faktura, avtal eller liknande handling som avgör
//! (BFNAR 2016:10 punkt 2.4). Gränsen är 7 000 kr för räkenskapsår som
//! börjar efter den 31 december 2025 (BFNAR 2025:2) och 5 000 kr före.
//! Periodiseringen återförs normalt nästa år; [`reversal`] ger de raderna.

use chrono::NaiveDate;

use bok_core::voucher::Line;
use bok_core::{AccountNumber, Money};

use crate::{ClosingError, Result};

/// Sista dagen ett räkenskapsår kan börja och ändå ha den äldre gränsen.
const OLD_LIMIT_UNTIL: (i32, u32, u32) = (2025, 12, 31);
/// Gränsen i kronor före och efter BFNAR 2025:2.
const OLD_LIMIT_KRONOR: i64 = 5_000;
const NEW_LIMIT_KRONOR: i64 = 7_000;

/// Beloppsgränsen i K2 punkt 2.4 för ett räkenskapsår som börjar `start`.
pub fn k2_limit(start: NaiveDate) -> Money {
    let (y, m, d) = OLD_LIMIT_UNTIL;
    let old = NaiveDate::from_ymd_opt(y, m, d).is_some_and(|until| start <= until);
    Money::from_ore(
        if old {
            OLD_LIMIT_KRONOR
        } else {
            NEW_LIMIT_KRONOR
        } * 100,
    )
}

/// Slaget av periodisering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccrualKind {
    /// Förutbetald kostnad.
    PrepaidExpense,
    /// Upplupen kostnad.
    AccruedExpense,
    /// Förutbetald intäkt.
    PrepaidIncome,
    /// Upplupen intäkt.
    AccruedIncome,
}

impl AccrualKind {
    /// Namnet på kommandoraden och i texter.
    pub fn as_str(self) -> &'static str {
        match self {
            AccrualKind::PrepaidExpense => "forutbetald-kostnad",
            AccrualKind::AccruedExpense => "upplupen-kostnad",
            AccrualKind::PrepaidIncome => "forutbetald-intakt",
            AccrualKind::AccruedIncome => "upplupen-intakt",
        }
    }

    /// Läser namnet.
    pub fn parse(s: &str) -> Option<AccrualKind> {
        [
            AccrualKind::PrepaidExpense,
            AccrualKind::AccruedExpense,
            AccrualKind::PrepaidIncome,
            AccrualKind::AccruedIncome,
        ]
        .into_iter()
        .find(|k| k.as_str() == s)
    }

    /// Interimskontot i BAS som brukar användas: 1790 för fordringar och
    /// 2990 för skulder.
    pub fn default_balance_account(self) -> AccountNumber {
        let n = match self {
            AccrualKind::PrepaidExpense | AccrualKind::AccruedIncome => 1790,
            AccrualKind::AccruedExpense | AccrualKind::PrepaidIncome => 2990,
        };
        AccountNumber::new(n).unwrap_or_else(|_| unreachable!("{n} är ett giltigt konto"))
    }

    /// Sant om beloppet redan är bokfört i år och delen efter årsskiftet
    /// ska flyttas fram; falskt om årets del ska bokföras.
    fn is_prepaid(self) -> bool {
        matches!(
            self,
            AccrualKind::PrepaidExpense | AccrualKind::PrepaidIncome
        )
    }

    fn describe(self) -> &'static str {
        match self {
            AccrualKind::PrepaidExpense => "Förutbetald kostnad",
            AccrualKind::AccruedExpense => "Upplupen kostnad",
            AccrualKind::PrepaidIncome => "Förutbetald intäkt",
            AccrualKind::AccruedIncome => "Upplupen intäkt",
        }
    }
}

/// En periodisering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accrual {
    /// Slaget.
    pub kind: AccrualKind,
    /// Kostnads- eller intäktskontot.
    pub account: AccountNumber,
    /// Interimskontot.
    pub balance_account: AccountNumber,
    /// Hela beloppet på fakturan eller avtalet, utan avdragsgill moms.
    pub amount: Money,
    /// Första dagen som beloppet avser.
    pub from: NaiveDate,
    /// Sista dagen som beloppet avser.
    pub to: NaiveDate,
    /// Vad det gäller, till exempel "Försäkring nov 2026–okt 2027".
    pub text: String,
}

/// Uträkningen för ett räkenskapsår.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccrualPlan {
    /// Beloppet som förs till interimskontot.
    pub amount: Money,
    /// Dagar som beloppet avser.
    pub days: i64,
    /// Dagar inom räkenskapsåret.
    pub days_in_year: i64,
    /// Sant om hela beloppet understiger K2-gränsen och inte behöver
    /// periodiseras.
    pub below_limit: bool,
    /// Gränsen för året.
    pub limit: Money,
}

fn days(from: NaiveDate, to: NaiveDate) -> i64 {
    if to < from {
        0
    } else {
        (to - from).num_days() + 1
    }
}

/// Räknar ut periodiseringen för räkenskapsåret `start`–`end`.
pub fn plan(a: &Accrual, start: NaiveDate, end: NaiveDate) -> Result<AccrualPlan> {
    if a.amount.ore() <= 0 {
        return Err(ClosingError::Invalid(
            "beloppet måste vara större än noll".into(),
        ));
    }
    if a.to < a.from {
        return Err(ClosingError::Invalid(format!(
            "perioden {}–{} går baklänges",
            a.from, a.to
        )));
    }
    if !a.account.is_result_account() || !a.balance_account.is_balance_account() {
        return Err(ClosingError::Invalid(format!(
            "periodiseringen går mellan ett resultatkonto och ett balanskonto, inte {} och {}",
            a.account, a.balance_account
        )));
    }
    let total = days(a.from, a.to);
    let in_year = days(a.from.max(start), a.to.min(end));
    let share_days = if a.kind.is_prepaid() {
        days(a.from.max(end.succ_opt().unwrap_or(end)), a.to)
    } else {
        in_year
    };
    if share_days == 0 {
        return Err(ClosingError::Invalid(if a.kind.is_prepaid() {
            format!("{}–{} slutar inom året; inget avser nästa år", a.from, a.to)
        } else {
            format!("{}–{} ligger helt utanför räkenskapsåret", a.from, a.to)
        }));
    }
    // Andelen av beloppet, avrundad till närmaste öre.
    let num = i128::from(a.amount.ore()) * i128::from(share_days);
    let den = i128::from(total);
    let amount = Money::from_ore(i64::try_from((2 * num + den) / (2 * den)).unwrap_or(0));
    let limit = k2_limit(start);
    Ok(AccrualPlan {
        amount,
        days: total,
        days_in_year: in_year,
        below_limit: a.amount < limit,
        limit,
    })
}

/// Raderna för periodiseringen med beloppet `amount`.
pub fn lines(a: &Accrual, amount: Money) -> Vec<Line> {
    let text = format!("{}: {}", a.kind.describe(), a.text.trim());
    let (debit, credit) = match a.kind {
        AccrualKind::PrepaidExpense => (a.balance_account, a.account),
        AccrualKind::AccruedExpense => (a.account, a.balance_account),
        AccrualKind::PrepaidIncome => (a.account, a.balance_account),
        AccrualKind::AccruedIncome => (a.balance_account, a.account),
    };
    vec![
        Line::debit(debit, amount).with_text(text.clone()),
        Line::credit(credit, amount).with_text(text),
    ]
}

/// Återföringen: samma rader med debet och kredit omkastade.
pub fn reversal(lines: &[Line]) -> Vec<Line> {
    lines
        .iter()
        .map(|l| {
            let mut r = l.clone();
            std::mem::swap(&mut r.debit, &mut r.credit);
            r.text = l.text.as_ref().map(|t| format!("Återföring: {t}"));
            r
        })
        .collect()
}

/// Lägger periodiseringen som förslag daterat räkenskapsårets sista dag,
/// och med `reverse` också återföringen daterad nästa års första dag (om det
/// året finns). Ett belopp under K2-gränsen föreslås bara med `anyway`.
pub fn propose(
    store: &mut bok_store::Store,
    actor: &bok_store::Actor,
    year: &bok_store::FiscalYear,
    accrual: &Accrual,
    reverse: bool,
    anyway: bool,
    provenance: bok_store::Provenance,
) -> Result<(crate::proposal::Proposed, AccrualPlan)> {
    use crate::proposal::{Proposed, propose};
    let p = plan(accrual, year.start, year.end)?;
    if p.below_limit && !anyway {
        return Err(ClosingError::Invalid(format!(
            "beloppet {} understiger {} och behöver inte periodiseras (BFNAR 2016:10 punkt 2.4); \
             ange --anda om du ändå vill",
            accrual.amount, p.limit
        )));
    }
    let l = lines(accrual, p.amount);
    let description = format!("{}: {}", accrual.kind.describe(), accrual.text.trim());
    let id = propose(
        store,
        actor,
        accrual.kind.as_str(),
        year.end,
        description.clone(),
        l.clone(),
        provenance.clone(),
    )?;
    let mut notes = vec![format!(
        "Periodiserat belopp {} av {} ({} dagar, varav {} inom räkenskapsåret).",
        p.amount, accrual.amount, p.days, p.days_in_year
    )];
    if reverse {
        let next = year.end.succ_opt().and_then(|start| {
            store
                .fiscal_years()
                .ok()?
                .into_iter()
                .find(|y| y.start == start)
        });
        match next {
            Some(next) => {
                let rid = propose(
                    store,
                    actor,
                    &format!("{}-aterforing", accrual.kind.as_str()),
                    next.start,
                    format!("Återföring: {description}"),
                    reversal(&l),
                    provenance,
                )?;
                notes.push(format!(
                    "Återföringen är förslag {rid}, daterat {}.",
                    next.start
                ));
            }
            None => notes.push(format!(
                "Räkenskapsåret efter {} finns inte än; lägg till det och föreslå återföringen då.",
                year.label()
            )),
        }
    }
    Ok((Proposed { id, notes }, p))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn acc(n: i64) -> AccountNumber {
        AccountNumber::new(n).unwrap()
    }

    fn insurance() -> Accrual {
        Accrual {
            kind: AccrualKind::PrepaidExpense,
            account: acc(6310),
            balance_account: AccrualKind::PrepaidExpense.default_balance_account(),
            amount: Money::from_ore(1_200_000),
            from: d(2026, 11, 1),
            to: d(2027, 10, 31),
            text: "Företagsförsäkring".into(),
        }
    }

    #[test]
    fn the_limit_depends_on_when_the_year_starts() {
        assert_eq!(k2_limit(d(2026, 4, 27)), Money::from_ore(700_000));
        assert_eq!(k2_limit(d(2026, 1, 1)), Money::from_ore(700_000));
        assert_eq!(k2_limit(d(2025, 7, 1)), Money::from_ore(500_000));
    }

    #[test]
    fn prepaid_expense_moves_the_part_after_year_end() {
        let p = plan(&insurance(), d(2026, 4, 27), d(2026, 12, 31)).unwrap();
        // 304 av 365 dagar efter årsskiftet: 12 000 · 304 / 365 = 9 994,52.
        assert_eq!(p.days, 365);
        assert_eq!(p.days_in_year, 61);
        assert_eq!(p.amount, Money::from_ore(999_452));
        assert!(!p.below_limit);
        let l = lines(&insurance(), p.amount);
        assert_eq!((l[0].account, l[0].debit), (acc(1790), p.amount));
        assert_eq!((l[1].account, l[1].credit), (acc(6310), p.amount));
        let r = reversal(&l);
        assert_eq!((r[0].account, r[0].credit), (acc(1790), p.amount));
        assert!(r[0].text.as_deref().unwrap().starts_with("Återföring"));
    }

    #[test]
    fn accrued_expense_books_the_part_in_the_year() {
        let a = Accrual {
            kind: AccrualKind::AccruedExpense,
            account: acc(6230),
            balance_account: AccrualKind::AccruedExpense.default_balance_account(),
            amount: Money::from_ore(900_000),
            from: d(2026, 12, 1),
            to: d(2027, 2, 28),
            text: "Konsultarvode".into(),
        };
        let p = plan(&a, d(2026, 1, 1), d(2026, 12, 31)).unwrap();
        // 31 av 90 dagar.
        assert_eq!(p.amount, Money::from_ore(310_000));
        let l = lines(&a, p.amount);
        assert_eq!((l[0].account, l[1].account), (acc(6230), acc(2990)));
    }

    #[test]
    fn small_amounts_are_marked_and_wrong_inputs_refused() {
        let mut a = insurance();
        a.amount = Money::from_ore(699_999);
        assert!(
            plan(&a, d(2026, 1, 1), d(2026, 12, 31))
                .unwrap()
                .below_limit
        );
        let mut a = insurance();
        a.to = d(2026, 12, 31);
        assert!(plan(&a, d(2026, 1, 1), d(2026, 12, 31)).is_err());
        let mut a = insurance();
        a.account = acc(1930);
        assert!(plan(&a, d(2026, 1, 1), d(2026, 12, 31)).is_err());
        let mut a = insurance();
        a.balance_account = acc(6310);
        assert!(plan(&a, d(2026, 1, 1), d(2026, 12, 31)).is_err());
        assert_eq!(
            AccrualKind::parse("upplupen-intakt"),
            Some(AccrualKind::AccruedIncome)
        );
        assert_eq!(AccrualKind::parse("annat"), None);
    }
}
