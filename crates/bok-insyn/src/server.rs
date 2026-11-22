//! Insynen live på 127.0.0.1.
//!
//! Skydden, i den ordning de prövas:
//! 1. Host-huvudet måste vara 127.0.0.1 eller localhost med rätt port (mot DNS-rebinding),
//!    eller adressen tjänsten nås på när den körs med `--adress` bakom
//!    `tailscale serve`.
//! 2. Bara GET besvaras. Insynen har inga formulär och kan inte ändra bokföringen.
//! 3. En länk ger en sessionskaka; utan den visas ingenting utom stilmallen.
//!    Lokalt är det startlänken, som fungerar en gång. Som tjänst är det en
//!    namngiven länk från `bok insyn lank --namn`, som gäller till sitt
//!    slutdatum eller tills den återkallas (se [`crate::links`]). Sessionen
//!    gäller högst [`crate::links::SESSION_TTL`] och aldrig längre än länken,
//!    och lagret tillfrågas vid varje sida om länken fortfarande gäller.
//!    Varje inloggning loggas i behandlingshistoriken innan kakan ges; går
//!    det inte att logga blir det ingen inloggning.
//! 4. Sidorna har en CSP utan skript och utan externa resurser. Underlag som
//!    inte är bild, PDF eller text skickas bara som nedladdning.
//!
//! Sidorna tas fram på nytt vid varje förfrågan, så det som visas är
//! bokföringen som den står just då.

use std::io::{self, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use bok_archive::Archive;
pub use bok_review::router::{Request, Response};
use bok_review::server::read_request;
pub use bok_review::server::{bind, random_token};
use bok_store::{Store, VerifyReport};
use chrono::{DateTime, Utc};

use crate::html::esc;
use crate::{File, Insyn, Mode, CSS};

const PAGE_CSP: &str = "default-src 'none'; style-src 'self'; img-src 'self'; object-src 'self'; \
frame-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

/// För underlag som visas direkt: bilder och text. Inget i dem får köras.
const FILE_CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; sandbox";

/// Kakans namn. Kakor skiljer inte på portar, så den får inte heta som
/// granskningsvyns.
const COOKIE: &str = "bok_insyn";

fn same_secret(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

fn page(status: u16, body: Vec<u8>) -> Response {
    Response {
        status,
        headers: vec![
            ("Content-Type".into(), "text/html; charset=utf-8".into()),
            ("Content-Security-Policy".into(), PAGE_CSP.into()),
            ("X-Frame-Options".into(), "DENY".into()),
            ("X-Content-Type-Options".into(), "nosniff".into()),
            ("Referrer-Policy".into(), "no-referrer".into()),
            ("Cache-Control".into(), "no-store".into()),
        ],
        body,
    }
}

fn message(status: u16, title: &str, text: &str) -> Response {
    let body = format!(
        "<!doctype html>\n<html lang=\"sv\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{t}</title>\n\
         <link rel=\"stylesheet\" href=\"/insyn.css\">\n</head>\n<body>\n<main class=\"blad\">\n<h1>{t}</h1>\n<p>{x}</p>\n</main>\n</body>\n</html>\n",
        t = esc(title),
        x = esc(text)
    );
    page(status, body.into_bytes())
}

fn file_response(file: File) -> Response {
    if file.content_type.starts_with("text/html") {
        return page(200, file.body);
    }
    let mut headers = vec![
        ("Content-Type".to_string(), file.content_type.to_string()),
        ("X-Content-Type-Options".into(), "nosniff".into()),
        ("Referrer-Policy".into(), "no-referrer".into()),
        ("Cache-Control".into(), "no-store".into()),
    ];
    if file.download {
        headers.push(("Content-Disposition".into(), "attachment".into()));
        headers.push((
            "Content-Security-Policy".into(),
            "default-src 'none'; sandbox".into(),
        ));
    } else if file.content_type != "application/pdf" && !file.content_type.starts_with("text/css") {
        // PDF-läsaren i webbläsaren fungerar inte i en sandlåda; en PDF körs
        // ändå aldrig som en webbsida.
        headers.push(("Content-Security-Policy".into(), FILE_CSP.into()));
    }
    Response {
        status: 200,
        headers,
        body: file.body,
    }
}

/// En inloggning som ska loggas innan den släpps in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Login {
    /// Med den namngivna länken `id`; sessionen gäller till `until`.
    Link { id: i64, until: DateTime<Utc> },
    /// Med startlänken på datorn med bokföringen.
    Local,
}

/// Skriver en inloggning i behandlingshistoriken. Insynen läser bara; det
/// här är det enda den låter skriva, och det görs genom en egen anslutning.
pub type LoginLog<'a> = Box<dyn FnMut(Login) -> Result<(), String> + 'a>;

fn cookie_value(req: &Request) -> Option<&str> {
    req.header("cookie").and_then(|c| {
        c.split(';')
            .filter_map(|part| part.trim().split_once('='))
            .find(|(k, _)| *k == COOKIE)
            .map(|(_, v)| v)
    })
}

/// Insynen live: håller sessionen och tar fram sidorna ur bokföringen.
pub struct LiveView<'a> {
    store: &'a Store,
    archive: &'a Archive,
    db: &'a Path,
    program: &'a str,
    /// Token i inloggningslänken. Den fungerar en gång och är inte kakans värde.
    login_token: Option<String>,
    session_token: String,
    allowed_hosts: Vec<String>,
    /// Filen med engångslänkar från `bok insyn lank`, när insynen är en tjänst.
    link_file: Option<PathBuf>,
    /// Kakan får `Secure` när insynen nås över https.
    secure_cookie: bool,
    /// Kontrollen av bokföringen och när den gjordes. Den läser varje
    /// underlag i arkivet, så den görs en gång och inte för varje sida.
    checked: Option<(VerifyReport, String)>,
    /// Loggar inloggningarna; utan den loggas inget (i tester).
    login_log: Option<LoginLog<'a>>,
}

impl<'a> LiveView<'a> {
    /// `port` är porten insynen lyssnar på. Nycklarna ska vara olika och slumpmässiga.
    pub fn new(
        store: &'a Store,
        archive: &'a Archive,
        db: &'a Path,
        program: &'a str,
        login_token: String,
        session_token: String,
        port: u16,
    ) -> Self {
        Self {
            store,
            archive,
            db,
            program,
            login_token: Some(login_token),
            session_token,
            allowed_hosts: vec![format!("127.0.0.1:{port}"), format!("localhost:{port}")],
            link_file: None,
            secure_cookie: false,
            checked: None,
            login_log: None,
        }
    }

    /// Loggar varje inloggning med `log` innan den släpps in.
    pub fn with_login_log(mut self, log: LoginLog<'a>) -> Self {
        self.login_log = Some(log);
        self
    }

    fn log(&mut self, login: Login) -> Result<(), String> {
        match &mut self.login_log {
            Some(log) => log(login),
            None => Ok(()),
        }
    }

    /// Länken bakom en giltig session som tjänst, om den gäller nu.
    fn remote_session(&self, req: &Request) -> bool {
        let (Some(value), Some(file)) = (cookie_value(req), &self.link_file) else {
            return false;
        };
        let now = self.store.now();
        crate::links::session_link(file, value, now.timestamp())
            .ok()
            .flatten()
            .and_then(|id| self.store.insyn_link(id).ok().flatten())
            .is_some_and(|l| l.active_at(now))
    }

    /// Insynen som tjänst: den nås också på `origin`, till exempel
    /// `https://bokis.tailnet.ts.net:8443` bakom `tailscale serve`, och
    /// inloggningslänkarna tas fram med `bok insyn lank` och läses ur
    /// `link_file`. Startlänken gäller inte.
    pub fn with_remote(mut self, origin: &str, link_file: PathBuf) -> Self {
        let host = origin
            .split_once("://")
            .map_or(origin, |(_, rest)| rest)
            .split('/')
            .next()
            .unwrap_or("")
            .to_string();
        if !host.is_empty() {
            if let Some((name, "443")) = host.rsplit_once(':') {
                self.allowed_hosts.push(name.to_string());
            }
            self.allowed_hosts.push(host);
        }
        self.secure_cookie = origin.starts_with("https://");
        self.login_token = None;
        self.link_file = Some(link_file);
        self
    }

    fn has_session(&self, req: &Request) -> bool {
        if self.link_file.is_some() {
            return self.remote_session(req);
        }
        cookie_value(req).is_some_and(|v| same_secret(v, &self.session_token))
    }

    /// Inloggning med en namngiven länk: länken ska finnas i filen och gälla
    /// enligt lagret. Ger sessionens hemlighet och sista tid.
    fn named_login(&mut self, token: &str) -> Result<Option<(String, DateTime<Utc>)>, String> {
        let Some(file) = self.link_file.clone() else {
            return Ok(None);
        };
        let now = self.store.now();
        let Some(id) =
            crate::links::link_for(&file, token, now.timestamp()).map_err(|e| e.to_string())?
        else {
            return Ok(None);
        };
        let Some(link) = self
            .store
            .insyn_link(id)
            .map_err(|e| e.to_string())?
            .filter(|l| l.active_at(now))
        else {
            return Ok(None);
        };
        let until = link
            .valid_until
            .min(now + chrono::Duration::seconds(crate::links::SESSION_TTL));
        self.log(Login::Link { id, until })?;
        let session = random_token().map_err(|e| e.to_string())?;
        crate::links::start_session(&file, id, &session, until.timestamp(), now.timestamp())
            .map_err(|e| e.to_string())?;
        Ok(Some((session, until)))
    }

    pub fn handle(&mut self, req: &Request) -> Response {
        let host = req.header("host").unwrap_or("");
        if !self.allowed_hosts.iter().any(|h| h == host) {
            return message(403, "Fel adress", "Insynen svarar bara på sin egen adress.");
        }
        if req.method != "GET" {
            let mut r = message(
                405,
                "Bara läsning",
                "Insynen visar bokföringen och tar inte emot något.",
            );
            r.headers.push(("Allow".into(), "GET".into()));
            return r;
        }
        let (path, query) = req
            .target
            .split_once('?')
            .unwrap_or((req.target.as_str(), ""));
        if path == "/login" {
            let token = query
                .split('&')
                .filter_map(|kv| kv.split_once('='))
                .find(|(k, _)| *k == "token")
                .map(|(_, v)| v);
            let started = match (token, &self.login_token) {
                (Some(t), Some(expected)) => same_secret(t, expected),
                _ => false,
            };
            let (session, max_age) = if started {
                if let Err(e) = self.log(Login::Local) {
                    return message(500, "Inloggningen kunde inte loggas", &e);
                }
                self.login_token = None;
                (self.session_token.clone(), String::new())
            } else {
                match token.filter(|t| !t.is_empty()).map(|t| self.named_login(t)) {
                    Some(Ok(Some((session, until)))) => {
                        let seconds = (until - self.store.now()).num_seconds().max(0);
                        (session, format!("; Max-Age={seconds}"))
                    }
                    Some(Err(e)) => {
                        return message(500, "Inloggningen kunde inte loggas", &e);
                    }
                    _ => {
                        let hint = if self.link_file.is_some() {
                            "Länken stämmer inte, har gått ut eller är återkallad. Be om en ny länk."
                        } else {
                            "Länken stämmer inte eller är redan använd. Starta insynen igen med bok insyn för en ny länk."
                        };
                        return message(403, "Fel länk", hint);
                    }
                }
            };
            let secure = if self.secure_cookie { "; Secure" } else { "" };
            return Response {
                status: 303,
                headers: vec![
                    ("Location".into(), "/".into()),
                    (
                        "Set-Cookie".into(),
                        format!(
                            "{COOKIE}={session}; HttpOnly; SameSite=Strict; Path=/{max_age}{secure}"
                        ),
                    ),
                    ("Cache-Control".into(), "no-store".into()),
                    ("Referrer-Policy".into(), "no-referrer".into()),
                ],
                body: Vec::new(),
            };
        }
        if path == "/insyn.css" {
            return file_response(File {
                content_type: "text/css; charset=utf-8",
                body: CSS.as_bytes().to_vec(),
                download: false,
            });
        }
        if !self.has_session(req) {
            return message(
                403,
                "Inte inloggad",
                if self.link_file.is_some() {
                    "Logga in med din länk igen, eller be om en ny om den har gått ut eller återkallats."
                } else {
                    "Öppna länken som visades i terminalen när insynen startade."
                },
            );
        }
        if path == "/kontrollera" {
            self.checked = None;
            return Response {
                status: 303,
                headers: vec![
                    ("Location".into(), "/".into()),
                    ("Cache-Control".into(), "no-store".into()),
                ],
                body: Vec::new(),
            };
        }
        let generated = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
        let mut insyn = match Insyn::new(
            self.store,
            self.archive,
            self.db,
            self.program,
            generated.clone(),
            Mode::Live,
        ) {
            Ok(i) => i,
            Err(e) => return message(500, "Fel", &e.to_string()),
        };
        if let Some((report, at)) = &self.checked {
            insyn = insyn.with_verify(report.clone(), at.clone());
        }
        let response = match insyn.get(path) {
            Ok(Some(file)) => file_response(file),
            Ok(None) => message(404, "Finns inte", "Sidan finns inte i insynen."),
            Err(e) => message(500, "Fel", &e.to_string()),
        };
        if self.checked.is_none() {
            self.checked = insyn.take_verify().map(|r| (r, generated));
        }
        response
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        303 => "See Other",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Internal Server Error",
    }
}

/// Skriver ett svar och stänger anslutningen.
pub fn write_response(stream: &mut TcpStream, response: &Response) -> io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {} {}\r\n",
        response.status,
        reason(response.status)
    );
    for (k, v) in &response.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        response.body.len()
    ));
    stream.write_all(head.as_bytes())?;
    stream.write_all(&response.body)?;
    stream.flush()
}

/// Besvarar förfrågningar tills `max_requests` är nått, eller för alltid.
pub fn serve(
    view: &mut LiveView,
    listener: &TcpListener,
    max_requests: Option<usize>,
) -> io::Result<()> {
    let mut handled = 0usize;
    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("bok insyn: anslutningen misslyckades: {e}");
                continue;
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        let response = match read_request(&mut stream) {
            Ok(Some(req)) => Some(view.handle(&req)),
            Ok(None) => None,
            Err(e) => Some(message(400, "Felaktig förfrågan", &e.to_string())),
        };
        if let Some(r) = response {
            if let Err(e) = write_response(&mut stream, &r) {
                eprintln!("bok insyn: kunde inte svara: {e}");
            }
        }
        handled += 1;
        if max_requests.is_some_and(|m| handled >= m) {
            break;
        }
    }
    Ok(())
}
