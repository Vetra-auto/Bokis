//! Räkenskapsschemat INK2R ur saldobalansen.
//!
//! Varje konto med saldo förs till sitt fält: efter kontoplanens egen
//! SRU-kod om den är ett INK2R-fält, annars efter kopplingen. Beloppen får
//! fältets tecken: positiva när de följer blankettens tryckta tecken eller,
//! för balansräkningen, när tillgångar har debetsaldo och eget kapital och
//! skulder kreditsaldo (Skatteverket, SKV 294).
//!
//! Årets resultat räknas ur resultatkontona (klass 3–8 utom 899x) och blir
//! 3.26 eller 3.27. Är resultatet inte bokfört mot eget kapital ännu läggs
//! det till fritt eget kapital (2.28), så att balansräkningen visar läget
//! efter resultatdispositionen som den gör i årsredovisningen.

use std::collections::BTreeMap;

use crate::DeclError;
use crate::catalog::{Catalog, FormKind, Side};
use crate::declaration::{AccountBalance, FieldValue, Part};
use crate::mapping::{AccountMap, Target};

/// Fältet för årets resultat, vinst (3.26).
pub const PROFIT: &str = "7450";
/// Fältet för årets resultat, förlust (3.27).
pub const LOSS: &str = "7550";
/// Fritt eget kapital (2.28), där ett obokfört resultat läggs.
pub const FREE_EQUITY: &str = "7302";

/// Konton för årets resultat (899x), som räknas fram och inte förs till
/// något fält.
fn is_year_result(account: u16) -> bool {
    (8990..=8999).contains(&account)
}

/// INK2R med beloppen i öre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ink2R {
    /// Fälten per kod, bara de med belopp.
    pub values: BTreeMap<String, FieldValue>,
    /// Årets resultat i öre, positivt för vinst.
    pub result_ore: i64,
    /// Resultat som inte var bokfört mot eget kapital och lades till 2.28.
    pub unbooked_result_ore: i64,
    /// Summan av tillgångarna (2.1–2.26) i öre.
    pub assets_ore: i64,
    /// Summan av eget kapital och skulder (2.27–2.50) i öre.
    pub equity_and_liabilities_ore: i64,
    /// Sådant som granskaren bör se.
    pub warnings: Vec<String>,
}

fn add(values: &mut BTreeMap<String, FieldValue>, code: &str, part: Part) {
    let v = values
        .entry(code.to_string())
        .or_insert_with(|| FieldValue {
            code: code.to_string(),
            ore: 0,
            parts: Vec::new(),
        });
    v.ore += part.ore;
    v.parts.push(part);
}

fn label(b: &AccountBalance) -> String {
    format!("{} {}", b.account, b.name).trim().to_string()
}

/// Räknar fram INK2R. Konton med saldo som inte hör till något fält är ett
/// fel, eftersom balansräkningen annars inte stämmer.
pub fn compute(
    catalog: &Catalog,
    map: &AccountMap,
    balances: &[AccountBalance],
) -> Result<Ink2R, DeclError> {
    let mut values = BTreeMap::new();
    let mut warnings = Vec::new();
    let mut unmapped = Vec::new();
    // Par (plus/minus) summeras per rad innan nettot avgör koden.
    let mut groups: BTreeMap<usize, Vec<&AccountBalance>> = BTreeMap::new();
    let mut result_ore = 0_i64;
    let mut all_result_ore = 0_i64;

    for b in balances.iter().filter(|b| !b.balance.is_zero()) {
        let n = b.account.value();
        if b.account.is_result_account() {
            all_result_ore -= b.balance.ore();
            if !is_year_result(n) {
                result_ore -= b.balance.ore();
            }
        }
        if is_year_result(n) {
            continue;
        }
        let own = b.sru.as_deref().map(str::trim).filter(|s| !s.is_empty());
        if let Some(code) = own {
            match catalog.field(FormKind::Ink2R, code) {
                Some(f) if code != PROFIT && code != LOSS && f.side != Side::Neither => {
                    let ore = if f.side == Side::Debit {
                        b.balance.ore()
                    } else {
                        -b.balance.ore()
                    };
                    add(&mut values, code, Part { label: label(b), ore });
                    continue;
                }
                _ => warnings.push(format!(
                    "konto {} har SRU-koden {code} i kontoplanen, som inte är ett fält i INK2R; kopplingen används i stället",
                    b.account
                )),
            }
        }
        match map.rule_for(b.account) {
            Some(rule) => groups.entry(rule).or_default().push(b),
            None => unmapped.push(label(b)),
        }
    }
    if !unmapped.is_empty() {
        return Err(DeclError::Unmapped(unmapped));
    }

    for (rule, accounts) in groups {
        let net: i64 = accounts.iter().map(|b| b.balance.ore()).sum();
        let (code, debit_positive) = match &map.rules[rule].target {
            Target::Single(code) => {
                let f = catalog
                    .field(FormKind::Ink2R, code)
                    .ok_or_else(|| DeclError::UnknownField(code.clone()))?;
                (code.clone(), f.side == Side::Debit)
            }
            // Kreditnetto (intäkt) till pluskoden, debetnetto till minuskoden.
            Target::Pair { plus, minus } => {
                if net <= 0 {
                    (plus.clone(), false)
                } else {
                    (minus.clone(), true)
                }
            }
        };
        for b in accounts {
            let ore = if debit_positive {
                b.balance.ore()
            } else {
                -b.balance.ore()
            };
            add(
                &mut values,
                &code,
                Part {
                    label: label(b),
                    ore,
                },
            );
        }
    }

    let unbooked_result_ore = all_result_ore;
    if unbooked_result_ore != 0 {
        add(
            &mut values,
            FREE_EQUITY,
            Part {
                label: "Årets resultat, ännu inte bokfört mot eget kapital".into(),
                ore: unbooked_result_ore,
            },
        );
    }
    if result_ore >= 0 {
        add(
            &mut values,
            PROFIT,
            Part {
                label: "Årets resultat (klass 3–8)".into(),
                ore: result_ore,
            },
        );
    } else {
        add(
            &mut values,
            LOSS,
            Part {
                label: "Årets resultat (klass 3–8)".into(),
                ore: -result_ore,
            },
        );
    }
    values.retain(|_, v| v.ore != 0);

    let side_sum = |side: Side| -> i64 {
        values
            .values()
            .filter(|v| {
                catalog
                    .field(FormKind::Ink2R, &v.code)
                    .is_some_and(|f| f.number.starts_with("2.") && f.side == side)
            })
            .map(|v| v.ore)
            .sum()
    };
    let assets_ore = side_sum(Side::Debit);
    let equity_and_liabilities_ore = side_sum(Side::Credit);
    Ok(Ink2R {
        values,
        result_ore,
        unbooked_result_ore,
        assets_ore,
        equity_and_liabilities_ore,
        warnings,
    })
}

/// Resultaträkningens fält (3.1–3.25) med blankettens tecken, i öre. Ska
/// vara lika med årets resultat.
pub fn income_statement_sum(catalog: &Catalog, ink2r: &Ink2R) -> i64 {
    use crate::catalog::Sign;
    ink2r
        .values
        .values()
        .filter_map(|v| {
            let f = catalog.field(FormKind::Ink2R, &v.code)?;
            if !f.number.starts_with("3.") || v.code == PROFIT || v.code == LOSS {
                return None;
            }
            Some(match f.sign {
                Sign::Minus => -v.ore,
                _ => v.ore,
            })
        })
        .sum()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::testdata::{balances, catalog, kr};

    fn ink2r() -> Ink2R {
        compute(&catalog(), &AccountMap::ink2r_bas().unwrap(), &balances()).unwrap()
    }

    #[test]
    fn fields_get_the_forms_signs() {
        let r = ink2r();
        // Inventarier 36 000 minus avskrivningar 8 000.
        assert_eq!(r.values["7215"].ore, kr(28_000));
        assert_eq!(r.values["7215"].parts.len(), 2);
        // Bank och skattekonto.
        assert_eq!(r.values["7281"].ore, kr(150_000));
        assert_eq!(r.values["7261"].ore, kr(1_000));
        // Aktiekapital och leverantörsskulder är kreditsaldon.
        assert_eq!(r.values["7301"].ore, kr(25_000));
        assert_eq!(r.values["7365"].ore, kr(10_000));
        // Intäkter och kostnader positiva.
        assert_eq!(r.values["7410"].ore, kr(200_000));
        assert_eq!(r.values["7513"].ore, kr(40_000));
        assert_eq!(r.values["7515"].ore, kr(8_000));
        // Ränta på skattekontot är en ränteintäkt (3.16).
        assert_eq!(r.values["7417"].ore, kr(100));
    }

    #[test]
    fn the_year_result_and_the_balance_check() {
        let r = ink2r();
        // 200 000 + 100 − 40 000 − 8 000 − 20 000 − 500 = 131 600.
        assert_eq!(r.result_ore, kr(131_600));
        assert_eq!(r.values[PROFIT].ore, kr(131_600));
        assert!(!r.values.contains_key(LOSS));
        assert_eq!(r.unbooked_result_ore, kr(131_600));
        assert_eq!(r.assets_ore, r.equity_and_liabilities_ore);
        assert_eq!(income_statement_sum(&catalog(), &r), r.result_ore);
    }

    #[test]
    fn a_booked_result_is_not_added_twice() {
        let mut b = balances();
        // Resultatet bokfört: debet 8999, kredit 2099.
        b.push(crate::testdata::balance(
            8999,
            "Årets resultat",
            kr(131_600),
        ));
        b.push(crate::testdata::balance(
            2099,
            "Årets resultat",
            -kr(131_600),
        ));
        let r = compute(&catalog(), &AccountMap::ink2r_bas().unwrap(), &b).unwrap();
        assert_eq!(r.unbooked_result_ore, 0);
        assert_eq!(r.result_ore, kr(131_600));
        assert_eq!(r.values[FREE_EQUITY].ore, kr(131_600));
        assert_eq!(r.assets_ore, r.equity_and_liabilities_ore);
    }

    #[test]
    fn a_loss_goes_to_3_27() {
        let b = vec![
            crate::testdata::balance(1930, "Bank", kr(5_000)),
            crate::testdata::balance(2081, "Aktiekapital", -kr(25_000)),
            crate::testdata::balance(6570, "Bankkostnader", kr(20_000)),
        ];
        let r = compute(&catalog(), &AccountMap::ink2r_bas().unwrap(), &b).unwrap();
        assert_eq!(r.values[LOSS].ore, kr(20_000));
        assert_eq!(r.values[FREE_EQUITY].ore, -kr(20_000));
        assert_eq!(r.assets_ore, r.equity_and_liabilities_ore);
    }

    #[test]
    fn own_sru_codes_win_and_unknown_ones_are_warned_about() {
        let mut b = balances();
        b[0].sru = Some("7216".into());
        b[1].sru = Some("7714".into());
        let r = compute(&catalog(), &AccountMap::ink2r_bas().unwrap(), &b).unwrap();
        assert_eq!(r.values["7216"].ore, b[0].balance.ore());
        assert_eq!(r.warnings.len(), 1, "{:?}", r.warnings);
    }

    #[test]
    fn unmapped_accounts_are_an_error() {
        let mut b = balances();
        b.push(crate::testdata::balance(2010, "Eget kapital", kr(1)));
        let e = compute(&catalog(), &AccountMap::ink2r_bas().unwrap(), &b).unwrap_err();
        assert!(matches!(e, DeclError::Unmapped(ref v) if v[0].starts_with("2010")));
    }

    #[test]
    fn pairs_follow_the_net() {
        let map = AccountMap::parse("t", "7414/7518;8000-8099\n7281;1930\n7302;2091").unwrap();
        let b = vec![
            crate::testdata::balance(8010, "Utdelning", -kr(500)),
            crate::testdata::balance(8020, "Förlust", kr(200)),
            crate::testdata::balance(1930, "Bank", kr(300)),
        ];
        let r = compute(&catalog(), &map, &b).unwrap();
        assert_eq!(r.values["7414"].ore, kr(300));
        assert!(!r.values.contains_key("7518"));
    }
}
