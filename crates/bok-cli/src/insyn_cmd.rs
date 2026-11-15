//! `bok insyn`: bokföringen som läsbara sidor för revisor och Skatteverket.
//!
//! Utan underkommando startar insynen live på 127.0.0.1 med en engångslänk.
//! `bok insyn export <katalog>` skriver samma sidor som fristående filer med
//! underlagen, SIE 4 och CSV per år och `manifest.sha256`, så att kopian kan
//! lämnas på ett USB-minne och läsas utan Bokis.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};

use bok_insyn::server::{LiveView, bind, random_token, serve};
use bok_insyn::{ExtraFile, Insyn, Mode};

use crate::{Ctx, PROGRAM, resolve_year, sie_bytes};

#[derive(Args)]
pub struct InsynArgs {
    #[command(subcommand)]
    cmd: Option<InsynCmd>,
    /// Porten på 127.0.0.1 [standard: en ledig port].
    #[arg(long, default_value_t = 0)]
    port: u16,
}

#[derive(Subcommand)]
pub enum InsynCmd {
    /// Skriver insynen som fristående sidor i en ny katalog, med underlag, SIE 4, CSV och manifest.
    Export {
        /// Katalogen som skapas, till exempel på ett USB-minne. En befintlig katalog skrivs aldrig över.
        target: PathBuf,
        /// Bara ett räkenskapsår [standard: hela bokföringen].
        #[arg(long)]
        year: Option<String>,
    },
}

fn now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M").to_string()
}

pub fn insyn(ctx: &Ctx, args: InsynArgs) -> Result<()> {
    match args.cmd {
        None => live(ctx, args.port),
        Some(InsynCmd::Export { target, year }) => export(ctx, &target, year.as_deref()),
    }
}

fn live(ctx: &Ctx, port: u16) -> Result<()> {
    let store = ctx.open()?;
    let archive = ctx.archive().open()?;
    let company = store.company()?;
    let (listener, port) = bind(port).context("kan inte lyssna på 127.0.0.1")?;
    let login = random_token()?;
    let session = random_token()?;
    println!(
        "Insynen för {} ({}) är igång på 127.0.0.1:{port}.",
        crate::render::safe(&company.name),
        company.org_number
    );
    println!("Öppna länken i webbläsaren; den fungerar en gång:");
    println!("  http://127.0.0.1:{port}/login?token={login}");
    println!("Insynen visar bokföringen och kan inte ändra den. Avsluta med Ctrl+C.");
    let mut view = LiveView::new(&store, &archive, &ctx.db, PROGRAM, login, session, port);
    serve(&mut view, &listener, None)?;
    Ok(())
}

fn export(ctx: &Ctx, target: &std::path::Path, year: Option<&str>) -> Result<()> {
    let mut store = ctx.open()?;
    let archive = ctx.archive().open()?;
    let only = match year {
        Some(y) => Some(resolve_year(&store, Some(y))?),
        None => None,
    };
    let years = match only {
        Some(y) => vec![y],
        None => store.fiscal_years()?,
    };
    let mut files = Vec::new();
    for y in &years {
        let label = y.label().replace('/', "-");
        files.push(ExtraFile {
            path: format!("sie/bokis-{label}.se"),
            body: sie_bytes(&store, y)?,
            description: format!(
                "bokföringen för {} i SIE 4, för import i revisions- och bokföringsprogram",
                y.label()
            ),
        });
    }
    let report = {
        let mut insyn = Insyn::new(&store, &archive, &ctx.db, PROGRAM, now(), Mode::Export)?;
        if let Some(y) = only {
            insyn = insyn.only_year(y.id)?;
        }
        let insyn = insyn.with_files(files)?;
        bok_insyn::export(&insyn, target)
            .with_context(|| format!("kunde inte exportera till {}", target.display()))?
    };
    for y in &years {
        store.record_export(
            &ctx.actor(),
            "insyn",
            y.id,
            &report.dir.display().to_string(),
            &report.manifest_sha256,
        )?;
    }
    println!(
        "Insynen är exporterad till {}: {} filer.",
        report.dir.display(),
        report.files
    );
    println!("Öppna index.html i en webbläsare; kopian behöver varken Bokis eller nät.");
    println!(
        "Manifestets SHA-256: {}. Kontrollera kopian med `sha256sum -c manifest.sha256` i katalogen.",
        report.manifest_sha256
    );
    Ok(())
}
