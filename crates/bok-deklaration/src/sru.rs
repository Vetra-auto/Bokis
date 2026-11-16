//! Deklarationen som SRU-filer: `INFO.SRU` och `BLANKETTER.SRU` med
//! blanketterna INK2, INK2R och INK2S i den ordningen.
//!
//! Filerna skrivs bara när alla kontroller har gått igenom, och de läses
//! tillbaka och prövas mot fältförteckningen innan de lämnas ut: varje
//! fältkod ska finnas på sin blankett, varje belopp rymmas i fältets
//! datatyp, och blankettnamnet ska vara periodens.

use chrono::NaiveDateTime;

use bok_sru::{Form, MediaProvider, Value};

use crate::DeclError;
use crate::catalog::{Catalog, DataType, FormKind};
use crate::declaration::Declaration;

/// Fältkoderna för räkenskapsårets början och slut.
const YEAR_START: &str = "7011";
const YEAR_END: &str = "7012";

/// De två filerna, kodade i ISO 8859-1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SruFiles {
    /// `INFO.SRU`.
    pub info: Vec<u8>,
    /// `BLANKETTER.SRU`.
    pub forms: Vec<u8>,
}

/// Blanketterna för deklarationen.
pub fn forms(d: &Declaration, created: NaiveDateTime) -> Vec<Form> {
    [FormKind::Ink2, FormKind::Ink2R, FormKind::Ink2S]
        .into_iter()
        .map(|kind| {
            let mut fields = vec![
                (YEAR_START.to_string(), Value::Date(d.year_start)),
                (YEAR_END.to_string(), Value::Date(d.year_end)),
            ];
            fields.extend(
                d.amounts(kind)
                    .into_iter()
                    .map(|(f, k, _)| (f.code.clone(), Value::Integer(k))),
            );
            fields.extend(
                d.texts
                    .iter()
                    .filter(|((form, _), t)| *form == kind && !t.is_empty())
                    .map(|((_, code), t)| (code.clone(), Value::Text(t.clone()))),
            );
            Form {
                name: d.catalog.form_name(kind),
                org_number: d.org_number.clone(),
                // Huvudblanketten har ingen namnpost i fältförteckningen.
                taxpayer_name: (kind != FormKind::Ink2).then(|| d.company_name.clone()),
                created,
                fields,
            }
        })
        .collect()
}

/// Skriver filerna. Vägrar om någon kontroll inte gick igenom.
pub fn write(
    d: &Declaration,
    provider: &MediaProvider,
    program: &str,
    created: NaiveDateTime,
) -> Result<SruFiles, DeclError> {
    let failed: Vec<String> = d
        .checks
        .iter()
        .filter(|c| !c.ok)
        .map(|c| format!("{} ({})", c.name, c.detail))
        .collect();
    if !failed.is_empty() {
        return Err(DeclError::Check(format!(
            "deklarationen klarar inte kontrollerna: {}",
            failed.join("; ")
        )));
    }
    let files = SruFiles {
        info: bok_sru::write_info(provider, program, created)?,
        forms: bok_sru::write_forms(&forms(d, created))?,
    };
    validate(&d.catalog, &files)?;
    Ok(files)
}

/// Läser filerna och prövar dem mot fältförteckningen.
pub fn validate(catalog: &Catalog, files: &SruFiles) -> Result<(), DeclError> {
    bok_sru::read_info(&files.info)?;
    let read = bok_sru::read_forms(&files.forms)?;
    let bad = |m: String| Err(DeclError::Check(m));
    let expected: Vec<String> = [FormKind::Ink2, FormKind::Ink2R, FormKind::Ink2S]
        .into_iter()
        .map(|k| catalog.form_name(k))
        .collect();
    let names: Vec<String> = read.iter().map(|f| f.name.clone()).collect();
    if names != expected {
        return bad(format!("blanketterna är {names:?}, inte {expected:?}"));
    }
    for (form, kind) in read
        .iter()
        .zip([FormKind::Ink2, FormKind::Ink2R, FormKind::Ink2S])
    {
        for (code, value) in &form.fields {
            let Some(field) = catalog.field(kind, code) else {
                return bad(format!("{}: {code} finns inte på blanketten", form.name));
            };
            let ok = match field.data_type {
                DataType::Date => value.len() == 8 && value.chars().all(|c| c.is_ascii_digit()),
                DataType::Text => !value.is_empty(),
                _ => value.parse::<i64>().is_ok_and(|n| field.accepts(n)),
            };
            if !ok {
                return bad(format!("{}: {code} har värdet {value}", form.name));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::testdata::{d, input};

    fn provider() -> MediaProvider {
        MediaProvider {
            org_number: "559000-0005".into(),
            name: "Exempelbolaget AB".into(),
            postal_code: "12345".into(),
            city: "Småstad".into(),
            ..MediaProvider::default()
        }
    }

    fn created() -> NaiveDateTime {
        d(2027, 2, 10).and_hms_opt(14, 30, 0).unwrap()
    }

    #[test]
    fn three_forms_in_order_with_the_right_values() {
        let decl = Declaration::compute_bas(&input()).unwrap();
        let files = write(&decl, &provider(), "Bokis test", created()).unwrap();
        let read = bok_sru::read_forms(&files.forms).unwrap();
        assert_eq!(read.len(), 3);
        assert_eq!(read[0].name, "INK2-2026P4");
        assert_eq!(read[0].org_number, "165590000005");
        assert_eq!(read[0].taxpayer_name, None);
        assert_eq!(read[0].fields["7011"], "20260427");
        assert_eq!(read[0].fields["7012"], "20261231");
        assert_eq!(read[0].fields["7104"], "142000");
        assert_eq!(read[1].name, "INK2R-2026P4");
        assert_eq!(read[1].taxpayer_name.as_deref(), Some("Exempelbolaget AB"));
        assert_eq!(read[1].fields["7215"], "28000");
        assert_eq!(read[1].fields["7450"], "131600");
        assert_eq!(read[2].fields["7650"], "131600");
        assert_eq!(read[2].fields["7653"], "10500");
        assert_eq!(read[2].fields["7754"], "100");
        assert_eq!(read[2].fields["7670"], "142000");
        // Inga tomma fält och inga nollor.
        assert!(read.iter().all(|f| f.fields.values().all(|v| v != "0")));
    }

    #[test]
    fn a_failed_check_writes_nothing() {
        let mut i = input();
        i.manual = vec![("4.14a".into(), crate::declaration::ManualValue::Amount(-1))];
        let decl = Declaration::compute_bas(&i).unwrap();
        assert!(matches!(
            write(&decl, &provider(), "Bokis", created()),
            Err(DeclError::Check(_))
        ));
    }

    #[test]
    fn validation_catches_unknown_codes() {
        let decl = Declaration::compute_bas(&input()).unwrap();
        let mut forms = forms(&decl, created());
        forms[1].fields.push(("9999".into(), Value::Integer(1)));
        let files = SruFiles {
            info: bok_sru::write_info(&provider(), "Bokis", created()).unwrap(),
            forms: bok_sru::write_forms(&forms).unwrap(),
        };
        assert!(validate(&decl.catalog, &files).is_err());
    }
}
