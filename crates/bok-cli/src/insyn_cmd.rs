//! `bok insyn`: bokföringen som läsbara sidor för revisor och Skatteverket.
//!
//! Utan underkommando startar insynen live på 127.0.0.1 med en engångslänk.
//! Med `--adress` körs den som tjänst bakom `tailscale serve`, och
//! inloggningslänkarna tas fram med `bok insyn lank`.
//! `bok insyn export <katalog>` skriver samma sidor som fristående filer med
//! underlagen, SIE 4 och CSV per år och `manifest.sha256`, så att kopian kan
//! lämnas på ett USB-minne och läsas utan Bokis.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};

use bok_insyn::server::{LiveView, bind, random_token, serve};
use bok_insyn::{ExtraFile, Insyn, Mode, links};

use crate::{Ctx, PROGRAM, resolve_year, sie_bytes};

#[derive(Args)]
pub struct InsynArgs {
    #[command(subcommand)]
    cmd: Option<InsynCmd>,
    /// Porten på 127.0.0.1 [standard: en ledig port].
    #[arg(long, default_value_t = 0)]
    port: u16,
    /// Kör som tjänst som också nås på den här adressen, till exempel
    /// https://bokis.ditt-tailnet.ts.net:8443 bakom `tailscale serve`. Ingen
    /// startlänk; ta fram inloggningslänkar med `bok insyn lank`.
    #[arg(long, visible_alias = "address")]
    adress: Option<String>,
}

#[derive(Subcommand)]
pub enum InsynCmd {
    /// Ny inloggningslänk till insynen som körs som tjänst. Den fungerar en gång i 15 minuter.
    #[command(visible_alias = "link")]
    Lank,
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
        None => live(ctx, args.port, args.adress.as_deref()),
        Some(InsynCmd::Lank) => link(ctx),
        Some(InsynCmd::Export { target, year }) => export(ctx, &target, year.as_deref()),
    }
}

fn live(ctx: &Ctx, port: u16, adress: Option<&str>) -> Result<()> {
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
    let mut view = LiveView::new(
        &store,
        &archive,
        &ctx.db,
        PROGRAM,
        login.clone(),
        session,
        port,
    );
    match adress {
        Some(origin) => {
            let origin = origin.trim_end_matches('/');
            if !origin.starts_with("https://") && !origin.starts_with("http://") {
                anyhow::bail!("--adress ska börja med https://");
            }
            let file = links::file_beside(&ctx.db);
            links::set_origin(&file, origin, chrono::Utc::now().timestamp())
                .with_context(|| format!("kan inte skriva {}", file.display()))?;
            view = view.with_remote(origin, file);
            println!("Den nås på {origin}. Ta fram en inloggningslänk med `bok insyn lank`.");
        }
        None => {
            println!("Öppna länken i webbläsaren; den fungerar en gång:");
            println!("  http://127.0.0.1:{port}/login?token={login}");
        }
    }
    println!("Insynen visar bokföringen och kan inte ändra den. Avsluta med Ctrl+C.");
    serve(&mut view, &listener, None)?;
    Ok(())
}

fn link(ctx: &Ctx) -> Result<()> {
    let file = links::file_beside(&ctx.db);
    let Some(origin) = links::origin(&file)? else {
        anyhow::bail!(
            "insynen körs inte som tjänst; starta den med `bok insyn --adress <url>`, \
             eller kör bara `bok insyn` för en länk på den här datorn"
        );
    };
    let token = random_token()?;
    links::issue(&file, &token, chrono::Utc::now().timestamp())
        .with_context(|| format!("kan inte skriva {}", file.display()))?;
    println!("Öppna länken i webbläsaren; den fungerar en gång i 15 minuter:");
    println!("  {origin}/login?token={token}");
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
