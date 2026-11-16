//! Inkomstdeklaration 2 för aktiebolag: huvudblanketten INK2,
//! räkenskapsschemat INK2R och de skattemässiga justeringarna INK2S, räknade
//! ur saldobalansen och skrivna som SRU-filer.
//!
//! Inget här är hårdkodat om vilka konton som hör till vilka fält. Det är
//! data:
//!
//! - fältförteckningen per beskattningsperiod (`data/ink2-2026p4.txt`),
//!   avskriven ur Skatteverkets tabeller,
//! - kopplingen konto → INK2R-fält (`docs/ink2r-bas.txt`), efter BAS
//!   kopplingstabell, och
//! - kopplingen konto → INK2S-fält (`docs/ink2s-bas.txt`) för bokförda
//!   kostnader som inte får dras av och intäkter som inte ska tas upp.
//!
//! Ett konto som har en egen SRU-kod i kontoplanen följer den.
//!
//! Modulerna:
//! - [`catalog`] – fältförteckningen och blankettnamnen (`INK2R-2026P4`).
//! - [`mapping`] – kontokopplingen.
//! - [`ink2r`]: räkenskapsschemat ur saldobalansen, med balanskontroll.
//! - [`ink2s`]: justeringarna, överskott eller underskott, och huvudblanketten.
//! - [`tax`] – bolagsskatten, 20,6 % av överskottet avrundat nedåt till tiotal kronor.
//! - [`declaration`] – hela deklarationen och dess kontroller.
//! - [`sru`] – deklarationen som `INFO.SRU` och `BLANKETTER.SRU`.
//! - [`summary`] – en läsbar sammanställning.

pub mod catalog;
pub mod declaration;
pub mod ink2r;
pub mod ink2s;
pub mod mapping;
pub mod sru;
pub mod summary;
pub mod tax;

#[cfg(test)]
mod testdata;

pub use catalog::{Catalog, Field, FormKind};
pub use declaration::{AccountBalance, Declaration, Input, ManualValue};
pub use mapping::AccountMap;
pub use tax::{CorporateTax, corporate_tax};

use thiserror::Error;

/// Fel när deklarationen räknas fram eller skrivs.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DeclError {
    /// Fältförteckningen eller en kopplingsfil går inte att läsa.
    #[error("{file} rad {line}: {message}")]
    Data {
        /// Filen.
        file: String,
        /// Radnumret, från 1.
        line: usize,
        /// Vad som är fel.
        message: String,
    },
    /// Det finns ingen fältförteckning för perioden.
    #[error("Bokis har ingen fältförteckning för beskattningsperiod {0}; bara {1} finns")]
    NoCatalog(String, String),
    /// Konton med saldo som inte hör till något fält.
    #[error("konton med saldo saknar fält i INK2R: {}; lägg till dem i kopplingen eller ge dem en SRU-kod", .0.join(", "))]
    Unmapped(Vec<String>),
    /// Ett fält som inte finns i fältförteckningen.
    #[error("{0} är inget fält i INK2, INK2R eller INK2S för perioden")]
    UnknownField(String),
    /// Ett fältnummer som har två koder (plus och minus) måste anges med kod.
    #[error("fält {0} har både en plus- och en minuskod; ange koden ({1})")]
    AmbiguousField(String, String),
    /// Ett fält som räknas fram och inte kan anges för hand.
    #[error("fält {0} räknas fram ur bokföringen och kan inte anges för hand")]
    Computed(String),
    /// En kontroll som inte går igenom.
    #[error("{0}")]
    Check(String),
    /// Ett fel i SRU-filen.
    #[error(transparent)]
    Sru(#[from] bok_sru::SruError),
}
