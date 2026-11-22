//! Arkivplanen i behandlingshistoriken.
//!
//! Planen har ingen egen tabell. Varje version blir en händelse
//! (`arkivplan.satt`) med hela planen som nya värden och den förra versionen
//! som gamla, så hashkedjan visar vilken plan som gällde när. Den gällande
//! planen är den senaste händelsen. Bara en människa lägger in en plan.

use rusqlite::{OptionalExtension, params};

use bok_arkivplan::ArchivePlan;

use super::require_human;
use crate::write::{Actor, EventDraft};
use crate::{Result, Store, StoreError};

/// Händelsen när en arkivplan läggs in.
pub const ARCHIVE_PLAN_SET: &str = "arkivplan.satt";

/// Den gällande arkivplanen och när den lades in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivePlanRecord {
    /// Planen.
    pub plan: ArchivePlan,
    /// När den lades in (händelsens tid).
    pub time: String,
    /// Vem som lade in den.
    pub actor: String,
    /// Händelsens nummer i historiken.
    pub event_seq: i64,
}

impl Store {
    /// Lägger in en ny version av arkivplanen. Vägrar en plan som är
    /// densamma som den gällande.
    pub fn set_archive_plan(&mut self, actor: &Actor, plan: &ArchivePlan) -> Result<i64> {
        require_human(actor, "lägga in arkivplanen")?;
        plan.validate()
            .map_err(|e| StoreError::Invalid(e.to_string()))?;
        let old = self.archive_plan()?;
        if old.as_ref().is_some_and(|o| &o.plan == plan) {
            return Err(StoreError::Invalid(
                "arkivplanen är densamma som den som redan gäller".into(),
            ));
        }
        let new = serde_json::to_value(plan)?;
        self.write(actor, |ctx| {
            let mut event = EventDraft::new(ARCHIVE_PLAN_SET, "arkivplan").new_values(new);
            if let Some(o) = &old {
                event = event.old(serde_json::to_value(&o.plan)?);
            }
            Ok((ctx.event_seq, event))
        })
    }

    /// Den gällande arkivplanen, om någon är inlagd.
    pub fn archive_plan(&self) -> Result<Option<ArchivePlanRecord>> {
        let row: Option<(i64, String, String, Option<String>)> = self
            .conn()
            .query_row(
                "SELECT seq, tid, aktor, nya FROM handelser WHERE handling = ?1
                 ORDER BY seq DESC LIMIT 1",
                params![ARCHIVE_PLAN_SET],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let Some((event_seq, time, actor, values)) = row else {
            return Ok(None);
        };
        let plan = ArchivePlan::from_json(values.as_deref().unwrap_or(""))
            .map_err(|e| StoreError::Invalid(format!("arkivplanen i händelse {event_seq}: {e}")))?;
        Ok(Some(ArchivePlanRecord {
            plan,
            time,
            actor,
            event_seq,
        }))
    }
}
