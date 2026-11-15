//! Skrivningar: en transaktion, en händelse.

use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use bok_core::chain::{EventRecord, GENESIS};
use bok_core::period::ActorKind;

use crate::{
    Result, Store, StoreError, display_timestamp, format_timestamp, local_date, parse_timestamp,
};

/// Den som utför en handling: människa, agent eller system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Actor {
    pub kind: ActorKind,
    pub name: String,
    pub role: String,
}

impl Actor {
    /// Den som granskar och bokför med sin nyckel.
    pub fn human(name: impl Into<String>) -> Actor {
        Actor {
            kind: ActorKind::Human,
            name: name.into(),
            role: "granskare".into(),
        }
    }

    /// En AI-agent. Den får lägga förslag men aldrig bokföra.
    pub fn agent(name: impl Into<String>) -> Actor {
        Actor {
            kind: ActorKind::Agent,
            name: name.into(),
            role: "agent".into(),
        }
    }

    pub fn system(name: impl Into<String>) -> Actor {
        Actor {
            kind: ActorKind::System,
            name: name.into(),
            role: "system".into(),
        }
    }
}

/// Händelsen som en skrivning lägger i historiken.
pub(crate) struct EventDraft {
    pub action: &'static str,
    pub object: Option<String>,
    pub old: Option<serde_json::Value>,
    pub new: Option<serde_json::Value>,
    pub signature: Option<String>,
}

impl EventDraft {
    pub fn new(action: &'static str, object: impl Into<String>) -> Self {
        EventDraft {
            action,
            object: Some(object.into()),
            old: None,
            new: None,
            signature: None,
        }
    }

    pub fn old(mut self, value: serde_json::Value) -> Self {
        self.old = Some(value);
        self
    }

    pub fn new_values(mut self, value: serde_json::Value) -> Self {
        self.new = Some(value);
        self
    }

    pub fn signed(mut self, signature: String) -> Self {
        self.signature = Some(signature);
        self
    }
}

/// Det en skrivning har tillgång till inne i transaktionen.
pub(crate) struct WriteCtx<'t> {
    pub tx: &'t Transaction<'t>,
    pub now_str: String,
    pub today: NaiveDate,
    pub actor: &'t Actor,
    /// Numret som transaktionens händelse får. Rader som ska peka på
    /// händelsen (med en uppskjuten främmande nyckel) använder det.
    pub event_seq: i64,
}

impl Store {
    /// Kör `f` i en egen transaktion (`BEGIN IMMEDIATE`) och lägger dess
    /// händelse sist i samma transaktion. Ett fel någonstans rullar tillbaka
    /// allt, även en uppräknad nummerserie. Står klockan före den senaste
    /// händelsen skrivs ingenting.
    pub(crate) fn write<T>(
        &mut self,
        actor: &Actor,
        f: impl FnOnce(&WriteCtx<'_>) -> Result<(T, EventDraft)>,
    ) -> Result<T> {
        self.write_checked(actor, true, f)
    }

    /// Som [`Store::write`], men utan spärren mot en klocka som står före den
    /// senaste händelsen. Bara för händelsen som godtar just det.
    pub(crate) fn write_despite_clock<T>(
        &mut self,
        actor: &Actor,
        f: impl FnOnce(&WriteCtx<'_>) -> Result<(T, EventDraft)>,
    ) -> Result<T> {
        self.write_checked(actor, false, f)
    }

    fn write_checked<T>(
        &mut self,
        actor: &Actor,
        check_clock: bool,
        f: impl FnOnce(&WriteCtx<'_>) -> Result<(T, EventDraft)>,
    ) -> Result<T> {
        let now = self.now();
        let program = self.program_version.clone();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if check_clock {
            clock_after_last_event(&tx, now)?;
        }
        let value = {
            let event_seq: i64 =
                tx.query_row("SELECT COALESCE(MAX(seq), 0) + 1 FROM handelser", [], |r| {
                    r.get(0)
                })?;
            let ctx = WriteCtx {
                tx: &tx,
                now_str: format_timestamp(now),
                today: local_date(now),
                actor,
                event_seq,
            };
            let (value, draft) = f(&ctx)?;
            append_event(&ctx, draft, &program)?;
            value
        };
        tx.commit()?;
        Ok(value)
    }
}

/// Spärren mot en klocka som har gått bakåt. En dator utan batteriklocka,
/// till exempel en Raspberry Pi, startar efter ett strömavbrott med gammal tid
/// tills nätet har ställt den. Då vägrar Bokis hellre än skriver händelser med
/// fel tid i historiken.
fn clock_after_last_event(tx: &Transaction<'_>, now: DateTime<Utc>) -> Result<()> {
    let last: Option<String> = tx
        .query_row(
            "SELECT tid FROM handelser ORDER BY seq DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    match last {
        Some(last) if now < parse_timestamp(&last)? => Err(StoreError::ClockBehind {
            now: display_timestamp(&format_timestamp(now)),
            last: display_timestamp(&last),
        }),
        _ => Ok(()),
    }
}

fn append_event(ctx: &WriteCtx<'_>, draft: EventDraft, program: &str) -> Result<()> {
    let prev: String = ctx
        .tx
        .query_row(
            "SELECT hash FROM handelser ORDER BY seq DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or_else(|| GENESIS.to_string());
    let record = EventRecord {
        seq: ctx.event_seq,
        time: ctx.now_str.clone(),
        actor_kind: ctx.actor.kind.as_db_str().to_string(),
        actor: ctx.actor.name.clone(),
        role: ctx.actor.role.clone(),
        action: draft.action.to_string(),
        object: draft.object,
        old_values: draft.old.map(|v| v.to_string()),
        new_values: draft.new.map(|v| v.to_string()),
        program_version: program.to_string(),
        rule_version: None,
        signature: draft.signature,
        prev_hash: prev,
    };
    let hash = record.compute_hash();
    ctx.tx.execute(
        "INSERT INTO handelser (seq, tid, aktor_typ, aktor, roll, handling, objekt, gamla, nya,
                                programversion, regelversion, signatur, foregaende_hash, hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            record.seq,
            record.time,
            record.actor_kind,
            record.actor,
            record.role,
            record.action,
            record.object,
            record.old_values,
            record.new_values,
            record.program_version,
            record.rule_version,
            record.signature,
            record.prev_hash,
            hash,
        ],
    )?;
    Ok(())
}
