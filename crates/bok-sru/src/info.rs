//! `INFO.SRU`: leveransens beskrivning och uppgiftslämnaren
//! (medieleverantören).
//!
//! Posterna kommer i den ordning Skatteverket kräver. Valfria poster utan
//! värde lämnas bort; en saknad obligatorisk post är ett fel, eftersom hela
//! leveransen då avvisas.

use chrono::NaiveDateTime;

use crate::orgnr::sru_orgnr;
use crate::text::{check_value, one_line, to_latin1};
use crate::{FORMS_FILE, SruError};

/// Den som lämnar filerna: företaget självt eller en redovisningsbyrå.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MediaProvider {
    /// Organisationsnumret, till exempel "559000-0005".
    pub org_number: String,
    /// Namnet.
    pub name: String,
    /// Postnumret, siffror (mellanslag tas bort).
    pub postal_code: String,
    /// Postorten.
    pub city: String,
    /// Utdelningsadressen (valfri).
    pub address: Option<String>,
    /// Avdelningen (valfri).
    pub department: Option<String>,
    /// Kontaktpersonen (valfri).
    pub contact: Option<String>,
    /// E-postadressen (valfri).
    pub email: Option<String>,
    /// Telefonnumret (valfritt, högst 15 tecken).
    pub phone: Option<String>,
}

fn required(name: &str, value: &str) -> Result<String, SruError> {
    let value = one_line(value);
    if value.is_empty() {
        return Err(SruError::Missing(name.to_string()));
    }
    check_value(name, &value)?;
    Ok(value)
}

fn optional(out: &mut String, name: &str, value: Option<&str>) -> Result<(), SruError> {
    let Some(value) = value.map(one_line).filter(|v| !v.is_empty()) else {
        return Ok(());
    };
    check_value(name, &value)?;
    out.push_str(&format!("{name} {value}\n"));
    Ok(())
}

/// Skriver `INFO.SRU` i ISO 8859-1. `program` är programmets namn och
/// version, och `created` tiden då filerna skapades.
pub fn write_info(
    provider: &MediaProvider,
    program: &str,
    created: NaiveDateTime,
) -> Result<Vec<u8>, SruError> {
    let orgnr = sru_orgnr(&provider.org_number)?;
    let name = required("#NAMN", &provider.name)?;
    let postal: String = provider
        .postal_code
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if postal.is_empty() || !postal.chars().all(|c| c.is_ascii_digit()) {
        return Err(SruError::Missing("#POSTNR (bara siffror)".into()));
    }
    let city = required("#POSTORT", &provider.city)?;
    if let Some(phone) = &provider.phone
        && phone.chars().count() > 15
    {
        return Err(SruError::Missing("#TELEFON (högst 15 tecken)".into()));
    }

    let mut out = String::new();
    out.push_str("#DATABESKRIVNING_START\n");
    out.push_str("#PRODUKT SRU\n");
    out.push_str(&format!("#SKAPAD {}\n", created.format("%Y%m%d %H%M%S")));
    optional(&mut out, "#PROGRAM", Some(program))?;
    out.push_str(&format!("#FILNAMN {FORMS_FILE}\n"));
    out.push_str("#DATABESKRIVNING_SLUT\n");
    out.push_str("#MEDIELEV_START\n");
    out.push_str(&format!("#ORGNR {orgnr}\n"));
    out.push_str(&format!("#NAMN {name}\n"));
    optional(&mut out, "#ADRESS", provider.address.as_deref())?;
    out.push_str(&format!("#POSTNR {postal}\n"));
    out.push_str(&format!("#POSTORT {city}\n"));
    optional(&mut out, "#AVDELNING", provider.department.as_deref())?;
    optional(&mut out, "#KONTAKT", provider.contact.as_deref())?;
    optional(&mut out, "#EMAIL", provider.email.as_deref())?;
    optional(&mut out, "#TELEFON", provider.phone.as_deref())?;
    out.push_str("#MEDIELEV_SLUT\n");
    to_latin1(&out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(crate) mod tests {
    use super::*;
    use crate::text::from_latin1;

    pub fn provider() -> MediaProvider {
        MediaProvider {
            org_number: "559000-0005".into(),
            name: "Exempelbolaget i Åre AB".into(),
            postal_code: "123 45".into(),
            city: "Småstad".into(),
            address: Some("Storgatan 1".into()),
            contact: Some("Kim Exempel".into()),
            ..MediaProvider::default()
        }
    }

    pub fn created() -> NaiveDateTime {
        chrono::NaiveDate::from_ymd_opt(2027, 3, 1)
            .unwrap()
            .and_hms_opt(9, 5, 7)
            .unwrap()
    }

    #[test]
    fn records_in_the_required_order() {
        let text = from_latin1(&write_info(&provider(), "Bokis 0.1", created()).unwrap());
        let names: Vec<&str> = text.lines().map(|l| l.split(' ').next().unwrap()).collect();
        assert_eq!(
            names,
            [
                "#DATABESKRIVNING_START",
                "#PRODUKT",
                "#SKAPAD",
                "#PROGRAM",
                "#FILNAMN",
                "#DATABESKRIVNING_SLUT",
                "#MEDIELEV_START",
                "#ORGNR",
                "#NAMN",
                "#ADRESS",
                "#POSTNR",
                "#POSTORT",
                "#KONTAKT",
                "#MEDIELEV_SLUT",
            ]
        );
        assert!(text.contains("#ORGNR 165590000005\n"));
        assert!(text.contains("#POSTNR 12345\n"));
        assert!(text.contains("#SKAPAD 20270301 090507\n"));
        assert!(text.contains("#FILNAMN BLANKETTER.SRU\n"));
        assert!(!text.contains('\r'));
    }

    #[test]
    fn missing_required_values_are_errors() {
        let mut p = provider();
        p.city = " ".into();
        assert!(write_info(&p, "Bokis", created()).is_err());
        let mut p = provider();
        p.postal_code = "SE-123".into();
        assert!(write_info(&p, "Bokis", created()).is_err());
        let mut p = provider();
        p.org_number = "559000-0006".into();
        assert!(write_info(&p, "Bokis", created()).is_err());
    }
}
