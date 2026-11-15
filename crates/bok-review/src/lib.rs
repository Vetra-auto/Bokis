//! Granskningsvy för Bokis.
//!
//! En lokal webbsida där människan ser agentens förslag bredvid underlaget,
//! med kontering, varningar, motivering och härkomst, och godkänner eller
//! avvisar. Ett godkännande bokför; implementationen mot lagret signerar med
//! granskarens hårdvarunyckel, så det kräver fysisk beröring av nyckeln.
//!
//! - [`backend::ReviewBackend`] är gränssnittet mot lagret.
//! - [`router::Router`] innehåller rutterna och skydden.
//! - [`server`] är ett minimalt HTTP-lager på 127.0.0.1.
//! - [`html`] bygger sidorna; allt dynamiskt innehåll escapas.
//! - `store::StoreReviewBackend` och `signer::SshKeygenSigner` (funktionen
//!   `store`) arbetar mot Bokis databas och signerar med hårdvarunyckeln.

pub mod backend;
pub mod html;
pub mod router;
pub mod server;
#[cfg(feature = "store")]
pub mod signer;
#[cfg(feature = "store")]
pub mod store;
