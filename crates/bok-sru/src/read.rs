//! Läser tillbaka `INFO.SRU` och `BLANKETTER.SRU`.
//!
//! Läsaren är sträng: okända poster, fel ordning, poster utan värde och en
//! fil som inte slutar med `#FIL_SLUT` är fel. Den används för att pröva att
//! det Bokis skriver har den form Skatteverket kräver.

use std::collections::BTreeMap;

use crate::text::from_latin1;
use crate::{FORMS_FILE, SruError};

/// En inläst blankett.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadForm {
    /// Blankettens namn.
    pub name: String,
    /// Organisationsnumret i SRU-form.
    pub org_number: String,
    /// Datum och tid, som de står.
    pub created: (String, String),
    /// `#NAMN`, om den finns.
    pub taxpayer_name: Option<String>,
    /// Uppgifterna per fältkod, som de står.
    pub fields: BTreeMap<String, String>,
}

fn syntax(line: usize, message: impl Into<String>) -> SruError {
    SruError::Syntax {
        line,
        message: message.into(),
    }
}

/// Raderna som (radnummer, postnamn, resten).
fn records(bytes: &[u8]) -> Result<Vec<(usize, String, String)>, SruError> {
    let text = from_latin1(bytes);
    if !text.ends_with('\n') {
        return Err(syntax(
            text.lines().count(),
            "filen slutar inte med en radbrytning",
        ));
    }
    let mut out = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.trim().is_empty() {
            continue;
        }
        let Some(rest) = line.strip_prefix('#') else {
            return Err(syntax(i + 1, "raden börjar inte med #"));
        };
        let (name, value) = match rest.split_once([' ', '\t']) {
            Some((n, v)) => (n, v.trim()),
            None => (rest, ""),
        };
        out.push((i + 1, format!("#{name}"), value.to_string()));
    }
    Ok(out)
}

/// Läser `INFO.SRU` och ger posterna i ordning, som (postnamn, värde).
/// Kontrollerar ordningen och att de obligatoriska posterna finns.
pub fn read_info(bytes: &[u8]) -> Result<Vec<(String, String)>, SruError> {
    const ORDER: [(&str, bool); 19] = [
        ("#DATABESKRIVNING_START", true),
        ("#PRODUKT", true),
        ("#MEDIAID", false),
        ("#SKAPAD", false),
        ("#PROGRAM", false),
        ("#FILNAMN", true),
        ("#DATABESKRIVNING_SLUT", true),
        ("#MEDIELEV_START", true),
        ("#ORGNR", true),
        ("#NAMN", true),
        ("#ADRESS", false),
        ("#POSTNR", true),
        ("#POSTORT", true),
        ("#AVDELNING", false),
        ("#KONTAKT", false),
        ("#EMAIL", false),
        ("#TELEFON", false),
        ("#FAX", false),
        ("#MEDIELEV_SLUT", true),
    ];
    let recs = records(bytes)?;
    let mut pos = 0;
    for (line, name, value) in &recs {
        let Some(found) = ORDER[pos..].iter().position(|(n, _)| n == name) else {
            return Err(syntax(
                *line,
                format!("{name} är okänd eller i fel ordning"),
            ));
        };
        if let Some((missing, _)) = ORDER[pos..pos + found].iter().find(|(_, req)| *req) {
            return Err(syntax(*line, format!("{missing} saknas före {name}")));
        }
        let needs_value = !name.ends_with("_START") && !name.ends_with("_SLUT");
        if needs_value && value.is_empty() {
            return Err(syntax(*line, format!("{name} saknar värde")));
        }
        if name == "#PRODUKT" && value != "SRU" {
            return Err(syntax(*line, "#PRODUKT ska vara SRU"));
        }
        if name == "#FILNAMN" && value != FORMS_FILE {
            return Err(syntax(*line, format!("#FILNAMN ska vara {FORMS_FILE}")));
        }
        if name == "#ORGNR" && !(value.len() == 12 && value.chars().all(|c| c.is_ascii_digit())) {
            return Err(syntax(*line, "#ORGNR ska ha tolv siffror"));
        }
        pos += found + 1;
    }
    if let Some((missing, _)) = ORDER[pos..].iter().find(|(_, req)| *req) {
        return Err(syntax(recs.len(), format!("{missing} saknas")));
    }
    Ok(recs.into_iter().map(|(_, n, v)| (n, v)).collect())
}

/// Läser `BLANKETTER.SRU`.
pub fn read_forms(bytes: &[u8]) -> Result<Vec<ReadForm>, SruError> {
    let recs = records(bytes)?;
    let mut forms = Vec::new();
    let mut current: Option<ReadForm> = None;
    let mut ended = false;
    for (line, name, value) in recs {
        if ended {
            return Err(syntax(line, "poster efter #FIL_SLUT"));
        }
        match (name.as_str(), current.as_mut()) {
            ("#BLANKETT", None) => {
                if value.is_empty() {
                    return Err(syntax(line, "#BLANKETT saknar namn"));
                }
                current = Some(ReadForm {
                    name: value,
                    org_number: String::new(),
                    created: (String::new(), String::new()),
                    taxpayer_name: None,
                    fields: BTreeMap::new(),
                });
            }
            ("#IDENTITET", Some(f)) if f.org_number.is_empty() => {
                let parts: Vec<&str> = value.split_whitespace().collect();
                let [orgnr, date, time] = parts.as_slice() else {
                    return Err(syntax(line, "#IDENTITET ska ha orgnr, datum och tid"));
                };
                let digits =
                    |s: &str, n: usize| s.len() == n && s.chars().all(|c| c.is_ascii_digit());
                if !digits(orgnr, 12) || !digits(date, 8) || !digits(time, 6) {
                    return Err(syntax(line, "#IDENTITET har fel form"));
                }
                f.org_number = (*orgnr).to_string();
                f.created = ((*date).to_string(), (*time).to_string());
            }
            ("#NAMN", Some(f)) if f.fields.is_empty() && !f.org_number.is_empty() => {
                f.taxpayer_name = Some(value);
            }
            ("#UPPGIFT", Some(f)) if !f.org_number.is_empty() => {
                let Some((code, v)) = value.split_once([' ', '\t']) else {
                    return Err(syntax(line, "#UPPGIFT saknar värde"));
                };
                if f.fields
                    .insert(code.to_string(), v.trim().to_string())
                    .is_some()
                {
                    return Err(syntax(
                        line,
                        format!("fältkoden {code} förekommer två gånger"),
                    ));
                }
            }
            ("#SYSTEMINFO", Some(f)) if !f.org_number.is_empty() => {}
            ("#BLANKETTSLUT", Some(f)) if !f.org_number.is_empty() => {
                if f.fields.is_empty() {
                    return Err(syntax(line, "blanketten har inga uppgifter"));
                }
                forms.extend(current.take());
            }
            ("#FIL_SLUT", None) => ended = true,
            (other, _) => return Err(syntax(line, format!("{other} är okänd eller på fel plats"))),
        }
    }
    if !ended {
        return Err(syntax(0, "#FIL_SLUT saknas"));
    }
    if forms.is_empty() {
        return Err(syntax(0, "filen har inga blanketter"));
    }
    Ok(forms)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::form::tests::form;
    use crate::info::tests::{created, provider};
    use crate::{write_forms, write_info};

    #[test]
    fn what_is_written_reads_back() {
        let info = read_info(&write_info(&provider(), "Bokis", created()).unwrap()).unwrap();
        assert_eq!(info[1], ("#PRODUKT".into(), "SRU".into()));
        let forms = read_forms(&write_forms(&[form(), form()]).unwrap()).unwrap();
        assert_eq!(forms.len(), 2);
        assert_eq!(forms[0].org_number, "165590000005");
        assert_eq!(forms[0].fields["7215"], "28000");
        assert_eq!(
            forms[0].taxpayer_name.as_deref(),
            Some("Exempelbolaget i Åre AB")
        );
    }

    #[test]
    fn broken_files_are_refused() {
        assert!(read_info(b"#DATABESKRIVNING_START\n#FILNAMN BLANKETTER.SRU\n").is_err());
        assert!(read_forms(b"#BLANKETT X\n#UPPGIFT 7011 1\n#BLANKETTSLUT\n#FIL_SLUT\n").is_err());
        assert!(read_forms(
            b"#BLANKETT X\n#IDENTITET 165590000005 20270301 090507\n#UPPGIFT 1 1\n#BLANKETTSLUT\n"
        )
        .is_err());
        assert!(read_forms(
            b"#BLANKETT X\n#IDENTITET 165590000005 20270301 090507\n#UPPGIFT 1 1\n#UPPGIFT 1 2\n#BLANKETTSLUT\n#FIL_SLUT\n"
        )
        .is_err());
    }
}
