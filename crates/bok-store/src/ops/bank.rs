//! Bankrader och avstämning (schema version 2).
//!
//! Raderna läses in från ett mottaget kontoutdrag och kan bara växa. En rad
//! avstäms en gång, av en människa, antingen mot en bokförd verifikation som
//! rör bankens bokföringskonto med exakt radens belopp eller med en skriftlig
//! förklaring. Agenten och andra kan lägga matchningsförslag; de avstämmer
//! ingenting. Periodövergången vägrar stänga eller låsa en period med
//! bankrader som inte är avstämda.

use chrono::NaiveDate;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::json;

use bok_core::{AccountNumber, Money, VoucherRef};

use super::{account_at, parse_series, require_human};
use crate::model::*;
use crate::write::{Actor, EventDraft};
use crate::{Result, Store, StoreError, parse_date};

const ROW_COLUMNS: &str =
    "b.id, b.underlag_id, b.post_id, b.bankkonto, b.konto, b.bokforingsdag, b.valutadag,
     b.belopp, b.valuta, b.text, b.referens, b.motpart, b.saldo, b.importerad,
     m.verifikation_id, m.forklaring, m.aktor, m.tid";

const ROW_FROM: &str = "FROM bankrader b LEFT JOIN bankmatchningar m ON m.bankrad_id = b.id";

fn row_from(r: &Row<'_>) -> rusqlite::Result<BankRowRecord> {
    let account: i64 = r.get(4)?;
    let account = AccountNumber::new(account).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Integer, Box::new(e))
    })?;
    let matched_actor: Option<String> = r.get(16)?;
    Ok(BankRowRecord {
        id: r.get(0)?,
        attachment_id: r.get(1)?,
        post_id: r.get(2)?,
        bank_account: r.get(3)?,
        account,
        booking_date: parse_date(&r.get::<_, String>(5)?)?,
        value_date: r
            .get::<_, Option<String>>(6)?
            .map(|d| parse_date(&d))
            .transpose()?,
        amount: Money::from_ore(r.get(7)?),
        currency: r.get(8)?,
        text: r.get(9)?,
        reference: r.get(10)?,
        counterparty: r.get(11)?,
        balance: r.get::<_, Option<i64>>(12)?.map(Money::from_ore),
        imported_at: r.get(13)?,
        reconciliation: match matched_actor {
            None => None,
            Some(actor) => Some(Reconciliation {
                voucher_id: r.get(14)?,
                explanation: r.get(15)?,
                actor,
                time: r.get(17)?,
            }),
        },
    })
}

pub(crate) fn bank_row_by_id(conn: &Connection, id: i64) -> Result<BankRowRecord> {
    conn.query_row(
        &format!("SELECT {ROW_COLUMNS} {ROW_FROM} WHERE b.id = ?1"),
        params![id],
        row_from,
    )
    .optional()?
    .ok_or_else(|| StoreError::missing("bankrad", id))
}

/// Verifikationens nettorörelse (debet minus kredit) på ett konto, och
/// dess referens. `None` om verifikationen inte finns.
fn voucher_net(
    conn: &Connection,
    voucher_id: i64,
    account: AccountNumber,
) -> Result<Option<(VoucherRef, i64)>> {
    let head: Option<(String, i64)> = conn
        .query_row(
            "SELECT serie, nummer FROM verifikationer WHERE id = ?1",
            params![voucher_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((series, number)) = head else {
        return Ok(None);
    };
    let net: i64 = conn.query_row(
        "SELECT COALESCE(SUM(debet - kredit), 0) FROM rader WHERE verifikation_id = ?1 AND konto = ?2",
        params![voucher_id, i64::from(account)],
        |r| r.get(0),
    )?;
    Ok(Some((
        VoucherRef {
            series: parse_series(&series)?,
            number,
        },
        net,
    )))
}

/// Kontrollerar att verifikationen motsvarar bankraden: samma belopp på
/// bankens bokföringskonto.
fn check_voucher_amount(
    conn: &Connection,
    row: &BankRowRecord,
    voucher_id: i64,
) -> Result<VoucherRef> {
    if row.amount.is_zero() {
        return Err(StoreError::Invalid(format!(
            "bankrad {} är på 0 kr; den stäms av med en förklaring, inte mot en verifikation",
            row.id
        )));
    }
    let (reference, net) = voucher_net(conn, voucher_id, row.account)?
        .ok_or_else(|| StoreError::missing("verifikation", voucher_id))?;
    let touches: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM rader WHERE verifikation_id = ?1 AND konto = ?2)",
        params![voucher_id, i64::from(row.account)],
        |r| r.get(0),
    )?;
    if !touches {
        return Err(StoreError::Invalid(format!(
            "verifikation {reference} rör inte konto {}",
            row.account
        )));
    }
    if net != row.amount.ore() {
        return Err(StoreError::Invalid(format!(
            "verifikation {reference} rör konto {} med {} men bankrad {} är {}",
            row.account,
            Money::from_ore(net),
            row.id,
            row.amount
        )));
    }
    Ok(reference)
}

/// id, räkenskapsår, serie, nummer, datum, beskrivning, motpart.
type CandidateRow = (i64, i64, String, i64, String, String, Option<String>);

fn tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() >= 3)
        .filter(|t| !matches!(*t, "och" | "the" | "for" | "ab" | "aktiebolag" | "faktura"))
        .map(str::to_string)
        .collect()
}

fn digits(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_ascii_digit())
        .filter(|t| t.len() >= 4)
        .map(str::to_string)
        .collect()
}

impl Store {
    /// Läser in rader från ett mottaget kontoutdrag. `account` är bankens
    /// bokföringskonto, till exempel 1930. Rader vars `post_id` redan finns
    /// hoppas över, så samma fil kan läsas in igen.
    pub fn import_bank_rows(
        &mut self,
        actor: &Actor,
        attachment_id: i64,
        account: AccountNumber,
        rows: &[NewBankRow],
    ) -> Result<BankImport> {
        let conn = self.conn();
        let exists: bool = conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM underlag WHERE id = ?1)",
            params![attachment_id],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(StoreError::missing("underlag", attachment_id));
        }
        let today = self.today();
        match account_at(conn, account, today)? {
            Some(a) if a.active => {}
            Some(_) => return Err(StoreError::InactiveAccount(account, today)),
            None => return Err(StoreError::UnknownAccount(account, today)),
        }
        let mut new_rows = Vec::new();
        let mut existing = 0;
        for row in rows {
            if row.post_id.len() != 64 || !row.post_id.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(StoreError::Invalid(format!(
                    "bankradens id måste vara 64 hexadecimala tecken: {}",
                    row.post_id
                )));
            }
            if row.bank_account.trim().is_empty() {
                return Err(StoreError::Invalid("bankraden saknar bankkonto".into()));
            }
            // Bokföringen är i kronor och avstämningen jämför belopp i öre.
            if !row.currency.trim().eq_ignore_ascii_case("SEK") {
                return Err(StoreError::Invalid(format!(
                    "bankraden {} är i {}; bara rader i SEK kan läsas in och stämmas av",
                    row.booking_date,
                    row.currency.trim()
                )));
            }
            let known: bool = conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM bankrader WHERE post_id = ?1)",
                params![row.post_id],
                |r| r.get(0),
            )?;
            if known
                || new_rows
                    .iter()
                    .any(|r: &&NewBankRow| r.post_id == row.post_id)
            {
                existing += 1;
            } else {
                new_rows.push(row);
            }
        }
        if new_rows.is_empty() {
            return Ok(BankImport { new: 0, existing });
        }
        let first = new_rows.iter().map(|r| r.booking_date).min();
        let last = new_rows.iter().map(|r| r.booking_date).max();
        let total: i64 = new_rows.iter().map(|r| r.amount.ore()).sum();
        self.write(actor, |ctx| {
            for row in &new_rows {
                ctx.tx.execute(
                    "INSERT INTO bankrader (underlag_id, post_id, bankkonto, konto, bokforingsdag, valutadag, belopp,
                                           valuta, text, referens, motpart, saldo, importerad, handelse)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                    params![
                        attachment_id,
                        row.post_id.to_ascii_lowercase(),
                        row.bank_account.trim(),
                        i64::from(account),
                        row.booking_date.to_string(),
                        row.value_date.map(|d| d.to_string()),
                        row.amount.ore(),
                        row.currency,
                        row.text,
                        row.reference,
                        row.counterparty,
                        row.balance.map(Money::ore),
                        ctx.now_str,
                        ctx.event_seq,
                    ],
                )?;
            }
            let event = EventDraft::new("bankrader.inlasta", format!("underlag:{attachment_id}")).new_values(json!({
                "konto": i64::from(account),
                "nya": new_rows.len(),
                "fanns": existing,
                "forsta": first.map(|d| d.to_string()),
                "sista": last.map(|d| d.to_string()),
                "summa_ore": total,
            }));
            Ok((
                BankImport {
                    new: new_rows.len(),
                    existing,
                },
                event,
            ))
        })
    }

    /// Bankraderna i datumordning. Med `unreconciled_only` bara de som inte
    /// är avstämda.
    pub fn bank_rows(&self, unreconciled_only: bool) -> Result<Vec<BankRowRecord>> {
        let filter = if unreconciled_only {
            "WHERE m.id IS NULL"
        } else {
            ""
        };
        let mut stmt = self.conn().prepare(&format!(
            "SELECT {ROW_COLUMNS} {ROW_FROM} {filter} ORDER BY b.bokforingsdag, b.id"
        ))?;
        let rows = stmt
            .query_map([], row_from)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn bank_row(&self, id: i64) -> Result<BankRowRecord> {
        bank_row_by_id(self.conn(), id)
    }

    /// Stämmer av en bankrad. Bara en människa får göra det, och bara en gång
    /// per rad. En verifikation måste röra bankens bokföringskonto med exakt
    /// radens belopp och får inte redan vara matchad mot en annan rad.
    pub fn reconcile_bank_row(
        &mut self,
        actor: &Actor,
        bank_row_id: i64,
        target: &MatchTarget,
    ) -> Result<()> {
        require_human(actor, "stämma av bankrader")?;
        self.write(actor, |ctx| {
            let row = bank_row_by_id(ctx.tx, bank_row_id)?;
            if row.reconciliation.is_some() {
                return Err(StoreError::Invalid(format!("bankrad {bank_row_id} är redan avstämd")));
            }
            let event = match target {
                MatchTarget::Voucher(voucher_id) => {
                    let reference = check_voucher_amount(ctx.tx, &row, *voucher_id)?;
                    let taken: Option<i64> = ctx
                        .tx
                        .query_row(
                            "SELECT bankrad_id FROM bankmatchningar WHERE verifikation_id = ?1",
                            params![voucher_id],
                            |r| r.get(0),
                        )
                        .optional()?;
                    if let Some(other) = taken {
                        return Err(StoreError::Invalid(format!(
                            "verifikation {reference} är redan avstämd mot bankrad {other}"
                        )));
                    }
                    ctx.tx.execute(
                        "INSERT INTO bankmatchningar (bankrad_id, verifikation_id, aktor, tid, handelse)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![bank_row_id, voucher_id, ctx.actor.name, ctx.now_str, ctx.event_seq],
                    )?;
                    EventDraft::new("bankrad.avstamd", format!("bankrad:{bank_row_id}")).new_values(json!({
                        "verifikation": reference.to_string(),
                        "verifikation_id": voucher_id,
                        "belopp_ore": row.amount.ore(),
                    }))
                }
                MatchTarget::Explanation(text) => {
                    let text = text.trim();
                    if text.is_empty() {
                        return Err(StoreError::Invalid("en förklaring får inte vara tom".into()));
                    }
                    ctx.tx.execute(
                        "INSERT INTO bankmatchningar (bankrad_id, forklaring, aktor, tid, handelse)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![bank_row_id, text, ctx.actor.name, ctx.now_str, ctx.event_seq],
                    )?;
                    EventDraft::new("bankrad.forklarad", format!("bankrad:{bank_row_id}")).new_values(json!({
                        "forklaring": text,
                        "belopp_ore": row.amount.ore(),
                    }))
                }
            };
            Ok(((), event))
        })
    }

    /// Lägger ett matchningsförslag. Vem som helst, även en agent, får göra
    /// det; förslaget avstämmer ingenting. En föreslagen verifikation måste ha
    /// rätt belopp på bankens konto.
    pub fn suggest_match(
        &mut self,
        actor: &Actor,
        bank_row_id: i64,
        s: &NewMatchSuggestion,
    ) -> Result<i64> {
        if let Some(c) = s.confidence
            && !(0.0..=1.0).contains(&c)
        {
            return Err(StoreError::Invalid(
                "säkerheten ska ligga mellan 0 och 1".into(),
            ));
        }
        self.write(actor, |ctx| {
            let row = bank_row_by_id(ctx.tx, bank_row_id)?;
            let (voucher_id, proposal_id, explanation, described) = match &s.target {
                SuggestedMatch::Voucher(id) => {
                    let reference = check_voucher_amount(ctx.tx, &row, *id)?;
                    (Some(*id), None, None, reference.to_string())
                }
                SuggestedMatch::Proposal(id) => {
                    let exists: bool = ctx.tx.query_row(
                        "SELECT EXISTS (SELECT 1 FROM forslag WHERE id = ?1)",
                        params![id],
                        |r| r.get(0),
                    )?;
                    if !exists {
                        return Err(StoreError::missing("förslag", id));
                    }
                    (None, Some(*id), None, format!("förslag {id}"))
                }
                SuggestedMatch::Explanation(text) => {
                    let text = text.trim();
                    if text.is_empty() {
                        return Err(StoreError::Invalid("en förklaring får inte vara tom".into()));
                    }
                    (None, None, Some(text.to_string()), "förklaring".to_string())
                }
            };
            ctx.tx.execute(
                "INSERT INTO matchningsforslag (bankrad_id, verifikation_id, forslag_id, forklaring, kalla, aktor,
                                                sakerhet, modell, motivering, skapad, handelse)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    bank_row_id,
                    voucher_id,
                    proposal_id,
                    explanation,
                    ctx.actor.kind.as_db_str(),
                    ctx.actor.name,
                    s.confidence,
                    s.model,
                    s.rationale,
                    ctx.now_str,
                    ctx.event_seq,
                ],
            )?;
            let id = ctx.tx.last_insert_rowid();
            let event = EventDraft::new("matchning.foreslagen", format!("bankrad:{bank_row_id}"))
                .new_values(json!({ "forslag": id, "mot": described }));
            Ok((id, event))
        })
    }

    /// Matchningsförslagen, äldst först; för en rad eller för alla.
    pub fn match_suggestions(&self, bank_row_id: Option<i64>) -> Result<Vec<MatchSuggestion>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, bankrad_id, verifikation_id, forslag_id, forklaring, kalla, aktor, sakerhet, modell,
                    motivering, skapad
             FROM matchningsforslag WHERE ?1 IS NULL OR bankrad_id = ?1 ORDER BY id",
        )?;
        let rows = stmt
            .query_map(params![bank_row_id], |r| {
                let voucher: Option<i64> = r.get(2)?;
                let proposal: Option<i64> = r.get(3)?;
                let explanation: Option<String> = r.get(4)?;
                let target = match (voucher, proposal, explanation) {
                    (Some(v), _, _) => SuggestedMatch::Voucher(v),
                    (None, Some(p), _) => SuggestedMatch::Proposal(p),
                    (None, None, e) => SuggestedMatch::Explanation(e.unwrap_or_default()),
                };
                Ok(MatchSuggestion {
                    id: r.get(0)?,
                    bank_row_id: r.get(1)?,
                    target,
                    source: r.get(5)?,
                    actor: r.get(6)?,
                    confidence: r.get(7)?,
                    model: r.get(8)?,
                    rationale: r.get(9)?,
                    created_at: r.get(10)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Bokförda verifikationer som kan motsvara bankraden: exakt belopp på
    /// bankens konto, datum inom `window_days` och inte redan avstämda.
    /// Bäst först: nära i tid, och referens eller motpart som känns igen.
    pub fn match_candidates(
        &self,
        bank_row_id: i64,
        window_days: i64,
    ) -> Result<Vec<MatchCandidate>> {
        let conn = self.conn();
        let row = bank_row_by_id(conn, bank_row_id)?;
        if row.amount.is_zero() {
            return Ok(Vec::new());
        }
        let from = row.booking_date - chrono::Duration::days(window_days);
        let to = row.booking_date + chrono::Duration::days(window_days);
        let mut stmt = conn.prepare(
            "SELECT v.id, v.rakenskapsar_id, v.serie, v.nummer, v.datum, v.beskrivning, v.motpart_namn
             FROM verifikationer v
             WHERE v.datum BETWEEN ?1 AND ?2
               AND EXISTS (SELECT 1 FROM rader r0 WHERE r0.verifikation_id = v.id AND r0.konto = ?3)
               AND (SELECT COALESCE(SUM(r.debet - r.kredit), 0) FROM rader r
                    WHERE r.verifikation_id = v.id AND r.konto = ?3) = ?4
               AND NOT EXISTS (SELECT 1 FROM bankmatchningar m WHERE m.verifikation_id = v.id)
             ORDER BY v.id",
        )?;
        let found: Vec<CandidateRow> = stmt
            .query_map(
                params![
                    from.to_string(),
                    to.to_string(),
                    i64::from(row.account),
                    row.amount.ore()
                ],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                    ))
                },
            )?
            .collect::<rusqlite::Result<_>>()?;
        let bank_text = format!(
            "{} {} {}",
            row.text,
            row.reference.as_deref().unwrap_or(""),
            row.counterparty.as_deref().unwrap_or("")
        );
        let bank_tokens = tokens(&bank_text);
        let bank_digits = digits(&bank_text);
        let mut candidates = Vec::new();
        for (id, year, series, number, date, description, counterparty) in found {
            let date: NaiveDate = parse_date(&date)?;
            let days_apart = (date - row.booking_date).num_days();
            let voucher_text = format!("{description} {}", counterparty.as_deref().unwrap_or(""));
            let voucher_tokens = tokens(&voucher_text);
            let mut score = 100 - 2 * days_apart.abs();
            if bank_digits
                .iter()
                .any(|d| voucher_text.contains(d.as_str()))
            {
                score += 30;
            }
            if bank_tokens.iter().any(|t| voucher_tokens.contains(t)) {
                score += 20;
            }
            candidates.push(MatchCandidate {
                voucher_id: id,
                fiscal_year_id: year,
                reference: VoucherRef {
                    series: parse_series(&series)?,
                    number,
                },
                date,
                description,
                counterparty,
                days_apart,
                score,
            });
        }
        candidates.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then(a.days_apart.abs().cmp(&b.days_apart.abs()))
                .then(a.voucher_id.cmp(&b.voucher_id))
        });
        Ok(candidates)
    }
}

/// Antal bankrader med bokföringsdag i perioden som inte är avstämda.
pub(crate) fn unreconciled_in(conn: &Connection, start: NaiveDate, end: NaiveDate) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM bankrader b
         WHERE b.bokforingsdag BETWEEN ?1 AND ?2
           AND NOT EXISTS (SELECT 1 FROM bankmatchningar m WHERE m.bankrad_id = b.id)",
        params![start.to_string(), end.to_string()],
        |r| r.get(0),
    )?)
}
