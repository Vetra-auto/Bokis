//! `bok insyn`: bokföringen som läsbara sidor för revisor och Skatteverket.
//!
//! Utan underkommando startar insynen live på 127.0.0.1 med en engångslänk.
//! Med `--adress` körs den som tjänst bakom `tailscale serve`, och
//! inloggningslänkarna tas fram per person med `bok insyn lank --namn
//! "Revisor X" --giltig 30d`. Länken gäller till sitt slutdatum eller tills
//! den återkallas med `bok insyn aterkalla`, och varje inloggning loggas i
//! behandlingshistoriken. `bok insyn lankar` listar länkarna.
//! `bok insyn export <katalog>` skriver samma sidor som fristående filer med
//! underlagen, SIE 4 och CSV per år och `manifest.sha256`, så att kopian kan
//! lämnas på ett USB-minne och läsas utan Bokis.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};

use bok_insyn::server::{LiveView, Login, bind, random_token, serve};
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
    /// Ny namngiven inloggningslänk till insynen som körs som tjänst. Den
    /// gäller till sitt slutdatum eller tills den återkallas.
    #[command(visible_alias = "link")]
    Lank {
        /// Vem länken är till, till exempel "Revisor X" [standard: --user].
        #[arg(long, visible_alias = "name")]
        namn: Option<String>,
        /// Hur länge länken gäller: minuter (15m), timmar (12h) eller dagar (30d), högst 366d.
        #[arg(long, visible_alias = "valid", default_value = "15m")]
        giltig: String,
    },
    /// Listar insynslänkarna: vem, giltig till, återkallad och inloggningar.
    #[command(visible_alias = "links")]
    Lankar,
    /// Återkallar en insynslänk; dess sessioner slutar gälla direkt.
    #[command(visible_alias = "revoke")]
    Aterkalla {
        /// Länkens nummer ur `bok insyn lankar`.
        id: i64,
    },
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
        Some(InsynCmd::Lank { namn, giltig }) => link(ctx, namn, &giltig),
        Some(InsynCmd::Lankar) => list_links(ctx),
        Some(InsynCmd::Aterkalla { id }) => revoke(ctx, id),
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
    // Inloggningarna skrivs genom en egen anslutning; insynen själv läser bara.
    let mut writer = ctx.open()?;
    let user = ctx.user.clone();
    let mut view = LiveView::new(
        &store,
        &archive,
        &ctx.db,
        PROGRAM,
        login.clone(),
        session,
        port,
    )
    .with_login_log(Box::new(move |login| {
        match login {
            Login::Link { id, until } => writer.record_insyn_login(id, until),
            Login::Local => writer.record_local_insyn_login(&user),
        }
        .map_err(|e| e.to_string())
    }));
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
            println!(
                "Den nås på {origin}. Ta fram en länk per person med \
                 `bok insyn lank --namn \"Revisor X\" --giltig 30d`."
            );
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

/// En tidpunkt i lokal tid för visning.
fn shown(t: chrono::DateTime<chrono::Utc>) -> String {
    bok_store::display_timestamp(&bok_store::format_timestamp(t))
}

/// Tolkar en giltighetstid som 15m, 12h eller 30d.
fn parse_validity(text: &str) -> Result<chrono::Duration> {
    let text = text.trim();
    let bad = || {
        anyhow::anyhow!(
            "--giltig {text:?}: ange minuter, timmar eller dagar, till exempel 15m, 12h eller 30d"
        )
    };
    let (number, unit) = text.split_at(text.len().saturating_sub(1));
    let n: i64 = number.parse().map_err(|_| bad())?;
    if n <= 0 {
        return Err(bad());
    }
    match unit {
        "m" => Ok(chrono::Duration::minutes(n)),
        "h" => Ok(chrono::Duration::hours(n)),
        "d" => Ok(chrono::Duration::days(n)),
        _ => Err(bad()),
    }
}

fn link(ctx: &Ctx, name: Option<String>, validity: &str) -> Result<()> {
    let file = links::file_beside(&ctx.db);
    let Some(origin) = links::origin(&file)? else {
        anyhow::bail!(
            "insynen körs inte som tjänst; starta den med `bok insyn --adress <url>`, \
             eller kör bara `bok insyn` för en länk på den här datorn"
        );
    };
    let validity = parse_validity(validity)?;
    let name = name.unwrap_or_else(|| ctx.user.clone());
    let mut store = ctx.open()?;
    let now = store.now();
    let until = now + validity;
    // Länken loggas i historiken innan den finns i filen, så att det aldrig
    // finns en länk utan händelse.
    let id = store.create_insyn_link(&ctx.actor(), &name, until)?;
    let token = random_token()?;
    links::issue(&file, id, &token, until.timestamp(), now.timestamp())
        .with_context(|| format!("kan inte skriva {}", file.display()))?;
    println!(
        "Insynslänk {id} till {} gäller till {}. Den kan användas flera gånger; varje \
         inloggning loggas. Återkalla med `bok insyn aterkalla {id}`.",
        crate::render::safe(&name),
        shown(until)
    );
    println!("  {origin}/login?token={token}");
    Ok(())
}

fn list_links(ctx: &Ctx) -> Result<()> {
    let store = ctx.open()?;
    let links = store.insyn_links()?;
    if links.is_empty() {
        println!(
            "Inga insynslänkar. Skapa en med `bok insyn lank --namn \"Revisor X\" --giltig 30d`."
        );
        return Ok(());
    }
    let now = store.now();
    for l in links {
        let state = match (&l.revoked, l.active_at(now)) {
            (Some(t), _) => format!("återkallad {}", bok_store::display_timestamp(t)),
            (None, true) => format!("gäller till {}", shown(l.valid_until)),
            (None, false) => format!("gick ut {}", shown(l.valid_until)),
        };
        let last = l
            .last_login
            .as_deref()
            .map(|t| format!(", senast {}", bok_store::display_timestamp(t)))
            .unwrap_or_default();
        println!(
            "{:>4}  {}  {state}  {} {}{last}  (skapad av {})",
            l.id,
            crate::render::safe(&l.name),
            l.logins,
            if l.logins == 1 {
                "inloggning"
            } else {
                "inloggningar"
            },
            crate::render::safe(&l.created_by)
        );
    }
    Ok(())
}

fn revoke(ctx: &Ctx, id: i64) -> Result<()> {
    let mut store = ctx.open()?;
    let link = store.revoke_insyn_link(&ctx.actor(), id)?;
    let file = links::file_beside(&ctx.db);
    if file.exists() {
        links::revoke(&file, id, store.now().timestamp())
            .with_context(|| format!("kan inte skriva {}", file.display()))?;
    }
    println!(
        "Insynslänk {id} till {} är återkallad. Dess sessioner slutar gälla direkt.",
        crate::render::safe(&link.name)
    );
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn validity_in_minutes_hours_and_days() {
        assert_eq!(
            parse_validity("15m").unwrap(),
            chrono::Duration::minutes(15)
        );
        assert_eq!(parse_validity("12h").unwrap(), chrono::Duration::hours(12));
        assert_eq!(parse_validity(" 30d ").unwrap(), chrono::Duration::days(30));
        for bad in ["", "d", "0d", "-1d", "30", "30x", "1,5d"] {
            assert!(parse_validity(bad).is_err(), "{bad:?}");
        }
    }
}
