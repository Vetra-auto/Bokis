//! Systemdokumentationen, genererad ur bokföringen själv.
//!
//! Dokumentet beskriver det som faktiskt gäller i databasen: spärrarna som
//! de står i schemat, agentens och granskarens operationer ur
//! operationsregistret, nycklarna, perioderna, kontoplanens momskoder och
//! resultatet av en full verifiering vid framtagandet. Det skrivs till en ny
//! fil (en befintlig skrivs aldrig över) och loggas i behandlingshistoriken
//! med sin SHA-256.

use std::path::Path;

use anyhow::{Context, Result, bail};

use bok_store::Store;
use bok_store::bok_core::canonical::sha256_hex;

use crate::{Ctx, PROGRAM};

/// Dokumentet som Markdown (genereras i `bok-insyn`).
pub fn render(store: &Store, db: &Path, generated: &str) -> Result<String> {
    let archive = crate::archive::Archive::beside(db).open()?;
    Ok(bok_insyn::sysdoc::render(
        store, &archive, db, PROGRAM, generated,
    )?)
}

/// Skriver dokumentet till en ny fil och loggar det i historiken.
pub fn write(ctx: &Ctx, out: &Path) -> Result<()> {
    if out.exists() {
        bail!("{} finns redan; välj ett nytt filnamn", out.display());
    }
    let mut store = ctx.open()?;
    let generated = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    let text = render(&store, &ctx.db, &generated)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(out)
        .with_context(|| format!("kan inte skapa {}", out.display()))?;
    std::io::Write::write_all(&mut file, text.as_bytes())?;
    let sha = sha256_hex(text.as_bytes());
    println!(
        "Systemdokumentationen är skriven till {} ({}).",
        out.display(),
        &sha[..12]
    );
    if let Some(year) = store.fiscal_year_for_date(store.today())? {
        store.record_export(
            &ctx.actor(),
            "systemdokumentation",
            year.id,
            &out.display().to_string(),
            &sha,
        )?;
    }
    Ok(())
}
