//! Bokslut: förslag om årets resultat (serie B) och ingående balans för
//! nästa år (serie I). Båda är vanliga förslag; de bokförs först när en
//! människa godkänner dem med sin nyckel.

use bok_core::voucher::Line;
use bok_core::{AccountNumber, Money, Series};
use rusqlite::params;

use super::fiscal_year_by_id;
use crate::model::*;
use crate::write::Actor;
use crate::{Result, Store, StoreError};

impl Store {
    /// Årets resultat enligt resultatkontona (klass 3–8): positivt är vinst.
    /// När resultatet är bokfört mot eget kapital (serie B) är det noll.
    pub fn year_result(&self, fiscal_year_id: i64) -> Result<Money> {
        let net: i64 = self
            .trial_balance(fiscal_year_id)?
            .iter()
            .filter(|r| r.account.is_result_account())
            .map(|r| r.closing.ore())
            .sum();
        Ok(Money::from_ore(-net))
    }

    /// Föreslår bokslutsverifikationen som för årets resultat från
    /// `result_account` (normalt 8999) till `equity_account` (normalt 2099),
    /// daterad räkenskapsårets sista dag.
    pub fn propose_year_result(
        &mut self,
        actor: &Actor,
        fiscal_year_id: i64,
        result_account: AccountNumber,
        equity_account: AccountNumber,
    ) -> Result<i64> {
        let year = fiscal_year_by_id(self.conn(), fiscal_year_id)?;
        if !result_account.is_result_account() || !equity_account.is_balance_account() {
            return Err(StoreError::Invalid(format!(
                "resultatet förs från ett resultatkonto till ett balanskonto, inte från {result_account} till {equity_account}"
            )));
        }
        let result = self.year_result(fiscal_year_id)?;
        if result.is_zero() {
            return Err(StoreError::Invalid(format!(
                "årets resultat för {} är noll; det är redan bokfört eller året saknar resultatposter",
                year.label()
            )));
        }
        // Vinst: debet resultatkontot, kredit eget kapital. Förlust: tvärtom.
        let lines = vec![
            Line::from_signed(result_account, result),
            Line::from_signed(equity_account, -result),
        ];
        let proposal = NewProposal {
            series: Series::B,
            date: year.end,
            description: format!("Årets resultat {}", year.label()),
            counterparty: None,
            lines,
            attachments: vec![],
            idempotency_key: Some(format!(
                "bokslut:resultat:{fiscal_year_id}:{}",
                result.ore()
            )),
            provenance: Provenance::default(),
        };
        self.create_proposal(actor, &proposal)
    }

    /// Föreslår ingående balans för räkenskapsåret efter `fiscal_year_id`:
    /// varje balanskontos utgående saldo, daterat nya årets första dag, i
    /// serie I. Balanskontona måste gå jämnt ut, det vill säga årets resultat
    /// måste vara bokfört, och nästa år får inte redan ha en ingående balans.
    pub fn propose_opening_balances(&mut self, actor: &Actor, fiscal_year_id: i64) -> Result<i64> {
        let conn = self.conn();
        let from = fiscal_year_by_id(conn, fiscal_year_id)?;
        let next = self
            .fiscal_years()?
            .into_iter()
            .find(|y| from.end.succ_opt() == Some(y.start))
            .ok_or_else(|| {
                StoreError::Invalid(format!(
                    "det finns inget räkenskapsår som börjar dagen efter {}; lägg till det först",
                    from.end
                ))
            })?;
        let posted: i64 = conn.query_row(
            "SELECT COUNT(*) FROM verifikationer WHERE rakenskapsar_id = ?1 AND serie = 'I'",
            params![next.id],
            |r| r.get(0),
        )?;
        if posted > 0 {
            return Err(StoreError::Invalid(format!(
                "räkenskapsåret {} har redan en bokförd ingående balans",
                next.label()
            )));
        }
        let balances: Vec<(AccountNumber, Money)> = self
            .trial_balance(fiscal_year_id)?
            .into_iter()
            .filter(|r| r.account.is_balance_account() && !r.closing.is_zero())
            .map(|r| (r.account, r.closing))
            .collect();
        let sum: i64 = balances.iter().map(|(_, m)| m.ore()).sum();
        if sum != 0 {
            return Err(StoreError::Invalid(format!(
                "balanskontona för {} går inte jämnt ut (skillnad {}); bokför årets resultat ({}) i serie B först",
                from.label(),
                Money::from_ore(sum),
                self.year_result(fiscal_year_id)?
            )));
        }
        if balances.is_empty() {
            return Err(StoreError::Invalid(format!(
                "inga balanskonton har saldo vid utgången av {}",
                from.label()
            )));
        }
        let fingerprint: String = balances
            .iter()
            .map(|(a, m)| format!("{a}={}", m.ore()))
            .collect::<Vec<_>>()
            .join(",");
        let proposal = NewProposal {
            series: Series::I,
            date: next.start,
            description: format!("Ingående balans {} från {}", next.label(), from.label()),
            counterparty: None,
            lines: balances
                .iter()
                .map(|(a, m)| Line::from_signed(*a, *m))
                .collect(),
            attachments: vec![],
            idempotency_key: Some(format!(
                "bokslut:ib:{fiscal_year_id}:{}",
                bok_core::canonical::sha256_hex(fingerprint.as_bytes())
            )),
            provenance: Provenance::default(),
        };
        self.create_proposal(actor, &proposal)
    }
}
