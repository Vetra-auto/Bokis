//! MCP-server för Bokis.
//!
//! AI-agenten ansluter med en MCP-klient (Claude Code, Qwen Code eller en
//! annan) och får verktyg för att läsa underlag, kontoplan, regler och
//! tidigare konteringar, och för att lägga förslag, frågor och flaggor.
//! Bokföring, periodstängning och regeländringar finns inte bland verktygen:
//! de kräver granskarens signatur och görs med `bok`.
//!
//! - [`registry`] definierar alla operationer med roll och risk.
//! - [`server`] talar MCP över stdio och exponerar bara agentens operationer.
//! - [`backend::AgentBackend`] är gränssnittet mot lagret.
//! - [`validation`] kontrollerar förslag innan de sparas.
//! - [`preview`] skalar ner kvittofoton som skickas till agenten; originalet rörs inte.
//! - [`memory::MemoryBackend`] är ett lager i minnet för tester och demo.
//! - `store::StoreBackend` (funktionen `store`) arbetar mot Bokis databas.

pub mod backend;
pub mod memory;
pub mod preview;
pub mod registry;
pub mod server;
#[cfg(feature = "store")]
pub mod store;
pub mod validation;

/// SHA-256 som hexsträng.
pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(data))
}
