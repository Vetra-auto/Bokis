//! Årets avskrivningar ur inventarieregistret som förslag, och
//! avstämningen av registret mot huvudboken.

use bok_core::{AccountNumber, Money};
use bok_inventarier::{DepreciationRow, depreciation_lines, register_balances, year_depreciation};
use bok_store::{Actor, FiscalYear, Provenance, Store};

use crate::proposal::{Proposed, propose};
use crate::{ClosingError, Result};

/// Ett konto där registret och huvudboken jämförs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterCheck {
    /// Kontot (tillgång eller ackumulerade avskrivningar).
    pub account: AccountNumber,
    /// Värdet enligt registret och planen, bokföringens tecken.
    pub register: Money,
    /// Bokfört saldo.
    pub ledger: Money,
}

impl RegisterCheck {
    /// Bokfört minus registrets värde.
    pub fn difference(&self) -> Money {
        Money::from_ore(self.ledger.ore() - self.register.ore())
    }
}

/// Registret mot huvudboken vid räkenskapsårets slut, för varje konto som
/// registret använder.
pub fn reconcile(store: &Store, year: &FiscalYear) -> Result<Vec<RegisterCheck>> {
    let assets = store.assets()?;
    let mut accounts: Vec<AccountNumber> = assets
        .iter()
        .flat_map(|a| [a.details.accounts.asset, a.details.accounts.accumulated])
        .collect();
    accounts.sort();
    accounts.dedup();
    let register = register_balances(&assets, year.end);
    accounts
        .into_iter()
        .map(|account| {
            Ok(RegisterCheck {
                account,
                register: register.get(&account).copied().unwrap_or(Money::ZERO),
                ledger: store.account_balance(account, year.end)?,
            })
        })
        .collect()
}

/// Lägger årets avskrivningar som förslag. Vägrar om de redan är bokförda,
/// det vill säga om huvudboken redan visar planens ackumulerade belopp.
pub fn propose_depreciation(
    store: &mut Store,
    actor: &Actor,
    year: &FiscalYear,
    provenance: Provenance,
) -> Result<(Proposed, Vec<DepreciationRow>)> {
    let rows = year_depreciation(&store.assets()?, year.start, year.end);
    if rows.is_empty() {
        return Err(ClosingError::Invalid(format!(
            "inga inventarier i registret ska skrivas av {}",
            year.label()
        )));
    }
    let mut notes = Vec::new();
    let checks = reconcile(store, year)?;
    let mut already = true;
    for account in rows.iter().map(|r| r.accumulated_account) {
        let planned: i64 = rows
            .iter()
            .filter(|r| r.accumulated_account == account)
            .map(|r| r.amount.ore())
            .sum();
        let Some(c) = checks.iter().find(|c| c.account == account) else {
            continue;
        };
        // Registret visar läget efter årets avskrivning (kredit, negativt).
        let before = c.register.ore() + planned;
        if c.ledger.ore() != c.register.ore() {
            already = false;
        }
        if c.ledger.ore() != before && c.ledger.ore() != c.register.ore() {
            notes.push(format!(
                "Konto {account} har saldot {} men registret väntar {} före årets avskrivning; stäm av registret mot huvudboken.",
                c.ledger,
                Money::from_ore(before)
            ));
        }
    }
    if already {
        return Err(ClosingError::Invalid(format!(
            "årets avskrivningar enligt plan är redan bokförda för {}",
            year.label()
        )));
    }
    let id = propose(
        store,
        actor,
        "avskrivning",
        year.end,
        format!("Avskrivningar enligt plan {}", year.label()),
        depreciation_lines(&rows),
        provenance,
    )?;
    Ok((Proposed { id, notes }, rows))
}
