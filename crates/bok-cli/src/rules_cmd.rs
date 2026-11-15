//! `bok regel`: regler som agenten följer, till exempel hur en viss
//! leverantörs fakturor konteras. Varje version signeras med nyckeln och
//! gäller från ett datum; agenten läser dem med `hamta_regler`.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Subcommand;

use crate::{Ctx, input, render};

#[derive(Subcommand)]
pub enum RuleCmd {
    /// Lägger en ny version av en regel. Signeras med nyckeln.
    #[command(visible_alias = "lagg-till")]
    Add {
        /// Regelns namn, till exempel "molntjanster".
        name: String,
        /// Regeln i klartext.
        #[arg(long, conflicts_with = "file")]
        text: Option<String>,
        /// Regeln som JSON-objekt i en fil.
        #[arg(long)]
        file: Option<PathBuf>,
        /// Gäller från [standard: i dag].
        #[arg(long)]
        from: Option<String>,
        /// Gäller till och med.
        #[arg(long)]
        to: Option<String>,
    },
    /// Visar reglerna som gäller ett datum, eller alla versioner.
    #[command(visible_alias = "lista")]
    List {
        #[arg(long)]
        date: Option<String>,
        #[arg(long)]
        all: bool,
    },
}

fn text_of(content: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(content) {
        Ok(v) => v["text"]
            .as_str()
            .map_or_else(|| content.to_string(), str::to_string),
        Err(_) => content.to_string(),
    }
}

pub fn rule(ctx: &Ctx, cmd: RuleCmd) -> Result<()> {
    match cmd {
        RuleCmd::Add {
            name,
            text,
            file,
            from,
            to,
        } => {
            let content = match (text, file) {
                (Some(t), None) => t,
                (None, Some(f)) => std::fs::read_to_string(&f)
                    .with_context(|| format!("kan inte läsa {}", f.display()))?,
                _ => bail!("ange regeln med --text eller --file"),
            };
            let mut store = ctx.open()?;
            let from = match from {
                Some(s) => input::parse_date(&s)?,
                None => store.today(),
            };
            let to = to.as_deref().map(input::parse_date).transpose()?;
            let row = store.add_rule(&ctx.actor(), &name, from, to, &content, &ctx.signer())?;
            println!(
                "Regeln {} version {} gäller från {}{}.",
                row.name,
                row.version,
                row.valid_from,
                row.valid_to
                    .map(|t| format!(" till och med {t}"))
                    .unwrap_or_default()
            );
        }
        RuleCmd::List { date, all } => {
            let store = ctx.open()?;
            let rows = if all {
                store.all_rules()?
            } else {
                let at = match date {
                    Some(s) => input::parse_date(&s)?,
                    None => store.today(),
                };
                store.rules(at)?
            };
            if rows.is_empty() {
                println!("Inga regler.");
            }
            for r in rows {
                println!(
                    "{} v{}  {} – {}  {}",
                    render::safe(&r.name),
                    r.version,
                    r.valid_from,
                    r.valid_to
                        .map(|t| t.to_string())
                        .unwrap_or_else(|| "tills vidare".into()),
                    render::safe(&text_of(&r.content))
                );
            }
        }
    }
    Ok(())
}
