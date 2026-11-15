//! Regler som data: konteringsregler, momsregler och liknande som agenten
//! läser med `hamta_regler`. En regel ändras bara genom en ny version, och
//! varje version är en signerad systemändring, som kontoplanen.

use chrono::NaiveDate;
use rusqlite::params;
use serde_json::json;

use bok_core::canonical::{CanonicalWriter, sha256_hex};
use bok_core::signature::{NS_SYSTEM, Signer};

use super::{require_human, verify_registered};
use crate::model::*;
use crate::write::{Actor, EventDraft};
use crate::{Result, Store, StoreError};

/// Hashen som signeras för en regelversion.
pub(crate) fn rule_hash(
    name: &str,
    version: i64,
    valid_from: NaiveDate,
    valid_to: Option<NaiveDate>,
    content: &str,
) -> String {
    let mut w = CanonicalWriter::new("bokis-regel-v1");
    w.field("namn", name);
    w.int("version", version);
    w.field("giltig_fran", &valid_from.to_string());
    w.opt("giltig_till", valid_to.map(|d| d.to_string()).as_deref());
    w.field("innehall", content);
    sha256_hex(&w.finish())
}

impl Store {
    /// Lägger en ny version av regeln `name` (version 1 om den är ny).
    /// Innehållet är JSON; ren text sparas som `{"text": "…"}`. Ändringen
    /// signeras med en registrerad nyckel.
    pub fn add_rule(
        &mut self,
        actor: &Actor,
        name: &str,
        valid_from: NaiveDate,
        valid_to: Option<NaiveDate>,
        content: &str,
        signer: &dyn Signer,
    ) -> Result<RuleRow> {
        require_human(actor, "ändra regler")?;
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 100 {
            return Err(StoreError::Invalid(
                "en regel måste ha ett namn på högst 100 tecken".into(),
            ));
        }
        let content = content.trim();
        if content.is_empty() {
            return Err(StoreError::Invalid("regeln saknar innehåll".into()));
        }
        if let Some(to) = valid_to
            && to < valid_from
        {
            return Err(StoreError::Invalid(format!(
                "regeln slutar gälla {to}, före den börjar gälla {valid_from}"
            )));
        }
        let content = match serde_json::from_str::<serde_json::Value>(content) {
            Ok(v) if v.is_object() => v.to_string(),
            _ => json!({ "text": content }).to_string(),
        };
        self.write(actor, |ctx| {
            let version: i64 = ctx.tx.query_row(
                "SELECT COALESCE(MAX(version), 0) + 1 FROM regler WHERE namn = ?1",
                params![name],
                |r| r.get(0),
            )?;
            let hash = rule_hash(name, version, valid_from, valid_to, &content);
            let sig = signer.sign(NS_SYSTEM, hash.as_bytes())?;
            let key = verify_registered(ctx.tx, NS_SYSTEM, hash.as_bytes(), &sig)?;
            ctx.tx.execute(
                "INSERT INTO regler (namn, version, giltig_fran, giltig_till, innehall, andring)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    name,
                    version,
                    valid_from.to_string(),
                    valid_to.map(|d| d.to_string()),
                    content,
                    ctx.event_seq
                ],
            )?;
            let event = EventDraft::new("regel.andrad", format!("regel:{name}"))
                .new_values(json!({
                    "namn": name,
                    "version": version,
                    "giltig_fran": valid_from.to_string(),
                    "giltig_till": valid_to.map(|d| d.to_string()),
                    "beslutshash": hash,
                    "nyckel": key.fingerprint,
                }))
                .signed(sig);
            Ok((
                RuleRow {
                    name: name.to_string(),
                    version,
                    valid_from,
                    valid_to,
                    content: content.clone(),
                },
                event,
            ))
        })
    }

    /// Alla versioner av alla regler, sorterade på namn och version.
    pub fn all_rules(&self) -> Result<Vec<RuleRow>> {
        let mut stmt = self.conn().prepare(
            "SELECT namn, version, giltig_fran, giltig_till, innehall FROM regler ORDER BY namn, version",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(RuleRow {
                    name: r.get(0)?,
                    version: r.get(1)?,
                    valid_from: crate::parse_date(&r.get::<_, String>(2)?)?,
                    valid_to: r
                        .get::<_, Option<String>>(3)?
                        .map(|d| crate::parse_date(&d))
                        .transpose()?,
                    content: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }
}
