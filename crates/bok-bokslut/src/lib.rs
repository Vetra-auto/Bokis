//! Bokslutsposterna i Bokis, som förslag.
//!
//! Ingenting här bokför. Varje funktion räknar fram verifikationsrader och
//! lägger dem som ett förslag i serie B (bokslut), daterat räkenskapsårets
//! sista dag eller avgångsdagen, som en människa sedan granskar och
//! signerar med `bok godkann`. Samma förslag läggs inte två gånger:
//! idempotensnyckeln bygger på beloppen.
//!
//! - [`accrual`] – periodisering av förutbetalda och upplupna kostnader och
//!   intäkter, med K2:s beloppsgräns (ren logik).
//! - [`depreciation`] – årets avskrivningar ur inventarieregistret, och
//!   avstämningen av registret mot huvudboken.
//! - [`disposal`] – försäljning och utrangering av ett inventarium.
//! - [`tax`] – bolagsskatten ur deklarationens överskott.
//! - [`input`] – deklarationens indata ur lagret.
//! - [`proposal`] – det gemensamma för förslagen.

pub mod accrual;
pub mod depreciation;
pub mod disposal;
pub mod input;
pub mod proposal;
pub mod tax;

use thiserror::Error;

/// Fel när ett bokslutsförslag tas fram.
#[derive(Debug, Error)]
pub enum ClosingError {
    /// Från lagret.
    #[error(transparent)]
    Store(#[from] bok_store::StoreError),
    /// Från deklarationen.
    #[error(transparent)]
    Declaration(#[from] bok_deklaration::DeclError),
    /// Från inventarieregistret.
    #[error(transparent)]
    Asset(#[from] bok_inventarier::AssetError),
    /// Något som inte går att föreslå.
    #[error("{0}")]
    Invalid(String),
}

/// Resultat för bokslutsfunktionerna.
pub type Result<T> = std::result::Result<T, ClosingError>;
