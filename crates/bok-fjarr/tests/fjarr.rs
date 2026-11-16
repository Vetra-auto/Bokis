//! Hela vägen som Claude och telefonen går: upptäckt, registrering,
//! godkännande med kopplingskod, PKCE, nycklar, MCP-anrop med bilder, och
//! uppladdning av kvittofoton. Och skydden: värd, ursprung, återkallade
//! anslutningar och länkar, och att inget kan bokföras.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{Cursor, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};
use tempfile::TempDir;

use bok_fjarr::http::{Request, Response};
use bok_fjarr::state::{self, Link, LinkCode, Pairing, StateFile};
use bok_fjarr::{server, App, Config};
use bok_store::bok_core::signature::SoftwareSigner;
use bok_store::{NewBook, Store};

const ORIGIN: &str = "https://bokis.example";
const HOST: &str = "bokis.example";
const CALLBACK: &str = "https://claude.ai/api/mcp/auth_callback";
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
const START: i64 = 1_800_000_000;

struct Fixture {
    _dir: TempDir,
    db: PathBuf,
    app: App,
    clock: Arc<AtomicI64>,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("bokis.db");
        let signer = SoftwareSigner::from_seed([9; 32]);
        Store::create(
            &db,
            "test",
            NewBook {
                company_name: "Fjärrbolaget AB",
                org_number: "559999-0002",
                key_name: "granskare",
                public_key: &signer.public_key_openssh(),
                signer: &signer,
                security_level: bok_store::bok_core::security::SecurityLevel::Software,
            },
        )
        .unwrap();
        let config = Config {
            origin: Config::origin_from(ORIGIN).unwrap(),
            port: 8770,
            db: db.clone(),
            user: "henrik".into(),
            program: "bok-fjarr test".into(),
        };
        let mut app = App::open(config).unwrap();
        let clock = Arc::new(AtomicI64::new(START));
        let c = Arc::clone(&clock);
        app.set_clock(move || c.load(Ordering::SeqCst));
        Fixture {
            _dir: dir,
            db,
            app,
            clock,
        }
    }

    fn state(&self) -> StateFile {
        StateFile::beside(&self.db)
    }

    fn advance(&self, seconds: i64) {
        self.clock.fetch_add(seconds, Ordering::SeqCst);
    }

    fn now(&self) -> i64 {
        self.clock.load(Ordering::SeqCst)
    }

    fn send(
        &mut self,
        method: &str,
        target: &str,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> Response {
        let mut all = vec![("Host".to_string(), HOST.to_string())];
        all.extend(headers.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        self.app.handle(&Request {
            method: method.into(),
            target: target.into(),
            headers: all,
            body: body.to_vec(),
        })
    }

    fn json(r: &Response) -> Value {
        serde_json::from_slice(&r.body)
            .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&r.body)))
    }

    fn text(r: &Response) -> String {
        String::from_utf8_lossy(&r.body).into_owned()
    }

    /// Som `bok-fjarr kod`.
    fn pairing(&self, code: &str) {
        let now = self.now();
        self.state()
            .update(now, |s| {
                s.pairing = Some(Pairing {
                    hash: state::hash(&state::normalize_code(code)),
                    expires: now + state::PAIRING_TTL,
                    attempts: 0,
                });
            })
            .unwrap();
    }

    /// Som `bok-fjarr uppladdning <namn>`.
    fn link_code(&self, name: &str, code: &str) {
        let now = self.now();
        self.state()
            .update(now, |s| {
                s.link_code = Some(LinkCode {
                    hash: state::hash(&state::normalize_code(code)),
                    name: name.into(),
                    expires: now + state::LINK_CODE_TTL,
                    attempts: 0,
                });
            })
            .unwrap();
    }

    /// Telefonen skriver koden på startsidan.
    fn redeem(&mut self, code: &str) -> Response {
        let body = format!("kod={}", bok_fjarr::http::encode(code));
        self.send(
            "POST",
            "/",
            &[
                ("Content-Type", "application/x-www-form-urlencoded"),
                ("Origin", ORIGIN),
            ],
            body.as_bytes(),
        )
    }

    fn register(&mut self, redirect: &str) -> String {
        let r = self.send(
            "POST",
            "/oauth/register",
            &[("Content-Type", "application/json")],
            json!({"client_name": "Claude", "redirect_uris": [redirect], "token_endpoint_auth_method": "none",
                   "grant_types": ["authorization_code", "refresh_token"], "response_types": ["code"]})
            .to_string()
            .as_bytes(),
        );
        assert_eq!(r.status, 201, "{}", Self::text(&r));
        Self::json(&r)["client_id"].as_str().unwrap().to_string()
    }

    fn authorize_query(client: &str, redirect: &str) -> String {
        format!(
            "response_type=code&client_id={client}&redirect_uri={}&code_challenge={CHALLENGE}&code_challenge_method=S256&state=xyz&scope=bokis&resource={}",
            bok_fjarr::http::encode(redirect),
            bok_fjarr::http::encode(&format!("{ORIGIN}/mcp"))
        )
    }

    fn approve(&mut self, client: &str, redirect: &str, code: &str) -> Response {
        let body = format!(
            "{}&kod={}",
            Self::authorize_query(client, redirect),
            bok_fjarr::http::encode(code)
        );
        self.send(
            "POST",
            "/oauth/authorize",
            &[
                ("Content-Type", "application/x-www-form-urlencoded"),
                ("Origin", ORIGIN),
            ],
            body.as_bytes(),
        )
    }

    fn exchange(&mut self, client: &str, auth_code: &str, verifier: &str) -> Response {
        let body = format!(
            "grant_type=authorization_code&code={auth_code}&client_id={client}&redirect_uri={}&code_verifier={verifier}",
            bok_fjarr::http::encode(CALLBACK)
        );
        self.send(
            "POST",
            "/oauth/token",
            &[("Content-Type", "application/x-www-form-urlencoded")],
            body.as_bytes(),
        )
    }

    /// Registrering, kopplingskod, godkännande och nycklar: som Claude gör.
    fn connect(&mut self) -> (String, String, String) {
        let client = self.register(CALLBACK);
        self.pairing("7KQM-X3PD");
        let r = self.approve(&client, CALLBACK, "7kqm x3pd");
        assert_eq!(r.status, 303, "{}", Self::text(&r));
        let code = code_from(&r);
        let r = self.exchange(&client, &code, VERIFIER);
        assert_eq!(r.status, 200, "{}", Self::text(&r));
        let tokens = Self::json(&r);
        (
            client,
            tokens["access_token"].as_str().unwrap().to_string(),
            tokens["refresh_token"].as_str().unwrap().to_string(),
        )
    }

    fn rpc(&mut self, token: &str, message: Value) -> Response {
        let auth = format!("Bearer {token}");
        self.send(
            "POST",
            "/mcp",
            &[
                ("Authorization", auth.as_str()),
                ("Content-Type", "application/json"),
                ("Accept", "application/json, text/event-stream"),
                ("MCP-Protocol-Version", "2025-06-18"),
            ],
            message.to_string().as_bytes(),
        )
    }
}

fn code_from(r: &Response) -> String {
    let location = r.header("Location").unwrap();
    let query = location.split_once('?').unwrap().1;
    let form = bok_fjarr::http::parse_form(query);
    bok_fjarr::http::form_value(&form, "code")
        .unwrap()
        .to_string()
}

#[test]
fn claude_finds_the_login_through_the_metadata() {
    let mut f = Fixture::new();
    let r = f.send("POST", "/mcp", &[], b"{}");
    assert_eq!(r.status, 401);
    assert_eq!(
        r.header("WWW-Authenticate"),
        Some("Bearer resource_metadata=\"https://bokis.example/.well-known/oauth-protected-resource/mcp\"")
    );
    let pr = Fixture::json(&f.send("GET", "/.well-known/oauth-protected-resource/mcp", &[], b""));
    assert_eq!(pr["resource"], "https://bokis.example/mcp");
    assert_eq!(
        pr["authorization_servers"],
        json!(["https://bokis.example"])
    );
    let root = Fixture::json(&f.send("GET", "/.well-known/oauth-protected-resource", &[], b""));
    assert_eq!(root, pr);
    let asm = Fixture::json(&f.send("GET", "/.well-known/oauth-authorization-server", &[], b""));
    assert_eq!(asm["issuer"], ORIGIN);
    assert_eq!(
        asm["registration_endpoint"],
        "https://bokis.example/oauth/register"
    );
    assert_eq!(asm["code_challenge_methods_supported"], json!(["S256"]));
    assert_eq!(
        asm["token_endpoint_auth_methods_supported"],
        json!(["none", "client_secret_post", "client_secret_basic"])
    );
    let bad = f.send("POST", "/mcp", &[("Authorization", "Bearer fel")], b"{}");
    assert_eq!(bad.status, 401);
    assert!(bad
        .header("WWW-Authenticate")
        .unwrap()
        .contains("error=\"invalid_token\""));
}

#[test]
fn the_whole_connection_needs_the_pairing_code_and_pkce() {
    let mut f = Fixture::new();
    let client = f.register(CALLBACK);

    let page = f.send(
        "GET",
        &format!(
            "/oauth/authorize?{}",
            Fixture::authorize_query(&client, CALLBACK)
        ),
        &[],
        b"",
    );
    assert_eq!(page.status, 200);
    let html = Fixture::text(&page);
    assert!(html.contains("Ansluta Claude till bokföringen?"), "{html}");
    assert!(html.contains("Efter godkännandet skickas du tillbaka till claude.ai."));
    assert!(page
        .header("Content-Security-Policy")
        .unwrap()
        .contains("form-action 'self' https://claude.ai"));
    // Med no-referrer skickar webbläsare `Origin: null` på formuläret, och då nekas det.
    assert_eq!(page.header("Referrer-Policy"), Some("same-origin"));
    assert!(!html.contains("no-referrer"));
    assert!(!html.contains("<script"));

    // Ingen kod finns ännu.
    let r = f.approve(&client, CALLBACK, "AAAA-AAAA");
    assert_eq!(r.status, 400);
    assert!(Fixture::text(&r).contains("ingen giltig kopplingskod"));

    f.pairing("7KQM-X3PD");
    let r = f.approve(&client, CALLBACK, "AAAA-AAAA");
    assert_eq!(r.status, 400);
    assert!(Fixture::text(&r).contains("4 försök kvar"));
    let r = f.approve(&client, CALLBACK, " 7kqm x3pd ");
    assert_eq!(r.status, 303);
    let location = r.header("Location").unwrap();
    assert!(
        location.starts_with("https://claude.ai/api/mcp/auth_callback?code="),
        "{location}"
    );
    assert!(location.contains("&state=xyz&iss=https%3A%2F%2Fbokis.example"));
    // Koden fungerar en gång. Skickas formuläret igen, till exempel med ett
    // dubbelklick, får man veta att det redan är klart, men ingen ny kod.
    let again = f.approve(&client, CALLBACK, "7KQM-X3PD");
    assert_eq!(again.status, 200);
    assert!(Fixture::text(&again).contains("redan godkänd"));
    assert!(again.header("Location").is_none());

    // Godkännandet står i behandlingshistoriken.
    let store = Store::open(&f.db, "test").unwrap();
    let last = store.events(Some(1)).unwrap().remove(0);
    assert_eq!(last.action, "fjarr.anslutning.godkand");
    assert_eq!(last.object.as_deref(), Some("fjarr:Claude"));
    assert_eq!(last.actor, "henrik");
    assert!(store.verify().unwrap().is_ok());

    // Fel verifierare: koden förbrukas ändå.
    let code = code_from(&r);
    let wrong = f.exchange(&client, &code, &"x".repeat(43));
    assert_eq!(wrong.status, 400);
    assert_eq!(Fixture::json(&wrong)["error"], "invalid_grant");
    assert_eq!(
        Fixture::json(&f.exchange(&client, &code, VERIFIER))["error"],
        "invalid_grant"
    );

    // En ny omgång går hela vägen.
    f.pairing("2345-6789");
    let r = f.approve(&client, CALLBACK, "23456789");
    let code = code_from(&r);
    assert_eq!(
        Fixture::json(&f.exchange("c-annan", &code, VERIFIER))["error"],
        "invalid_grant"
    );
    f.pairing("2345-6789");
    let code = code_from(&f.approve(&client, CALLBACK, "23456789"));
    let tokens = f.exchange(&client, &code, VERIFIER);
    assert_eq!(tokens.status, 200);
    assert_eq!(tokens.header("Cache-Control"), Some("no-store"));
    let t = Fixture::json(&tokens);
    assert_eq!(t["token_type"], "Bearer");
    assert_eq!(t["expires_in"], 3600);
    assert!(t["refresh_token"].as_str().unwrap().len() == 64);
}

#[test]
fn the_agent_gets_its_tools_but_can_never_post() {
    let mut f = Fixture::new();
    let (_, access, refresh) = f.connect();

    let init = f.rpc(
        &access,
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "claude-ai", "version": "1"}}}),
    );
    assert_eq!(init.status, 200);
    assert_eq!(init.header("Content-Type"), Some("application/json"));
    let init = Fixture::json(&init);
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(init["result"]["serverInfo"]["name"], "bok-mcp");

    assert_eq!(
        f.rpc(
            &access,
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
        )
        .status,
        202
    );
    let tools = Fixture::json(&f.rpc(
        &access,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    ));
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"foresla_verifikation") && names.contains(&"hamta_underlag"));
    assert!(!names.contains(&"bokfor_forslag"));
    let post = Fixture::json(&f.rpc(
        &access,
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "bokfor_forslag", "arguments": {"forslag_id": "1", "signatur": "x"}}}),
    ));
    assert!(post["error"]["message"]
        .as_str()
        .unwrap()
        .contains("finns inte"));

    assert_eq!(f.send("GET", "/mcp", &[], b"").status, 405);
    let browser = f.send(
        "GET",
        "/mcp",
        &[("Accept", "text/html,application/xhtml+xml")],
        b"",
    );
    assert_eq!(browser.status, 405);
    assert!(Fixture::text(&browser).contains("anpassad anslutning i Claude"));
    let stream = f.send("GET", "/mcp", &[("Accept", "text/event-stream")], b"");
    assert!(stream.status == 405 && stream.body.is_empty());
    let auth = format!("Bearer {access}");
    let future = f.send(
        "POST",
        "/mcp",
        &[
            ("Authorization", auth.as_str()),
            ("MCP-Protocol-Version", "2099-01-01"),
        ],
        b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}",
    );
    assert_eq!(future.status, 400);
    assert!(Fixture::text(&future).contains("2025-11-25"));
    let garbage = f.send(
        "POST",
        "/mcp",
        &[("Authorization", auth.as_str())],
        b"{inte json",
    );
    assert_eq!(garbage.status, 400);

    // Förnyelse byter ut förnyelsenyckeln; den gamla fungerar inte igen.
    let body = format!("grant_type=refresh_token&refresh_token={refresh}");
    let renewed = f.send("POST", "/oauth/token", &[], body.as_bytes());
    assert_eq!(renewed.status, 200);
    let new_access = Fixture::json(&renewed)["access_token"]
        .as_str()
        .unwrap()
        .to_string();
    let again = f.send("POST", "/oauth/token", &[], body.as_bytes());
    assert_eq!(Fixture::json(&again)["error"], "invalid_grant");
    assert_eq!(
        f.rpc(
            &new_access,
            json!({"jsonrpc": "2.0", "id": 4, "method": "ping"})
        )
        .status,
        200
    );

    // Åtkomstnyckeln går ut efter en timme.
    f.advance(3601);
    assert_eq!(
        f.rpc(
            &new_access,
            json!({"jsonrpc": "2.0", "id": 5, "method": "ping"})
        )
        .status,
        401
    );
}

#[test]
fn a_revoked_connection_stops_working_at_once() {
    let mut f = Fixture::new();
    let (client, access, refresh) = f.connect();
    assert_eq!(
        f.rpc(
            &access,
            json!({"jsonrpc": "2.0", "id": 1, "method": "ping"})
        )
        .status,
        200
    );
    // Som `bok-fjarr aterkalla`.
    let now = f.now();
    f.state()
        .update(now, |s| {
            s.client_mut(&client).unwrap().revoked = Some(now);
            s.tokens.retain(|t| t.client_id != client);
        })
        .unwrap();
    assert_eq!(
        f.rpc(
            &access,
            json!({"jsonrpc": "2.0", "id": 2, "method": "ping"})
        )
        .status,
        401
    );
    let body = format!("grant_type=refresh_token&refresh_token={refresh}");
    assert_eq!(
        Fixture::json(&f.send("POST", "/oauth/token", &[], body.as_bytes()))["error"],
        "invalid_grant"
    );
    let page = f.send(
        "GET",
        &format!(
            "/oauth/authorize?{}",
            Fixture::authorize_query(&client, CALLBACK)
        ),
        &[],
        b"",
    );
    assert_eq!(page.status, 400);
}

#[test]
fn redirects_codes_and_attempts_are_checked() {
    let mut f = Fixture::new();
    let bad = f.send(
        "POST",
        "/oauth/register",
        &[],
        json!({"client_name": "x", "redirect_uris": ["http://evil.example/cb"]})
            .to_string()
            .as_bytes(),
    );
    assert_eq!(bad.status, 400);
    assert_eq!(Fixture::json(&bad)["error"], "invalid_redirect_uri");

    // Claude Code: omdirigering till den egna datorn, med valfri port.
    let local = f.register("http://localhost/callback");
    let ok = f.send(
        "GET",
        &format!(
            "/oauth/authorize?{}",
            Fixture::authorize_query(&local, "http://localhost:3118/callback")
        ),
        &[],
        b"",
    );
    assert_eq!(ok.status, 200);
    let evil = f.send(
        "GET",
        &format!(
            "/oauth/authorize?{}",
            Fixture::authorize_query(&local, "https://evil.example/cb")
        ),
        &[],
        b"",
    );
    assert_eq!(evil.status, 400);
    assert!(evil.header("Location").is_none());
    // Utan PKCE skickas felet tillbaka till klienten.
    let no_pkce = f.send(
        "GET",
        &format!(
            "/oauth/authorize?response_type=code&client_id={local}&redirect_uri={}&state=s",
            bok_fjarr::http::encode("http://localhost:3118/callback")
        ),
        &[],
        b"",
    );
    assert_eq!(no_pkce.status, 303);
    assert!(no_pkce
        .header("Location")
        .unwrap()
        .contains("error=invalid_request"));

    // Kopplingskoden går ut efter 15 minuter.
    let client = f.register(CALLBACK);
    f.pairing("ABCD-EFGH");
    f.advance(state::PAIRING_TTL + 1);
    assert_eq!(f.approve(&client, CALLBACK, "ABCD-EFGH").status, 400);
    // Och efter fem felaktiga försök.
    f.pairing("ABCD-EFGH");
    for _ in 0..5 {
        assert_eq!(f.approve(&client, CALLBACK, "WXYZ-WXYZ").status, 400);
    }
    let last = f.approve(&client, CALLBACK, "ABCD-EFGH");
    assert_eq!(last.status, 400);
    assert!(Fixture::text(&last).contains("ingen giltig kopplingskod"));
}

#[test]
fn host_and_origin_are_checked() {
    let mut f = Fixture::new();
    let wrong_host = f.app.handle(&Request {
        method: "GET".into(),
        target: "/.well-known/oauth-authorization-server".into(),
        headers: vec![("Host".into(), "evil.example".into())],
        body: Vec::new(),
    });
    assert_eq!(wrong_host.status, 403);
    let local = f.app.handle(&Request {
        method: "GET".into(),
        target: "/.well-known/oauth-authorization-server".into(),
        headers: vec![("Host".into(), "127.0.0.1:8770".into())],
        body: Vec::new(),
    });
    assert_eq!(local.status, 200);
    let (_, access, _) = f.connect();
    let auth = format!("Bearer {access}");
    let cross = f.send(
        "POST",
        "/mcp",
        &[
            ("Authorization", auth.as_str()),
            ("Origin", "https://evil.example"),
        ],
        b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}",
    );
    assert_eq!(cross.status, 403);
    let client = f.register(CALLBACK);
    f.pairing("ABCD-EFGH");
    let body = format!(
        "{}&kod=ABCD-EFGH",
        Fixture::authorize_query(&client, CALLBACK)
    );
    let forged = f.send(
        "POST",
        "/oauth/authorize",
        &[("Origin", "https://evil.example")],
        body.as_bytes(),
    );
    assert_eq!(forged.status, 403);
}

fn jpeg() -> Vec<u8> {
    let img = image::RgbImage::from_fn(64, 48, |x, y| {
        image::Rgb([(x * 4) as u8, (y * 5) as u8, 200])
    });
    let mut out = Vec::new();
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Jpeg)
        .unwrap();
    out
}

fn multipart(files: &[(&str, &[u8])]) -> (String, Vec<u8>) {
    let boundary = "----bokis-test-7d93";
    let mut body = Vec::new();
    for (name, data) in files {
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"fil\"; filename=\"{name}\"\r\nContent-Type: application/octet-stream\r\n\r\n")
                .as_bytes(),
        );
        body.extend_from_slice(data);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

#[test]
fn the_phone_fetches_its_link_with_a_code() {
    let mut f = Fixture::new();

    // Vem som helst kan öppna startsidan, så den säger inte vems bokföringen är.
    let start = f.send("GET", "/", &[], b"");
    assert_eq!(start.status, 200);
    let html = Fixture::text(&start);
    assert!(html.contains("Hämta länken"), "{html}");
    assert!(html.contains(&format!("{ORIGIN}/mcp")));
    assert!(!html.contains("Fjärrbolaget"));
    assert_eq!(start.header("Referrer-Policy"), Some("same-origin"));

    // Utan kod och med fel kod blir det ingen länk, och svaret är detsamma.
    let r = f.redeem("7KQM-X3PD");
    assert_eq!(r.status, 400);
    let refused = Fixture::text(&r);
    assert!(refused.contains("Koden stämmer inte"));
    f.link_code("telefon", "7KQM-X3PD");
    for _ in 0..4 {
        let r = f.redeem("AAAA-BBBB");
        assert_eq!((r.status, Fixture::text(&r)), (400, refused.clone()));
    }

    // Rätt kod, som den skrivs på telefonen, ger en ny länk. Hemligheten
    // finns bara i adressen dit telefonen skickas.
    let r = f.redeem("7kqm x3pd");
    assert_eq!(r.status, 303, "{}", Fixture::text(&r));
    let location = r.header("Location").unwrap().to_string();
    let secret = location.strip_prefix("/in/").unwrap();
    assert!(secret.len() == 64 && secret.chars().all(|c| c.is_ascii_hexdigit()));
    let s = f.state().read().unwrap();
    assert!(s.link_code.is_none());
    assert_eq!(s.links.len(), 1);
    assert_eq!(s.links[0].name, "telefon");
    assert_eq!(s.links[0].hash, state::hash(secret));
    let store = Store::open(&f.db, "test").unwrap();
    let event = store.events(Some(1)).unwrap().remove(0);
    assert_eq!(
        (event.action.as_str(), event.object.as_deref()),
        ("fjarr.lank.skapad", Some("fjarr:telefon"))
    );

    // Koden gäller en gång. Bara webbläsaren som löste in den kommer, med
    // kakan, tillbaka till sin länk om formuläret skickas igen.
    let cookie = r.header("Set-Cookie").unwrap().to_string();
    assert!(
        cookie.contains("HttpOnly")
            && cookie.contains("Secure")
            && cookie.contains("SameSite=Strict")
    );
    assert_eq!(f.redeem("7KQM-X3PD").status, 400);
    let kaka = cookie.split(';').next().unwrap().to_string();
    let again = f.send(
        "POST",
        "/",
        &[
            ("Content-Type", "application/x-www-form-urlencoded"),
            ("Origin", ORIGIN),
            ("Cookie", &kaka),
        ],
        b"kod=7KQM-X3PD",
    );
    assert_eq!(again.status, 303);
    assert_eq!(again.header("Location"), Some(location.as_str()));
    let other = f.send(
        "POST",
        "/",
        &[
            ("Content-Type", "application/x-www-form-urlencoded"),
            ("Origin", ORIGIN),
            ("Cookie", "bok_lank=0000"),
        ],
        b"kod=7KQM-X3PD",
    );
    assert_eq!(other.status, 400);

    // Den nya sidan ber telefonen spara den, tills första underlaget är skickat.
    let page = Fixture::text(&f.send("GET", &location, &[], b""));
    assert!(page.contains("Spara den här sidan"), "{page}");
    assert!(page.contains("Fjärrbolaget AB"));
    let (ct, body) = multipart(&[("IMG_1.jpg", &jpeg())]);
    let r = f.send(
        "POST",
        &location,
        &[("Content-Type", ct.as_str()), ("Origin", ORIGIN)],
        &body,
    );
    assert_eq!(r.status, 303);
    let page = Fixture::text(&f.send("GET", &location, &[], b""));
    assert!(!page.contains("Spara den här sidan"));
    assert!(page.contains("är personlig. Spara den på hemskärmen"));

    // Efter fem fel slutar koden gälla, även för den som sedan gissar rätt.
    f.link_code("ipad", "ABCD-EFGH");
    for _ in 0..5 {
        assert_eq!(f.redeem("2222-3333").status, 400);
    }
    assert!(f.state().read().unwrap().link_code.is_none());
    assert_eq!(f.redeem("ABCD-EFGH").status, 400);

    // En kod gäller i 15 minuter.
    f.link_code("ipad", "ABCD-EFGH");
    f.advance(state::LINK_CODE_TTL + 1);
    assert_eq!(f.redeem("ABCD-EFGH").status, 400);

    // Ett namn som redan har en länk får ingen till.
    f.link_code("Telefon", "WXYZ-2345");
    let r = f.redeem("WXYZ-2345");
    assert_eq!(r.status, 400);
    assert!(Fixture::text(&r).contains("finns redan"));
    assert_eq!(f.state().read().unwrap().links.len(), 1);

    // Ett formulär från en annan webbplats kommer inte fram till koden.
    f.link_code("ipad", "ABCD-EFGH");
    let r = f.send(
        "POST",
        "/",
        &[
            ("Content-Type", "application/x-www-form-urlencoded"),
            ("Origin", "https://evil.example"),
        ],
        b"kod=ABCD-EFGH",
    );
    assert_eq!(r.status, 403);
    assert_eq!(
        f.state().read().unwrap().link_code.map(|c| c.attempts),
        Some(0)
    );
    assert_eq!(f.send("PUT", "/", &[], b"").status, 405);
}

#[test]
fn the_phone_sends_a_receipt_and_the_agent_sees_it() {
    let mut f = Fixture::new();
    let secret = state::random_secret().unwrap();
    let now = f.now();
    f.state()
        .update(now, |s| {
            s.links.push(Link {
                hash: state::hash(&secret),
                name: "iphone".into(),
                created: now,
                revoked: None,
                last_used: None,
            });
        })
        .unwrap();
    let path = format!("/in/{secret}");

    let page = f.send("GET", &path, &[], b"");
    assert_eq!(page.status, 200);
    let html = Fixture::text(&page);
    assert!(html.contains("Skicka till Bokis") && html.contains("Fjärrbolaget AB"));
    assert!(html.contains("enctype=\"multipart/form-data\""));
    assert_eq!(page.header("Referrer-Policy"), Some("same-origin"));
    assert_eq!(f.send("GET", "/in/fel", &[], b"").status, 404);

    let photo = jpeg();
    let (ct, body) = multipart(&[("IMG_2041.jpg", &photo), ("anteckning.txt", b"hej")]);
    let r = f.send(
        "POST",
        &path,
        &[("Content-Type", ct.as_str()), ("Origin", ORIGIN)],
        &body,
    );
    assert_eq!(r.status, 303, "{}", Fixture::text(&r));
    let location = r.header("Location").unwrap().to_string();
    assert_eq!(location, format!("{path}?mottaget=1&nekade=1"));

    let store = Store::open(&f.db, "test").unwrap();
    let a = store.attachments().unwrap().remove(0);
    assert_eq!(a.original_name.as_deref(), Some("IMG_2041.jpg"));
    assert_eq!(a.sender.as_deref(), Some("uppladdningslänken iphone"));
    assert_eq!(a.channel, "manuell");
    let event = store.events(Some(1)).unwrap().remove(0);
    assert_eq!(
        (event.action.as_str(), event.role.as_str()),
        ("underlag.mottaget", "mobil")
    );

    let done = Fixture::text(&f.send("GET", &location, &[], b""));
    assert!(
        done.contains("<strong>Mottaget:</strong> underlag 1, IMG_2041.jpg"),
        "{done}"
    );
    assert!(done.contains("<q>Bokför underlag 1</q>"));
    assert!(done.contains("En fil togs inte emot"));
    assert!(done.contains("väntar på agenten"));

    // Samma foto igen blir inget nytt underlag.
    let (ct, body) = multipart(&[("IMG_2041 kopia.jpg", &photo)]);
    let r = f.send("POST", &path, &[("Content-Type", ct.as_str())], &body);
    assert_eq!(
        r.header("Location"),
        Some(format!("{path}?fanns=1").as_str())
    );

    // Agenten ser fotot.
    let (_, access, _) = f.connect();
    let doc = Fixture::json(&f.rpc(
        &access,
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "hamta_underlag", "arguments": {"id": "1"}}}),
    ));
    let content = doc["result"]["content"].as_array().unwrap();
    let image = content.iter().find(|c| c["type"] == "image").unwrap();
    assert_eq!(image["mimeType"], "image/jpeg");

    // Från en annan webbplats, och efter återkallelse, går det inte.
    let (ct, body) = multipart(&[("x.jpg", &photo)]);
    assert_eq!(
        f.send(
            "POST",
            &path,
            &[
                ("Content-Type", ct.as_str()),
                ("Origin", "https://evil.example")
            ],
            &body
        )
        .status,
        403
    );
    let now = f.now();
    f.state()
        .update(now, |s| s.links[0].revoked = Some(now))
        .unwrap();
    assert_eq!(f.send("GET", &path, &[], b"").status, 404);
}

#[test]
fn the_server_speaks_http_over_a_socket() {
    let f = Fixture::new();
    let (listener, port) = server::bind(0).unwrap();
    let config = Config {
        origin: format!("http://127.0.0.1:{port}"),
        port,
        db: f.db.clone(),
        user: "henrik".into(),
        program: "test".into(),
    };
    let app = App::open(config).unwrap();
    let handle = std::thread::spawn(move || server::serve(app, &listener, Some(2)).unwrap());

    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        s,
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{{}}\r\n0\r\n\r\n"
    )
    .unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    assert!(out.starts_with("HTTP/1.1 401 Unauthorized\r\n"), "{out}");
    assert!(out.contains("WWW-Authenticate: Bearer resource_metadata="));

    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        s,
        "POST /oauth/register HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 3\r\nExpect: 100-continue\r\n\r\n"
    )
    .unwrap();
    let mut first = [0u8; 25];
    s.read_exact(&mut first).unwrap();
    assert_eq!(&first, b"HTTP/1.1 100 Continue\r\n\r\n");
    s.write_all(b"{x}").unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    assert!(out.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{out}");
    handle.join().unwrap();
}

#[test]
fn only_a_valid_link_may_send_a_large_upload() {
    let f = Fixture::new();
    let secret = state::random_secret().unwrap();
    let now = f.now();
    f.state()
        .update(now, |s| {
            s.links.push(Link {
                hash: state::hash(&secret),
                name: "telefon".into(),
                created: now,
                revoked: None,
                last_used: None,
            });
        })
        .unwrap();
    let (listener, port) = server::bind(0).unwrap();
    let origin = format!("http://127.0.0.1:{port}");
    let config = Config {
        origin: origin.clone(),
        port,
        db: f.db.clone(),
        user: "henrik".into(),
        program: "test".into(),
    };
    let app = App::open(config).unwrap();
    let handle = std::thread::spawn(move || server::serve(app, &listener, Some(2)).unwrap());

    // En okänd länk får svar direkt. Servern väntar inte på de 5 MB som
    // huvudet utlovar, och de skickas aldrig.
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .unwrap();
    write!(
        s,
        "POST /in/{} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: {origin}\r\nContent-Type: multipart/form-data; boundary=x\r\nContent-Length: 5000000\r\n\r\n",
        "0".repeat(64)
    )
    .unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    assert!(out.starts_with("HTTP/1.1 404 Not Found\r\n"), "{out}");
    assert!(out.contains("Länken gäller inte"));

    // Med den giltiga länken tas ett foto som är större än en vanlig
    // förfrågan emot.
    let mut photo = jpeg();
    photo.resize(2 * 1024 * 1024, 0);
    let (ct, body) = multipart(&[("IMG_2042.jpg", &photo)]);
    assert!(body.len() > bok_fjarr::MAX_BODY);
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        s,
        "POST /in/{secret} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: {origin}\r\nContent-Type: {ct}\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .unwrap();
    s.write_all(&body).unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    assert!(out.starts_with("HTTP/1.1 303 See Other\r\n"), "{out}");
    handle.join().unwrap();

    let store = Store::open(&f.db, "test").unwrap();
    assert_eq!(store.attachments().unwrap().len(), 1);
}

#[test]
fn a_client_that_asks_for_a_secret_must_use_it() {
    use base64::Engine;
    let mut f = Fixture::new();
    let r = f.send(
        "POST",
        "/oauth/register",
        &[("Content-Type", "application/json")],
        json!({"client_name": "Claude", "redirect_uris": [CALLBACK], "token_endpoint_auth_method": "client_secret_post"})
            .to_string()
            .as_bytes(),
    );
    assert_eq!(r.status, 201);
    let reg = Fixture::json(&r);
    assert_eq!(reg["token_endpoint_auth_method"], "client_secret_post");
    assert_eq!(reg["client_secret_expires_at"], 0);
    let client = reg["client_id"].as_str().unwrap().to_string();
    let secret = reg["client_secret"].as_str().unwrap().to_string();

    f.pairing("ABCD-EFGH");
    let code = code_from(&f.approve(&client, CALLBACK, "ABCD-EFGH"));
    let without = f.exchange(&client, &code, VERIFIER);
    assert_eq!(without.status, 401);
    assert_eq!(Fixture::json(&without)["error"], "invalid_client");

    f.pairing("ABCD-EFGH");
    let code = code_from(&f.approve(&client, CALLBACK, "ABCD-EFGH"));
    let body = format!(
        "grant_type=authorization_code&code={code}&client_id={client}&client_secret={secret}&redirect_uri={}&code_verifier={VERIFIER}",
        bok_fjarr::http::encode(CALLBACK)
    );
    let tokens = f.send("POST", "/oauth/token", &[], body.as_bytes());
    assert_eq!(tokens.status, 200, "{}", Fixture::text(&tokens));
    let refresh = Fixture::json(&tokens)["refresh_token"]
        .as_str()
        .unwrap()
        .to_string();

    // Förnyelse med Basic: id och hemlighet i huvudet.
    let basic = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{client}:{secret}"))
    );
    let body = format!("grant_type=refresh_token&refresh_token={refresh}");
    let wrong = f.send(
        "POST",
        "/oauth/token",
        &[("Authorization", "Basic YTpi")],
        body.as_bytes(),
    );
    assert_eq!(
        wrong.status,
        400,
        "en annan klient: {}",
        Fixture::text(&wrong)
    );
    let renewed = f.send(
        "POST",
        "/oauth/token",
        &[("Authorization", basic.as_str())],
        body.as_bytes(),
    );
    assert_eq!(renewed.status, 200, "{}", Fixture::text(&renewed));
}
