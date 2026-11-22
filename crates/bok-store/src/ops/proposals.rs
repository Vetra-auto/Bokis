//! Förslag: skapa, avvisa och föreslå rättelser. Inget här delar ut nummer.

use rusqlite::{OptionalExtension, params};
use serde_json::json;

use bok_core::correction::{correction_description, reversal_lines, validate_reason};
use bok_core::money::checked_sum;
use bok_core::voucher::{
    Counterparty, Line, normalize_counterparty, validate_counterparty, validate_description,
    validate_lines, validate_series, validate_series_lines,
};
use bok_core::{Series, VoucherRef};

use super::{
    check_accounts, check_attachment_usable, counterparty_by_id, decision_for, parse_series,
    period_for_date, proposal_by_id, require_human, voucher_lines,
};
use crate::model::*;
use crate::write::{Actor, EventDraft};
use crate::{Result, Store, StoreError, parse_date};

impl Store {
    /// Lägger ett förslag. Förslaget får inget nummer och ingen period; de
    /// bestäms först när det bokförs. Med samma idempotensnyckel ges det
    /// befintliga förslaget tillbaka i stället för ett nytt.
    pub fn create_proposal(&mut self, actor: &Actor, p: &NewProposal) -> Result<i64> {
        if let Some(key) = &p.idempotency_key
            && let Some(existing) = self
                .conn()
                .query_row(
                    "SELECT id FROM forslag WHERE idempotensnyckel = ?1",
                    params![key],
                    |r| r.get(0),
                )
                .optional()?
        {
            return Ok(existing);
        }
        validate_series(p.series, false)?;
        validate_description(&p.description)?;
        validate_lines(&p.lines)?;
        validate_series_lines(p.series, &p.lines)?;
        let conn = self.conn();
        if period_for_date(conn, p.date)?.is_none() {
            return Err(StoreError::NoPeriod(p.date));
        }
        check_accounts(conn, &p.lines, p.date, true)?;
        let (register_id, inline) = match &p.counterparty {
            None => (None, None),
            Some(ProposalCounterparty::Register(id)) => {
                counterparty_by_id(conn, *id)?;
                (Some(*id), None)
            }
            Some(ProposalCounterparty::Inline(c)) => {
                validate_counterparty(c)?;
                (None, Some(normalize_counterparty(c)))
            }
        };
        for id in &p.attachments {
            let exists: bool = conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM underlag WHERE id = ?1)",
                params![id],
                |r| r.get(0),
            )?;
            if !exists {
                return Err(StoreError::missing("underlag", id));
            }
            check_attachment_usable(conn, *id)?;
        }
        let lines_json = serde_json::to_string(&p.lines)?;
        let attachments_json = serde_json::to_string(&p.attachments)?;
        let total = checked_sum(p.lines.iter().map(|l| l.debit)).unwrap_or_default();
        let v = &p.provenance;
        self.write(actor, |ctx| {
            ctx.tx.execute(
                "INSERT INTO forslag (serie, datum, beskrivning, motpart_id, motpart_namn, motpart_orgnr, motpart_momsnr,
                                      rader, underlag, kalla, aktor, fragor, flaggor, sakerhet, modell, leverantor,
                                      region, prompt_hash, regelversion, ratt_svar, idempotensnyckel, skapad)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)",
                params![
                    p.series.as_str(),
                    p.date.to_string(),
                    p.description.trim(),
                    register_id,
                    inline.as_ref().map(|c| c.name.trim().to_string()),
                    inline.as_ref().and_then(|c| c.org_number.clone()),
                    inline.as_ref().and_then(|c| c.vat_number.clone()),
                    lines_json,
                    attachments_json,
                    ctx.actor.kind.as_db_str(),
                    ctx.actor.name,
                    v.questions,
                    v.flags,
                    v.confidence,
                    v.model,
                    v.provider,
                    v.region,
                    v.prompt_hash,
                    v.rule_version,
                    v.raw_response,
                    p.idempotency_key,
                    ctx.now_str,
                ],
            )?;
            let id = ctx.tx.last_insert_rowid();
            let event = EventDraft::new("forslag.skapat", format!("forslag:{id}")).new_values(json!({
                "serie": p.series.as_str(),
                "datum": p.date.to_string(),
                "beskrivning": p.description.trim(),
                "summa_ore": total.ore(),
                "rader": p.lines.len(),
                "underlag": p.attachments,
            }));
            Ok((id, event))
        })
    }

    /// Avvisar ett förslag med en orsak. Inget nummer förbrukas.
    pub fn reject(&mut self, actor: &Actor, proposal_id: i64, reason: &str) -> Result<()> {
        require_human(actor, "avvisa förslag")?;
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(StoreError::Invalid(
                "ett avvisat förslag måste ha en orsak".into(),
            ));
        }
        self.write(actor, |ctx| {
            proposal_by_id(ctx.tx, proposal_id)?;
            if decision_for(ctx.tx, proposal_id)?.is_some() {
                return Err(StoreError::AlreadyDecided(proposal_id));
            }
            ctx.tx.execute(
                "INSERT INTO beslut (forslag_id, utfall, aktor, tid, orsak) VALUES (?1, 'avvisat', ?2, ?3, ?4)",
                params![proposal_id, ctx.actor.name, ctx.now_str, reason],
            )?;
            let event = EventDraft::new("forslag.avvisat", format!("forslag:{proposal_id}"))
                .new_values(json!({ "orsak": reason }));
            Ok(((), event))
        })
    }

    /// Föreslår en rättelse av en bokförd verifikation: ett förslag i serie
    /// R med originalets rader omvända, och eventuellt rätta rader efter dem.
    /// Rättelsen bokförs som alla förslag, med `approve`.
    pub fn propose_correction(
        &mut self,
        actor: &Actor,
        original_id: i64,
        reason: &str,
        replacement: &[Line],
    ) -> Result<i64> {
        validate_reason(reason)?;
        let conn = self.conn();
        let (series, number, date, cp_name, cp_org, cp_vat): (String, i64, String, Option<String>, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT serie, nummer, datum, motpart_namn, motpart_orgnr, motpart_momsnr FROM verifikationer WHERE id = ?1",
                params![original_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .optional()?
            .ok_or_else(|| StoreError::missing("verifikation", original_id))?;
        let reference = VoucherRef {
            series: parse_series(&series)?,
            number,
        };
        let date = parse_date(&date)?;
        let original_lines = voucher_lines(conn, original_id)?;
        // Omvända rader måste finnas i kontoplanen men får vara inaktiva nu;
        // nya rader måste vara aktiva.
        check_accounts(conn, &original_lines, date, false)?;
        if !replacement.is_empty() {
            validate_lines(replacement)?;
            check_accounts(conn, replacement, date, true)?;
        }
        let mut lines = reversal_lines(&original_lines);
        lines.extend(replacement.iter().cloned());
        validate_lines(&lines)?;
        let description = correction_description(reference, reason);
        let counterparty = cp_name.map(|name| Counterparty {
            name,
            org_number: cp_org,
            vat_number: cp_vat,
        });
        let lines_json = serde_json::to_string(&lines)?;
        let reason = reason.trim().to_string();
        self.write(actor, |ctx| {
            ctx.tx.execute(
                "INSERT INTO forslag (serie, datum, beskrivning, motpart_namn, motpart_orgnr, motpart_momsnr,
                                      rader, underlag, rattar, rattelseorsak, kalla, aktor, skapad)
                 VALUES ('R', ?1, ?2, ?3, ?4, ?5, ?6, '[]', ?7, ?8, ?9, ?10, ?11)",
                params![
                    date.to_string(),
                    description,
                    counterparty.as_ref().map(|c| c.name.clone()),
                    counterparty.as_ref().and_then(|c| c.org_number.clone()),
                    counterparty.as_ref().and_then(|c| c.vat_number.clone()),
                    lines_json,
                    original_id,
                    reason,
                    ctx.actor.kind.as_db_str(),
                    ctx.actor.name,
                    ctx.now_str,
                ],
            )?;
            let id = ctx.tx.last_insert_rowid();
            let event = EventDraft::new("forslag.skapat", format!("forslag:{id}")).new_values(json!({
                "serie": Series::R.as_str(),
                "rattar": reference.to_string(),
                "rattar_id": original_id,
                "orsak": reason,
                "rader": lines.len(),
            }));
            Ok((id, event))
        })
    }
}
