//! När ett inventarium får dras av direkt i stället för att skrivas av:
//! inventarier av mindre värde (anskaffningsvärdet under ett halvt
//! prisbasbelopp, exklusive moms) och inventarier med en ekonomisk livslängd
//! på högst tre år (IL 18 kap. 4 §, BFNAR 2016:10 punkt 10.5–10.6). Det är
//! prisbasbeloppet för kalenderåret då beskattningsåret går ut som gäller
//! (IL 2 kap. 27 §). Gränsen är "understiger": exakt halva beloppet räcker inte.
//!
//! Prisbasbeloppen är data i `data/prisbasbelopp.txt`, med källa, och läggs
//! till där varje år. Ett år som saknas i tabellen ger inget svar hellre än
//! en gissning.

use bok_core::Money;

/// Tabellen med prisbasbelopp, `ÅR;BELOPP` i hela kronor.
const TABLE: &str = include_str!("../data/prisbasbelopp.txt");

/// Längsta ekonomiska livslängd för korttidsinventarier, i månader.
pub const SHORT_LIFE_MONTHS: u32 = 36;

/// Prisbasbeloppet för ett år enligt tabellen, eller `None` om året saknas.
pub fn price_base_amount(year: i32) -> Option<Money> {
    TABLE.lines().find_map(|line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            return None;
        }
        let (y, amount) = line.split_once(';')?;
        if y.trim().parse::<i32>().ok()? != year {
            return None;
        }
        let kronor: i64 = amount.trim().parse().ok()?;
        Some(Money::from_ore(kronor.checked_mul(100)?))
    })
}

/// Halva prisbasbeloppet för ett år.
pub fn half_price_base_amount(year: i32) -> Option<Money> {
    price_base_amount(year).map(|m| Money::from_ore(m.ore() / 2))
}

/// Om ett inventarium får dras av direkt, och varför.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectExpense {
    /// Anskaffningsvärdet understiger halva prisbasbeloppet (`limit`).
    LowValue {
        /// Halva prisbasbeloppet.
        limit: Money,
    },
    /// Den ekonomiska livslängden är högst tre år.
    ShortLife,
    /// Inventariet ska skrivas av.
    No {
        /// Halva prisbasbeloppet.
        limit: Money,
    },
}

/// Prövar om ett inventarium med anskaffningsvärdet `cost` (utan avdragsgill
/// moms) och livslängden `life_months` får dras av direkt, med
/// prisbasbeloppet för `year`, kalenderåret då beskattningsåret går ut. `None` om prisbasbeloppet för året saknas.
///
/// Inventarier som hör ihop (till exempel möbler till ett kontor) ska
/// bedömas tillsammans; det avgör den som anger beloppet.
pub fn direct_expense(cost: Money, life_months: u32, year: i32) -> Option<DirectExpense> {
    let limit = half_price_base_amount(year)?;
    Some(if cost < limit {
        DirectExpense::LowValue { limit }
    } else if life_months <= SHORT_LIFE_MONTHS {
        DirectExpense::ShortLife
    } else {
        DirectExpense::No { limit }
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn every_line_in_the_table_reads() {
        for line in TABLE.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with("//") {
                continue;
            }
            let (y, _) = line.split_once(';').unwrap();
            let year: i32 = y.parse().unwrap();
            assert!(price_base_amount(year).is_some(), "{line}");
        }
    }

    #[test]
    fn missing_year_gives_no_answer() {
        assert_eq!(price_base_amount(1900), None);
        assert_eq!(direct_expense(Money::from_ore(100), 60, 1900), None);
    }

    #[test]
    fn the_amount_for_2026() {
        // Förordning (2025:872): 59 200 kr, halva 29 600 kr.
        assert_eq!(
            half_price_base_amount(2026),
            Some(Money::from_ore(2_960_000))
        );
    }

    #[test]
    fn threshold_and_short_life() {
        let limit = half_price_base_amount(2026).unwrap();
        let below = Money::from_ore(limit.ore() - 1);
        assert_eq!(
            direct_expense(below, 60, 2026),
            Some(DirectExpense::LowValue { limit })
        );
        assert_eq!(
            direct_expense(limit, 60, 2026),
            Some(DirectExpense::No { limit })
        );
        assert_eq!(
            direct_expense(limit, 36, 2026),
            Some(DirectExpense::ShortLife)
        );
    }
}
