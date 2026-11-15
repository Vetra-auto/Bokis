//! `bok arkivpaket`: ett räkenskapsårs räkenskapsinformation i en katalog som
//! går att läsa och kontrollera utan Bokis, under hela bevarandetiden.
//!
//! Paketet innehåller SIE 4, grundbok, huvudbok, saldobalans och momsrapport
//! som text, underlagen byte för byte, hela behandlingshistoriken,
//! systemdokumentationen och programmet som skapade det. `manifest.sha256`
//! har samma form som `sha256sum`, så paketet kontrolleras med
//! `sha256sum -c manifest.sha256`. En befintlig katalog skrivs aldrig över.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use bok_store::bok_core::PeriodStatus;
use bok_store::bok_core::canonical::sha256_hex;

use crate::{Ctx, PROGRAM, resolve_year, sysdoc};

fn files_under(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in
            fs::read_dir(&dir).with_context(|| format!("kan inte läsa {}", dir.display()))?
        {
            let path = entry?.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Kör `bok` med samma databas och sparar utskriften i en fil.
fn report(ctx: &Ctx, args: &[&str], out: &Path) -> Result<()> {
    let exe = std::env::current_exe().context("hittar inte programmet")?;
    let output = Command::new(exe)
        .args(args)
        .env("BOKIS_DB", &ctx.db)
        .env("BOKIS_KEY", &ctx.key)
        .env("BOKIS_USER", &ctx.user)
        .output()
        .with_context(|| format!("kunde inte köra bok {}", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "bok {} misslyckades: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fs::write(out, output.stdout).with_context(|| format!("kan inte skriva {}", out.display()))
}

pub fn write(ctx: &Ctx, year: Option<&str>, target: &Path) -> Result<()> {
    let store = ctx.open()?;
    let y = resolve_year(&store, year)?;
    let label = y.label();
    let company = store.company()?;
    let dir = target.join(format!("bokis-{}", label.replace('/', "-")));
    if dir.exists() {
        bail!(
            "{} finns redan; ett arkivpaket skrivs aldrig över",
            dir.display()
        );
    }
    let open_periods = store
        .periods(y.id)?
        .iter()
        .filter(|p| !matches!(p.status, PeriodStatus::Closed | PeriodStatus::Locked))
        .count();
    let generated = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    drop(store);

    for sub in ["sie", "rapporter", "underlag", "program"] {
        fs::create_dir_all(dir.join(sub))
            .with_context(|| format!("kan inte skapa {}", dir.display()))?;
    }

    // Bokföringen i SIE 4 och som läsbara rapporter.
    crate::export_sie(
        ctx,
        Some(&label),
        &dir.join("sie")
            .join(format!("bokis-{}.se", label.replace('/', "-"))),
    )?;
    let vat_period = format!("{}:{}", y.start.format("%Y-%m"), y.end.format("%Y-%m"));
    for (file, args) in [
        ("grundbok.txt", vec!["grundbok", "--year", &label]),
        ("huvudbok.txt", vec!["huvudbok", "--year", &label]),
        ("saldobalans.txt", vec!["saldobalans", "--year", &label]),
        ("momsrapport.txt", vec!["moms", &vat_period]),
    ] {
        report(ctx, &args, &dir.join("rapporter").join(file))?;
    }

    // Underlagen: de som årets verifikationer bygger på och de som togs emot under året.
    let mut store = ctx.open()?;
    let mut used_by: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for v in store.journal(y.id)? {
        for sha in &v.attachments {
            used_by
                .entry(sha.clone())
                .or_default()
                .push(v.reference.to_string());
        }
    }
    let attachments = store.attachments()?;
    let mut wanted: BTreeSet<String> = used_by.keys().cloned().collect();
    for a in &attachments {
        let received = a.received_at.get(..10).unwrap_or("");
        if received >= y.start.to_string().as_str() && received <= y.end.to_string().as_str() {
            wanted.insert(a.sha256.clone());
        }
    }
    let archive = ctx.archive().open()?;
    let mut index =
        String::from("sha256;storlek;mottagen;kanal;originalnamn;filtyp;verifikationer\n");
    for sha in &wanted {
        let data = archive
            .read(sha)
            .with_context(|| format!("underlaget {sha} kunde inte läsas ur arkivet"))?;
        let target_dir = dir.join("underlag").join(&sha[..2]);
        fs::create_dir_all(&target_dir)?;
        fs::write(target_dir.join(sha), &data)?;
        let row = attachments.iter().find(|a| &a.sha256 == sha);
        let field = |s: Option<&str>| s.unwrap_or("").replace([';', '\n'], " ");
        index.push_str(&format!(
            "{sha};{};{};{};{};{};{}\n",
            data.len(),
            field(row.map(|a| a.received_at.as_str())),
            field(row.map(|a| a.channel.as_str())),
            field(row.and_then(|a| a.original_name.as_deref())),
            field(row.and_then(|a| a.file_type.as_deref())),
            used_by.get(sha).map(|v| v.join(" ")).unwrap_or_default()
        ));
    }
    fs::write(dir.join("underlag").join("forteckning.csv"), index)?;

    // Hela behandlingshistoriken, en händelse per rad.
    let mut history = String::new();
    for e in store.events(None)? {
        history.push_str(&serde_json::to_string(&serde_json::json!({
            "seq": e.seq,
            "tid": e.time,
            "aktor_typ": e.actor_kind,
            "aktor": e.actor,
            "roll": e.role,
            "handling": e.action,
            "objekt": e.object,
            "gamla": e.old_values,
            "nya": e.new_values,
            "programversion": e.program_version,
            "signerad": e.signed,
            "hash": e.hash,
        }))?);
        history.push('\n');
    }
    fs::write(dir.join("historik.jsonl"), history)?;
    fs::write(
        dir.join("systemdokumentation.md"),
        sysdoc::render(&store, &ctx.db, &generated)?,
    )?;

    // Programmet, så att paketet kan läsas även när det här programmet inte längre finns.
    let exe = std::env::current_exe().context("hittar inte programmet")?;
    let program = dir.join("program").join("bok");
    fs::copy(&exe, &program).with_context(|| format!("kan inte kopiera {}", exe.display()))?;

    let mut readme = format!(
        "Arkivpaket för räkenskapsåret {label} ({} – {})\n{} ({})\nSkapat {generated} med {PROGRAM}.\n\n\
         Innehåll\n\
         \x20 sie/                     bokföringen i SIE 4 (#IB, #UB, #RES och verifikationer)\n\
         \x20 rapporter/               grundbok, huvudbok, saldobalans och momsrapport som text\n\
         \x20 underlag/                underlagen byte för byte, namngivna efter sin SHA-256; forteckning.csv\n\
         \x20 historik.jsonl           behandlingshistoriken, en händelse per rad, med hashkedja\n\
         \x20 systemdokumentation.md   systemdokumentationen när paketet skapades\n\
         \x20 program/bok              programmet som skapade paketet ({PROGRAM})\n\
         \x20 manifest.sha256          SHA-256 för varje fil ovan\n\n\
         Kontrollera paketet:  sha256sum -c manifest.sha256\n",
        y.start, y.end, company.name, company.org_number
    );
    if open_periods > 0 {
        readme.push_str(&format!(
            "\nObs: {open_periods} perioder var inte stängda när paketet skapades. Skapa ett nytt paket när året är stängt.\n"
        ));
    }
    fs::write(dir.join("LASMIG.txt"), readme)?;

    let mut manifest = String::new();
    for file in files_under(&dir)? {
        let rel = file.strip_prefix(&dir).unwrap_or(&file);
        let data = fs::read(&file)?;
        manifest.push_str(&format!("{}  {}\n", sha256_hex(&data), rel.display()));
    }
    fs::write(dir.join("manifest.sha256"), &manifest)?;
    let manifest_sha = sha256_hex(manifest.as_bytes());

    store.record_export(
        &ctx.actor(),
        "arkivpaket",
        y.id,
        &dir.display().to_string(),
        &manifest_sha,
    )?;
    println!(
        "Arkivpaketet för {label} är skrivet till {} med {} underlag.",
        dir.display(),
        wanted.len()
    );
    println!(
        "Manifestets SHA-256: {manifest_sha}. Kontrollera med `sha256sum -c manifest.sha256` i katalogen."
    );
    if open_periods > 0 {
        println!(
            "Obs: {open_periods} perioder är inte stängda; skapa ett nytt paket när året är stängt."
        );
    }
    Ok(())
}
