//! SRU-filer för Skatteverkets filöverföring.
//!
//! En leverans är två filer: `INFO.SRU`, som beskriver leveransen och
//! uppgiftslämnaren, och `BLANKETTER.SRU`, med en eller flera blanketter
//! (`#BLANKETT` … `#BLANKETTSLUT`) och `#FIL_SLUT` sist. Formatet följer
//! Skatteverkets "Teknisk information om filöverföring", som ersatte
//! SKV 269 den 1 januari 2024:
//!
//! - teckenkodning ISO 8859-1 och radslut `<LF>`,
//! - en post per rad, `#POSTNAMN värde …`, med mellanslag mellan fälten,
//! - organisationsnummer med tolv siffror och sekelsiffror (16 för juridiska
//!   personer), utan bindestreck,
//! - datum `ÅÅÅÅMMDD`, tid `TTMMSS`,
//! - belopp i hela kronor, utan tecken när de är positiva och med `-` när de
//!   är negativa,
//! - en post utan värde lämnas inte; en blankett utan uppgifter lämnas inte,
//! - `#` får inte förekomma i en sträng.
//!
//! Crate:n vet ingenting om vilka fält en viss blankett har; det är
//! `bok-deklaration`s sak. Den skriver, läser och kontrollerar formen.
//!
//! - [`text`] – kodning till ISO 8859-1 och kontroll av strängar.
//! - [`orgnr`] – organisationsnumret i SRU-form.
//! - [`info`] – `INFO.SRU`.
//! - [`form`] – blanketterna och `BLANKETTER.SRU`.
//! - [`read`] – läser tillbaka båda filerna, för kontroll och tester.

pub mod form;
pub mod info;
pub mod orgnr;
pub mod read;
pub mod text;

pub use form::{Form, Value, write_forms};
pub use info::{MediaProvider, write_info};
pub use orgnr::sru_orgnr;
pub use read::{ReadForm, read_forms, read_info};

use thiserror::Error;

/// Filnamnet för blanketterna, som `INFO.SRU` pekar ut.
pub const FORMS_FILE: &str = "BLANKETTER.SRU";
/// Filnamnet för leveransens beskrivning.
pub const INFO_FILE: &str = "INFO.SRU";

/// Fel i en SRU-fil eller i uppgifterna till den.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SruError {
    /// Ett tecken som inte finns i ISO 8859-1, eller ett styrtecken.
    #[error("{field}: tecknet {ch:?} kan inte skrivas i en SRU-fil (ISO 8859-1, inga styrtecken)")]
    BadChar {
        /// Posten eller fältet.
        field: String,
        /// Tecknet.
        ch: char,
    },
    /// `#` i en sträng.
    #[error("{0}: tecknet # får inte förekomma i en sträng")]
    Hash(String),
    /// En obligatorisk uppgift saknas eller är tom.
    #[error("{0} saknas eller är tom")]
    Missing(String),
    /// Organisationsnumret går inte att tolka.
    #[error("{0} är inget organisationsnummer (skriv 559000-0000)")]
    OrgNumber(String),
    /// Blankettens namn har otillåtna tecken.
    #[error("blankettnamnet {0} får bara ha versaler, siffror och bindestreck")]
    FormName(String),
    /// Samma fältkod två gånger i en blankett.
    #[error("{form}: fältkoden {code} förekommer mer än en gång")]
    Duplicate {
        /// Blanketten.
        form: String,
        /// Fältkoden.
        code: String,
    },
    /// En fältkod som inte är en kod.
    #[error("{0} är ingen fältkod")]
    BadCode(String),
    /// Filen går inte att läsa.
    #[error("rad {line}: {message}")]
    Syntax {
        /// Radnumret, från 1.
        line: usize,
        /// Vad som är fel.
        message: String,
    },
}
