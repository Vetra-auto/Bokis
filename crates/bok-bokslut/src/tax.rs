//! Bolagsskatten som förslag: deklarationens överskott ger skatten, och
//! förslaget bokför skillnaden mot det som redan är bokfört som skatt
//! (debet 8910 Skatt som belastar årets resultat, kredit 2512 Beräknad
//! inkomstskatt i BAS).
//!
//! Skatten räknas på samma sätt före och efter att den bokförts, eftersom
//! INK2S lägger tillbaka bokförd skatt (4.3a).

use bok_core::voucher::Line;
use bok_core::{AccountNumber, Money};
use bok_deklaration::{Declaration, ManualValue};
use bok_store::{Actor, FiscalYear, Provenance, Store};

use crate::input::declaration_input;
use crate::proposal::{Proposed, propose};
use crate::{ClosingError, Result};

/// Utfallet av skatteberäkningen.
#[derive(Debug, Clone)]
pub struct TaxOutcome {
    /// Förslaget, eller `None` om rätt skatt redan är bokförd.
    pub proposed: Option<Proposed>,
    /// Deklarationen som skatten bygger på.
    pub declaration: Declaration,
    /// Redan bokförd skatt på kostnadskontot.
    pub booked: Money,
}

/// Räknar bolagsskatten för räkenskapsåret och föreslår skillnaden mot den
/// bokförda.
pub fn propose_tax(
    store: &mut Store,
    actor: &Actor,
    year: &FiscalYear,
    manual: Vec<(String, ManualValue)>,
    expense: AccountNumber,
    liability: AccountNumber,
    provenance: Provenance,
) -> Result<TaxOutcome> {
    if !expense.is_result_account() || !liability.is_balance_account() {
        return Err(ClosingError::Invalid(format!(
            "skatten bokförs på ett resultatkonto mot ett balanskonto, inte {expense} och {liability}"
        )));
    }
    let input = declaration_input(store, year, manual)?;
    let declaration = Declaration::compute_bas(&input)?;
    let failed: Vec<String> = declaration
        .checks
        .iter()
        .filter(|c| !c.ok)
        .map(|c| c.name.clone())
        .collect();
    if !failed.is_empty() {
        return Err(ClosingError::Invalid(format!(
            "deklarationen klarar inte kontrollerna: {}",
            failed.join("; ")
        )));
    }
    let booked = store.account_balance(expense, year.end)?;
    let difference = Money::from_ore(declaration.tax.tax_ore - booked.ore());
    if difference.is_zero() {
        return Ok(TaxOutcome {
            proposed: None,
            declaration,
            booked,
        });
    }
    let text = format!(
        "20,6 % av {} kr",
        bok_deklaration::summary::grouped(declaration.tax.taxable)
    );
    let lines = vec![
        Line::from_signed(expense, difference).with_text(text.clone()),
        Line::from_signed(liability, -difference).with_text(text),
    ];
    let id = propose(
        store,
        actor,
        "skatt",
        year.end,
        format!("Beräknad inkomstskatt {}", year.label()),
        lines,
        provenance,
    )?;
    let mut notes = Vec::new();
    if !booked.is_zero() {
        notes.push(format!(
            "Redan bokförd skatt {booked}; förslaget bokför skillnaden {difference}."
        ));
    }
    Ok(TaxOutcome {
        proposed: Some(Proposed { id, notes }),
        declaration,
        booked,
    })
}
