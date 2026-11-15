//! Ärenden: frågor och flaggor till granskaren (schema version 2).
//!
//! Vem som helst, även en agent, kan öppna ett ärende. Ärendet ändrar inget i
//! bokföringen. Bara en människa besvarar det, och bara svaret på en flagga
//! kan ändra ett underlags status till dubblett eller ej räkenskapsinformation.
//! Ett bokfört underlag kan inte få en sådan status.

use rusqlite::{OptionalExtension, Row, params};
use serde_json::json;

use super::require_human;
use crate::model::*;
use crate::write::{Actor, EventDraft};
use crate::{Result, Store, StoreError};

const CASE_COLUMNS: &str =
    "a.id, a.typ, a.underlag_id, a.orsak, a.text, a.kalla, a.aktor, a.modell, a.sakerhet,
     a.skapad, s.svar, s.status, s.aktor, s.tid";

fn case_from(r: &Row<'_>) -> rusqlite::Result<CaseRow> {
    let kind: String = r.get(1)?;
    let reason: Option<String> = r.get(3)?;
    let answer: Option<String> = r.get(10)?;
    Ok(CaseRow {
        id: r.get(0)?,
        kind: if kind == "flagga" {
            CaseKind::Flag {
                reason: reason.unwrap_or_default(),
            }
        } else {
            CaseKind::Question
        },
        attachment_id: r.get(2)?,
        text: r.get(4)?,
        source: r.get(5)?,
        actor: r.get(6)?,
        model: r.get(7)?,
        confidence: r.get(8)?,
        created_at: r.get(9)?,
        answer: match answer {
            None => None,
            Some(text) => Some(CaseAnswer {
                text,
                status: r.get(11)?,
                actor: r.get(12)?,
                time: r.get(13)?,
            }),
        },
    })
}

impl Store {
    /// Öppnar ett ärende. En flagga måste gälla ett underlag och ha en av
    /// [`FLAG_REASONS`].
    pub fn open_case(&mut self, actor: &Actor, c: &NewCase) -> Result<i64> {
        let text = c.text.trim();
        if text.is_empty() {
            return Err(StoreError::Invalid("ett ärende måste ha en text".into()));
        }
        if let Some(conf) = c.confidence
            && !(0.0..=1.0).contains(&conf)
        {
            return Err(StoreError::Invalid(
                "säkerheten ska ligga mellan 0 och 1".into(),
            ));
        }
        let (kind, reason) = match &c.kind {
            CaseKind::Question => ("fraga", None),
            CaseKind::Flag { reason } => {
                if !FLAG_REASONS.contains(&reason.as_str()) {
                    return Err(StoreError::Invalid(format!(
                        "okänd orsak {reason:?}; tillåtna är {}",
                        FLAG_REASONS.join(", ")
                    )));
                }
                if c.attachment_id.is_none() {
                    return Err(StoreError::Invalid(
                        "en flagga måste gälla ett underlag".into(),
                    ));
                }
                ("flagga", Some(reason.as_str()))
            }
        };
        if let Some(id) = c.attachment_id {
            let exists: bool = self.conn().query_row(
                "SELECT EXISTS (SELECT 1 FROM underlag WHERE id = ?1)",
                params![id],
                |r| r.get(0),
            )?;
            if !exists {
                return Err(StoreError::missing("underlag", id));
            }
        }
        self.write(actor, |ctx| {
            ctx.tx.execute(
                "INSERT INTO arenden (typ, underlag_id, orsak, text, kalla, aktor, modell, sakerhet, skapad, handelse)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    kind,
                    c.attachment_id,
                    reason,
                    text,
                    ctx.actor.kind.as_db_str(),
                    ctx.actor.name,
                    c.model,
                    c.confidence,
                    ctx.now_str,
                    ctx.event_seq,
                ],
            )?;
            let id = ctx.tx.last_insert_rowid();
            let event = EventDraft::new("arende.oppnat", format!("arende:{id}")).new_values(json!({
                "typ": kind,
                "underlag": c.attachment_id,
                "orsak": reason,
            }));
            Ok((id, event))
        })
    }

    /// Ärendena, äldst först. Med `open_only` bara de obesvarade.
    pub fn cases(&self, open_only: bool) -> Result<Vec<CaseRow>> {
        let filter = if open_only { "WHERE s.id IS NULL" } else { "" };
        let mut stmt = self.conn().prepare(&format!(
            "SELECT {CASE_COLUMNS} FROM arenden a LEFT JOIN arendesvar s ON s.arende_id = a.id {filter} ORDER BY a.id"
        ))?;
        let rows = stmt
            .query_map([], case_from)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn case(&self, id: i64) -> Result<CaseRow> {
        self.conn()
            .query_row(
                &format!(
                    "SELECT {CASE_COLUMNS} FROM arenden a LEFT JOIN arendesvar s ON s.arende_id = a.id WHERE a.id = ?1"
                ),
                params![id],
                case_from,
            )
            .optional()?
            .ok_or_else(|| StoreError::missing("ärende", id))
    }

    /// Besvarar ett ärende. Med `verdict` ändras också underlagets status;
    /// det kräver att ärendet gäller ett underlag som inte är bokfört.
    pub fn answer_case(
        &mut self,
        actor: &Actor,
        case_id: i64,
        answer: &str,
        verdict: Option<AttachmentVerdict>,
    ) -> Result<()> {
        require_human(actor, "besvara ärenden")?;
        let answer = answer.trim();
        if answer.is_empty() {
            return Err(StoreError::Invalid("svaret får inte vara tomt".into()));
        }
        self.write(actor, |ctx| {
            let case: CaseRow = ctx
                .tx
                .query_row(
                    &format!(
                        "SELECT {CASE_COLUMNS} FROM arenden a LEFT JOIN arendesvar s ON s.arende_id = a.id WHERE a.id = ?1"
                    ),
                    params![case_id],
                    case_from,
                )
                .optional()?
                .ok_or_else(|| StoreError::missing("ärende", case_id))?;
            if case.answer.is_some() {
                return Err(StoreError::Invalid(format!(
                    "ärende {case_id} är redan besvarat"
                )));
            }
            if let Some(v) = verdict {
                if case.kind == CaseKind::Question {
                    return Err(StoreError::Invalid(format!(
                        "ärende {case_id} är en fråga och ändrar ingen status; en status ändras bara genom en flagga"
                    )));
                }
                let attachment = case.attachment_id.ok_or_else(|| {
                    StoreError::Invalid(format!(
                        "ärende {case_id} gäller inget underlag, så ingen status kan ändras"
                    ))
                })?;
                let status: String = ctx.tx.query_row(
                    "SELECT status FROM underlag_status WHERE underlag_id = ?1 ORDER BY id DESC LIMIT 1",
                    params![attachment],
                    |r| r.get(0),
                )?;
                if status != "mottagen" {
                    return Err(StoreError::Invalid(format!(
                        "underlag {attachment} har status {status} och kan inte markeras som {}",
                        v.as_db_str()
                    )));
                }
                ctx.tx.execute(
                    "INSERT INTO underlag_status (underlag_id, status, text, tid, handelse)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![attachment, v.as_db_str(), answer, ctx.now_str, ctx.event_seq],
                )?;
            }
            ctx.tx.execute(
                "INSERT INTO arendesvar (arende_id, svar, status, aktor, tid, handelse)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    case_id,
                    answer,
                    verdict.map(AttachmentVerdict::as_db_str),
                    ctx.actor.name,
                    ctx.now_str,
                    ctx.event_seq
                ],
            )?;
            let event = EventDraft::new("arende.besvarat", format!("arende:{case_id}")).new_values(json!({
                "svar": answer,
                "underlag": case.attachment_id,
                "status": verdict.map(AttachmentVerdict::as_db_str),
            }));
            Ok(((), event))
        })
    }
}
