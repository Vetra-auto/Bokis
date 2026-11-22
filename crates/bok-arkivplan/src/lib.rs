//! Arkivplanen i Bokis, som data i stället för text i programmet.
//!
//! Bokföringsnämndens allmänna råd (BFNAR 2013:2) säger att ett företag ska
//! ha en arkivplan när det behövs för att överblicka den arkiverade
//! räkenskapsinformationen: vad som arkiverats och var det förvaras, och vid
//! behov hur arkivet är uppbyggt (punkt 8.3). Planen ingår i
//! systemdokumentationen (punkt 9.2 d). Elektronisk räkenskapsinformation ska
//! säkerhetskopieras, och kopian ska förvaras åtskild från originalet (punkt
//! 8.2). Räkenskapsinformationen bevaras till och med det sjunde året efter
//! utgången av det kalenderår då räkenskapsåret avslutades, i Sverige
//! (bokföringslagen 7 kap. 2 §), eller i ett annat EU-land om platsen anmäls
//! till Skatteverket (7 kap. 3 a §).
//!
//! Varje företag beskriver sin egen plan i en JSON-fil (se
//! `docs/arkivplan-exempel.json`). `bok arkivplan satt` lägger in den i
//! bokföringens behandlingshistorik, och systemdokumentationen och insynen
//! visar den gällande planen tillsammans med det som Bokis själv kan räkna ut:
//! hur länge varje räkenskapsår ska bevaras och när kopiorna senast togs.
//!
//! - [`plan`] – planens delar och kontrollen av att filen är fullständig.
//! - [`review`] – iakttagelser mot BFL och BFNAR, och bevarandetiden.

pub mod plan;
pub mod review;

pub use plan::{ArchivePlan, Kind, Part, PlanError};
pub use review::{Observation, observations, retained_through};
