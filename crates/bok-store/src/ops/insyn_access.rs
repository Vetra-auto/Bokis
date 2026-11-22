//! Insyn per person: namngivna inloggningslänkar, återkallelse och varje
//! inloggning i behandlingshistoriken.
//!
//! Länkarna har ingen egen tabell. En länk är händelsen `insyn.lank.skapad`
//! (namn och sista giltighetstid), och händelsens nummer är länkens id. En
//! återkallelse är `insyn.lank.aterkallad` och en inloggning
//! `insyn.inloggning`. Historiken är därmed det som avgör om en länk gäller:
//! insynstjänsten frågar lagret vid varje inloggning och varje sida, och en
//! återkallelse gäller direkt. Själva hemligheten i länken sparas aldrig här,
//! bara dess SHA-256 i `insyn.json` bredvid databasen.

use chrono::{DateTime, Duration, Utc};
use rusqlite::{OptionalExtension, params};
use serde_json::json;

use super::require_human;
use crate::write::{Actor, EventDraft};
use crate::{Result, Store, StoreError, format_timestamp, parse_timestamp};

/// Händelsen när en insynslänk skapas.
pub const INSYN_LINK_CREATED: &str = "insyn.lank.skapad";
/// Händelsen när en insynslänk återkallas.
pub const INSYN_LINK_REVOKED: &str = "insyn.lank.aterkallad";
/// Händelsen när någon loggar in i insynen.
pub const INSYN_LOGIN: &str = "insyn.inloggning";
/// Längsta giltighetstid för en länk, i dagar.
pub const MAX_INSYN_LINK_DAYS: i64 = 366;

/// En namngiven insynslänk ur historiken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsynLink {
    /// Händelsenumret när länken skapades.
    pub id: i64,
    /// Vem länken är till, till exempel "Revisor X".
    pub name: String,
    /// När länken skapades.
    pub created: String,
    /// Vem som skapade den.
    pub created_by: String,
    /// Sista tidpunkten länken och dess sessioner gäller.
    pub valid_until: DateTime<Utc>,
    /// När länken återkallades, om den är det.
    pub revoked: Option<String>,
    /// Antal inloggningar med länken.
    pub logins: usize,
    /// Den senaste inloggningen.
    pub last_login: Option<String>,
}

impl InsynLink {
    /// Gäller länken vid `now`: inte återkallad och inte utgången.
    pub fn active_at(&self, now: DateTime<Utc>) -> bool {
        self.revoked.is_none() && now < self.valid_until
    }
}

fn check_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 100 || name.chars().any(char::is_control) {
        return Err(StoreError::Invalid(
            "insynslänken måste ha ett namn på högst 100 tecken, till exempel \"Revisor X\"".into(),
        ));
    }
    Ok(name.to_string())
}

impl Store {
    /// Skapar en namngiven insynslänk som gäller till och med `valid_until`.
    /// Ger länkens id. Bara en människa skapar länkar.
    pub fn create_insyn_link(
        &mut self,
        actor: &Actor,
        name: &str,
        valid_until: DateTime<Utc>,
    ) -> Result<i64> {
        require_human(actor, "skapa insynslänkar")?;
        let name = check_name(name)?;
        let now = self.now();
        if valid_until <= now {
            return Err(StoreError::Invalid(
                "insynslänken måste gälla en tid framåt".into(),
            ));
        }
        if valid_until > now + Duration::days(MAX_INSYN_LINK_DAYS) {
            return Err(StoreError::Invalid(format!(
                "en insynslänk får gälla högst {MAX_INSYN_LINK_DAYS} dagar"
            )));
        }
        self.write(actor, |ctx| {
            let event =
                EventDraft::new(INSYN_LINK_CREATED, format!("insynslank:{}", ctx.event_seq))
                    .new_values(json!({
                        "namn": name,
                        "giltig_till": format_timestamp(valid_until),
                    }));
            Ok((ctx.event_seq, event))
        })
    }

    /// Återkallar en insynslänk. Dess sessioner slutar gälla direkt.
    pub fn revoke_insyn_link(&mut self, actor: &Actor, id: i64) -> Result<InsynLink> {
        require_human(actor, "återkalla insynslänkar")?;
        let link = self
            .insyn_link(id)?
            .ok_or_else(|| StoreError::missing("insynslänk", id))?;
        if link.revoked.is_some() {
            return Err(StoreError::Invalid(format!(
                "insynslänk {id} ({}) är redan återkallad",
                link.name
            )));
        }
        let name = link.name.clone();
        self.write(actor, |_ctx| {
            let event = EventDraft::new(INSYN_LINK_REVOKED, format!("insynslank:{id}"))
                .new_values(json!({ "namn": name }));
            Ok(((), event))
        })?;
        self.insyn_link(id)?
            .ok_or_else(|| StoreError::missing("insynslänk", id))
    }

    /// Loggar en inloggning med länken `id`, med sessionens sista tid.
    /// Vägrar om länken är återkallad eller har gått ut, så att det aldrig
    /// blir en inloggning utan händelse.
    pub fn record_insyn_login(&mut self, id: i64, session_until: DateTime<Utc>) -> Result<()> {
        let link = self
            .insyn_link(id)?
            .ok_or_else(|| StoreError::missing("insynslänk", id))?;
        if !link.active_at(self.now()) {
            return Err(StoreError::Invalid(format!(
                "insynslänk {id} ({}) är återkallad eller har gått ut",
                link.name
            )));
        }
        let actor = Actor::insyn(link.name.clone());
        self.write(&actor, |_ctx| {
            let event =
                EventDraft::new(INSYN_LOGIN, format!("insynslank:{id}")).new_values(json!({
                    "namn": link.name,
                    "lank": id,
                    "session_till": format_timestamp(session_until),
                }));
            Ok(((), event))
        })
    }

    /// Loggar en inloggning med startlänken som `bok insyn` visar på
    /// datorn med bokföringen.
    pub fn record_local_insyn_login(&mut self, user: &str) -> Result<()> {
        let actor = Actor::insyn(check_name(user)?);
        let name = actor.name.clone();
        self.write(&actor, |_ctx| {
            let event = EventDraft::new(INSYN_LOGIN, "insyn:startlank").new_values(json!({
                "namn": name,
                "via": "startlänken på datorn med bokföringen",
            }));
            Ok(((), event))
        })
    }

    /// En insynslänk, om den finns.
    pub fn insyn_link(&self, id: i64) -> Result<Option<InsynLink>> {
        let created: Option<(String, String, Option<String>)> = self
            .conn()
            .query_row(
                "SELECT tid, aktor, nya FROM handelser WHERE seq = ?1 AND handling = ?2",
                params![id, INSYN_LINK_CREATED],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((time, actor, values)) = created else {
            return Ok(None);
        };
        let values: serde_json::Value = serde_json::from_str(values.as_deref().unwrap_or("{}"))?;
        let name = values["namn"].as_str().unwrap_or_default().to_string();
        let valid_until = parse_timestamp(values["giltig_till"].as_str().unwrap_or_default())?;
        let object = format!("insynslank:{id}");
        let revoked: Option<String> = self
            .conn()
            .query_row(
                "SELECT tid FROM handelser WHERE handling = ?1 AND objekt = ?2 ORDER BY seq LIMIT 1",
                params![INSYN_LINK_REVOKED, object],
                |r| r.get(0),
            )
            .optional()?;
        let (logins, last_login): (i64, Option<String>) = self.conn().query_row(
            "SELECT COUNT(*), MAX(tid) FROM handelser WHERE handling = ?1 AND objekt = ?2",
            params![INSYN_LOGIN, object],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(Some(InsynLink {
            id,
            name,
            created: time,
            created_by: actor,
            valid_until,
            revoked,
            logins: usize::try_from(logins).unwrap_or(0),
            last_login,
        }))
    }

    /// Alla insynslänkar, äldst först.
    pub fn insyn_links(&self) -> Result<Vec<InsynLink>> {
        let ids: Vec<i64> = self
            .conn()
            .prepare("SELECT seq FROM handelser WHERE handling = ?1 ORDER BY seq")?
            .query_map(params![INSYN_LINK_CREATED], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(link) = self.insyn_link(id)? {
                out.push(link);
            }
        }
        Ok(out)
    }
}
