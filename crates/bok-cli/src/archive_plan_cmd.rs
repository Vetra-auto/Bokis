//! `bok arkivplan`: företagets arkivplan, som data.
//!
//! `satt` läser en JSON-fil (se `docs/arkivplan-exempel.json`), kontrollerar
//! den och lägger in den som en ny version i behandlingshistoriken. `visa`
//! skriver avsnittet Arkivplan ur systemdokumentationen: planen, bevarandetiden
//! för varje räkenskapsår, de senaste kopiorna och det som behöver ses över.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Subcommand;

use bok_store::bok_arkivplan::{ArchivePlan, observations};

use crate::Ctx;

#[derive(Subcommand)]
pub enum ArchivePlanCmd {
    /// Lägger in arkivplanen ur en JSON-fil. Den förra versionen finns kvar i historiken.
    #[command(visible_alias = "satt")]
    Set { file: PathBuf },
    /// Visar den gällande arkivplanen med bevarandetid och senaste kopior.
    #[command(visible_alias = "visa")]
    Show,
}

/// Kör ett `bok arkivplan`-kommando.
pub fn archive_plan(ctx: &Ctx, cmd: ArchivePlanCmd) -> Result<()> {
    match cmd {
        ArchivePlanCmd::Set { file } => {
            let text = std::fs::read_to_string(&file)
                .with_context(|| format!("kan inte läsa {}", file.display()))?;
            let plan = ArchivePlan::from_json(&text)?;
            let mut store = ctx.open()?;
            let seq = store.set_archive_plan(&ctx.actor(), &plan)?;
            println!(
                "Arkivplanen med {} delar är inlagd (händelse {seq}). Den visas i \
                 systemdokumentationen och i insynen.",
                plan.parts.len()
            );
            for o in observations(&plan) {
                println!("Att se över: {} ({}).", o.text, o.source);
            }
        }
        ArchivePlanCmd::Show => {
            let store = ctx.open()?;
            let events = store.events(None)?;
            print!("{}", bok_insyn::sysdoc_archive::section(&store, &events)?);
        }
    }
    Ok(())
}
