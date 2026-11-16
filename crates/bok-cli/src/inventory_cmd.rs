//! `bok inventarie`: inventarieregistret. Registrera en anskaffning, lista
//! registret med bokfört värde, visa avskrivningsplanen och registrera en
//! försäljning, utrangering eller felregistrering.
//!
//! Registret bokför ingenting. Årets avskrivningar föreslås med
//! `bok bokslut avskrivningar`, och en avyttring blir ett förslag som
//! granskas och bokförs med `bok godkann`.

use anyhow::{Context, Result, anyhow, bail};
use clap::Subcommand;

use bok_bokslut::depreciation;
use bok_store::bok_core::{AccountNumber, Money, VoucherRef};
use bok_store::bok_inventarier::{
    Asset, AssetAccounts, DirectExpense, Disposal, DisposalAccounts, DisposalKind, NewAsset, Plan,
    direct_expense,
};
use bok_store::{Provenance, Store};

use crate::{Ctx, input, render, resolve_year};

#[derive(Subcommand)]
pub enum InventoryCmd {
    /// Registrerar ett inventarium som ska skrivas av enligt plan.
    #[command(visible_alias = "lagg-till")]
    Add {
        /// Vad det är, till exempel "Bärbar dator".
        #[arg(long)]
        name: String,
        /// Anskaffningsdagen.
        #[arg(long)]
        date: String,
        /// Anskaffningsvärdet utan avdragsgill moms, till exempel 36000,00.
        #[arg(long)]
        amount: String,
        /// Nyttjandeperioden i månader (K2 godtar fem år, 60, för inventarier).
        #[arg(long)]
        life: u32,
        /// Första månaden som skrivs av, ÅÅÅÅ-MM [standard: anskaffningsmånaden].
        #[arg(long)]
        from: Option<String>,
        /// Restvärde som inte skrivs av.
        #[arg(long, default_value = "0")]
        residual: String,
        /// Tillgångskontot.
        #[arg(long, default_value = "1220")]
        account: AccountNumber,
        /// Kontot för ackumulerade avskrivningar.
        #[arg(long, default_value = "1229")]
        depreciation_account: AccountNumber,
        /// Kostnadskontot för avskrivningen.
        #[arg(long, default_value = "7832")]
        expense_account: AccountNumber,
        /// Verifikationen som anskaffningen bokfördes på, till exempel A12.
        #[arg(long)]
        voucher: Option<VoucherRef>,
    },
    /// Listar registret med bokfört värde vid årets slut och stämmer av mot huvudboken.
    #[command(visible_alias = "lista")]
    List {
        /// Räkenskapsåret [standard: året som omfattar i dag].
        #[arg(long)]
        year: Option<String>,
    },
    /// Visar avskrivningsplanen för ett inventarium, år för år.
    Plan { id: i64 },
    /// Registrerar en försäljning, utrangering eller felregistrering och föreslår verifikationen.
    #[command(visible_alias = "avyttra")]
    Dispose {
        id: i64,
        /// Avgångsdagen.
        #[arg(long)]
        date: String,
        /// forsaljning, utrangering eller felregistrering.
        #[arg(long)]
        kind: String,
        /// Priset utan moms vid försäljning.
        #[arg(long)]
        price: Option<String>,
        /// Utgående moms enligt fakturan vid försäljning.
        #[arg(long, default_value = "0")]
        vat: String,
        /// En förklaring; krävs vid felregistrering.
        #[arg(long)]
        note: Option<String>,
        /// Dit betalningen kommer.
        #[arg(long, default_value = "1930")]
        payment_account: AccountNumber,
        #[arg(long, default_value = "2611")]
        vat_account: AccountNumber,
        #[arg(long, default_value = "3973")]
        gain_account: AccountNumber,
        #[arg(long, default_value = "7973")]
        loss_account: AccountNumber,
    },
}

fn money(s: &str) -> Result<Money> {
    Money::parse(s).with_context(|| format!("{s} är inget belopp"))
}

fn status(a: &Asset) -> String {
    match &a.disposal {
        None => "i bruk".into(),
        Some(d) => format!("{} {}", d.kind.as_str(), d.date),
    }
}

pub fn inventory(ctx: &Ctx, cmd: InventoryCmd) -> Result<()> {
    match cmd {
        InventoryCmd::Add {
            name,
            date,
            amount,
            life,
            from,
            residual,
            account,
            depreciation_account,
            expense_account,
            voucher,
        } => {
            let mut store = ctx.open()?;
            let acquired = input::parse_date(&date)?;
            let start = match from {
                Some(m) => input::parse_date(&format!("{m}-01"))?,
                None => NewAsset::default_start(acquired),
            };
            let asset = NewAsset {
                description: name,
                acquired,
                cost: money(&amount)?,
                life_months: life,
                depreciation_start: start,
                residual: money(&residual)?,
                accounts: AssetAccounts {
                    asset: account,
                    accumulated: depreciation_account,
                    expense: expense_account,
                },
            };
            let year = store
                .fiscal_year_for_date(acquired)?
                .ok_or_else(|| anyhow!("inget räkenskapsår omfattar {acquired}"))?;
            let voucher_id = match voucher {
                Some(r) => Some(store.voucher_by_ref(year.id, r)?.id),
                None => None,
            };
            use chrono::Datelike;
            match direct_expense(asset.cost, life, year.end.year()) {
                Some(DirectExpense::LowValue { limit }) => println!(
                    "Obs: anskaffningsvärdet understiger halva prisbasbeloppet ({}) och får dras av \
                     direkt som förbrukningsinventarie (IL 18 kap. 4 §, K2 punkt 10.5). Inventarier \
                     som hör ihop bedöms tillsammans.",
                    render::amount(limit)
                ),
                Some(DirectExpense::ShortLife) => println!(
                    "Obs: med en livslängd på högst tre år får inventariet dras av direkt \
                     (IL 18 kap. 4 §, K2 punkt 10.6)."
                ),
                Some(DirectExpense::No { .. }) => {}
                None => println!(
                    "Obs: prisbasbeloppet för {} saknas i Bokis; gränsen för direktavdrag är inte prövad.",
                    year.end.year()
                ),
            }
            let id = store.register_asset(&ctx.actor(), &asset, voucher_id)?;
            let plan = Plan::of(&asset);
            println!(
                "Inventarie {id} registrerat: {}, {} över {} månader från {}, {} i månaden.",
                asset.description,
                render::amount(asset.cost),
                life,
                start.format("%Y-%m"),
                render::amount(plan.accumulated_after(1))
            );
        }
        InventoryCmd::List { year } => {
            let store = ctx.open()?;
            let y = resolve_year(&store, year.as_deref())?;
            let assets = store.assets()?;
            if assets.is_empty() {
                println!("Registret är tomt. Lägg till med `bok inventarie lagg-till`.");
                return Ok(());
            }
            println!("Inventarier per {}", y.end);
            println!(
                "{:>3}  {:<28} {:<10} {:>14} {:>5} {:>14} {:>14}  status",
                "nr", "beskrivning", "anskaffad", "värde", "mån", "ack. avskr.", "bokfört värde"
            );
            for a in &assets {
                let plan = Plan::of(&a.details);
                let acc = if a.is_void() {
                    Money::ZERO
                } else {
                    plan.accumulated_through(y.end)
                };
                println!(
                    "{:>3}  {:<28} {:<10} {:>14} {:>5} {:>14} {:>14}  {}",
                    a.id,
                    truncate(&a.details.description, 28),
                    a.details.acquired,
                    render::amount(a.details.cost),
                    a.details.life_months,
                    render::amount(acc),
                    render::amount(Money::from_ore(a.details.cost.ore() - acc.ore())),
                    status(a)
                );
            }
            print_reconciliation(&store, &y)?;
        }
        InventoryCmd::Plan { id } => {
            let store = ctx.open()?;
            let a = store.asset(id)?;
            let plan = Plan::of(&a.details);
            println!(
                "Inventarie {id}: {} – {} över {} månader från {}",
                a.details.description,
                render::amount(a.details.cost),
                a.details.life_months,
                a.details.depreciation_start.format("%Y-%m")
            );
            let years = store.fiscal_years()?;
            let last = plan.last_month().unwrap_or(a.details.depreciation_start);
            for y in years
                .iter()
                .filter(|y| y.end >= a.details.depreciation_start)
            {
                let amount = plan.between(y.start, y.end);
                println!(
                    "  {:<8} avskrivning {:>14}  bokfört värde vid årets slut {:>14}",
                    y.label(),
                    render::amount(amount),
                    render::amount(Money::from_ore(
                        a.details.cost.ore() - plan.accumulated_through(y.end).ore()
                    ))
                );
            }
            println!("Sista månaden som skrivs av: {}.", last.format("%Y-%m"));
        }
        InventoryCmd::Dispose {
            id,
            date,
            kind,
            price,
            vat,
            note,
            payment_account,
            vat_account,
            gain_account,
            loss_account,
        } => {
            let mut store = ctx.open()?;
            let kind = match kind.as_str() {
                "forsaljning" | "försäljning" => DisposalKind::Sold {
                    price: money(price.as_deref().ok_or_else(|| {
                        anyhow!("ange priset utan moms med --price vid försäljning")
                    })?)?,
                },
                "utrangering" => DisposalKind::Scrapped,
                "felregistrering" => DisposalKind::Void,
                other => bail!(
                    "{other} är ingen avgång; skriv forsaljning, utrangering eller felregistrering"
                ),
            };
            let disposal = Disposal {
                date: input::parse_date(&date)?,
                kind,
                note,
            };
            let accounts = DisposalAccounts {
                payment: payment_account,
                output_vat: vat_account,
                gain: gain_account,
                loss: loss_account,
            };
            let out = bok_bokslut::disposal::dispose(
                &mut store,
                &ctx.actor(),
                id,
                &disposal,
                money(&vat)?,
                &accounts,
                Provenance::default(),
            )?;
            match out {
                None => println!("Inventarie {id} är struket ur registret som felregistrerat."),
                Some((proposed, v)) => {
                    println!(
                        "Inventarie {id} har lämnat registret. Bokfört värde {}, {} {}.",
                        render::amount(v.book_value),
                        if v.result.is_negative() {
                            "förlust"
                        } else {
                            "vinst"
                        },
                        render::amount(if v.result.is_negative() {
                            -v.result
                        } else {
                            v.result
                        })
                    );
                    show_proposal(&store, &proposed)?;
                }
            }
        }
    }
    Ok(())
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(n - 1).collect();
        t.push('…');
        t
    }
}

/// Registret mot huvudboken vid årets slut.
pub fn print_reconciliation(store: &Store, y: &bok_store::FiscalYear) -> Result<()> {
    let checks = depreciation::reconcile(store, y)?;
    println!();
    println!("Avstämning mot huvudboken per {}:", y.end);
    for c in &checks {
        let mark = if c.difference().is_zero() {
            "stämmer"
        } else {
            "SKILLNAD"
        };
        println!(
            "  {}  registret {:>14}  bokfört {:>14}  {mark} {}",
            c.account,
            render::amount(c.register),
            render::amount(c.ledger),
            if c.difference().is_zero() {
                String::new()
            } else {
                render::amount(c.difference())
            }
        );
    }
    if checks.iter().any(|c| !c.difference().is_zero()) {
        println!(
            "En skillnad kan vara årets avskrivningar som inte är bokförda än \
             (`bok bokslut avskrivningar`), eller en anskaffning som saknas i registret."
        );
    }
    Ok(())
}

/// Visar ett lagt bokslutsförslag och hur det bokförs.
pub fn show_proposal(store: &Store, p: &bok_bokslut::proposal::Proposed) -> Result<()> {
    let row = store.proposal(p.id)?;
    render::proposal(store, &row)?;
    for note in &p.notes {
        println!("Obs: {note}");
    }
    println!();
    if row.decision.is_some() {
        println!(
            "Förslag {} har redan ett beslut; samma förslag läggs inte igen.",
            p.id
        );
    } else {
        println!(
            "Förslag {} väntar på beslut. Bokför det med `bok godkann {}`.",
            p.id, p.id
        );
    }
    Ok(())
}
