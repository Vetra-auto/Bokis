//! Kontoavstämning mot ett externt saldo: skattekontot (1630) mot
//! Skatteverkets saldo, ett bankkonto mot kontoutdraget, ett lån mot
//! långivarens besked. Det bokförda saldot räknas fram, skillnaden visas och
//! avstämningen loggas i behandlingshistoriken med båda saldona.

use chrono::NaiveDate;
use rusqlite::params;
use serde_json::json;

use bok_core::{AccountNumber, Money};

use super::require_human;
use crate::model::*;
use crate::write::{Actor, EventDraft};
use crate::{Result, Store, StoreError};

impl Store {
    /// Kontots bokförda saldo vid utgången av `at`, inom räkenskapsåret som
    /// omfattar dagen: ingående balans (serie I) plus alla rader daterade
    /// till och med `at`.
    pub fn account_balance(&self, account: AccountNumber, at: NaiveDate) -> Result<Money> {
        let year = self
            .fiscal_year_for_date(at)?
            .ok_or_else(|| StoreError::Invalid(format!("inget räkenskapsår omfattar {at}")))?;
        let net: i64 = self.conn().query_row(
            "SELECT COALESCE(SUM(r.debet - r.kredit), 0)
             FROM rader r JOIN verifikationer v ON v.id = r.verifikation_id
             WHERE v.rakenskapsar_id = ?1 AND r.konto = ?2 AND (v.serie = 'I' OR v.datum <= ?3)",
            params![year.id, i64::from(account), at.to_string()],
            |r| r.get(0),
        )?;
        Ok(Money::from_ore(net))
    }

    /// Stämmer av ett konto mot ett externt saldo och loggar det. Saldot anges
    /// med bokföringens tecken (debet positivt): ett tillgodohavande på
    /// skattekontot är positivt. Ger avstämningen; en skillnad är inget fel i
    /// sig utan något att utreda.
    pub fn record_reconciliation(
        &mut self,
        actor: &Actor,
        account: AccountNumber,
        at: NaiveDate,
        external: Money,
        note: Option<&str>,
    ) -> Result<AccountReconciliation> {
        require_human(actor, "stämma av konton")?;
        let booked = self.account_balance(account, at)?;
        let difference = booked
            .checked_sub(external)
            .ok_or(bok_core::voucher::VoucherError::Overflow)?;
        let note = note.map(str::trim).filter(|n| !n.is_empty());
        let result = AccountReconciliation {
            account,
            date: at,
            booked,
            external,
            difference,
        };
        self.write(actor, |_ctx| {
            let event = EventDraft::new("konto.avstamt", format!("konto:{}", i64::from(account)))
                .new_values(json!({
                    "konto": i64::from(account),
                    "datum": at.to_string(),
                    "bokfort_ore": booked.ore(),
                    "externt_ore": external.ore(),
                    "skillnad_ore": difference.ore(),
                    "notering": note,
                }));
            Ok((result.clone(), event))
        })
    }

    /// Gjorda avstämningar ur behandlingshistoriken, äldst först.
    pub fn reconciliations(&self) -> Result<Vec<(String, String, AccountReconciliation)>> {
        let mut out = Vec::new();
        for e in self.events(None)? {
            if e.action != "konto.avstamt" {
                continue;
            }
            let Some(v) = e
                .new_values
                .as_deref()
                .and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok())
            else {
                continue;
            };
            let (Some(account), Some(date), Some(booked), Some(external), Some(difference)) = (
                v["konto"].as_i64(),
                v["datum"].as_str(),
                v["bokfort_ore"].as_i64(),
                v["externt_ore"].as_i64(),
                v["skillnad_ore"].as_i64(),
            ) else {
                continue;
            };
            let Ok(date) = NaiveDate::parse_from_str(date, "%Y-%m-%d") else {
                continue;
            };
            out.push((
                e.time.clone(),
                e.actor.clone(),
                AccountReconciliation {
                    account: AccountNumber::new(account)?,
                    date,
                    booked: Money::from_ore(booked),
                    external: Money::from_ore(external),
                    difference: Money::from_ore(difference),
                },
            ));
        }
        Ok(out)
    }
}
