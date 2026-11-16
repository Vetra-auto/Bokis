//! Avskrivningsplanen: linjär avskrivning med lika stor del varje månad.
//!
//! Planen börjar den första månaden som skrivs av (normalt
//! anskaffningsmånaden) och pågår nyttjandeperiodens antal månader. Hela
//! månader räknas; en månad som har börjat skrivs av fullt ut. Det
//! ackumulerade beloppet efter `k` månader är `avskrivningsbart · k / n`,
//! avrundat till närmaste öre, så att summan av alla månader alltid blir
//! exakt det avskrivningsbara beloppet utan öresfel på slutet.

use chrono::{Datelike, NaiveDate};

use bok_core::Money;

use crate::asset::NewAsset;

/// Månadens löpnummer: år · 12 + månad (0–11).
fn month_index(date: NaiveDate) -> i64 {
    i64::from(date.year()) * 12 + i64::from(date.month0())
}

/// En avskrivningsplan för ett inventarium.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    first_month: i64,
    months: u32,
    depreciable: i64,
}

impl Plan {
    /// Planen för ett inventarium.
    pub fn of(asset: &NewAsset) -> Plan {
        Plan {
            first_month: month_index(asset.depreciation_start),
            months: asset.life_months,
            depreciable: asset.depreciable().ore(),
        }
    }

    /// Nyttjandeperioden i månader.
    pub fn months(&self) -> u32 {
        self.months
    }

    /// Antalet avskrivna månader till och med månaden som `at` ligger i.
    pub fn months_through(&self, at: NaiveDate) -> u32 {
        let elapsed = month_index(at) - self.first_month + 1;
        elapsed.clamp(0, i64::from(self.months)) as u32
    }

    /// Antalet avskrivna månader före månaden som `date` ligger i.
    pub fn months_before(&self, date: NaiveDate) -> u32 {
        let elapsed = month_index(date) - self.first_month;
        elapsed.clamp(0, i64::from(self.months)) as u32
    }

    /// Ackumulerad avskrivning efter `months` månader.
    pub fn accumulated_after(&self, months: u32) -> Money {
        let k = i128::from(months.min(self.months));
        let n = i128::from(self.months);
        let value = (i128::from(self.depreciable) * k + n / 2) / n;
        // Kan inte bli större än det avskrivningsbara beloppet, som är en i64.
        Money::from_ore(i64::try_from(value).unwrap_or(self.depreciable))
    }

    /// Ackumulerad avskrivning vid utgången av månaden som `at` ligger i.
    pub fn accumulated_through(&self, at: NaiveDate) -> Money {
        self.accumulated_after(self.months_through(at))
    }

    /// Ackumulerad avskrivning före månaden som `date` ligger i.
    pub fn accumulated_before(&self, date: NaiveDate) -> Money {
        self.accumulated_after(self.months_before(date))
    }

    /// Avskrivningen för månaderna från och med `from` till och med `to`,
    /// till exempel ett räkenskapsår.
    pub fn between(&self, from: NaiveDate, to: NaiveDate) -> Money {
        if to < from {
            return Money::ZERO;
        }
        let end = self.accumulated_through(to).ore();
        let start = self.accumulated_before(from).ore();
        Money::from_ore(end - start)
    }

    /// Sista månaden som skrivs av, som dess första dag.
    pub fn last_month(&self) -> Option<NaiveDate> {
        let last = self.first_month + i64::from(self.months) - 1;
        let year = i32::try_from(last.div_euclid(12)).ok()?;
        let month = u32::try_from(last.rem_euclid(12)).ok()? + 1;
        NaiveDate::from_ymd_opt(year, month, 1)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::testdata::{d, kr, laptop};

    #[test]
    fn straight_line_by_month() {
        let plan = Plan::of(&laptop());
        // Maj–december 2026: 8 av 36 månader av 36 000 kr.
        assert_eq!(plan.months_through(d(2026, 12, 31)), 8);
        assert_eq!(plan.between(d(2026, 1, 1), d(2026, 12, 31)), kr(8_000));
        assert_eq!(plan.between(d(2027, 1, 1), d(2027, 12, 31)), kr(12_000));
        assert_eq!(plan.between(d(2028, 1, 1), d(2028, 12, 31)), kr(12_000));
        assert_eq!(plan.between(d(2029, 1, 1), d(2029, 12, 31)), kr(4_000));
        assert_eq!(plan.between(d(2030, 1, 1), d(2030, 12, 31)), Money::ZERO);
        assert_eq!(plan.last_month(), Some(d(2029, 4, 1)));
    }

    #[test]
    fn nothing_before_the_start() {
        let plan = Plan::of(&laptop());
        assert_eq!(plan.accumulated_through(d(2026, 4, 30)), Money::ZERO);
        assert_eq!(plan.accumulated_before(d(2026, 5, 20)), Money::ZERO);
        assert_eq!(plan.accumulated_through(d(2026, 5, 2)), kr(1_000));
    }

    #[test]
    fn rounding_never_leaves_a_remainder() {
        let mut a = laptop();
        a.cost = Money::from_ore(1_000_001);
        a.life_months = 7;
        let plan = Plan::of(&a);
        let mut sum = 0;
        for year in 2026..2029 {
            sum += plan.between(d(year, 1, 1), d(year, 12, 31)).ore();
        }
        assert_eq!(sum, 1_000_001);
        // Varje månad är lika stor utom avrundningen på ett öre.
        for k in 1..=7 {
            let m = plan.accumulated_after(k).ore() - plan.accumulated_after(k - 1).ore();
            assert!((142_857..=142_858).contains(&m), "månad {k}: {m}");
        }
    }

    #[test]
    fn residual_value_is_not_depreciated() {
        let mut a = laptop();
        a.residual = kr(6_000);
        let plan = Plan::of(&a);
        assert_eq!(plan.accumulated_through(d(2040, 1, 1)), kr(30_000));
    }

    #[test]
    fn broken_fiscal_year() {
        let plan = Plan::of(&laptop());
        // Räkenskapsår juli–juni: maj och juni 2026 hör till året innan.
        assert_eq!(plan.between(d(2025, 7, 1), d(2026, 6, 30)), kr(2_000));
        assert_eq!(plan.between(d(2026, 7, 1), d(2027, 6, 30)), kr(12_000));
    }
}
