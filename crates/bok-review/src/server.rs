//! Ett minimalt HTTP/1.1-lager för granskningsvyn: en förfrågan per
//! anslutning, bara på 127.0.0.1. Vyn har en enda användare, så ingen
//! samtidighet behövs.

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

use crate::backend::ReviewBackend;
use crate::router::{Request, Response, Router};

const MAX_HEAD: usize = 64 * 1024;
const MAX_BODY: usize = 1024 * 1024;

/// Slumpad token, 32 byte som hex.
pub fn random_token() -> io::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(hex::encode(bytes))
}

/// Binder till 127.0.0.1. Port 0 ger en ledig port.
pub fn bind(port: u16) -> io::Result<(TcpListener, u16)> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let port = listener.local_addr()?.port();
    Ok((listener, port))
}

fn find_head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

/// Läser en förfrågan. `Ok(None)` om klienten stängde utan att skicka något.
pub fn read_request(stream: &mut TcpStream) -> io::Result<Option<Request>> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let head_end = loop {
        if let Some(end) = find_head_end(&buf) {
            break end;
        }
        if buf.len() > MAX_HEAD {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "för stora huvuden",
            ));
        }
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            return if buf.is_empty() {
                Ok(None)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "ofullständig förfrågan",
                ))
            };
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("/").to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let length: usize = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    if length > MAX_BODY {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "för stor kropp"));
    }
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < length {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(length);
    Ok(Some(Request {
        method,
        target,
        headers,
        body,
    }))
}

fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        303 => "See Other",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        _ => "Internal Server Error",
    }
}

/// Skriver ett svar och stänger anslutningen.
pub fn write_response(stream: &mut TcpStream, response: &Response) -> io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {} {}\r\n",
        response.status,
        status_text(response.status)
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
pub fn serve<B: ReviewBackend>(
    router: &mut Router<B>,
    listener: &TcpListener,
    max_requests: Option<usize>,
) -> io::Result<()> {
    let mut handled = 0usize;
    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("bok-review: anslutningen misslyckades: {e}");
                continue;
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        match read_request(&mut stream) {
            Ok(Some(req)) => {
                let resp = router.handle(&req);
                if let Err(e) = write_response(&mut stream, &resp) {
                    eprintln!("bok-review: kunde inte svara: {e}");
                }
            }
            Ok(None) => {}
            Err(e) => {
                let resp = Response {
                    status: 400,
                    headers: vec![("Content-Type".into(), "text/plain; charset=utf-8".into())],
                    body: format!("Felaktig förfrågan: {e}").into_bytes(),
                };
                let _ = write_response(&mut stream, &resp);
            }
        }
        handled += 1;
        if max_requests.is_some_and(|m| handled >= m) {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::backend::MemoryReviewBackend;

    fn exchange(port: u16, raw: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.write_all(raw.as_bytes()).unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    }

    #[test]
    fn tokens_are_random_and_long() {
        let a = random_token().unwrap();
        let b = random_token().unwrap();
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
    }

    #[test]
    fn real_socket_round_trip() {
        let (listener, port) = bind(0).unwrap();
        let login = "l".repeat(64);
        let session = "s".repeat(64);
        let csrf = "c".repeat(64);
        let mut router = Router::new(
            MemoryReviewBackend::demo(),
            login.clone(),
            session.clone(),
            csrf.clone(),
            port,
        );
        let worker = std::thread::spawn(move || {
            serve(&mut router, &listener, Some(3)).unwrap();
            router.backend().approved.clone()
        });

        let login = exchange(
            port,
            &format!("GET /login?token={login} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"),
        );
        assert!(login.starts_with("HTTP/1.1 303"), "{login}");
        assert!(login.contains("Set-Cookie: bok_session="));

        let list = exchange(
            port,
            &format!(
                "GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nCookie: bok_session={session}\r\n\r\n"
            ),
        );
        assert!(list.starts_with("HTTP/1.1 200"));
        assert!(list.contains("Förslag att granska"));

        let body = format!("csrf={csrf}");
        let approve = exchange(
            port,
            &format!(
                "POST /forslag/F1/godkann HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nCookie: bok_session={session}\r\nOrigin: http://127.0.0.1:{port}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            ),
        );
        assert!(approve.starts_with("HTTP/1.1 200"), "{approve}");
        assert_eq!(
            worker.join().unwrap(),
            vec![("F1".to_string(), "A1".to_string())]
        );
    }
}
