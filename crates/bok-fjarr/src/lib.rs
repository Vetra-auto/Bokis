//! Bokis på distans: agenten i Claude, ChatGPT eller Claude Code ansluter
//! över HTTPS, och telefonen skickar kvittofoton.
//!
//! Servern lyssnar bara på 127.0.0.1. Den nås utifrån genom något som
//! sköter TLS och publicerar den, till exempel `tailscale funnel`. Claude i
//! webbläsaren och mobilappen ansluter från Anthropics moln, så adressen
//! måste vara publik för dem.
//!
//! - Agenten ansluter till `/mcp` (MCP över Streamable HTTP) och får samma
//!   verktyg som över stdio: läsa och föreslå, aldrig bokföra.
//! - Agenten loggar in med OAuth 2.1: registrering (RFC 7591), PKCE med S256
//!   och en kopplingskod som `bok-fjarr kod` skriver ut i terminalen. Utan
//!   koden blir ingen anslutning godkänd, även om adressen är publik.
//! - Telefonen skickar underlag till `/in/<länk>`, en personlig länk. Den
//!   hämtar länken på startsidan `/` med koden som `bok-fjarr uppladdning`
//!   skriver ut. Filen sparas oförändrad i arkivet och registreras som
//!   underlag; sedan ber man agenten bokföra det.
//!
//! Inget här kan bokföra. Varje bokföring signeras fortfarande med
//! granskarens nyckel i `bok` eller granskningsvyn.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::json;

use bok_archive::Archive;
use bok_mcp::server::Server;
use bok_mcp::store::StoreBackend;
use bok_store::Store;

pub mod http;
mod oauth;
mod pages;
pub mod server;
pub mod state;
mod upload;

use http::{Request, Response};
use state::StateFile;

/// Största kropp för en förfrågan till agentens gränssnitt och inloggningen.
pub const MAX_BODY: usize = 1024 * 1024;
/// Största kropp vid uppladdning: flera foton på en gång.
pub const MAX_UPLOAD: usize = 60 * 1024 * 1024;
/// Största fil vid uppladdning.
pub const MAX_FILE: usize = 25 * 1024 * 1024;

/// Hur servern nås och vem som använder den.
#[derive(Debug, Clone)]
pub struct Config {
    /// Den publika adressen utan snedstreck på slutet, till exempel
    /// `https://bokis.ditt-tailnet.ts.net`.
    pub origin: String,
    /// Porten på 127.0.0.1.
    pub port: u16,
    pub db: PathBuf,
    /// Den som godkänner anslutningar och lämnar underlag, för historiken.
    pub user: String,
    pub program: String,
}

impl Config {
    /// Kontrollerar och normaliserar den publika adressen. Den måste vara
    /// https, utom för 127.0.0.1 och localhost.
    pub fn origin_from(input: &str) -> Result<String, String> {
        let origin = input.trim().trim_end_matches('/');
        let rest = if let Some(r) = origin.strip_prefix("https://") {
            r
        } else if let Some(r) = origin.strip_prefix("http://") {
            let host = r.split(':').next().unwrap_or("");
            if host != "127.0.0.1" && host != "localhost" {
                return Err(format!(
                    "{origin}: adressen måste börja med https:// (bara 127.0.0.1 och localhost får vara http)"
                ));
            }
            r
        } else {
            return Err(format!("{origin}: adressen måste börja med https://"));
        };
        if rest.is_empty()
            || rest.contains(['/', '?', '#', '@'])
            || !rest
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'))
        {
            return Err(format!(
                "{origin}: ange bara schema och värd, till exempel https://bokis.ditt-tailnet.ts.net"
            ));
        }
        Ok(origin.to_ascii_lowercase())
    }

    /// Värden i den publika adressen, som den står i Host-huvudet.
    pub fn public_host(&self) -> &str {
        self.origin.split_once("://").map_or("", |(_, h)| h)
    }

    /// Agentens adress, som den skrivs in i Claude.
    pub fn mcp_url(&self) -> String {
        format!("{}/mcp", self.origin)
    }
}

/// En utfärdad auktoriseringskod, i minnet tills den byts mot nycklar.
#[derive(Debug, Clone)]
struct AuthCode {
    client_id: String,
    redirect_uri: String,
    challenge: String,
    expires: i64,
}

/// Servern: håller lagret, arkivet och agenternas sessioner. Allt körs i en
/// tråd; anslutningarna läses och skrivs i egna trådar (se [`server`]).
pub struct App {
    config: Config,
    state: StateFile,
    store: Store,
    archive: Archive,
    company: String,
    /// En MCP-server per godkänd klient, så att klientens namn hamnar rätt i
    /// förslagens härkomst.
    agents: HashMap<String, Server<StoreBackend>>,
    codes: HashMap<String, AuthCode>,
    clock: Box<dyn Fn() -> i64 + Send>,
}

fn archive_beside(db: &Path) -> PathBuf {
    db.parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
        .join("underlag")
}

impl App {
    pub fn open(config: Config) -> Result<Self, String> {
        let store = Store::open(&config.db, &config.program)
            .map_err(|e| format!("kan inte öppna {}: {e}", config.db.display()))?;
        let archive = Archive::open(archive_beside(&config.db)).map_err(|e| e.to_string())?;
        let company = store.company().map_err(|e| e.to_string())?.name;
        let state = StateFile::beside(&config.db);
        let origin = config.origin.clone();
        state
            .update(chrono::Utc::now().timestamp(), |s| s.origin = Some(origin))
            .map_err(|e| e.to_string())?;
        Ok(Self {
            config,
            state,
            store,
            archive,
            company,
            agents: HashMap::new(),
            codes: HashMap::new(),
            clock: Box::new(|| chrono::Utc::now().timestamp()),
        })
    }

    /// Byter klockan, för tester.
    pub fn set_clock(&mut self, clock: impl Fn() -> i64 + Send + 'static) {
        self.clock = Box::new(clock);
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    fn now(&self) -> i64 {
        (self.clock)()
    }

    fn host_allowed(&self, req: &Request) -> bool {
        let host = req.header("host").unwrap_or("").to_ascii_lowercase();
        host == self.config.public_host()
            || host == format!("127.0.0.1:{}", self.config.port)
            || host == format!("localhost:{}", self.config.port)
    }

    /// Ett Origin-huvud, om det finns, måste vara serverns egen adress. Det
    /// stoppar formulär och anrop från andra webbplatser i en webbläsare.
    fn origin_allowed(&self, req: &Request) -> bool {
        match req.header("origin") {
            None => true,
            Some(o) => {
                let o = o.trim_end_matches('/').to_ascii_lowercase();
                o == self.config.origin
                    || o == format!("http://127.0.0.1:{}", self.config.port)
                    || o == format!("http://localhost:{}", self.config.port)
            }
        }
    }

    /// Svarar på en förfrågan.
    pub fn handle(&mut self, req: &Request) -> Response {
        if !self.host_allowed(req) {
            return pages::message(
                403,
                "Fel adress",
                "Den här servern svarar bara på sin egen adress.",
            );
        }
        let path = req.path().to_string();
        let method = req.method.as_str();
        if method == "POST"
            && !self.origin_allowed(req)
            && !path.starts_with("/oauth/token")
            && path != "/oauth/register"
        {
            return pages::message(403, "Nekat", "Anropet kom från en annan webbplats.");
        }
        match (method, path.as_str()) {
            ("GET", "/.well-known/oauth-protected-resource")
            | ("GET", "/.well-known/oauth-protected-resource/mcp") => {
                Response::json(200, &oauth::resource_metadata(&self.config))
            }
            ("GET", "/.well-known/oauth-authorization-server")
            | ("GET", "/.well-known/oauth-authorization-server/mcp") => {
                Response::json(200, &oauth::server_metadata(&self.config))
            }
            ("POST", "/oauth/register") => self.register(req),
            ("GET", "/oauth/authorize") => self.authorize_get(req),
            ("POST", "/oauth/authorize") => self.authorize_post(req),
            ("POST", "/oauth/token") => self.token(req),
            ("POST", "/mcp") => self.mcp(req),
            // Den som öppnar agentens adress i en webbläsare får veta vad den är till för.
            ("GET", "/mcp")
                if req
                    .header("accept")
                    .is_some_and(|a| a.contains("text/html")) =>
            {
                pages::message(
                    405,
                    "Adressen är till för agenten",
                    "Lägg till den här adressen som anpassad anslutning i Claude, ChatGPT eller Claude Code. I en webbläsare finns inget att visa här.",
                )
                .with("Allow", "POST")
            }
            (_, "/mcp") => Response::empty(405).with("Allow", "POST"),
            ("GET", "/stil.css") => Response::new(200, "text/css; charset=utf-8", pages::CSS),
            (_, "/") => self.start(req),
            (_, p) if p.starts_with("/in/") => self.upload(req),
            _ => pages::message(404, "Finns inte", "Sidan finns inte."),
        }
    }

    /// Agentens gränssnitt: ett JSON-RPC-meddelande per POST.
    fn mcp(&mut self, req: &Request) -> Response {
        let now = self.now();
        let token = req
            .header("authorization")
            .and_then(|h| h.split_once(' '))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
            .map(|(_, t)| t.trim().to_string());
        let client = match &token {
            Some(t) => self
                .state
                .read()
                .ok()
                .and_then(|s| s.access(t, now).cloned()),
            None => None,
        };
        let Some(client) = client else {
            let metadata = format!(
                "{}/.well-known/oauth-protected-resource/mcp",
                self.config.origin
            );
            let challenge = if token.is_some() {
                format!("Bearer error=\"invalid_token\", resource_metadata=\"{metadata}\"")
            } else {
                format!("Bearer resource_metadata=\"{metadata}\"")
            };
            return Response::json(
                401,
                &json!({"error": "invalid_token", "error_description": "Anslut med OAuth; se resource_metadata."}),
            )
            .with("WWW-Authenticate", challenge);
        };
        if let Some(v) = req.header("mcp-protocol-version") {
            if !bok_mcp::server::PROTOCOL_VERSIONS.contains(&v.trim()) {
                return Response::json(
                    400,
                    &json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32600, "message": format!(
                        "Protokollversionen {} stöds inte. Servern talar {}.",
                        v.trim(),
                        bok_mcp::server::PROTOCOL_VERSIONS.join(", ")
                    )}}),
                );
            }
        }
        let Ok(text) = std::str::from_utf8(&req.body) else {
            return Response::json(
                400,
                &json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "Parse error"}}),
            );
        };
        let _ = self.state.update(now, |s| {
            if let Some(c) = s.client_mut(&client.id) {
                c.last_used = Some(now);
            }
        });
        if !self.agents.contains_key(&client.id) {
            match StoreBackend::open(&self.config.db, &self.config.program) {
                Ok(backend) => {
                    self.agents.insert(client.id.clone(), Server::new(backend));
                }
                Err(e) => return pages::message(500, "Fel", &e.to_string()),
            }
        }
        let Some(agent) = self.agents.get_mut(&client.id) else {
            return pages::message(500, "Fel", "agenten saknas");
        };
        match agent.handle_line(text) {
            Some(reply) => {
                let parse_error = serde_json::from_str::<serde_json::Value>(&reply)
                    .map(|v| v.get("error").is_some() && v["id"].is_null())
                    .unwrap_or(false);
                Response::new(
                    if parse_error { 400 } else { 200 },
                    "application/json",
                    reply,
                )
            }
            None => Response::empty(202),
        }
    }
}
