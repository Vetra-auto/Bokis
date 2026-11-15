//! `bok-fjarr`: Bokis på distans för agenten och telefonen.
//!
//! ```text
//! bok-fjarr starta --adress https://bokis.ditt-tailnet.ts.net   servern på 127.0.0.1:8770
//! bok-fjarr kod                                               kopplingskod för en ny agentanslutning
//! bok-fjarr uppladdning telefon                               kod som telefonen hämtar sin länk med
//! bok-fjarr lista                                             anslutningar och länkar
//! bok-fjarr aterkalla iphone                                  återkallar en anslutning eller länk
//! ```

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use serde_json::json;

use bok_fjarr::state::{self, Link, LinkCode, Pairing, StateFile, LINK_CODE_TTL, PAIRING_TTL};
use bok_fjarr::{server, App, Config};
use bok_store::{Actor, Store};

const PROGRAM: &str = concat!("bok-fjarr ", env!("CARGO_PKG_VERSION"));

#[derive(Parser)]
#[command(
    name = "bok-fjarr",
    version,
    about = "Bokis på distans: agenten i Claude, ChatGPT eller Claude Code, och kvittofoton från telefonen.",
    after_help = "Agenten kan läsa och föreslå men aldrig bokföra. Varje bokföring signeras med din nyckel i bok eller granskningsvyn."
)]
struct Cli {
    /// Databasen [standard: ~/.local/share/bokis/bokis.db]
    #[arg(long, env = "BOKIS_DB", global = true)]
    db: Option<PathBuf>,
    /// Ditt namn i behandlingshistoriken [standard: $USER]
    #[arg(long, env = "BOKIS_USER", global = true)]
    user: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Startar servern på 127.0.0.1. Publicera den med till exempel tailscale funnel.
    Starta {
        /// Den publika adressen, till exempel https://bokis.ditt-tailnet.ts.net
        #[arg(long)]
        adress: String,
        /// Porten på 127.0.0.1.
        #[arg(long, default_value_t = 8770)]
        port: u16,
    },
    /// En kopplingskod för att ansluta en agent. Gäller i 15 minuter och en gång.
    Kod,
    /// En personlig uppladdningslänk för en telefon. Telefonen hämtar den med
    /// en kod på serverns startsida.
    Uppladdning {
        /// Ett namn på telefonen, till exempel telefon eller iphone.
        namn: String,
        /// Skriv ut hela länken direkt i stället för en kod, för att skicka den på annat sätt.
        #[arg(long)]
        lank: bool,
    },
    /// Anslutna agenter och uppladdningslänkar.
    Lista,
    /// Återkallar en agentanslutning eller en uppladdningslänk.
    Aterkalla {
        /// Namnet eller id:t som bok-fjarr lista visar.
        namn: String,
    },
}

fn default_db() -> Result<PathBuf, String> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .ok_or("ange databasen med --db eller BOKIS_DB")?;
    Ok(data.join("bokis").join("bokis.db"))
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn when(t: i64) -> String {
    chrono::DateTime::from_timestamp(t, 0)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "?".into())
}

fn valid_name(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty()
        && n.chars().count() <= 40
        && n.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'))
}

fn run(cli: Cli) -> Result<(), String> {
    let db = match cli.db {
        Some(p) => p,
        None => default_db()?,
    };
    let user = cli
        .user
        .or_else(|| std::env::var("USER").ok())
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| "granskare".into());
    let state_file = StateFile::beside(&db);
    let io = |e: std::io::Error| e.to_string();
    match cli.cmd {
        Cmd::Starta { adress, port } => {
            let origin = Config::origin_from(&adress)?;
            let (listener, port) = server::bind(port)
                .map_err(|e| format!("kan inte lyssna på 127.0.0.1:{port}: {e}"))?;
            let config = Config {
                origin: origin.clone(),
                port,
                db: db.clone(),
                user,
                program: PROGRAM.into(),
            };
            let mcp = config.mcp_url();
            let app = App::open(config)?;
            let company = app.store().company().map_err(|e| e.to_string())?.name;
            println!("Bokis på distans för {company}");
            println!("  Lyssnar på 127.0.0.1:{port}. Publik adress: {origin}");
            println!("  Agentens adress: {mcp}");
            println!();
            println!("Anslut en agent: kör `bok-fjarr kod` och lägg sedan till {mcp} som anpassad");
            println!("anslutning (custom connector) i Claude. Skriv in koden när Bokis frågar.");
            println!("Telefonen: kör `bok-fjarr uppladdning telefon`, öppna {origin} i telefonen och skriv koden.");
            println!();
            println!("Adressen måste nå den här servern över HTTPS. Med Tailscale gör");
            println!("`tailscale funnel --bg {port}` den publik, vilket Claude i webbläsaren och mobilappen kräver,");
            println!("och `tailscale serve --bg {port}` håller den inom ditt tailnet. Avsluta med Ctrl+C.");
            server::serve(app, &listener, None).map_err(io)
        }
        Cmd::Kod => {
            let code = state::pairing_code().map_err(io)?;
            let t = now();
            let origin = state_file
                .update(t, |s| {
                    s.pairing = Some(Pairing {
                        hash: state::hash(&state::normalize_code(&code)),
                        expires: t + PAIRING_TTL,
                        attempts: 0,
                    });
                    s.origin.clone()
                })
                .map_err(io)?;
            println!("Kopplingskod: {code}");
            println!("Den gäller i 15 minuter och en gång.");
            match origin {
                Some(o) => println!(
                    "Lägg till {o}/mcp som anslutning i Claude, ChatGPT eller Claude Code och skriv in koden när Bokis frågar."
                ),
                None => println!("Starta servern med `bok-fjarr starta --adress https://…` om den inte redan körs."),
            }
            Ok(())
        }
        Cmd::Uppladdning { namn, lank } => {
            let name = namn.trim().to_string();
            if !valid_name(&name) {
                return Err(
                    "namnet får ha högst 40 tecken: bokstäver, siffror, mellanslag, - _ och ."
                        .into(),
                );
            }
            let current = state_file.read().map_err(io)?;
            let Some(origin) = current.origin.clone() else {
                return Err("starta servern en gång med `bok-fjarr starta --adress https://…` först, så att länken får rätt adress".into());
            };
            if current.link_named(&name) {
                return Err(format!("det finns redan en länk som heter {name}; återkalla den först eller välj ett annat namn"));
            }
            if !lank {
                // Telefonen hämtar länken med koden på startsidan. Servern skapar
                // länken först då, så hemligheten visas aldrig här.
                let code = state::pairing_code().map_err(io)?;
                let t = now();
                let earlier = state_file
                    .update(t, |s| {
                        s.link_code
                            .replace(LinkCode {
                                hash: state::hash(&state::normalize_code(&code)),
                                name: name.clone(),
                                expires: t + LINK_CODE_TTL,
                                attempts: 0,
                            })
                            .map(|c| c.name)
                    })
                    .map_err(io)?;
                println!("Öppna {origin} i telefonen och skriv koden:");
                println!();
                println!("    {code}");
                println!();
                println!("Koden gäller i 15 minuter och en gång. Telefonen får då uppladdningslänken {name}.");
                println!("Spara sidan som öppnas på hemskärmen. Återkalla länken med `bok-fjarr aterkalla {name}`.");
                if let Some(earlier) = earlier {
                    println!("Den förra koden, för {earlier}, gäller inte längre.");
                }
                return Ok(());
            }
            let secret = state::random_secret().map_err(io)?;
            let mut store = Store::open(&db, PROGRAM).map_err(|e| e.to_string())?;
            store
                .record_remote_access(
                    &Actor::human(user),
                    "fjarr.lank.skapad",
                    &name,
                    json!({"namn": name}),
                )
                .map_err(|e| e.to_string())?;
            let t = now();
            state_file
                .update(t, |s| {
                    s.links.push(Link {
                        hash: state::hash(&secret),
                        name: name.clone(),
                        created: t,
                        revoked: None,
                        last_used: None,
                    });
                })
                .map_err(io)?;
            println!("Uppladdningslänken {name}:");
            println!("  {origin}/in/{secret}");
            println!(
                "Öppna den i telefonen och spara den på hemskärmen. Den visas bara nu; den som har"
            );
            println!("länken kan skicka underlag. Återkalla den med `bok-fjarr aterkalla {name}`.");
            Ok(())
        }
        Cmd::Lista => {
            let s = state_file.read().map_err(io)?;
            println!("Agentanslutningar:");
            let mut any = false;
            for c in &s.clients {
                any = true;
                let status = match (c.approved, c.revoked) {
                    (_, Some(r)) => format!("återkallad {}", when(r)),
                    (Some(a), None) => format!(
                        "godkänd {}{}",
                        when(a),
                        c.last_used
                            .map(|u| format!(", senast använd {}", when(u)))
                            .unwrap_or_default()
                    ),
                    (None, None) => format!("väntar på kopplingskod sedan {}", when(c.registered)),
                };
                println!("  {} ({}): {status}", c.name, c.id);
            }
            if !any {
                println!("  inga");
            }
            println!("Uppladdningslänkar:");
            if s.links.is_empty() {
                println!("  inga");
            }
            for l in &s.links {
                let status = match l.revoked {
                    Some(r) => format!("återkallad {}", when(r)),
                    None => format!(
                        "skapad {}{}",
                        when(l.created),
                        l.last_used
                            .map(|u| format!(", senast använd {}", when(u)))
                            .unwrap_or_default()
                    ),
                };
                println!("  {}: {status}", l.name);
            }
            if s.pairing.as_ref().is_some_and(|p| p.expires > now()) {
                println!("En kopplingskod väntar på att användas.");
            }
            if let Some(c) = s.link_code.as_ref().filter(|c| c.expires > now()) {
                println!(
                    "En kod för uppladdningslänken {} väntar på telefonen till {}.",
                    c.name,
                    when(c.expires)
                );
            }
            Ok(())
        }
        Cmd::Aterkalla { namn } => {
            let wanted = namn.trim().to_string();
            let t = now();
            let found = state_file
                .update(t, |s| {
                    let clients: Vec<usize> = s
                        .clients
                        .iter()
                        .enumerate()
                        .filter(|(_, c)| c.revoked.is_none() && (c.id == wanted || c.name.eq_ignore_ascii_case(&wanted)))
                        .map(|(i, _)| i)
                        .collect();
                    let links: Vec<usize> = s
                        .links
                        .iter()
                        .enumerate()
                        .filter(|(_, l)| l.revoked.is_none() && l.name.eq_ignore_ascii_case(&wanted))
                        .map(|(i, _)| i)
                        .collect();
                    match (clients.as_slice(), links.as_slice()) {
                        ([i], []) => {
                            let c = &mut s.clients[*i];
                            c.revoked = Some(t);
                            let (id, name) = (c.id.clone(), c.name.clone());
                            s.tokens.retain(|tok| tok.client_id != id);
                            Ok(("fjarr.anslutning.aterkallad", name, json!({"klient_id": id})))
                        }
                        ([], [i]) => {
                            let l = &mut s.links[*i];
                            l.revoked = Some(t);
                            Ok(("fjarr.lank.aterkallad", l.name.clone(), json!({"namn": l.name})))
                        }
                        ([], []) => Err(format!("hittar ingen anslutning eller länk som heter {wanted}; se bok-fjarr lista")),
                        _ => Err(format!("flera anslutningar heter {wanted}; ange id:t som bok-fjarr lista visar")),
                    }
                })
                .map_err(io)??;
            let (action, name, values) = found;
            println!("{name} är återkallad. Nycklarna slutar gälla direkt.");
            let mut store = Store::open(&db, PROGRAM).map_err(|e| e.to_string())?;
            store
                .record_remote_access(&Actor::human(user), action, &name, values)
                .map_err(|e| format!("återkallad, men kunde inte loggas i historiken: {e}"))
        }
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("bok-fjarr: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn command_names_never_suggest_deleting_anything() {
        const FORBIDDEN: [&str; 15] = [
            "restore",
            "reset",
            "delete",
            "remove",
            "drop",
            "purge",
            "wipe",
            "truncate",
            "overwrite",
            "import-db",
            "aterstall",
            "nollstall",
            "radera",
            "ta-bort",
            "rensa",
        ];
        for sub in Cli::command().get_subcommands() {
            let name = sub.get_name();
            assert!(!FORBIDDEN.iter().any(|f| name.contains(f)), "{name}");
        }
        assert!(valid_name("iphone"));
        assert!(valid_name("Henriks telefon"));
        assert!(!valid_name("a/b"));
        assert!(!valid_name(""));
    }
}
