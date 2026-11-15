//! Säkerhetsnivån: vilka signaturer bokföringen godtar (se
//! [`bok_core::security`]). Varje beslut är en signerad systemändring, och
//! [`super::verify_registered`] prövar varje ny signatur mot nivån som gäller.

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;

use bok_core::canonical::{CanonicalWriter, sha256_hex};
use bok_core::security::SecurityLevel;
use bok_core::signature::{NS_SYSTEM, Signer, sk_flags};

use super::{require_human, verify_registered};
use crate::model::SecurityDecision;
use crate::write::{Actor, EventDraft};
use crate::{Result, Store, StoreError};

/// Händelsen för ett beslut om säkerhetsnivån.
pub const SECURITY_LEVEL_DECIDED: &str = "sakerhetsniva.beslutad";

/// Hashen som signeras för ett beslut om säkerhetsnivån.
pub(crate) fn security_level_hash(
    level: SecurityLevel,
    previous: Option<SecurityLevel>,
    decided: &str,
) -> String {
    let mut w = CanonicalWriter::new("bokis-sakerhetsniva-v1");
    w.field("niva", level.as_str());
    w.opt("tidigare", previous.map(SecurityLevel::as_str));
    w.field("beslutad", decided);
    sha256_hex(&w.finish())
}

fn parse_level(s: &str) -> rusqlite::Result<SecurityLevel> {
    s.parse().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

/// Nivån som gäller för nästa händelse: det senaste beslutet, eller
/// mjukvara när bokföringen saknar beslut (skapad före schema 3).
pub(crate) fn level_now(conn: &Connection) -> Result<SecurityLevel> {
    Ok(conn
        .query_row(
            "SELECT niva FROM sakerhetsniva ORDER BY handelse DESC LIMIT 1",
            [],
            |r| parse_level(&r.get::<_, String>(0)?),
        )
        .optional()?
        .unwrap_or(SecurityLevel::Software))
}

/// Alla beslut, äldst först.
pub(crate) fn decisions(conn: &Connection) -> Result<Vec<SecurityDecision>> {
    let mut stmt = conn.prepare(
        "SELECT s.niva, s.tidigare, s.beslutad, s.handelse, h.aktor
         FROM sakerhetsniva s LEFT JOIN handelser h ON h.seq = s.handelse
         ORDER BY s.handelse",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(SecurityDecision {
                level: parse_level(&r.get::<_, String>(0)?)?,
                previous: r
                    .get::<_, Option<String>>(1)?
                    .as_deref()
                    .map(parse_level)
                    .transpose()?,
                decided: r.get(2)?,
                event: r.get(3)?,
                actor: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

impl Store {
    /// Nivån som gäller nu.
    pub fn security_level(&self) -> Result<SecurityLevel> {
        level_now(self.conn())
    }

    /// Alla beslut om säkerhetsnivån, äldst först. Tom för en bokföring som
    /// skapades före nivåerna; den godtar alla registrerade nycklar.
    pub fn security_decisions(&self) -> Result<Vec<SecurityDecision>> {
        decisions(self.conn())
    }

    /// Beslutar en ny säkerhetsnivå. Signaturen måste klara både nivån som
    /// gäller och den nya: den som höjer visar att den har en nyckel för den
    /// nya nivån, och den som sänker att den har en nyckel för den gamla. En
    /// agent eller ett intrång med en nyckelfil kan alltså inte sänka nivån.
    pub fn set_security_level(
        &mut self,
        actor: &Actor,
        level: SecurityLevel,
        signer: &dyn Signer,
    ) -> Result<SecurityDecision> {
        require_human(actor, "ändra säkerhetsnivån")?;
        let previous = self.security_decisions()?.last().map(|d| d.level);
        if previous == Some(level) {
            return Err(StoreError::Invalid(format!(
                "säkerhetsnivån är redan {}",
                level.title()
            )));
        }
        self.decide_security_level(actor, level, previous, signer)
    }

    pub(crate) fn decide_security_level(
        &mut self,
        actor: &Actor,
        level: SecurityLevel,
        previous: Option<SecurityLevel>,
        signer: &dyn Signer,
    ) -> Result<SecurityDecision> {
        let seq = self.write(actor, |ctx| {
            let hash = security_level_hash(level, previous, &ctx.now_str);
            let sig = signer.sign(NS_SYSTEM, hash.as_bytes())?;
            // Mot nivån som gäller, så att en sänkning kräver den gamla
            // nivåns nyckel ...
            let key = verify_registered(ctx.tx, NS_SYSTEM, hash.as_bytes(), &sig)?;
            // ... och mot den nya, så att en höjning kräver den nya nivåns.
            level.check(key.hardware, sk_flags(&sig)?)?;
            ctx.tx.execute(
                "INSERT INTO sakerhetsniva (niva, tidigare, beslutad, handelse) VALUES (?1, ?2, ?3, ?4)",
                params![
                    level.as_str(),
                    previous.map(SecurityLevel::as_str),
                    ctx.now_str,
                    ctx.event_seq
                ],
            )?;
            let event = EventDraft::new(SECURITY_LEVEL_DECIDED, "sakerhetsniva")
                .new_values(json!({
                    "niva": level.as_str(),
                    "tidigare": previous.map(SecurityLevel::as_str),
                    "beslutshash": hash,
                    "nyckel": key.fingerprint,
                }))
                .signed(sig);
            Ok((ctx.event_seq, event))
        })?;
        self.security_decisions()?
            .into_iter()
            .find(|d| d.event == seq)
            .ok_or_else(|| StoreError::missing("beslut om säkerhetsnivån", seq))
    }
}
