//! Bokföring av ett godkänt förslag.
//!
//! Allt sker i en transaktion: numret tas ur räknaren, den kanoniska formen
//! räknas fram och signeras med nyckeln, verifikationen och raderna skrivs,
//! motparten kopieras, underlagen länkas, rättelsen kopplas till originalet,
//! beslutet sparas och kedjan förlängs. Misslyckas något, till exempel
//! signaturen, rullas allt tillbaka och numret är oförbrukat.

use rusqlite::{OptionalExtension, params};
use serde_json::json;

use bok_core::VoucherRef;
use bok_core::chain::{GENESIS, voucher_chain_hash};
use bok_core::correction::{Placement, placement};
use bok_core::signature::{NS_VOUCHER, Signer};
use bok_core::voucher::{
    PostedVoucher, validate_description, validate_lines, validate_series, validate_series_lines,
};

use super::{
    check_accounts, check_attachment_usable, corrected_original, counterparty_by_id, decision_for,
    fiscal_year_by_id, period_by_id, period_for_date, proposal_by_id, require_human,
    stored_local_date, verify_registered,
};
use crate::model::*;
use crate::write::{Actor, EventDraft};
use crate::{Result, Store, StoreError};

impl Store {
    /// Godkänner och bokför ett förslag. `signer` signerar verifikationens
    /// kanoniska hash; signaturen måste komma från en registrerad nyckel.
    pub fn approve(
        &mut self,
        actor: &Actor,
        proposal_id: i64,
        signer: &dyn Signer,
    ) -> Result<Posted> {
        require_human(actor, "bokföra")?;
        self.write(actor, |ctx| {
            let tx = ctx.tx;
            let p = proposal_by_id(tx, proposal_id)?;
            if decision_for(tx, proposal_id)?.is_some() {
                return Err(StoreError::AlreadyDecided(proposal_id));
            }

            // Perioden: förslagets datum, eller för en rättelse originalets
            // period om den är öppen och annars den period som är öppen nu.
            let (period, date, changes) = match p.corrects {
                None => {
                    let period = period_for_date(tx, p.date)?.ok_or(StoreError::NoPeriod(p.date))?;
                    (period, p.date, None)
                }
                Some(original_id) => {
                    let original_period_id: i64 = tx.query_row(
                        "SELECT period_id FROM verifikationer WHERE id = ?1",
                        params![original_id],
                        |r| r.get(0),
                    )?;
                    let original_period = period_by_id(tx, original_period_id)?;
                    match placement(original_period.status) {
                        Placement::OriginalPeriod => (original_period, p.date, None),
                        Placement::CurrentPeriod => {
                            let today = ctx.today;
                            let period = period_for_date(tx, today)?.ok_or(StoreError::NoPeriod(today))?;
                            let changes = json!({
                                "datum": { "forslag": p.date.to_string(), "bokford": today.to_string() },
                                "skal": format!(
                                    "originalets period {} är {}; rättelsen bokförs i den period som är öppen nu",
                                    original_period.label(),
                                    original_period.status
                                ),
                            });
                            (period, today, Some(changes))
                        }
                    }
                }
            };
            if !period.status.allows_posting() {
                return Err(StoreError::PeriodNotOpen { period: period.label(), status: period.status });
            }
            let year = fiscal_year_by_id(tx, period.fiscal_year_id)?;

            validate_series(p.series, p.corrects.is_some())?;
            validate_description(&p.description)?;
            validate_lines(&p.lines)?;
            validate_series_lines(p.series, &p.lines)?;
            check_accounts(tx, &p.lines, date, p.corrects.is_none())?;

            let counterparty = match &p.counterparty {
                None => None,
                Some(ProposalCounterparty::Register(id)) => Some(counterparty_by_id(tx, *id)?),
                Some(ProposalCounterparty::Inline(c)) => Some(c.clone()),
            };
            let mut attachments = Vec::new();
            for id in &p.attachments {
                let sha: String = tx
                    .query_row("SELECT sha256 FROM underlag WHERE id = ?1", params![id], |r| r.get(0))
                    .optional()?
                    .ok_or_else(|| StoreError::missing("underlag", id))?;
                // Granskarens beslut om dubblett kan ha kommit efter förslaget.
                check_attachment_usable(tx, *id)?;
                attachments.push((*id, sha));
            }
            let corrects = match p.corrects {
                None => None,
                Some(original_id) => Some(corrected_original(tx, original_id)?),
            };

            let number: i64 = tx
                .query_row(
                    "UPDATE nummerserier SET nasta = nasta + 1
                     WHERE rakenskapsar_id = ?1 AND serie = ?2 RETURNING nasta - 1",
                    params![year.id, p.series.as_str()],
                    |r| r.get(0),
                )
                .optional()?
                .ok_or_else(|| StoreError::missing("nummerserie", format!("{} {}", p.series, year.label())))?;
            let reference = VoucherRef { series: p.series, number };

            let voucher = PostedVoucher {
                series: p.series,
                number,
                fiscal_year: (year.start, year.end),
                period: (period.start, period.end),
                date,
                compiled_on: stored_local_date(&p.created_at)?,
                registered_at: ctx.now_str.clone(),
                description: p.description.clone(),
                counterparty: counterparty.clone(),
                proposal_id: p.id,
                attachments: attachments.iter().map(|(_, sha)| sha.clone()).collect(),
                corrects: corrects.clone(),
                lines: p.lines.clone(),
            };
            let canonical_hash = voucher.canonical_hash();
            let signature = signer.sign(NS_VOUCHER, canonical_hash.as_bytes())?;
            let key = verify_registered(tx, NS_VOUCHER, canonical_hash.as_bytes(), &signature)?;

            let previous_chain: String = tx
                .query_row("SELECT kedjehash FROM verifikationer ORDER BY id DESC LIMIT 1", [], |r| r.get(0))
                .optional()?
                .unwrap_or_else(|| GENESIS.to_string());
            let chain_hash = voucher_chain_hash(&previous_chain, &canonical_hash);

            tx.execute(
                "INSERT INTO verifikationer (rakenskapsar_id, serie, nummer, period_id, datum, sammanstallningsdag,
                    registrerad, beskrivning, motpart_namn, motpart_orgnr, motpart_momsnr, forslag_id, antal_rader,
                    kanonisk_hash, signatur, nyckel_id, kedjehash, motpart_adress)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
                params![
                    year.id,
                    p.series.as_str(),
                    number,
                    period.id,
                    date.to_string(),
                    voucher.compiled_on.to_string(),
                    voucher.registered_at,
                    voucher.description,
                    counterparty.as_ref().map(|c| c.name.clone()),
                    counterparty.as_ref().and_then(|c| c.org_number.clone()),
                    counterparty.as_ref().and_then(|c| c.vat_number.clone()),
                    p.id,
                    voucher.lines.len() as i64,
                    canonical_hash,
                    signature,
                    key.id,
                    chain_hash,
                    counterparty.as_ref().and_then(|c| c.address.clone()),
                ],
            )?;
            let voucher_id = tx.last_insert_rowid();
            for (i, line) in voucher.lines.iter().enumerate() {
                tx.execute(
                    "INSERT INTO rader (verifikation_id, radnr, konto, debet, kredit, momskod, text)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        voucher_id,
                        i as i64 + 1,
                        i64::from(line.account),
                        line.debit.ore(),
                        line.credit.ore(),
                        line.vat_code,
                        line.text,
                    ],
                )?;
            }
            if let (Some(original_id), Some(reason)) = (p.corrects, &p.correction_reason) {
                tx.execute(
                    "INSERT INTO rattelser (original_id, rattelse_id, orsak) VALUES (?1, ?2, ?3)",
                    params![original_id, voucher_id, reason],
                )?;
            }
            for (attachment_id, _) in &attachments {
                tx.execute(
                    "INSERT INTO underlag_status (underlag_id, status, verifikation_id, text, tid, handelse)
                     VALUES (?1, 'bokford', ?2, ?3, ?4, ?5)",
                    params![
                        attachment_id,
                        voucher_id,
                        format!("bokförd som {reference} ({})", year.label()),
                        ctx.now_str,
                        ctx.event_seq
                    ],
                )?;
            }
            tx.execute(
                "INSERT INTO beslut (forslag_id, utfall, aktor, tid, andringar, verifikation_id, signatur, nyckel_id)
                 VALUES (?1, 'godkant', ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    p.id,
                    ctx.actor.name,
                    ctx.now_str,
                    changes.as_ref().map(|c| c.to_string()),
                    voucher_id,
                    signature,
                    key.id
                ],
            )?;

            let event = EventDraft::new("verifikation.bokford", format!("verifikation:{voucher_id}"))
                .new_values(json!({
                    "verifikation": reference.to_string(),
                    "rakenskapsar": year.label(),
                    "period": period.label(),
                    "datum": date.to_string(),
                    "forslag": p.id,
                    "kanonisk_hash": canonical_hash,
                    "kedjehash": chain_hash,
                    "nyckel": key.fingerprint,
                    "rattar": corrects.as_ref().map(|c| c.reference.to_string()),
                }))
                .signed(signature.clone());
            Ok((
                Posted {
                    voucher_id,
                    fiscal_year_id: year.id,
                    reference,
                    period: period.label(),
                    date,
                    canonical_hash,
                },
                event,
            ))
        })
    }
}
