//! `bok deklaration ink2`: inkomstdeklaration 2 ur bokföringen, som SRU-filer
//! för Skatteverkets filöverföring (`INFO.SRU` och `BLANKETTER.SRU`) och en
//! läsbar sammanställning (`ink2-sammanstallning.md`).
//!
//! Filerna skrivs bara om deklarationen klarar kontrollerna, och aldrig
//! över befintliga filer. Varje fil loggas i behandlingshistoriken med sin
//! SHA-256.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Subcommand};

use bok_deklaration::{Declaration, FormKind, ManualValue};
use bok_sru::MediaProvider;
use bok_store::bok_core::Money;
use bok_store::bok_core::canonical::sha256_hex;

use crate::{Ctx, PROGRAM, resolve_year};

/// Sammanställningens filnamn.
pub const SUMMARY_FILE: &str = "ink2-sammanstallning.md";

#[derive(Subcommand)]
pub enum DeclarationCmd {
    /// Inkomstdeklaration 2 (aktiebolag): INK2, INK2R och INK2S som SRU-filer.
    Ink2(Ink2Args),
}

#[derive(Args)]
pub struct Ink2Args {
    /// Katalogen som filerna skrivs till. Befintliga filer skrivs aldrig över.
    dir: PathBuf,
    /// Räkenskapsåret [standard: året som omfattar i dag].
    #[arg(long)]
    year: Option<String>,
    /// Uppgift för hand, FÄLT=VÄRDE: 4.14a=150000 (hela kronor) eller 8041=X.
    #[arg(long = "falt", value_name = "FÄLT=VÄRDE")]
    fields: Vec<String>,
    /// Postnumret i INFO.SRU.
    #[arg(long)]
    postnr: String,
    /// Postorten i INFO.SRU.
    #[arg(long)]
    postort: String,
    /// Utdelningsadressen (valfri).
    #[arg(long)]
    adress: Option<String>,
    /// Kontaktpersonen (valfri).
    #[arg(long)]
    kontakt: Option<String>,
    /// E-postadressen (valfri).
    #[arg(long)]
    epost: Option<String>,
    /// Telefonnumret (valfritt).
    #[arg(long)]
    telefon: Option<String>,
}

/// Tolkar `FÄLT=VÄRDE`: ett belopp i hela kronor eller `X`.
pub fn parse_manual(s: &str) -> Result<(String, ManualValue)> {
    let (key, value) = s
        .split_once('=')
        .ok_or_else(|| anyhow!("skriv fältet som FÄLT=VÄRDE, till exempel 4.14a=150000"))?;
    let value = value.trim();
    if value.eq_ignore_ascii_case("x") {
        return Ok((key.trim().to_string(), ManualValue::Text("X".into())));
    }
    let m = Money::parse(value).with_context(|| format!("{value} är inget belopp"))?;
    if m.ore() % 100 != 0 {
        bail!("{key}: deklarationen tar hela kronor, inte {value}");
    }
    Ok((key.trim().to_string(), ManualValue::Amount(m.ore() / 100)))
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write as _;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("kan inte skapa {} (finns den redan?)", path.display()))?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}

pub fn declaration(ctx: &Ctx, cmd: DeclarationCmd) -> Result<()> {
    let DeclarationCmd::Ink2(a) = cmd;
    let mut store = ctx.open()?;
    let y = resolve_year(&store, a.year.as_deref())?;
    let manual = a
        .fields
        .iter()
        .map(|f| parse_manual(f))
        .collect::<Result<Vec<_>>>()?;
    let input = bok_bokslut::input::declaration_input(&store, &y, manual)?;
    let decl = Declaration::compute_bas(&input)?;
    let summary = bok_deklaration::summary::markdown(&decl);

    for c in &decl.checks {
        println!("{} {}", if c.ok { "ok  " } else { "FEL " }, c.name);
        if !c.ok && !c.detail.is_empty() {
            println!("     {}", c.detail);
        }
    }
    for w in &decl.warnings {
        println!("Obs: {w}");
    }
    let provider = MediaProvider {
        org_number: input.org_number.clone(),
        name: input.company_name.clone(),
        postal_code: a.postnr,
        city: a.postort,
        address: a.adress,
        contact: a.kontakt,
        email: a.epost,
        phone: a.telefon,
        department: None,
    };
    let created = chrono::Local::now().naive_local();
    let files = bok_deklaration::sru::write(&decl, &provider, PROGRAM, created)?;

    std::fs::create_dir_all(&a.dir)
        .with_context(|| format!("kan inte skapa katalogen {}", a.dir.display()))?;
    let targets = [
        (bok_sru::INFO_FILE, files.info.as_slice()),
        (bok_sru::FORMS_FILE, files.forms.as_slice()),
        (SUMMARY_FILE, summary.as_bytes()),
    ];
    for (name, _) in &targets {
        if a.dir.join(name).exists() {
            bail!(
                "{} finns redan; välj en ny katalog",
                a.dir.join(name).display()
            );
        }
    }
    for (name, bytes) in &targets {
        write_new(&a.dir.join(name), bytes)?;
        store.record_export(&ctx.actor(), "ink2-sru", y.id, name, &sha256_hex(bytes))?;
    }

    let k = |form, code| decl.kronor(form, code).unwrap_or(0);
    println!();
    println!(
        "{} för {} ({} – {}), blanketterna {}, {} och {}.",
        decl.company_name,
        y.label(),
        decl.year_start,
        decl.year_end,
        decl.catalog.form_name(FormKind::Ink2),
        decl.catalog.form_name(FormKind::Ink2R),
        decl.catalog.form_name(FormKind::Ink2S)
    );
    println!(
        "  Årets resultat (3.26/3.27)   {:>14}",
        k(FormKind::Ink2R, "7450") - k(FormKind::Ink2R, "7550")
    );
    println!(
        "  Överskott (4.15, 1.1)        {:>14}",
        k(FormKind::Ink2S, "7670")
    );
    println!(
        "  Underskott (4.16)            {:>14}",
        k(FormKind::Ink2S, "7770")
    );
    println!(
        "  Bolagsskatt 20,6 %           {:>14}",
        bok_deklaration::summary::ore_text(decl.tax.tax_ore)
    );
    println!();
    for (name, _) in &targets {
        println!("Skrev {}", a.dir.join(name).display());
    }
    println!(
        "Granska sammanställningen innan du lämnar in filerna i Skatteverkets filöverföring. \
         Filerna är loggade i historiken."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_values() {
        let (k, v) = parse_manual("4.14a=150 000").unwrap();
        assert_eq!((k.as_str(), v), ("4.14a", ManualValue::Amount(150_000)));
        assert_eq!(
            parse_manual("8041=x").unwrap().1,
            ManualValue::Text("X".into())
        );
        assert!(parse_manual("4.14a=1,50").is_err());
        assert!(parse_manual("4.14a").is_err());
        assert!(parse_manual("4.14a=abc").is_err());
    }
}
