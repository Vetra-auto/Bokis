//! Nätverksdelen: varje anslutning läses och skrivs i en egen tråd, så att
//! en långsam uppladdning från en telefon inte stoppar agenten. Själva
//! förfrågningarna hanteras en i taget i en tråd som äger lagret.

use std::cell::Cell;
use std::io;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::http::{self, ReadError, Request, Response};
use crate::state::StateFile;
use crate::{pages, App, MAX_BODY, MAX_UPLOAD};

/// Högst så många anslutningar samtidigt.
const MAX_CONNECTIONS: usize = 32;

/// Binder till 127.0.0.1. Port 0 ger en ledig port.
pub fn bind(port: u16) -> io::Result<(TcpListener, u16)> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let port = listener.local_addr()?.port();
    Ok((listener, port))
}

/// Största kropp för en förfrågan, och om den gällde en uppladdningslänk som
/// inte finns. Bara en giltig länk får skicka stora filer. Länken kontrolleras
/// innan kroppen läses, så att den som inte har en länk inte kan få servern att
/// hålla 60 MB i minnet för var och en av sina anslutningar.
fn body_limit(links: &StateFile, method: &str, path: &str) -> (usize, bool) {
    match path.strip_prefix("/in/") {
        Some(secret) if method == "POST" => {
            if links.read().is_ok_and(|s| s.link(secret).is_some()) {
                (MAX_UPLOAD, false)
            } else {
                (MAX_BODY, true)
            }
        }
        _ => (MAX_BODY, false),
    }
}

type Job = (Request, mpsc::Sender<Response>);

/// Text utan styrtecken, högst `max` tecken, för loggen.
fn printable(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect()
}

/// En rad i åtkomstloggen: vad som anropades, men aldrig nycklar, koder,
/// länkarnas hemligheter eller innehåll.
pub(crate) fn describe(req: &Request) -> String {
    let path = req.path();
    let shown = if path.starts_with("/in/") {
        "/in/…"
    } else {
        path
    };
    let detail = match (req.method.as_str(), path) {
        ("POST", "/mcp") => serde_json::from_slice::<serde_json::Value>(&req.body)
            .ok()
            .map(|v| {
                let method = v["method"].as_str().unwrap_or("svar");
                match v["params"]["name"].as_str() {
                    Some(tool) => format!(" {method} {tool}"),
                    None => format!(" {method}"),
                }
            })
            .unwrap_or_default(),
        ("POST", "/oauth/token") => {
            let form = http::parse_form(&String::from_utf8_lossy(&req.body));
            format!(" {}", http::form_value(&form, "grant_type").unwrap_or("?"))
        }
        ("POST", "/oauth/register") => serde_json::from_slice::<serde_json::Value>(&req.body)
            .ok()
            .map(|v| {
                format!(
                    " {} ({})",
                    v["client_name"].as_str().unwrap_or("utan namn"),
                    v["token_endpoint_auth_method"]
                        .as_str()
                        .unwrap_or("ingen metod angiven")
                )
            })
            .unwrap_or_default(),
        _ => String::new(),
    };
    let agent = req
        .header("user-agent")
        .map(|ua| format!("  [{}]", printable(ua, 60)))
        .unwrap_or_default();
    format!(
        "{} {}{}{}",
        req.method,
        shown,
        printable(&detail, 120),
        agent
    )
}

fn log(line: &str, status: u16) {
    println!(
        "{}  {status}  {line}",
        chrono::Local::now().format("%H:%M:%S")
    );
}

fn connection(mut stream: TcpStream, jobs: &mpsc::Sender<Job>, links: &StateFile) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));
    let unknown_link = Cell::new(false);
    let limit = |method: &str, path: &str| {
        let (max, unknown) = body_limit(links, method, path);
        unknown_link.set(unknown);
        max
    };
    let (response, line) = match http::read_request(&mut stream, &limit) {
        Ok(Some(req)) => {
            let line = describe(&req);
            let (reply, answer) = mpsc::channel();
            if jobs.send((req, reply)).is_err() {
                return;
            }
            match answer.recv() {
                Ok(r) => (r, line),
                Err(_) => return,
            }
        }
        Ok(None) => return,
        // Samma svar som en liten förfrågan till en okänd länk får.
        Err(ReadError::TooLarge) if unknown_link.get() => (
            pages::link_gone(),
            "POST /in/…  okänd länk, kroppen lästes inte".to_string(),
        ),
        Err(ReadError::TooLarge) => (
            pages::message(413, "För stort", "Förfrågan är för stor."),
            "för stor förfrågan".to_string(),
        ),
        Err(ReadError::Bad(text)) => (
            pages::message(400, "Felaktig förfrågan", text),
            format!("felaktig förfrågan: {text}"),
        ),
        Err(ReadError::Io(_)) => return,
    };
    log(&line, response.status);
    if let Err(e) = http::write_response(&mut stream, &response) {
        eprintln!("bok-fjarr: kunde inte svara: {e}");
    }
}

/// Besvarar förfrågningar för alltid, eller tills `max_requests` är nått.
pub fn serve(app: App, listener: &TcpListener, max_requests: Option<usize>) -> io::Result<()> {
    let (jobs, queue) = mpsc::channel::<Job>();
    let links = app.state.clone();
    let worker = thread::spawn(move || {
        let mut app = app;
        for (req, reply) in queue {
            let response = app.handle(&req);
            let _ = reply.send(response);
        }
    });
    let active = Arc::new(AtomicUsize::new(0));
    let mut handled = 0usize;
    let mut threads = Vec::new();
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("bok-fjarr: anslutningen misslyckades: {e}");
                continue;
            }
        };
        if active.load(Ordering::SeqCst) >= MAX_CONNECTIONS {
            let mut stream = stream;
            let _ = http::write_response(
                &mut stream,
                &pages::message(
                    503,
                    "Upptaget",
                    "För många anslutningar just nu. Försök igen.",
                ),
            );
        } else {
            active.fetch_add(1, Ordering::SeqCst);
            let jobs = jobs.clone();
            let links = links.clone();
            let active = Arc::clone(&active);
            let handle = thread::spawn(move || {
                connection(stream, &jobs, &links);
                active.fetch_sub(1, Ordering::SeqCst);
            });
            // Bara när servern ska stanna behöver trådarna väntas in.
            if max_requests.is_some() {
                threads.push(handle);
            }
        }
        handled += 1;
        if max_requests.is_some_and(|m| handled >= m) {
            break;
        }
    }
    for t in threads {
        let _ = t.join();
    }
    drop(jobs);
    let _ = worker.join();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(method: &str, target: &str, body: &str) -> Request {
        Request {
            method: method.into(),
            target: target.into(),
            headers: vec![("User-Agent".into(), "Claude-User\u{7}/1.0".into())],
            body: body.as_bytes().to_vec(),
        }
    }

    #[test]
    fn the_log_never_shows_secrets() {
        let secret = "ab".repeat(32);
        let line = describe(&req("POST", &format!("/in/{secret}?mottaget=1"), "data"));
        assert_eq!(line, "POST /in/…  [Claude-User/1.0]");
        let line = describe(&req(
            "POST",
            "/oauth/token",
            "grant_type=refresh_token&refresh_token=hemlig&client_secret=hemligare",
        ));
        assert_eq!(line, "POST /oauth/token refresh_token  [Claude-User/1.0]");
        let line = describe(&req(
            "POST",
            "/mcp",
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"hamta_underlag","arguments":{"id":"16"}}}"#,
        ));
        assert_eq!(
            line,
            "POST /mcp tools/call hamta_underlag  [Claude-User/1.0]"
        );
        let line = describe(&req("GET", "/oauth/authorize?code_challenge=x&state=y", ""));
        assert!(!line.contains("code_challenge") && !line.contains("state"));
    }
}
