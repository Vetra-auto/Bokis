//! Spärrarna i databasen, som de faktiskt ser ut i schemat.
//!
//! Systemdokumentationen beskriver spärrarna utifrån det här, inte utifrån
//! vad koden är tänkt att göra, och testerna kontrollerar att varje tabell
//! som bara ska kunna växa har sina spärrar.

use crate::{Result, Store};

/// En trigger i schemat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guard {
    pub table: String,
    pub trigger: String,
    /// "INSERT", "UPDATE" eller "DELETE".
    pub event: String,
    /// Felmeddelandena triggern ger när den stoppar något.
    pub messages: Vec<String>,
}

/// Texterna i `RAISE(ABORT, '...')`.
fn abort_messages(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = sql;
    while let Some(i) = rest.find("RAISE(ABORT, '") {
        rest = &rest[i + "RAISE(ABORT, '".len()..];
        match rest.find("')") {
            Some(end) => {
                out.push(rest[..end].to_string());
                rest = &rest[end..];
            }
            None => break,
        }
    }
    out
}

/// Tabellerna som bara kan växa: varken ändras eller raderas. (Motpartsregistret
/// kan ändras men inte raderas; perioder och nummerserier ändras bara på
/// tillåtna sätt.)
pub const APPEND_ONLY_TABLES: [&str; 19] = [
    "handelser",
    "foretag",
    "nycklar",
    "rakenskapsar",
    "kontoplan",
    "regler",
    "underlag",
    "underlag_status",
    "verifikationer",
    "rader",
    "rattelser",
    "forslag",
    "beslut",
    "bankrader",
    "bankmatchningar",
    "matchningsforslag",
    "arenden",
    "arendesvar",
    "sakerhetsniva",
];

impl Store {
    /// Alla triggers i schemat, sorterade på tabell och namn.
    pub fn guards(&self) -> Result<Vec<Guard>> {
        let mut stmt = self.conn().prepare(
            "SELECT tbl_name, name, sql FROM sqlite_master WHERE type = 'trigger' ORDER BY tbl_name, name",
        )?;
        let rows: Vec<(String, String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows
            .into_iter()
            .map(|(table, trigger, sql)| {
                let upper = sql.to_uppercase();
                let event = ["INSERT", "UPDATE", "DELETE"]
                    .into_iter()
                    .find(|e| {
                        upper.contains(&format!("BEFORE {e}"))
                            || upper.contains(&format!("AFTER {e}"))
                    })
                    .unwrap_or("OKÄND")
                    .to_string();
                let messages = abort_messages(&sql);
                Guard {
                    table,
                    trigger,
                    event,
                    messages,
                }
            })
            .collect())
    }

    /// Tabeller i [`APPEND_ONLY_TABLES`] som saknar en spärr mot ändring
    /// eller radering. Tom lista om alla spärrar finns.
    pub fn missing_guards(&self) -> Result<Vec<String>> {
        let guards = self.guards()?;
        let mut missing = Vec::new();
        for table in APPEND_ONLY_TABLES {
            for event in ["UPDATE", "DELETE"] {
                if !guards.iter().any(|g| g.table == table && g.event == event) {
                    missing.push(format!("{table}: {event}"));
                }
            }
        }
        Ok(missing)
    }
}
