//! Periodövergångar. Stängning och låsning är signerade beslut.

use rusqlite::params;
use serde_json::json;

use bok_core::PeriodStatus;
use bok_core::decision::period_decision_hash;
use bok_core::period::check_transition;
use bok_core::signature::{NS_PERIOD, Signer};

use super::{period_by_id, verify_registered};
use crate::model::*;
use crate::write::{Actor, EventDraft};
use crate::{Result, Store, StoreError};

pub(crate) fn transition_action(to: PeriodStatus) -> &'static str {
    match to {
        PeriodStatus::Open => "period.oppnad",
        PeriodStatus::UnderReview => "period.under_avstamning",
        PeriodStatus::Closed => "period.stangd",
        PeriodStatus::Locked => "period.last",
    }
}

impl Store {
    /// Ändrar en periods status. Stängd och låst kan aldrig öppnas igen.
    /// Stängning och låsning kräver `signer`.
    pub fn transition_period(
        &mut self,
        actor: &Actor,
        period_id: i64,
        to: PeriodStatus,
        signer: Option<&dyn Signer>,
    ) -> Result<Transition> {
        self.write(actor, |ctx| {
            let period = period_by_id(ctx.tx, period_id)?;
            let from = period.status;
            check_transition(from, to, actor.kind)?;
            let mut new_values = json!({
                "status": to.as_db_str(),
                "period": period.label(),
                "start": period.start.to_string(),
                "slut": period.end.to_string(),
            });
            // Banken styr fullständigheten: en period med bankrader som inte är
            // avstämda kan varken stängas eller låsas.
            if matches!(to, PeriodStatus::Closed | PeriodStatus::Locked) {
                let count = super::bank::unreconciled_in(ctx.tx, period.start, period.end)?;
                if count > 0 {
                    return Err(StoreError::UnreconciledBankRows { period: period.label(), count });
                }
            }
            let mut signature = None;
            if to.requires_signature() {
                let signer = signer.ok_or_else(|| {
                    StoreError::Invalid("att stänga eller låsa en period måste signeras med nyckeln".into())
                })?;
                let hash = period_decision_hash(period.start, period.end, from, to, &ctx.now_str);
                let sig = signer.sign(NS_PERIOD, hash.as_bytes())?;
                let key = verify_registered(ctx.tx, NS_PERIOD, hash.as_bytes(), &sig)?;
                new_values["beslutshash"] = json!(hash);
                new_values["nyckel"] = json!(key.fingerprint);
                signature = Some(sig);
            }
            let changed = ctx.tx.execute(
                "UPDATE perioder SET status = ?1 WHERE id = ?2 AND status = ?3",
                params![to.as_db_str(), period_id, from.as_db_str()],
            )?;
            if changed != 1 {
                return Err(StoreError::Invalid(format!("perioden {} ändrades samtidigt", period.label())));
            }
            let pending: i64 = ctx.tx.query_row(
                "SELECT COUNT(*) FROM forslag f
                 WHERE f.datum BETWEEN ?1 AND ?2 AND NOT EXISTS (SELECT 1 FROM beslut b WHERE b.forslag_id = f.id)",
                params![period.start.to_string(), period.end.to_string()],
                |r| r.get(0),
            )?;
            let mut event = EventDraft::new(transition_action(to), format!("period:{period_id}"))
                .old(json!({ "status": from.as_db_str() }))
                .new_values(new_values);
            if let Some(sig) = signature {
                event = event.signed(sig);
            }
            Ok((
                Transition { period: period.label(), from, to, pending_proposals: pending as usize },
                event,
            ))
        })
    }
}
