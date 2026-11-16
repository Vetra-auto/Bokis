//! Deklarationens indata ur lagret: företaget, räkenskapsåret, utgående
//! saldon från saldobalansen och kontoplanens SRU-koder vid årets slut.

use std::collections::HashMap;

use bok_deklaration::{AccountBalance, Input, ManualValue};
use bok_store::{FiscalYear, Store};

use crate::Result;

/// Indata för räkenskapsåret, med uppgifter som anges för hand.
pub fn declaration_input(
    store: &Store,
    year: &FiscalYear,
    manual: Vec<(String, ManualValue)>,
) -> Result<Input> {
    let company = store.company()?;
    let sru: HashMap<_, _> = store
        .chart(year.end)?
        .into_iter()
        .map(|a| (a.number, a.sru))
        .collect();
    let balances = store
        .trial_balance(year.id)?
        .into_iter()
        .filter(|r| !r.closing.is_zero())
        .map(|r| AccountBalance {
            account: r.account,
            sru: sru.get(&r.account).cloned().flatten(),
            name: r.name,
            balance: r.closing,
        })
        .collect();
    Ok(Input {
        company_name: company.name,
        org_number: company.org_number,
        year_start: year.start,
        year_end: year.end,
        balances,
        manual,
    })
}
