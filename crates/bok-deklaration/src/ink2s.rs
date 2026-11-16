//! De skattemässiga justeringarna INK2S och huvudblanketten INK2.
//!
//! INK2S börjar med årets resultat ur INK2R (3.26 → 4.1, 3.27 → 4.2) och
//! skatten på årets resultat (3.25 → 4.3a). Kopplingen ger bokförda
//! kostnader som inte får dras av (plusfält, kontonas debetsaldo) och
//! bokförda intäkter som inte ska tas upp (minusfält, kreditsaldo). Annat
//! anges för hand. Överskottet eller underskottet är summan av fälten 4.1–
//! 4.14c med blankettens tecken (4.15 eller 4.16), och det förs till
//! huvudblanketten: 4.15 → 1.1, och 4.16 plus 4.14c → 1.2.
//!
//! Alla belopp här är hela kronor (lagrade som öre), avrundade fält för fält,
//! så att blanketten går jämnt ut i de belopp som lämnas.

use std::collections::BTreeMap;

use crate::DeclError;
use crate::catalog::{Catalog, FormKind, Sign};
use crate::declaration::{AccountBalance, FieldValue, Part};
use crate::ink2r::{Ink2R, LOSS, PROFIT};
use crate::mapping::{AccountMap, Target};

/// INK2S 4.1, årets resultat vinst.
pub const RESULT_PROFIT: &str = "7650";
/// INK2S 4.2, årets resultat förlust.
pub const RESULT_LOSS: &str = "7750";
/// INK2S 4.3a, skatt på årets resultat.
pub const TAX_EXPENSE: &str = "7651";
/// INK2S 4.14c, reduktion av underskott som också ska till 1.2.
pub const DEFICIT_REDUCTION: &str = "7672";
/// INK2S 4.15, överskott.
pub const SURPLUS: &str = "7670";
/// INK2S 4.16, underskott.
pub const DEFICIT: &str = "7770";
/// INK2 1.1, överskott av näringsverksamhet.
pub const MAIN_SURPLUS: &str = "7104";
/// INK2 1.2, underskott av näringsverksamhet.
pub const MAIN_DEFICIT: &str = "7114";
/// INK2R 3.25, skatt på årets resultat.
const INK2R_TAX: &str = "7528";

/// Fälten som räknas fram och inte kan anges för hand.
pub const COMPUTED: [&str; 8] = [
    RESULT_PROFIT,
    RESULT_LOSS,
    TAX_EXPENSE,
    SURPLUS,
    DEFICIT,
    MAIN_SURPLUS,
    MAIN_DEFICIT,
    "7011",
];

/// Avrundar öre till hela kronor, halva bort från noll.
pub fn kronor(ore: i64) -> i64 {
    let k = ore / 100;
    let rest = ore % 100;
    if rest >= 50 {
        k + 1
    } else if rest <= -50 {
        k - 1
    } else {
        k
    }
}

/// Ett belopp angivet för hand, i hela kronor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualAmount {
    /// Blanketten.
    pub form: FormKind,
    /// Fältkoden.
    pub code: String,
    /// Beloppet i kronor, med fältets tecken.
    pub kronor: i64,
}

/// INK2S och INK2, i hela kronor (lagrade som öre).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adjustments {
    /// INK2S per kod.
    pub ink2s: BTreeMap<String, FieldValue>,
    /// INK2 per kod.
    pub ink2: BTreeMap<String, FieldValue>,
    /// Summan av 4.1–4.14c med tecken, i kronor: överskott om positiv.
    pub total: i64,
}

fn put(values: &mut BTreeMap<String, FieldValue>, code: &str, label: &str, kr: i64) {
    if kr == 0 {
        return;
    }
    let v = values
        .entry(code.to_string())
        .or_insert_with(|| FieldValue {
            code: code.to_string(),
            ore: 0,
            parts: Vec::new(),
        });
    v.ore += kr * 100;
    v.parts.push(Part {
        label: label.to_string(),
        ore: kr * 100,
    });
}

/// Fältnumrets delar, till exempel 4.14c → (4, 14).
fn number_parts(number: &str) -> Option<(u32, u32)> {
    let (major, rest) = number.split_once('.')?;
    let minor: String = rest.chars().take_while(char::is_ascii_digit).collect();
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// Räknar fram INK2S och INK2.
pub fn compute(
    catalog: &Catalog,
    map: &AccountMap,
    ink2r: &Ink2R,
    balances: &[AccountBalance],
    manual: &[ManualAmount],
) -> Result<Adjustments, DeclError> {
    let mut s = BTreeMap::new();
    let r = |code: &str| ink2r.values.get(code).map_or(0, |v| kronor(v.ore));
    put(
        &mut s,
        RESULT_PROFIT,
        "Årets resultat, vinst (INK2R 3.26)",
        r(PROFIT),
    );
    put(
        &mut s,
        RESULT_LOSS,
        "Årets resultat, förlust (INK2R 3.27)",
        r(LOSS),
    );
    put(
        &mut s,
        TAX_EXPENSE,
        "Skatt på årets resultat (INK2R 3.25)",
        r(INK2R_TAX),
    );

    // Konton enligt kopplingen, summerade per rad i öre och avrundade per fält.
    let mut by_code: BTreeMap<String, Vec<(String, i64)>> = BTreeMap::new();
    let mut by_rule: BTreeMap<usize, Vec<&AccountBalance>> = BTreeMap::new();
    for b in balances.iter().filter(|b| !b.balance.is_zero()) {
        if let Some(rule) = map.rule_for(b.account) {
            by_rule.entry(rule).or_default().push(b);
        }
    }
    for (rule, accounts) in by_rule {
        let net: i64 = accounts.iter().map(|b| b.balance.ore()).sum();
        let (code, debit_positive) = match &map.rules[rule].target {
            Target::Single(code) => {
                let f = catalog
                    .field(FormKind::Ink2S, code)
                    .ok_or_else(|| DeclError::UnknownField(code.clone()))?;
                (code.clone(), f.sign != Sign::Minus)
            }
            // I INK2S ökar ett plusfält inkomsten: debetnetto (kostnad) dit.
            Target::Pair { plus, minus } => {
                if net >= 0 {
                    (plus.clone(), true)
                } else {
                    (minus.clone(), false)
                }
            }
        };
        for b in accounts {
            let ore = if debit_positive {
                b.balance.ore()
            } else {
                -b.balance.ore()
            };
            by_code
                .entry(code.clone())
                .or_default()
                .push((format!("{} {}", b.account, b.name), ore));
        }
    }
    for (code, parts) in by_code {
        let total = kronor(parts.iter().map(|(_, o)| o).sum());
        if total == 0 {
            continue;
        }
        let v = s.entry(code.clone()).or_insert_with(|| FieldValue {
            code: code.clone(),
            ore: 0,
            parts: Vec::new(),
        });
        v.ore += total * 100;
        v.parts
            .extend(parts.into_iter().map(|(label, ore)| Part { label, ore }));
    }

    let mut main = BTreeMap::new();
    for m in manual {
        if COMPUTED.contains(&m.code.as_str()) {
            return Err(DeclError::Computed(m.code.clone()));
        }
        match m.form {
            FormKind::Ink2S => put(&mut s, &m.code, "Angivet för hand", m.kronor),
            FormKind::Ink2 => put(&mut main, &m.code, "Angivet för hand", m.kronor),
            FormKind::Ink2R => return Err(DeclError::Computed(m.code.clone())),
        }
    }

    let total: i64 = s
        .values()
        .filter_map(|v| {
            let f = catalog.field(FormKind::Ink2S, &v.code)?;
            let (major, minor) = number_parts(&f.number)?;
            if major != 4 || minor > 14 {
                return None;
            }
            Some(match f.sign {
                Sign::Minus => -v.ore / 100,
                _ => v.ore / 100,
            })
        })
        .sum();
    let reduction = s.get(DEFICIT_REDUCTION).map_or(0, |v| v.ore / 100);
    if total >= 0 {
        put(&mut s, SURPLUS, "Summan av 4.1–4.14c", total);
        put(&mut main, MAIN_SURPLUS, "Överskott (INK2S 4.15)", total);
    } else {
        put(&mut s, DEFICIT, "Summan av 4.1–4.14c", -total);
        put(&mut main, MAIN_DEFICIT, "Underskott (INK2S 4.16)", -total);
    }
    put(
        &mut main,
        MAIN_DEFICIT,
        "Reduktion av underskott (INK2S 4.14c)",
        reduction,
    );
    Ok(Adjustments {
        ink2s: s,
        ink2: main,
        total,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::testdata::{balances, catalog};

    fn adjustments(manual: &[ManualAmount]) -> Adjustments {
        let c = catalog();
        let b = balances();
        let r = crate::ink2r::compute(&c, &AccountMap::ink2r_bas().unwrap(), &b).unwrap();
        compute(&c, &AccountMap::ink2s_bas().unwrap(), &r, &b, manual).unwrap()
    }

    #[test]
    fn rounding_half_away_from_zero() {
        assert_eq!(kronor(150), 2);
        assert_eq!(kronor(149), 1);
        assert_eq!(kronor(-150), -2);
        assert_eq!(kronor(-149), -1);
    }

    #[test]
    fn result_plus_non_deductible_minus_tax_free() {
        let a = adjustments(&[]);
        assert_eq!(a.ink2s[RESULT_PROFIT].ore / 100, 131_600);
        // Representation 10 000 och kostnadsränta på skattekontot 500.
        assert_eq!(a.ink2s["7653"].ore / 100, 10_500);
        assert_eq!(a.ink2s["7653"].parts.len(), 2);
        // Skattefri ränta 100.
        assert_eq!(a.ink2s["7754"].ore / 100, 100);
        assert_eq!(a.total, 142_000);
        assert_eq!(a.ink2s[SURPLUS].ore / 100, 142_000);
        assert_eq!(a.ink2[MAIN_SURPLUS].ore / 100, 142_000);
        assert!(!a.ink2s.contains_key(DEFICIT));
    }

    #[test]
    fn manual_fields_and_a_deficit() {
        let a = adjustments(&[ManualAmount {
            form: FormKind::Ink2S,
            code: "7763".into(),
            kronor: 150_000,
        }]);
        assert_eq!(a.total, -8_000);
        assert_eq!(a.ink2s[DEFICIT].ore / 100, 8_000);
        assert_eq!(a.ink2[MAIN_DEFICIT].ore / 100, 8_000);
        assert!(!a.ink2.contains_key(MAIN_SURPLUS));
    }

    #[test]
    fn the_reduction_also_goes_to_1_2() {
        let a = adjustments(&[
            ManualAmount {
                form: FormKind::Ink2S,
                code: "7763".into(),
                kronor: 150_000,
            },
            ManualAmount {
                form: FormKind::Ink2S,
                code: DEFICIT_REDUCTION.into(),
                kronor: 3_000,
            },
        ]);
        // −8 000 + 3 000 = −5 000 i 4.16, och 1.2 = 5 000 + 3 000.
        assert_eq!(a.ink2s[DEFICIT].ore / 100, 5_000);
        assert_eq!(a.ink2[MAIN_DEFICIT].ore / 100, 8_000);
    }

    #[test]
    fn computed_fields_cannot_be_set_by_hand() {
        let c = catalog();
        let b = balances();
        let r = crate::ink2r::compute(&c, &AccountMap::ink2r_bas().unwrap(), &b).unwrap();
        let e = compute(
            &c,
            &AccountMap::ink2s_bas().unwrap(),
            &r,
            &b,
            &[ManualAmount {
                form: FormKind::Ink2S,
                code: SURPLUS.into(),
                kronor: 1,
            }],
        );
        assert!(matches!(e, Err(DeclError::Computed(_))));
    }
}
