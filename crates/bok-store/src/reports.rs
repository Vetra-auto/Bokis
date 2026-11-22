//! Läsningar och rapporter: grundbok, huvudbok, saldobalans och historik.

use std::collections::HashMap;

use chrono::NaiveDate;
use rusqlite::{Connection, OptionalExtension, params};

use bok_core::voucher::{Counterparty, VoucherError};
use bok_core::{AccountNumber, Money, Series, VoucherRef};

use crate::model::*;
use crate::ops::{
    KEY_COLUMNS, PROPOSAL_COLUMNS, account_from_row, account_name, decision_for, fiscal_year_by_id,
    fiscal_year_from_row, key_from_row, parse_series, period_by_id, period_for_date,
    period_from_row, proposal_by_id, proposal_from_row, voucher_lines,
};
use crate::{Result, Store, StoreError, parse_date};

fn correction_link(r: &rusqlite::Row<'_>) -> rusqlite::Result<CorrectionLink> {
    Ok(CorrectionLink {
        voucher_id: r.get(0)?,
        fiscal_year_id: r.get(1)?,
        reference: VoucherRef {
            series: parse_series(&r.get::<_, String>(2)?)?,
            number: r.get(3)?,
        },
        reason: r.get(4)?,
    })
}

pub(crate) fn voucher_view(conn: &Connection, id: i64) -> Result<VoucherView> {
    let mut view = conn
        .query_row(
            "SELECT v.id, v.rakenskapsar_id, v.serie, v.nummer, v.period_id, v.datum, v.sammanstallningsdag,
                    v.registrerad, v.beskrivning, v.motpart_namn, v.motpart_orgnr, v.motpart_momsnr, v.forslag_id,
                    v.kanonisk_hash, v.signatur, n.namn, n.fingeravtryck, v.kedjehash, v.motpart_adress
             FROM verifikationer v JOIN nycklar n ON n.id = v.nyckel_id WHERE v.id = ?1",
            params![id],
            |r| {
                let counterparty = r.get::<_, Option<String>>(9)?.map(|name| Counterparty {
                    name,
                    org_number: r.get(10).ok().flatten(),
                    vat_number: r.get(11).ok().flatten(),
                    address: r.get(18).ok().flatten(),
                });
                Ok(VoucherView {
                    id: r.get(0)?,
                    fiscal_year_id: r.get(1)?,
                    reference: VoucherRef { series: parse_series(&r.get::<_, String>(2)?)?, number: r.get(3)? },
                    period_id: r.get(4)?,
                    date: parse_date(&r.get::<_, String>(5)?)?,
                    compiled_on: parse_date(&r.get::<_, String>(6)?)?,
                    registered_at: r.get(7)?,
                    description: r.get(8)?,
                    counterparty,
                    proposal_id: r.get(12)?,
                    lines: Vec::new(),
                    canonical_hash: r.get(13)?,
                    signature: r.get(14)?,
                    key_name: r.get(15)?,
                    key_fingerprint: r.get(16)?,
                    chain_hash: r.get(17)?,
                    corrected_by: Vec::new(),
                    corrects: None,
                    attachments: Vec::new(),
                })
            },
        )
        .optional()?
        .ok_or_else(|| StoreError::missing("verifikation", id))?;
    view.lines = voucher_lines(conn, id)?;
    let mut stmt = conn.prepare_cached(
        "SELECT r.rattelse_id, v.rakenskapsar_id, v.serie, v.nummer, r.orsak
         FROM rattelser r JOIN verifikationer v ON v.id = r.rattelse_id
         WHERE r.original_id = ?1 ORDER BY r.rattelse_id",
    )?;
    view.corrected_by = stmt
        .query_map(params![id], correction_link)?
        .collect::<rusqlite::Result<_>>()?;
    view.corrects = conn
        .query_row(
            "SELECT r.original_id, v.rakenskapsar_id, v.serie, v.nummer, r.orsak
             FROM rattelser r JOIN verifikationer v ON v.id = r.original_id WHERE r.rattelse_id = ?1",
            params![id],
            correction_link,
        )
        .optional()?;
    view.attachments = attachment_hashes(conn, id)?;
    Ok(view)
}

/// SHA-256 för underlagen som länkades när verifikationen bokfördes.
pub(crate) fn attachment_hashes(conn: &Connection, voucher_id: i64) -> Result<Vec<String>> {
    let mut stmt = conn.prepare_cached(
        "SELECT u.sha256 FROM underlag_status s JOIN underlag u ON u.id = s.underlag_id
         WHERE s.verifikation_id = ?1 AND s.status = 'bokford' ORDER BY u.sha256",
    )?;
    let hashes = stmt
        .query_map(params![voucher_id], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(hashes)
}

impl Store {
    pub fn company(&self) -> Result<Company> {
        self.conn()
            .query_row(
                "SELECT namn, orgnr FROM foretag ORDER BY id DESC LIMIT 1",
                [],
                |r| {
                    Ok(Company {
                        name: r.get(0)?,
                        org_number: r.get(1)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::missing("företag", ""))
    }

    pub fn keys(&self) -> Result<Vec<KeyRow>> {
        let mut stmt = self
            .conn()
            .prepare(&format!("SELECT {KEY_COLUMNS} FROM nycklar ORDER BY id"))?;
        let keys = stmt
            .query_map([], key_from_row)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(keys)
    }

    pub fn fiscal_years(&self) -> Result<Vec<FiscalYear>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT id, start, slut FROM rakenskapsar ORDER BY start")?;
        let years = stmt
            .query_map([], fiscal_year_from_row)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(years)
    }

    pub fn fiscal_year(&self, id: i64) -> Result<FiscalYear> {
        fiscal_year_by_id(self.conn(), id)
    }

    pub fn fiscal_year_for_date(&self, date: NaiveDate) -> Result<Option<FiscalYear>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT id, start, slut FROM rakenskapsar WHERE start <= ?1 AND slut >= ?1",
                params![date.to_string()],
                fiscal_year_from_row,
            )
            .optional()?)
    }

    pub fn periods(&self, fiscal_year_id: i64) -> Result<Vec<Period>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, rakenskapsar_id, start, slut, status FROM perioder WHERE rakenskapsar_id = ?1 ORDER BY start",
        )?;
        let periods = stmt
            .query_map(params![fiscal_year_id], period_from_row)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(periods)
    }

    pub fn period(&self, id: i64) -> Result<Period> {
        period_by_id(self.conn(), id)
    }

    pub fn period_for_date(&self, date: NaiveDate) -> Result<Option<Period>> {
        period_for_date(self.conn(), date)
    }

    /// Perioden som börjar i månaden "YYYY-MM".
    pub fn period_by_label(&self, label: &str) -> Result<Period> {
        let first = NaiveDate::parse_from_str(&format!("{}-01", label.trim()), "%Y-%m-%d")
            .map_err(|_| {
                StoreError::Invalid(format!(
                    "\"{label}\" är ingen period (skriv till exempel 2026-03)"
                ))
            })?;
        self.conn()
            .query_row(
                "SELECT id, rakenskapsar_id, start, slut, status FROM perioder WHERE start = ?1",
                params![first.to_string()],
                period_from_row,
            )
            .optional()?
            .ok_or_else(|| StoreError::missing("period", label))
    }

    /// Kontoplanen som den ser ut vid `at`.
    pub fn chart(&self, at: NaiveDate) -> Result<Vec<AccountDef>> {
        let mut stmt = self.conn().prepare(
            "SELECT k.konto, k.namn, k.aktiv, k.sru, k.momskod, k.giltig_fran FROM kontoplan k
             WHERE k.giltig_fran = (SELECT MAX(k2.giltig_fran) FROM kontoplan k2
                                    WHERE k2.konto = k.konto AND k2.giltig_fran <= ?1)
             ORDER BY k.konto",
        )?;
        let accounts = stmt
            .query_map(params![at.to_string()], account_from_row)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(accounts)
    }

    pub fn counterparties(&self) -> Result<Vec<CounterpartyRow>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT id, namn, orgnr, momsnr, adress FROM motparter ORDER BY namn, id")?;
        let rows = stmt
            .query_map([], |r| {
                Ok(CounterpartyRow {
                    id: r.get(0)?,
                    counterparty: Counterparty {
                        name: r.get(1)?,
                        org_number: r.get(2)?,
                        vat_number: r.get(3)?,
                        address: r.get(4)?,
                    },
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn attachments(&self) -> Result<Vec<AttachmentRow>> {
        let mut stmt = self.conn().prepare(
            "SELECT u.id, u.sha256, u.storlek, u.mottagen, u.kanal, u.avsandare, u.originalnamn, u.filtyp,
                    s.status, s.text
             FROM underlag u
             JOIN underlag_status s ON s.id = (SELECT MAX(id) FROM underlag_status WHERE underlag_id = u.id)
             ORDER BY u.id",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(AttachmentRow {
                    id: r.get(0)?,
                    sha256: r.get(1)?,
                    size: r.get(2)?,
                    received_at: r.get(3)?,
                    channel: r.get(4)?,
                    sender: r.get(5)?,
                    original_name: r.get(6)?,
                    file_type: r.get(7)?,
                    status: r.get(8)?,
                    status_text: r.get(9)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Förslagen, äldst först. Med `pending_only` bara de utan beslut.
    pub fn proposals(&self, pending_only: bool) -> Result<Vec<ProposalRow>> {
        let sql = format!(
            "SELECT {PROPOSAL_COLUMNS} FROM forslag f {} ORDER BY f.id",
            if pending_only {
                "WHERE NOT EXISTS (SELECT 1 FROM beslut b WHERE b.forslag_id = f.id)"
            } else {
                ""
            }
        );
        let mut stmt = self.conn().prepare(&sql)?;
        let mut rows: Vec<ProposalRow> = stmt
            .query_map([], proposal_from_row)?
            .collect::<rusqlite::Result<_>>()?;
        for row in &mut rows {
            row.decision = decision_for(self.conn(), row.id)?;
        }
        Ok(rows)
    }

    pub fn proposal(&self, id: i64) -> Result<ProposalRow> {
        proposal_by_id(self.conn(), id)
    }

    /// Reglerna som gäller vid `at`: för varje regelnamn den högsta version
    /// vars giltighet (`giltig_fran` till och med `giltig_till`) omfattar
    /// datumet. Sorterade efter namn.
    pub fn rules(&self, at: NaiveDate) -> Result<Vec<RuleRow>> {
        let mut stmt = self.conn().prepare(
            "SELECT r.namn, r.version, r.giltig_fran, r.giltig_till, r.innehall FROM regler r
             WHERE r.giltig_fran <= ?1 AND (r.giltig_till IS NULL OR r.giltig_till >= ?1)
               AND r.version = (SELECT MAX(r2.version) FROM regler r2
                                WHERE r2.namn = r.namn AND r2.giltig_fran <= ?1
                                  AND (r2.giltig_till IS NULL OR r2.giltig_till >= ?1))
             ORDER BY r.namn",
        )?;
        let rows = stmt
            .query_map(params![at.to_string()], |r| {
                Ok(RuleRow {
                    name: r.get(0)?,
                    version: r.get(1)?,
                    valid_from: parse_date(&r.get::<_, String>(2)?)?,
                    valid_to: r
                        .get::<_, Option<String>>(3)?
                        .map(|d| parse_date(&d))
                        .transpose()?,
                    content: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn voucher(&self, id: i64) -> Result<VoucherView> {
        voucher_view(self.conn(), id)
    }

    pub fn voucher_by_ref(
        &self,
        fiscal_year_id: i64,
        reference: VoucherRef,
    ) -> Result<VoucherView> {
        let id: i64 = self
            .conn()
            .query_row(
                "SELECT id FROM verifikationer WHERE rakenskapsar_id = ?1 AND serie = ?2 AND nummer = ?3",
                params![fiscal_year_id, reference.series.as_str(), reference.number],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::missing("verifikation", reference))?;
        voucher_view(self.conn(), id)
    }

    /// Grundboken: årets verifikationer i registreringsordning. Inom varje
    /// serie är det också nummerordning, eftersom numret sätts vid
    /// bokföringen.
    pub fn journal(&self, fiscal_year_id: i64) -> Result<Vec<VoucherView>> {
        fiscal_year_by_id(self.conn(), fiscal_year_id)?;
        let mut stmt = self
            .conn()
            .prepare("SELECT id FROM verifikationer WHERE rakenskapsar_id = ?1 ORDER BY id")?;
        let ids: Vec<i64> = stmt
            .query_map(params![fiscal_year_id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        ids.into_iter()
            .map(|id| voucher_view(self.conn(), id))
            .collect()
    }

    /// Huvudboken per konto: ingående balans (serie I), sedan årets poster i
    /// registreringsordning med löpande saldo, och utgående balans.
    pub fn ledger(
        &self,
        fiscal_year_id: i64,
        account: Option<AccountNumber>,
    ) -> Result<Vec<LedgerAccount>> {
        let conn = self.conn();
        let year = fiscal_year_by_id(conn, fiscal_year_id)?;
        let corrections = corrections_by_original(conn)?;
        let mut stmt = conn.prepare(
            "SELECT r.konto, v.id, v.serie, v.nummer, v.datum, v.registrerad, COALESCE(r.text, v.beskrivning),
                    r.debet, r.kredit
             FROM rader r JOIN verifikationer v ON v.id = r.verifikation_id
             WHERE v.rakenskapsar_id = ?1 AND (?2 IS NULL OR r.konto = ?2)
             ORDER BY r.konto, v.id, r.radnr",
        )?;
        let rows = stmt.query_map(params![fiscal_year_id, account.map(i64::from)], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                parse_series(&r.get::<_, String>(2)?)?,
                r.get::<_, i64>(3)?,
                parse_date(&r.get::<_, String>(4)?)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                Money::from_ore(r.get(7)?),
                Money::from_ore(r.get(8)?),
            ))
        })?;
        let mut accounts: Vec<LedgerAccount> = Vec::new();
        for row in rows {
            let (konto, voucher_id, series, number, date, registered_at, text, debit, credit) =
                row?;
            let acc = AccountNumber::new(konto)?;
            if accounts.last().is_none_or(|a| a.account != acc) {
                accounts.push(LedgerAccount {
                    account: acc,
                    name: account_name(conn, acc, year.end)?,
                    opening: Money::ZERO,
                    entries: Vec::new(),
                    closing: Money::ZERO,
                });
            }
            let current = accounts.last_mut().expect("kontot lades till ovan");
            if series == Series::I {
                // Ingående balans står före årets poster även om serie I
                // bokfördes senare.
                current.opening = current
                    .opening
                    .checked_add(debit - credit)
                    .ok_or(VoucherError::Overflow)?;
                continue;
            }
            current.entries.push(LedgerEntry {
                voucher_id,
                reference: VoucherRef { series, number },
                date,
                registered_at,
                text,
                debit,
                credit,
                balance: Money::ZERO,
                corrected_by: corrections.get(&voucher_id).cloned().unwrap_or_default(),
            });
        }
        for a in &mut accounts {
            let mut balance = a.opening;
            for e in &mut a.entries {
                balance = balance
                    .checked_add(e.debit)
                    .and_then(|b| b.checked_sub(e.credit))
                    .ok_or(VoucherError::Overflow)?;
                e.balance = balance;
            }
            a.closing = balance;
        }
        Ok(accounts)
    }

    /// Saldobalans för ett räkenskapsår: bara bokförda rader i årets
    /// verifikationer. Förslag finns inte i `rader` alls.
    pub fn trial_balance(&self, fiscal_year_id: i64) -> Result<Vec<TrialBalanceRow>> {
        let conn = self.conn();
        let year = fiscal_year_by_id(conn, fiscal_year_id)?;
        let mut stmt = conn.prepare(
            "SELECT r.konto,
                    SUM(CASE WHEN v.serie = 'I' THEN r.debet - r.kredit ELSE 0 END),
                    SUM(CASE WHEN v.serie <> 'I' THEN r.debet ELSE 0 END),
                    SUM(CASE WHEN v.serie <> 'I' THEN r.kredit ELSE 0 END)
             FROM rader r JOIN verifikationer v ON v.id = r.verifikation_id
             WHERE v.rakenskapsar_id = ?1
             GROUP BY r.konto ORDER BY r.konto",
        )?;
        let rows = stmt.query_map(params![fiscal_year_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (konto, opening, debit, credit) = row?;
            let account = AccountNumber::new(konto)?;
            let closing = opening
                .checked_add(debit)
                .and_then(|x| x.checked_sub(credit))
                .ok_or(VoucherError::Overflow)?;
            out.push(TrialBalanceRow {
                account,
                name: account_name(conn, account, year.end)?,
                opening: Money::from_ore(opening),
                debit: Money::from_ore(debit),
                credit: Money::from_ore(credit),
                closing: Money::from_ore(closing),
            });
        }
        Ok(out)
    }

    /// Behandlingshistoriken i ordning. Med `limit` bara de senaste.
    pub fn events(&self, limit: Option<usize>) -> Result<Vec<EventRow>> {
        let total: i64 = self
            .conn()
            .query_row("SELECT COUNT(*) FROM handelser", [], |r| r.get(0))?;
        let skip = limit.map(|l| (total - l as i64).max(0)).unwrap_or(0);
        let mut stmt = self.conn().prepare(
            "SELECT seq, tid, aktor_typ, aktor, roll, handling, objekt, gamla, nya, programversion,
                    signatur IS NOT NULL, hash
             FROM handelser WHERE seq > ?1 ORDER BY seq",
        )?;
        let rows = stmt
            .query_map(params![skip], |r| {
                Ok(EventRow {
                    seq: r.get(0)?,
                    time: r.get(1)?,
                    actor_kind: r.get(2)?,
                    actor: r.get(3)?,
                    role: r.get(4)?,
                    action: r.get(5)?,
                    object: r.get(6)?,
                    old_values: r.get(7)?,
                    new_values: r.get(8)?,
                    program_version: r.get(9)?,
                    signed: r.get(10)?,
                    hash: r.get(11)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }
}

/// Rättelser per original, för "rättad genom R4" i alla vyer.
fn corrections_by_original(conn: &Connection) -> Result<HashMap<i64, Vec<CorrectionLink>>> {
    let mut stmt = conn.prepare(
        "SELECT r.original_id, r.rattelse_id, v.rakenskapsar_id, v.serie, v.nummer, r.orsak
         FROM rattelser r JOIN verifikationer v ON v.id = r.rattelse_id ORDER BY r.rattelse_id",
    )?;
    let mut map: HashMap<i64, Vec<CorrectionLink>> = HashMap::new();
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            CorrectionLink {
                voucher_id: r.get(1)?,
                fiscal_year_id: r.get(2)?,
                reference: VoucherRef {
                    series: parse_series(&r.get::<_, String>(3)?)?,
                    number: r.get(4)?,
                },
                reason: r.get(5)?,
            },
        ))
    })?;
    for row in rows {
        let (original, link) = row?;
        map.entry(original).or_default().push(link);
    }
    Ok(map)
}
