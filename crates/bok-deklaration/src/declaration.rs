//! Hela deklarationen: indata, beräkning och kontroller.
//!
//! Indata är företaget, räkenskapsåret, de utgående saldona (debet positivt)
//! och uppgifter som anges för hand. Ur dem räknas INK2R, INK2S, INK2 och
//! bolagsskatten fram, och deklarationen kontrolleras mot fältförteckningen
//! innan den kan skrivas som SRU-filer.

use std::collections::BTreeMap;

use chrono::NaiveDate;

use bok_core::{AccountNumber, Money};

use crate::DeclError;
use crate::catalog::{Catalog, DataType, Field, FormKind};
use crate::ink2r::{self, Ink2R};
use crate::ink2s::{self, Adjustments, ManualAmount, kronor};
use crate::mapping::AccountMap;
use crate::summary::{grouped, ore_text};
use crate::tax::{CorporateTax, corporate_tax};

/// Fält som inte får finnas samtidigt med belopp, enligt reglerna i
/// Skatteverkets fältförteckning ("Får ej förekomma om …").
pub const EXCLUSIVE: [(&str, &str); 6] = [
    ("7450", "7550"),
    ("7650", "7750"),
    ("7670", "7770"),
    ("7132", "7133"),
    ("8040", "8041"),
    ("8044", "8045"),
];

/// Ett kontos utgående saldo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountBalance {
    /// Kontot.
    pub account: AccountNumber,
    /// Kontots namn, för sammanställningen.
    pub name: String,
    /// Saldot vid årets slut, debet positivt.
    pub balance: Money,
    /// Kontots SRU-kod i kontoplanen, om den har någon.
    pub sru: Option<String>,
}

/// Ett värde som anges för hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManualValue {
    /// Ett belopp i hela kronor, med fältets tecken.
    Amount(i64),
    /// Text, till exempel `X` i en kryssruta.
    Text(String),
}

/// En del av ett fälts belopp: ett konto eller en beräkning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    /// Vad delen är, till exempel "1930 Företagskonto".
    pub label: String,
    /// Beloppet i öre, med fältets tecken.
    pub ore: i64,
}

/// Ett fälts belopp och vad det består av.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldValue {
    /// Fältkoden.
    pub code: String,
    /// Beloppet i öre, med fältets tecken.
    pub ore: i64,
    /// Delarna.
    pub parts: Vec<Part>,
}

/// Indata till deklarationen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    /// Företagets namn.
    pub company_name: String,
    /// Organisationsnumret.
    pub org_number: String,
    /// Räkenskapsårets första dag.
    pub year_start: NaiveDate,
    /// Räkenskapsårets sista dag.
    pub year_end: NaiveDate,
    /// Utgående saldon.
    pub balances: Vec<AccountBalance>,
    /// Uppgifter för hand: fältnummer eller kod, och värde.
    pub manual: Vec<(String, ManualValue)>,
}

/// En kontroll av deklarationen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    /// Vad som kontrolleras.
    pub name: String,
    /// Om den gick igenom.
    pub ok: bool,
    /// Detaljer, särskilt när den inte gick igenom.
    pub detail: String,
}

/// En färdigräknad deklaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    /// Fältförteckningen.
    pub catalog: Catalog,
    /// Företagets namn.
    pub company_name: String,
    /// Organisationsnumret.
    pub org_number: String,
    /// Räkenskapsårets första dag.
    pub year_start: NaiveDate,
    /// Räkenskapsårets sista dag.
    pub year_end: NaiveDate,
    /// Räkenskapsschemat i öre.
    pub ink2r: Ink2R,
    /// INK2S och INK2 i hela kronor.
    pub adjustments: Adjustments,
    /// Textfält (kryssrutor och övriga upplysningar).
    pub texts: BTreeMap<(FormKind, String), String>,
    /// Bolagsskatten på överskottet.
    pub tax: CorporateTax,
    /// Kontrollerna.
    pub checks: Vec<Check>,
    /// Sådant som granskaren bör se.
    pub warnings: Vec<String>,
}

impl Declaration {
    /// Räknar fram deklarationen med BAS-kopplingarna i `docs/`.
    pub fn compute_bas(input: &Input) -> Result<Declaration, DeclError> {
        Declaration::compute(input, &AccountMap::ink2r_bas()?, &AccountMap::ink2s_bas()?)
    }

    /// Räknar fram deklarationen med givna kopplingar.
    pub fn compute(
        input: &Input,
        ink2r_map: &AccountMap,
        ink2s_map: &AccountMap,
    ) -> Result<Declaration, DeclError> {
        let catalog = Catalog::for_year_end(input.year_end)?;
        let mut amounts = Vec::new();
        let mut texts = BTreeMap::new();
        for (key, value) in &input.manual {
            let field = catalog.lookup(key)?;
            match (value, field.data_type) {
                (ManualValue::Text(t), DataType::Text) => {
                    texts.insert((field.form, field.code.clone()), t.trim().to_string());
                }
                (
                    ManualValue::Amount(k),
                    DataType::Signed | DataType::Unsigned | DataType::Digits(_),
                ) => {
                    amounts.push(ManualAmount {
                        form: field.form,
                        code: field.code.clone(),
                        kronor: *k,
                    });
                }
                _ => {
                    return Err(DeclError::Check(format!(
                        "fält {} tar inte det värdet ({:?})",
                        field.label(),
                        field.data_type
                    )));
                }
            }
        }
        let ink2r = ink2r::compute(&catalog, ink2r_map, &input.balances)?;
        let adjustments = ink2s::compute(&catalog, ink2s_map, &ink2r, &input.balances, &amounts)?;
        let tax = corporate_tax(adjustments.total.max(0));
        let warnings = ink2r.warnings.clone();
        let mut d = Declaration {
            catalog,
            company_name: input.company_name.clone(),
            org_number: input.org_number.clone(),
            year_start: input.year_start,
            year_end: input.year_end,
            ink2r,
            adjustments,
            texts,
            tax,
            checks: Vec::new(),
            warnings,
        };
        d.checks = d.run_checks();
        Ok(d)
    }

    /// Beloppen på en blankett i hela kronor, i fältförteckningens ordning.
    pub fn amounts(&self, form: FormKind) -> Vec<(&Field, i64, &FieldValue)> {
        let values = match form {
            FormKind::Ink2R => &self.ink2r.values,
            FormKind::Ink2S => &self.adjustments.ink2s,
            FormKind::Ink2 => &self.adjustments.ink2,
        };
        self.catalog
            .form_fields(form)
            .filter_map(|f| {
                let v = values.get(&f.code)?;
                let k = kronor(v.ore);
                (k != 0).then_some((f, k, v))
            })
            .collect()
    }

    /// Ett fälts belopp i hela kronor, eller `None` om fältet är tomt.
    pub fn kronor(&self, form: FormKind, code: &str) -> Option<i64> {
        self.amounts(form)
            .into_iter()
            .find(|(f, _, _)| f.code == code)
            .map(|(_, k, _)| k)
    }

    /// Sant om alla kontroller gick igenom.
    pub fn is_ok(&self) -> bool {
        self.checks.iter().all(|c| c.ok)
    }

    fn run_checks(&self) -> Vec<Check> {
        let mut checks = Vec::new();
        let r = &self.ink2r;
        let assets: i64 = self
            .amounts(FormKind::Ink2R)
            .iter()
            .filter(|(f, _, _)| f.number.starts_with("2.") && f.side == crate::catalog::Side::Debit)
            .map(|(_, k, _)| k)
            .sum();
        let claims: i64 = self
            .amounts(FormKind::Ink2R)
            .iter()
            .filter(|(f, _, _)| {
                f.number.starts_with("2.") && f.side == crate::catalog::Side::Credit
            })
            .map(|(_, k, _)| k)
            .sum();
        checks.push(Check {
            name: "Balansräkningen balanserar (2.1–2.26 mot 2.27–2.50)".into(),
            ok: r.assets_ore == r.equity_and_liabilities_ore,
            detail: format!(
                "tillgångar {} kr, eget kapital och skulder {} kr{}",
                grouped(assets),
                grouped(claims),
                if assets == claims {
                    String::new()
                } else {
                    format!(
                        "; skillnaden {} kr är avrundning fält för fält, som Skatteverket bortser från",
                        assets - claims
                    )
                }
            ),
        });
        let sum = ink2r::income_statement_sum(&self.catalog, r);
        checks.push(Check {
            name: "Resultaträkningen (3.1–3.25) summerar till årets resultat (3.26/3.27)".into(),
            ok: sum == r.result_ore,
            detail: format!(
                "fälten ger {} kr, resultatkontona {} kr",
                ore_text(sum),
                ore_text(r.result_ore)
            ),
        });
        let k = |form, code| self.kronor(form, code).unwrap_or(0);
        let moved = k(FormKind::Ink2R, ink2r::PROFIT) == k(FormKind::Ink2S, ink2s::RESULT_PROFIT)
            && k(FormKind::Ink2R, ink2r::LOSS) == k(FormKind::Ink2S, ink2s::RESULT_LOSS);
        checks.push(Check {
            name: "Årets resultat är fört från INK2R 3.26/3.27 till INK2S 4.1/4.2".into(),
            ok: moved,
            detail: String::new(),
        });
        let main = k(FormKind::Ink2S, ink2s::SURPLUS) == k(FormKind::Ink2, ink2s::MAIN_SURPLUS)
            && k(FormKind::Ink2S, ink2s::DEFICIT) + k(FormKind::Ink2S, ink2s::DEFICIT_REDUCTION)
                == k(FormKind::Ink2, ink2s::MAIN_DEFICIT);
        checks.push(Check {
            name: "Överskott eller underskott är fört till INK2 1.1/1.2".into(),
            ok: main,
            detail: String::new(),
        });
        let mut outside = Vec::new();
        for form in [FormKind::Ink2, FormKind::Ink2R, FormKind::Ink2S] {
            for (f, value, _) in self.amounts(form) {
                if !f.accepts(value) {
                    outside.push(format!("{} {}: {value}", form.as_str(), f.label()));
                }
            }
        }
        checks.push(Check {
            name: "Varje belopp ryms i fältets datatyp".into(),
            ok: outside.is_empty(),
            detail: outside.join("; "),
        });
        let present = |code: &str| {
            [FormKind::Ink2, FormKind::Ink2R, FormKind::Ink2S]
                .into_iter()
                .any(|form| {
                    self.kronor(form, code).is_some()
                        || self.texts.contains_key(&(form, code.to_string()))
                })
        };
        let clashes: Vec<String> = EXCLUSIVE
            .iter()
            .filter(|(a, b)| present(a) && present(b))
            .map(|(a, b)| format!("{a} och {b}"))
            .collect();
        checks.push(Check {
            name: "Fält som utesluter varandra är inte ifyllda samtidigt".into(),
            ok: clashes.is_empty(),
            detail: clashes.join("; "),
        });
        checks
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::testdata::input;

    #[test]
    fn the_example_company() {
        let d = Declaration::compute_bas(&input()).unwrap();
        assert!(d.is_ok(), "{:?}", d.checks);
        assert_eq!(d.kronor(FormKind::Ink2R, "7450"), Some(131_600));
        assert_eq!(d.kronor(FormKind::Ink2S, "7670"), Some(142_000));
        assert_eq!(d.kronor(FormKind::Ink2, "7104"), Some(142_000));
        assert_eq!(d.tax.taxable, 142_000);
        assert_eq!(d.tax.tax_ore, 2_925_200);
    }

    #[test]
    fn manual_values_by_number_or_code() {
        let mut i = input();
        i.manual = vec![
            ("4.20".into(), ManualValue::Amount(5_000)),
            ("8041".into(), ManualValue::Text("X".into())),
            ("8045".into(), ManualValue::Text("X".into())),
        ];
        let d = Declaration::compute_bas(&i).unwrap();
        assert!(d.is_ok(), "{:?}", d.checks);
        assert_eq!(d.kronor(FormKind::Ink2S, "8026"), Some(5_000));
        // 4.20 är en upplysning och påverkar inte överskottet.
        assert_eq!(d.kronor(FormKind::Ink2S, "7670"), Some(142_000));
        assert_eq!(d.texts[&(FormKind::Ink2S, "8041".into())], "X");
    }

    #[test]
    fn clashing_and_wrongly_typed_values_are_caught() {
        let mut i = input();
        i.manual = vec![
            ("8040".into(), ManualValue::Text("X".into())),
            ("8041".into(), ManualValue::Text("X".into())),
        ];
        let d = Declaration::compute_bas(&i).unwrap();
        assert!(!d.is_ok());
        let mut i = input();
        i.manual = vec![("8040".into(), ManualValue::Amount(1))];
        assert!(Declaration::compute_bas(&i).is_err());
        let mut i = input();
        i.manual = vec![("4.15".into(), ManualValue::Amount(1))];
        assert!(matches!(
            Declaration::compute_bas(&i),
            Err(DeclError::Computed(_))
        ));
        let mut i = input();
        i.manual = vec![("2.4".into(), ManualValue::Amount(1))];
        assert!(matches!(
            Declaration::compute_bas(&i),
            Err(DeclError::Computed(_))
        ));
    }

    #[test]
    fn a_negative_amount_in_an_unsigned_field_fails_the_check() {
        let mut i = input();
        i.manual = vec![("4.14a".into(), ManualValue::Amount(-1))];
        let d = Declaration::compute_bas(&i).unwrap();
        assert!(!d.is_ok());
    }
}
