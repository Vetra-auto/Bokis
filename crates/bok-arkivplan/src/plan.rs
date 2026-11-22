//! Planens delar: vad som förvaras, i vilken form, var, hur ofta det
//! uppdateras, hur länge det sparas och hur det kontrolleras.
//!
//! Filen är JSON med svenska nycklar. Okända nycklar avvisas, så att ett
//! felstavat fält inte försvinner tyst.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Längsta tillåtna text i ett fält.
pub const MAX_TEXT: usize = 2_000;
/// Flest delar i en plan.
pub const MAX_PARTS: usize = 50;

/// Hela arkivplanen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchivePlan {
    /// Vem som ansvarar för arkivet, till exempel en roll eller ett namn.
    #[serde(rename = "ansvarig")]
    pub responsible: String,
    /// Var räkenskapsinformationen och kopiorna finns.
    #[serde(rename = "delar")]
    pub parts: Vec<Part>,
    /// Hur informationen tas fram i läsbar form under hela bevarandetiden.
    #[serde(
        rename = "lasbar_form",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub readable: Option<String>,
    /// Övrigt som en granskare behöver veta om arkivet.
    #[serde(rename = "kommentar", default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// En plats där något ur bokföringen förvaras.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Part {
    /// Vad som förvaras, till exempel "Databasen och underlagen".
    #[serde(rename = "vad")]
    pub what: String,
    /// Original, säkerhetskopia, arkivpaket eller papper.
    #[serde(rename = "slag")]
    pub kind: Kind,
    /// Var det förvaras: sökväg, dator, lokal.
    #[serde(rename = "plats")]
    pub place: String,
    /// Landet där platsen ligger.
    #[serde(rename = "land", default = "sweden")]
    pub country: String,
    /// När platsen anmäldes till Skatteverket, om den ligger utanför Sverige
    /// (bokföringslagen 7 kap. 3 a §).
    #[serde(rename = "anmald", default, skip_serializing_if = "Option::is_none")]
    pub notified: Option<String>,
    /// Hur ofta platsen uppdateras, till exempel "varje natt 02.30".
    #[serde(rename = "hur_ofta", default, skip_serializing_if = "Option::is_none")]
    pub frequency: Option<String>,
    /// Hur länge det sparas, till exempel "30 nätter" eller "hela bevarandetiden".
    #[serde(rename = "sparas", default, skip_serializing_if = "Option::is_none")]
    pub kept: Option<String>,
    /// Hur platsen kontrolleras och hur ofta.
    #[serde(rename = "kontroll", default, skip_serializing_if = "Option::is_none")]
    pub check: Option<String>,
    /// Förvaras åtskild från originalet (annan disk, annan lokal).
    #[serde(rename = "atskild", default)]
    pub separate: bool,
    /// Katalog som `bok kopia skapa` skriver till; systemdokumentationen
    /// visar då den senaste kopian dit ur behandlingshistoriken.
    #[serde(rename = "sokvag", default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

fn sweden() -> String {
    "Sverige".into()
}

/// Vad för slags förvaring en del är.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Den bokföring som används: databasen och underlagsarkivet.
    Original,
    /// En kopia som tas regelbundet (BFNAR 2013:2 punkt 8.2).
    Sakerhetskopia,
    /// Ett avslutat räkenskapsår som läsbart paket (`bok arkivpaket`).
    Arkivpaket,
    /// Underlag på papper, till exempel en kassabok.
    Papper,
}

impl Kind {
    /// Namnet på svenska.
    pub fn title(self) -> &'static str {
        match self {
            Kind::Original => "original",
            Kind::Sakerhetskopia => "säkerhetskopia",
            Kind::Arkivpaket => "arkivpaket",
            Kind::Papper => "papper",
        }
    }
}

/// Varför en plan inte kan läggas in.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PlanError {
    /// Filen är inte giltig JSON eller har fält som planen inte känner till.
    #[error("arkivplanen kan inte läsas: {0}")]
    Format(String),
    /// Planen har inga delar.
    #[error("arkivplanen måste ha minst en del")]
    NoParts,
    /// För många delar.
    #[error("arkivplanen har fler än {MAX_PARTS} delar")]
    TooManyParts,
    /// Ett obligatoriskt fält är tomt.
    #[error("{0} är tomt")]
    Empty(String),
    /// En text är för lång eller innehåller styrtecken.
    #[error("{0} är längre än {MAX_TEXT} tecken eller innehåller styrtecken")]
    BadText(String),
}

impl ArchivePlan {
    /// Läser och kontrollerar en plan ur JSON.
    pub fn from_json(text: &str) -> Result<ArchivePlan, PlanError> {
        let plan: ArchivePlan =
            serde_json::from_str(text).map_err(|e| PlanError::Format(e.to_string()))?;
        plan.validate()?;
        Ok(plan)
    }

    /// Planen som JSON (samma form som filen).
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Kontrollerar att planen är fullständig och att texterna går att visa.
    pub fn validate(&self) -> Result<(), PlanError> {
        required("ansvarig", &self.responsible)?;
        optional("lasbar_form", self.readable.as_deref())?;
        optional("kommentar", self.note.as_deref())?;
        if self.parts.is_empty() {
            return Err(PlanError::NoParts);
        }
        if self.parts.len() > MAX_PARTS {
            return Err(PlanError::TooManyParts);
        }
        for (i, p) in self.parts.iter().enumerate() {
            let n = i + 1;
            required(&format!("del {n}: vad"), &p.what)?;
            required(&format!("del {n}: plats"), &p.place)?;
            required(&format!("del {n}: land"), &p.country)?;
            for (field, value) in [
                ("anmald", &p.notified),
                ("hur_ofta", &p.frequency),
                ("sparas", &p.kept),
                ("kontroll", &p.check),
                ("sokvag", &p.path),
            ] {
                optional(&format!("del {n}: {field}"), value.as_deref())?;
            }
        }
        Ok(())
    }
}

impl Part {
    /// Ligger platsen i Sverige?
    pub fn in_sweden(&self) -> bool {
        let c = self.country.trim().to_lowercase();
        c == "sverige" || c == "se"
    }
}

fn required(field: &str, value: &str) -> Result<(), PlanError> {
    if value.trim().is_empty() {
        return Err(PlanError::Empty(field.into()));
    }
    optional(field, Some(value))
}

/// Radbrytningar och tabbar är tillåtna; andra styrtecken inte.
fn optional(field: &str, value: Option<&str>) -> Result<(), PlanError> {
    match value {
        Some(v)
            if v.chars().count() > MAX_TEXT
                || v.chars().any(|c| c.is_control() && c != '\n' && c != '\t') =>
        {
            Err(PlanError::BadText(field.into()))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(crate) mod tests {
    use super::*;

    /// En påhittad plan för ett påhittat företag.
    pub(crate) fn example() -> &'static str {
        r#"{
          "ansvarig": "Ekonomiansvarig",
          "delar": [
            {"vad": "Databasen och underlagen", "slag": "original",
             "plats": "/srv/bokis på kontorsservern", "kontroll": "bok verify varje natt"},
            {"vad": "Nattlig kopia", "slag": "sakerhetskopia", "plats": "/mnt/kopior",
             "hur_ofta": "varje natt 02.30", "sparas": "30 nätter", "atskild": true,
             "kontroll": "bok kopia kontrollera på senaste", "sokvag": "/mnt/kopior"},
            {"vad": "Arkivpaket per räkenskapsår", "slag": "arkivpaket",
             "plats": "USB-minne i kassaskåpet", "sparas": "hela bevarandetiden", "atskild": true}
          ],
          "lasbar_form": "bok insyn eller arkivpaketets HTML-sidor i en webbläsare"
        }"#
    }

    #[test]
    fn the_example_is_read() {
        let plan = ArchivePlan::from_json(example()).unwrap();
        assert_eq!(plan.parts.len(), 3);
        assert_eq!(plan.parts[1].kind, Kind::Sakerhetskopia);
        assert!(plan.parts[1].separate);
        assert_eq!(plan.parts[0].country, "Sverige");
        assert!(plan.parts[0].in_sweden());
        let again = ArchivePlan::from_json(&plan.to_json()).unwrap();
        assert_eq!(again, plan);
    }

    #[test]
    fn misspelled_fields_and_kinds_are_refused() {
        let typo = example().replace("\"sparas\": \"30 nätter\"", "\"sparad\": \"30 nätter\"");
        assert!(matches!(
            ArchivePlan::from_json(&typo),
            Err(PlanError::Format(_))
        ));
        let kind = example().replace("\"arkivpaket\"", "\"paket\"");
        assert!(matches!(
            ArchivePlan::from_json(&kind),
            Err(PlanError::Format(_))
        ));
    }

    #[test]
    fn empty_and_strange_text_is_refused() {
        let empty = r#"{"ansvarig": "X", "delar": []}"#;
        assert_eq!(ArchivePlan::from_json(empty), Err(PlanError::NoParts));
        let mut blank = ArchivePlan::from_json(example()).unwrap();
        blank.parts[1].place = " ".into();
        assert!(matches!(
            blank.validate(),
            Err(PlanError::Empty(f)) if f == "del 2: plats"
        ));
        let control = example().replace("Ekonomiansvarig", "Ekonomi\\u0007ansvarig");
        assert!(matches!(
            ArchivePlan::from_json(&control),
            Err(PlanError::BadText(_))
        ));
        let long = example().replace("Ekonomiansvarig", &"x".repeat(MAX_TEXT + 1));
        assert!(ArchivePlan::from_json(&long).is_err());
    }
}
