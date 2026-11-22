//! Skrivande operationer och de uppslag de bygger på.

pub(crate) mod archive_plan;
pub(crate) mod assets;
mod bank;
mod cases;
mod closing;
pub(crate) mod insyn_access;
mod periods;
mod posting;
mod proposals;
mod reconcile;
pub(crate) mod rules;
pub(crate) mod security;
mod setup;

use chrono::{DateTime, NaiveDate};
use rusqlite::{Connection, OptionalExtension, Row, params};

use bok_core::signature::{self, signer_fingerprint};
use bok_core::voucher::{CorrectedOriginal, Counterparty, Line};
use bok_core::{AccountNumber, Money, PeriodStatus, Series, VoucherRef};

use crate::model::*;
use crate::write::Actor;
use crate::{Result, StoreError, local_date, parse_date};

/// Ett underlag som granskaren har förklarat som dubblett eller som inte
/// räkenskapsinformation kan inte ligga till grund för en verifikation.
pub(crate) fn check_attachment_usable(conn: &Connection, id: i64) -> Result<()> {
    let status: Option<String> = conn
        .query_row(
            "SELECT status FROM underlag_status WHERE underlag_id = ?1 ORDER BY id DESC LIMIT 1",
            params![id],
            |r| r.get(0),
        )
        .optional()?;
    match status.as_deref() {
        Some(s @ ("dubblett" | "ej_rakenskapsinformation")) => Err(StoreError::Invalid(format!(
            "underlag {id} är markerat som {s} och kan inte ligga till grund för en verifikation"
        ))),
        _ => Ok(()),
    }
}

pub(crate) fn require_human(actor: &Actor, what: &'static str) -> Result<()> {
    if actor.kind == bok_core::period::ActorKind::Human {
        Ok(())
    } else {
        Err(StoreError::NotAllowed(what))
    }
}

pub(crate) fn fiscal_year_by_id(conn: &Connection, id: i64) -> Result<FiscalYear> {
    conn.query_row(
        "SELECT id, start, slut FROM rakenskapsar WHERE id = ?1",
        params![id],
        fiscal_year_from_row,
    )
    .optional()?
    .ok_or_else(|| StoreError::missing("räkenskapsår", id))
}

pub(crate) fn fiscal_year_from_row(r: &Row<'_>) -> rusqlite::Result<FiscalYear> {
    Ok(FiscalYear {
        id: r.get(0)?,
        start: parse_date(&r.get::<_, String>(1)?)?,
        end: parse_date(&r.get::<_, String>(2)?)?,
    })
}

pub(crate) fn period_from_row(r: &Row<'_>) -> rusqlite::Result<Period> {
    let status: String = r.get(4)?;
    Ok(Period {
        id: r.get(0)?,
        fiscal_year_id: r.get(1)?,
        start: parse_date(&r.get::<_, String>(2)?)?,
        end: parse_date(&r.get::<_, String>(3)?)?,
        status: PeriodStatus::from_db_str(&status).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e))
        })?,
    })
}

const PERIOD_COLUMNS: &str = "id, rakenskapsar_id, start, slut, status";

pub(crate) fn period_by_id(conn: &Connection, id: i64) -> Result<Period> {
    conn.query_row(
        &format!("SELECT {PERIOD_COLUMNS} FROM perioder WHERE id = ?1"),
        params![id],
        period_from_row,
    )
    .optional()?
    .ok_or_else(|| StoreError::missing("period", id))
}

pub(crate) fn period_for_date(conn: &Connection, date: NaiveDate) -> Result<Option<Period>> {
    Ok(conn
        .query_row(
            &format!("SELECT {PERIOD_COLUMNS} FROM perioder WHERE start <= ?1 AND slut >= ?1"),
            params![date.to_string()],
            period_from_row,
        )
        .optional()?)
}

/// Kontot som det ser ut i kontoplanen vid `date`.
pub(crate) fn account_at(
    conn: &Connection,
    number: AccountNumber,
    date: NaiveDate,
) -> Result<Option<AccountDef>> {
    Ok(conn
        .query_row(
            "SELECT konto, namn, aktiv, sru, momskod, giltig_fran FROM kontoplan
             WHERE konto = ?1 AND giltig_fran <= ?2
             ORDER BY giltig_fran DESC LIMIT 1",
            params![i64::from(number), date.to_string()],
            account_from_row,
        )
        .optional()?)
}

pub(crate) fn account_from_row(r: &Row<'_>) -> rusqlite::Result<AccountDef> {
    let number: i64 = r.get(0)?;
    Ok(AccountDef {
        number: AccountNumber::new(number).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Integer,
                Box::new(e),
            )
        })?,
        name: r.get(1)?,
        active: r.get::<_, i64>(2)? == 1,
        sru: r.get(3)?,
        vat_code: r.get(4)?,
        valid_from: parse_date(&r.get::<_, String>(5)?)?,
    })
}

/// Senaste namnet på ett konto, oavsett datum. För visning.
pub(crate) fn account_name(
    conn: &Connection,
    number: AccountNumber,
    at: NaiveDate,
) -> Result<String> {
    if let Some(def) = account_at(conn, number, at)? {
        return Ok(def.name);
    }
    Ok(conn
        .query_row(
            "SELECT namn FROM kontoplan WHERE konto = ?1 ORDER BY giltig_fran DESC LIMIT 1",
            params![i64::from(number)],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or_default())
}

/// Varje konto måste finnas i kontoplanen vid datumet och, om
/// `require_active`, vara aktivt.
pub(crate) fn check_accounts(
    conn: &Connection,
    lines: &[Line],
    date: NaiveDate,
    require_active: bool,
) -> Result<()> {
    for line in lines {
        match account_at(conn, line.account, date)? {
            None => return Err(StoreError::UnknownAccount(line.account, date)),
            Some(def) if require_active && !def.active => {
                return Err(StoreError::InactiveAccount(line.account, date));
            }
            Some(_) => {}
        }
    }
    Ok(())
}

pub(crate) fn key_from_row(r: &Row<'_>) -> rusqlite::Result<KeyRow> {
    Ok(KeyRow {
        id: r.get(0)?,
        name: r.get(1)?,
        public_key: r.get(2)?,
        fingerprint: r.get(3)?,
        algorithm: r.get(4)?,
        hardware: r.get::<_, i64>(5)? == 1,
        registered_at: r.get(6)?,
    })
}

pub(crate) const KEY_COLUMNS: &str =
    "id, namn, publik_nyckel, fingeravtryck, algoritm, hardvara, registrerad";

pub(crate) fn key_by_id(conn: &Connection, id: i64) -> Result<KeyRow> {
    conn.query_row(
        &format!("SELECT {KEY_COLUMNS} FROM nycklar WHERE id = ?1"),
        params![id],
        key_from_row,
    )
    .optional()?
    .ok_or_else(|| StoreError::missing("nyckel", id))
}

/// Slår upp nyckeln som gjorde signaturen bland de registrerade och
/// kontrollerar signaturen med den.
pub(crate) fn verify_registered(
    conn: &Connection,
    namespace: &str,
    message: &[u8],
    armored: &str,
) -> Result<KeyRow> {
    let fingerprint = signer_fingerprint(armored)?;
    let key = conn
        .query_row(
            &format!("SELECT {KEY_COLUMNS} FROM nycklar WHERE fingeravtryck = ?1"),
            params![fingerprint],
            key_from_row,
        )
        .optional()?
        .ok_or(StoreError::UnknownKey(fingerprint))?;
    signature::verify(&key.public_key, namespace, message, armored)?;
    // Varje ny signatur måste klara säkerhetsnivån som gäller.
    security::level_now(conn)?.check(key.hardware, signature::sk_flags(armored)?)?;
    Ok(key)
}

pub(crate) fn counterparty_by_id(conn: &Connection, id: i64) -> Result<Counterparty> {
    conn.query_row(
        "SELECT namn, orgnr, momsnr, adress FROM motparter WHERE id = ?1",
        params![id],
        |r| {
            Ok(Counterparty {
                name: r.get(0)?,
                org_number: r.get(1)?,
                vat_number: r.get(2)?,
                address: r.get(3)?,
            })
        },
    )
    .optional()?
    .ok_or_else(|| StoreError::missing("motpart", id))
}

pub(crate) fn parse_series(s: &str) -> rusqlite::Result<Series> {
    s.parse::<Series>().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

pub(crate) fn line_from_row(r: &Row<'_>) -> rusqlite::Result<Line> {
    let konto: i64 = r.get(0)?;
    Ok(Line {
        account: AccountNumber::new(konto).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Integer,
                Box::new(e),
            )
        })?,
        debit: Money::from_ore(r.get(1)?),
        credit: Money::from_ore(r.get(2)?),
        vat_code: r.get(3)?,
        text: r.get(4)?,
    })
}

pub(crate) fn voucher_lines(conn: &Connection, voucher_id: i64) -> Result<Vec<Line>> {
    let mut stmt = conn.prepare_cached(
        "SELECT konto, debet, kredit, momskod, text FROM rader WHERE verifikation_id = ?1 ORDER BY radnr",
    )?;
    let lines = stmt
        .query_map(params![voucher_id], line_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(lines)
}

/// Originalets uppgifter som en rättelse binder sig till.
pub(crate) fn corrected_original(conn: &Connection, original_id: i64) -> Result<CorrectedOriginal> {
    conn.query_row(
        "SELECT v.serie, v.nummer, r.start, v.kanonisk_hash
         FROM verifikationer v JOIN rakenskapsar r ON r.id = v.rakenskapsar_id WHERE v.id = ?1",
        params![original_id],
        |r| {
            Ok(CorrectedOriginal {
                reference: VoucherRef {
                    series: parse_series(&r.get::<_, String>(0)?)?,
                    number: r.get(1)?,
                },
                fiscal_year_start: parse_date(&r.get::<_, String>(2)?)?,
                canonical_hash: r.get(3)?,
            })
        },
    )
    .optional()?
    .ok_or_else(|| StoreError::missing("verifikation", original_id))
}

/// Kolumnerna som [`proposal_from_row`] läser, i den ordningen.
pub(crate) const PROPOSAL_COLUMNS: &str = "f.id, f.serie, f.datum, f.beskrivning, f.motpart_id, \
    f.motpart_namn, f.motpart_orgnr, f.motpart_momsnr, f.rader, f.underlag, f.rattar, \
    f.rattelseorsak, f.kalla, f.aktor, f.skapad, f.fragor, f.flaggor, f.sakerhet, f.modell, \
    f.leverantor, f.region, f.prompt_hash, f.regelversion, f.ratt_svar, f.motpart_adress";

pub(crate) fn proposal_by_id(conn: &Connection, id: i64) -> Result<ProposalRow> {
    let mut row = conn
        .query_row(
            &format!("SELECT {PROPOSAL_COLUMNS} FROM forslag f WHERE f.id = ?1"),
            params![id],
            proposal_from_row,
        )
        .optional()?
        .ok_or_else(|| StoreError::missing("förslag", id))?;
    row.decision = decision_for(conn, id)?;
    Ok(row)
}

pub(crate) fn proposal_from_row(r: &Row<'_>) -> rusqlite::Result<ProposalRow> {
    let json_err = |i: usize, e: serde_json::Error| {
        rusqlite::Error::FromSqlConversionFailure(i, rusqlite::types::Type::Text, Box::new(e))
    };
    let counterparty = match (r.get::<_, Option<i64>>(4)?, r.get::<_, Option<String>>(5)?) {
        (Some(id), _) => Some(ProposalCounterparty::Register(id)),
        (None, Some(name)) => Some(ProposalCounterparty::Inline(Counterparty {
            name,
            org_number: r.get(6)?,
            vat_number: r.get(7)?,
            address: r.get(24)?,
        })),
        (None, None) => None,
    };
    let lines: Vec<Line> =
        serde_json::from_str(&r.get::<_, String>(8)?).map_err(|e| json_err(8, e))?;
    let attachments: Vec<i64> =
        serde_json::from_str(&r.get::<_, String>(9)?).map_err(|e| json_err(9, e))?;
    Ok(ProposalRow {
        id: r.get(0)?,
        series: parse_series(&r.get::<_, String>(1)?)?,
        date: parse_date(&r.get::<_, String>(2)?)?,
        description: r.get(3)?,
        counterparty,
        lines,
        attachments,
        corrects: r.get(10)?,
        correction_reason: r.get(11)?,
        source: r.get(12)?,
        actor: r.get(13)?,
        created_at: r.get(14)?,
        provenance: Provenance {
            questions: r.get(15)?,
            flags: r.get(16)?,
            confidence: r.get(17)?,
            model: r.get(18)?,
            provider: r.get(19)?,
            region: r.get(20)?,
            prompt_hash: r.get(21)?,
            rule_version: r.get(22)?,
            raw_response: r.get(23)?,
        },
        decision: None,
    })
}

pub(crate) fn decision_for(conn: &Connection, proposal_id: i64) -> Result<Option<Decision>> {
    Ok(conn
        .query_row(
            "SELECT utfall, aktor, tid, orsak, andringar, verifikation_id FROM beslut WHERE forslag_id = ?1",
            params![proposal_id],
            |r| {
                Ok(Decision {
                    outcome: if r.get::<_, String>(0)? == "godkant" { Outcome::Approved } else { Outcome::Rejected },
                    actor: r.get(1)?,
                    time: r.get(2)?,
                    reason: r.get(3)?,
                    changes: r.get(4)?,
                    voucher_id: r.get(5)?,
                })
            },
        )
        .optional()?)
}

/// Lokalt datum för en lagrad tidpunkt.
pub(crate) fn stored_local_date(stored: &str) -> Result<NaiveDate> {
    let t = DateTime::parse_from_rfc3339(stored)
        .map_err(|e| StoreError::Invalid(format!("ogiltig tidpunkt {stored}: {e}")))?;
    Ok(local_date(t.to_utc()))
}
