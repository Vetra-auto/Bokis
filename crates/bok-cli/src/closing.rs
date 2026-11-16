//! `bok bokslut`: förslag om årets resultat, ingående balans, årets
//! avskrivningar, periodiseringar och bolagsskatt. Förslagen granskas och
//! bokförs som alla andra, med `bok godkann` och nyckeln. Avskrivningar,
//! periodiseringar och skatt hanteras i [`crate::year_end_cmd`].

use anyhow::Result;
use clap::Subcommand;

use bok_store::Actor;
use bok_store::bok_core::AccountNumber;

use crate::{Ctx, render, resolve_year};

#[derive(Subcommand)]
pub enum ClosingCmd {
    /// Föreslår bokslutsverifikationen som för årets resultat till eget kapital (serie B).
    #[command(visible_alias = "resultat")]
    Result {
        /// Räkenskapsåret [standard: året som omfattar i dag].
        #[arg(long)]
        year: Option<String>,
        /// Resultatkontot som nollställs.
        #[arg(long, default_value = "8999")]
        result_account: AccountNumber,
        /// Kontot i eget kapital som tar emot resultatet (2019 för enskild firma).
        #[arg(long, default_value = "2099")]
        equity_account: AccountNumber,
    },
    /// Föreslår ingående balans för året efter (serie I) ur balanskontonas utgående saldon.
    #[command(visible_alias = "ib")]
    OpeningBalance {
        /// Året vars utgående balans förs över [standard: året som omfattar i dag].
        #[arg(long)]
        year: Option<String>,
    },
    /// Föreslår årets avskrivningar enligt plan ur inventarieregistret (serie B).
    #[command(visible_alias = "avskrivningar")]
    Depreciation {
        /// Räkenskapsåret [standard: året som omfattar i dag].
        #[arg(long)]
        year: Option<String>,
    },
    /// Föreslår en periodisering av en förutbetald eller upplupen kostnad eller intäkt (serie B).
    #[command(visible_alias = "periodisera")]
    Accrual(crate::year_end_cmd::AccrualArgs),
    /// Räknar bolagsskatten ur deklarationens överskott och föreslår den (serie B).
    #[command(visible_alias = "skatt")]
    Tax(crate::year_end_cmd::TaxArgs),
}

pub fn closing(ctx: &Ctx, cmd: ClosingCmd) -> Result<()> {
    let cmd = match cmd {
        ClosingCmd::Depreciation { year } => {
            return crate::year_end_cmd::depreciation(ctx, year.as_deref());
        }
        ClosingCmd::Accrual(args) => return crate::year_end_cmd::accrual(ctx, args),
        ClosingCmd::Tax(args) => return crate::year_end_cmd::tax(ctx, args),
        other => other,
    };
    let mut store = ctx.open()?;
    let actor = Actor::human(ctx.user.clone());
    let id = match cmd {
        ClosingCmd::Result {
            year,
            result_account,
            equity_account,
        } => {
            let y = resolve_year(&store, year.as_deref())?;
            let result = store.year_result(y.id)?;
            println!(
                "Årets resultat {}: {} ({}).",
                y.label(),
                render::amount(result),
                if result.is_negative() {
                    "förlust"
                } else {
                    "vinst"
                }
            );
            store.propose_year_result(&actor, y.id, result_account, equity_account)?
        }
        ClosingCmd::OpeningBalance { year } => {
            let y = resolve_year(&store, year.as_deref())?;
            store.propose_opening_balances(&actor, y.id)?
        }
        ClosingCmd::Depreciation { .. } | ClosingCmd::Accrual(_) | ClosingCmd::Tax(_) => {
            unreachable!("hanteras ovan")
        }
    };
    render::proposal(&store, &store.proposal(id)?)?;
    println!();
    println!("Förslag {id} väntar på beslut. Bokför det med `bok godkann {id}`.");
    Ok(())
}
