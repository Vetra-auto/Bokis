//! Blanketterna och `BLANKETTER.SRU`.
//!
//! En blankett är `#BLANKETT <namn>`, `#IDENTITET <orgnr> <datum> <tid>`,
//! valfritt `#NAMN`, en `#UPPGIFT <fältkod> <värde>` per uppgift och
//! `#BLANKETTSLUT`. Filen slutar med `#FIL_SLUT`. En fältkod får förekomma
//! en gång per blankett, och en blankett utan uppgifter lämnas inte.

use std::collections::BTreeSet;

use chrono::{NaiveDate, NaiveDateTime};

use crate::SruError;
use crate::orgnr::sru_orgnr;
use crate::text::{check_value, one_line, to_latin1};

/// Ett värde i en `#UPPGIFT`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// Ett belopp eller annat heltal; negativt skrivs med `-`.
    Integer(i64),
    /// Ett datum, `ÅÅÅÅMMDD`.
    Date(NaiveDate),
    /// En sträng, till exempel `X` i en kryssruta.
    Text(String),
}

impl Value {
    /// Värdet som det skrivs i filen.
    pub fn render(&self) -> String {
        match self {
            Value::Integer(n) => n.to_string(),
            Value::Date(d) => d.format("%Y%m%d").to_string(),
            Value::Text(s) => one_line(s),
        }
    }
}

/// En blankett med sina uppgifter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Form {
    /// Blankettens namn med period, till exempel `INK2R-2026P4`.
    pub name: String,
    /// Uppgiftslämnarens organisationsnummer.
    pub org_number: String,
    /// Uppgiftslämnarens namn, om blanketten har en sådan post.
    pub taxpayer_name: Option<String>,
    /// När blanketten framställdes. En senare tid ersätter en tidigare
    /// inlämnad blankett.
    pub created: NaiveDateTime,
    /// Uppgifterna: fältkod och värde, i den ordning de skrivs.
    pub fields: Vec<(String, Value)>,
}

fn check_name(name: &str) -> Result<(), SruError> {
    let ok = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-');
    if ok {
        Ok(())
    } else {
        Err(SruError::FormName(name.to_string()))
    }
}

fn check_code(code: &str) -> Result<(), SruError> {
    if !code.is_empty() && code.len() <= 6 && code.chars().all(|c| c.is_ascii_digit()) {
        Ok(())
    } else {
        Err(SruError::BadCode(code.to_string()))
    }
}

impl Form {
    /// Blankettens poster som text, utan `#FIL_SLUT`.
    fn render(&self, out: &mut String) -> Result<(), SruError> {
        check_name(&self.name)?;
        let orgnr = sru_orgnr(&self.org_number)?;
        if self.fields.is_empty() {
            return Err(SruError::Missing(format!("{}: uppgifter", self.name)));
        }
        out.push_str(&format!("#BLANKETT {}\n", self.name));
        out.push_str(&format!(
            "#IDENTITET {orgnr} {}\n",
            self.created.format("%Y%m%d %H%M%S")
        ));
        if let Some(name) = self.taxpayer_name.as_deref().map(one_line) {
            check_value("#NAMN", &name)?;
            if !name.is_empty() {
                out.push_str(&format!("#NAMN {name}\n"));
            }
        }
        let mut seen = BTreeSet::new();
        for (code, value) in &self.fields {
            check_code(code)?;
            if !seen.insert(code.as_str()) {
                return Err(SruError::Duplicate {
                    form: self.name.clone(),
                    code: code.clone(),
                });
            }
            let rendered = value.render();
            if rendered.is_empty() {
                return Err(SruError::Missing(format!(
                    "{}: värdet för {code}",
                    self.name
                )));
            }
            check_value(&format!("#UPPGIFT {code}"), &rendered)?;
            out.push_str(&format!("#UPPGIFT {code} {rendered}\n"));
        }
        out.push_str("#BLANKETTSLUT\n");
        Ok(())
    }
}

/// Skriver `BLANKETTER.SRU` i ISO 8859-1 med blanketterna i ordning.
pub fn write_forms(forms: &[Form]) -> Result<Vec<u8>, SruError> {
    if forms.is_empty() {
        return Err(SruError::Missing("blanketter".into()));
    }
    let mut out = String::new();
    for form in forms {
        form.render(&mut out)?;
    }
    out.push_str("#FIL_SLUT\n");
    to_latin1(&out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(crate) mod tests {
    use super::*;
    use crate::text::from_latin1;

    pub fn form() -> Form {
        Form {
            name: "INK2R-2026P4".into(),
            org_number: "559000-0005".into(),
            taxpayer_name: Some("Exempelbolaget i Åre AB".into()),
            created: chrono::NaiveDate::from_ymd_opt(2027, 3, 1)
                .unwrap()
                .and_hms_opt(9, 5, 7)
                .unwrap(),
            fields: vec![
                (
                    "7011".into(),
                    Value::Date(NaiveDate::from_ymd_opt(2026, 4, 27).unwrap()),
                ),
                ("7215".into(), Value::Integer(28_000)),
                ("7281".into(), Value::Integer(-12)),
            ],
        }
    }

    #[test]
    fn a_whole_file() {
        let text = from_latin1(&write_forms(&[form()]).unwrap());
        assert_eq!(
            text,
            "#BLANKETT INK2R-2026P4\n\
             #IDENTITET 165590000005 20270301 090507\n\
             #NAMN Exempelbolaget i Åre AB\n\
             #UPPGIFT 7011 20260427\n\
             #UPPGIFT 7215 28000\n\
             #UPPGIFT 7281 -12\n\
             #BLANKETTSLUT\n\
             #FIL_SLUT\n"
        );
    }

    #[test]
    fn form_errors() {
        let mut f = form();
        f.fields.push(("7215".into(), Value::Integer(1)));
        assert!(matches!(write_forms(&[f]), Err(SruError::Duplicate { .. })));
        let mut f = form();
        f.name = "ink2r 2026".into();
        assert!(matches!(write_forms(&[f]), Err(SruError::FormName(_))));
        let mut f = form();
        f.fields.clear();
        assert!(write_forms(&[f]).is_err());
        let mut f = form();
        f.fields.push(("72a".into(), Value::Integer(1)));
        assert!(matches!(write_forms(&[f]), Err(SruError::BadCode(_))));
        let mut f = form();
        f.fields.push(("90".into(), Value::Text("A # B".into())));
        assert!(matches!(write_forms(&[f]), Err(SruError::Hash(_))));
        assert!(write_forms(&[]).is_err());
    }
}
