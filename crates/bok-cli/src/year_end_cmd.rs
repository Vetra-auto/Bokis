//! `bok bokslut avskrivningar|periodisera|skatt`: bokslutsposterna som
//! förslag i serie B, räknade av `bok-bokslut`.

use anyhow::{Context, Result, anyhow};
use clap::Args;

use bok_bokslut::accrual::{Accrual, AccrualKind};
use bok_store::Provenance;
use bok_store::bok_core::{AccountNumber, Money};

use crate::declaration_cmd::parse_manual;
use crate::inventory_cmd::{print_reconciliation, show_proposal};
use crate::{Ctx, input, render, resolve_year};

#[derive(Args)]
pub struct AccrualArgs {
    /// forutbetald-kostnad, upplupen-kostnad, forutbetald-intakt eller upplupen-intakt.
    #[arg(long)]
    kind: String,
    /// Kostnads- eller intäktskontot, till exempel 6310.
    #[arg(long)]
    account: AccountNumber,
    /// Hela beloppet på fakturan eller avtalet, utan avdragsgill moms.
    #[arg(long)]
    amount: String,
    /// Första dagen som beloppet avser.
    #[arg(long)]
    from: String,
    /// Sista dagen som beloppet avser.
    #[arg(long)]
    to: String,
    /// Vad det gäller, till exempel "Företagsförsäkring".
    #[arg(long)]
    text: String,
    /// Interimskontot [standard: 1790 för fordringar, 2990 för skulder].
    #[arg(long)]
    balance_account: Option<AccountNumber>,
    /// Räkenskapsåret [standard: året som omfattar i dag].
    #[arg(long)]
    year: Option<String>,
    /// Föreslår också återföringen på nästa års första dag.
    #[arg(long, visible_alias = "aterfor")]
    reverse: bool,
    /// Periodisera även ett belopp under K2-gränsen.
    #[arg(long, visible_alias = "anda")]
    anyway: bool,
}

#[derive(Args)]
pub struct TaxArgs {
    /// Räkenskapsåret [standard: året som omfattar i dag].
    #[arg(long)]
    year: Option<String>,
    /// Uppgift för hand till deklarationen, FÄLT=BELOPP, till exempel 4.14a=150000.
    #[arg(long = "falt", value_name = "FÄLT=VÄRDE")]
    fields: Vec<String>,
    /// Kostnadskontot för skatten.
    #[arg(long, default_value = "8910")]
    expense_account: AccountNumber,
    /// Skuldkontot för skatten.
    #[arg(long, default_value = "2512")]
    liability_account: AccountNumber,
}

pub fn depreciation(ctx: &Ctx, year: Option<&str>) -> Result<()> {
    let mut store = ctx.open()?;
    let y = resolve_year(&store, year)?;
    let (proposed, rows) = bok_bokslut::depreciation::propose_depreciation(
        &mut store,
        &ctx.actor(),
        &y,
        Provenance::default(),
    )?;
    println!("Avskrivningar enligt plan {}:", y.label());
    for r in &rows {
        println!(
            "  inventarie {:>3}  {:<30} {:>14}  bokfört värde efter {:>14}",
            r.asset_id,
            r.description,
            render::amount(r.amount),
            render::amount(r.book_value)
        );
    }
    println!();
    show_proposal(&store, &proposed)?;
    print_reconciliation(&store, &y)?;
    Ok(())
}

pub fn accrual(ctx: &Ctx, a: AccrualArgs) -> Result<()> {
    let kind = AccrualKind::parse(&a.kind).ok_or_else(|| {
        anyhow!(
            "{} är inget slag; skriv forutbetald-kostnad, upplupen-kostnad, forutbetald-intakt eller upplupen-intakt",
            a.kind
        )
    })?;
    let mut store = ctx.open()?;
    let y = resolve_year(&store, a.year.as_deref())?;
    let accrual = Accrual {
        kind,
        account: a.account,
        balance_account: a.balance_account.unwrap_or(kind.default_balance_account()),
        amount: Money::parse(&a.amount).with_context(|| format!("{} är inget belopp", a.amount))?,
        from: input::parse_date(&a.from)?,
        to: input::parse_date(&a.to)?,
        text: a.text,
    };
    let (proposed, _plan) = bok_bokslut::accrual::propose(
        &mut store,
        &ctx.actor(),
        &y,
        &accrual,
        a.reverse,
        a.anyway,
        Provenance::default(),
    )?;
    show_proposal(&store, &proposed)?;
    Ok(())
}

pub fn tax(ctx: &Ctx, a: TaxArgs) -> Result<()> {
    let mut store = ctx.open()?;
    let y = resolve_year(&store, a.year.as_deref())?;
    let manual = a
        .fields
        .iter()
        .map(|f| parse_manual(f))
        .collect::<Result<Vec<_>>>()?;
    let outcome = bok_bokslut::tax::propose_tax(
        &mut store,
        &ctx.actor(),
        &y,
        manual,
        a.expense_account,
        a.liability_account,
        Provenance::default(),
    )?;
    let t = outcome.declaration.tax;
    println!(
        "Överskott (INK2S 4.15) {} kr, beskattningsbar inkomst {} kr (avrundad nedåt till tiotal kronor, IL 1 kap. 7 §).",
        t.surplus, t.taxable
    );
    println!(
        "Bolagsskatt 20,6 % (IL 65 kap. 10 §): {}. Redan bokfört på {}: {}.",
        render::amount(Money::from_ore(t.tax_ore)),
        a.expense_account,
        render::amount(outcome.booked)
    );
    match outcome.proposed {
        None => println!("Rätt skatt är redan bokförd; inget förslag."),
        Some(p) => {
            println!();
            show_proposal(&store, &p)?;
        }
    }
    Ok(())
}
