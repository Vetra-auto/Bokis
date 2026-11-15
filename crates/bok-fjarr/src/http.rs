//! Ett litet HTTP/1.1-lager: en förfrågan per anslutning, kroppen med
//! `Content-Length` eller `Transfer-Encoding: chunked`, och svar på
//! `Expect: 100-continue`. TLS sköts av den som publicerar servern, till
//! exempel Tailscale.

use std::io::{self, BufRead, BufReader, Read, Write};

/// Största tillåtna huvud, i byte.
pub const MAX_HEAD: usize = 32 * 1024;

/// En HTTP-förfrågan.
#[derive(Debug, Clone, Default)]
pub struct Request {
    pub method: String,
    /// Sökväg med eventuell query, till exempel `/oauth/authorize?client_id=…`.
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn path(&self) -> &str {
        self.target
            .split_once('?')
            .map_or(self.target.as_str(), |(p, _)| p)
    }

    pub fn query(&self) -> &str {
        self.target.split_once('?').map_or("", |(_, q)| q)
    }
}

/// Ett HTTP-svar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn new(status: u16, content_type: &str, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            headers: vec![
                ("Content-Type".into(), content_type.into()),
                ("Cache-Control".into(), "no-store".into()),
                ("X-Content-Type-Options".into(), "nosniff".into()),
                ("Referrer-Policy".into(), "no-referrer".into()),
            ],
            body: body.into(),
        }
    }

    pub fn json(status: u16, value: &serde_json::Value) -> Self {
        Self::new(status, "application/json", value.to_string())
    }

    pub fn empty(status: u16) -> Self {
        let mut r = Self::new(status, "text/plain; charset=utf-8", Vec::new());
        r.headers.retain(|(k, _)| k != "Content-Type");
        r
    }

    pub fn with(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Som [`Response::with`], men ersätter ett huvud med samma namn.
    pub fn replace(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
        self.with(name, value)
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Fel vid läsning av en förfrågan, med statuskoden som svaret ska ha.
#[derive(Debug)]
pub enum ReadError {
    Io(io::Error),
    Bad(&'static str),
    TooLarge,
}

impl From<io::Error> for ReadError {
    fn from(e: io::Error) -> Self {
        ReadError::Io(e)
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Läser en förfrågan. `max_body` ger största kropp för metod och sökväg.
/// `Ok(None)` om klienten stängde utan att skicka något.
pub fn read_request<S: Read + Write>(
    stream: &mut S,
    max_body: &dyn Fn(&str, &str) -> usize,
) -> Result<Option<Request>, ReadError> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let head_end = loop {
        if let Some(end) = find(&buf, b"\r\n\r\n") {
            break end;
        }
        if buf.len() > MAX_HEAD {
            return Err(ReadError::Bad("för stora huvuden"));
        }
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            return if buf.is_empty() {
                Ok(None)
            } else {
                Err(ReadError::Bad("ofullständig förfrågan"))
            };
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = std::str::from_utf8(&buf[..head_end])
        .map_err(|_| ReadError::Bad("huvudet är inte UTF-8"))?;
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next().unwrap_or("").split_whitespace();
    let method = request_line.next().unwrap_or("").to_string();
    let target = request_line.next().unwrap_or("").to_string();
    if method.is_empty() || !target.starts_with('/') {
        return Err(ReadError::Bad("felaktig förfrågningsrad"));
    }
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let mut req = Request {
        method,
        target,
        headers,
        body: Vec::new(),
    };
    let limit = max_body(&req.method, req.path());
    let leftover = buf[head_end + 4..].to_vec();
    let chunked = req
        .header("transfer-encoding")
        .is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
    let length = match req.header("content-length") {
        Some(v) => Some(
            v.parse::<usize>()
                .map_err(|_| ReadError::Bad("felaktig Content-Length"))?,
        ),
        None => None,
    };
    if length.is_some_and(|l| l > limit) {
        return Err(ReadError::TooLarge);
    }
    let expects_continue = req
        .header("expect")
        .is_some_and(|v| v.eq_ignore_ascii_case("100-continue"));
    if expects_continue && leftover.is_empty() && (chunked || length.unwrap_or(0) > 0) {
        stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")?;
        stream.flush()?;
    }
    let mut reader = BufReader::new(io::Cursor::new(leftover).chain(stream));
    if chunked {
        loop {
            let mut size_line = String::new();
            if reader.read_line(&mut size_line)? == 0 {
                return Err(ReadError::Bad("ofullständig chunk"));
            }
            let size_hex = size_line.trim().split(';').next().unwrap_or("");
            let size = usize::from_str_radix(size_hex, 16)
                .map_err(|_| ReadError::Bad("felaktig chunkstorlek"))?;
            if size == 0 {
                // Avslutande huvuden fram till en tom rad.
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line)? == 0 || line.trim().is_empty() {
                        break;
                    }
                }
                break;
            }
            if req.body.len() + size > limit {
                return Err(ReadError::TooLarge);
            }
            let start = req.body.len();
            req.body.resize(start + size, 0);
            reader.read_exact(&mut req.body[start..])?;
            let mut crlf = [0u8; 2];
            reader.read_exact(&mut crlf)?;
        }
    } else if let Some(length) = length {
        req.body.resize(length, 0);
        reader.read_exact(&mut req.body)?;
    }
    Ok(Some(req))
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        302 => "Found",
        303 => "See Other",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        429 => "Too Many Requests",
        503 => "Service Unavailable",
        _ => "Internal Server Error",
    }
}

/// Skriver ett svar och stänger anslutningen.
pub fn write_response<W: Write>(w: &mut W, response: &Response) -> io::Result<()> {
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
    w.write_all(head.as_bytes())?;
    w.write_all(&response.body)?;
    w.flush()
}

/// Avkodar `application/x-www-form-urlencoded`, även en query.
pub fn parse_form(input: &str) -> Vec<(String, String)> {
    input
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            (decode(k), decode(v))
        })
        .collect()
}

pub fn form_value<'a>(form: &'a [(String, String)], key: &str) -> Option<&'a str> {
    form.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Avkodar procentkodning och `+` som mellanslag.
pub fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' => match (
                bytes.get(i + 1).copied().and_then(hex_digit),
                bytes.get(i + 2).copied().and_then(hex_digit),
            ) {
                (Some(hi), Some(lo)) => {
                    out.push(hi * 16 + lo);
                    i += 2;
                }
                _ => out.push(b'%'),
            },
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Kodar ett värde för en query eller ett formulär.
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// En del i en `multipart/form-data`-kropp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    pub name: String,
    pub filename: Option<String>,
    pub data: Vec<u8>,
}

fn param(header: &str, key: &str) -> Option<String> {
    header.split(';').skip(1).find_map(|p| {
        let (k, v) = p.trim().split_once('=')?;
        if !k.trim().eq_ignore_ascii_case(key) {
            return None;
        }
        Some(v.trim().trim_matches('"').to_string())
    })
}

/// Delarna i en `multipart/form-data`-kropp.
pub fn parse_multipart(content_type: &str, body: &[u8]) -> Result<Vec<Part>, &'static str> {
    if !content_type
        .to_ascii_lowercase()
        .starts_with("multipart/form-data")
    {
        return Err("formuläret är inte multipart/form-data");
    }
    let boundary = param(content_type, "boundary").ok_or("boundary saknas")?;
    if boundary.is_empty() || boundary.len() > 200 {
        return Err("felaktig boundary");
    }
    let delimiter = format!("--{boundary}");
    let delimiter = delimiter.as_bytes();
    let mut parts = Vec::new();
    let mut pos = find(body, delimiter).ok_or("kroppen saknar delar")? + delimiter.len();
    loop {
        if body[pos..].starts_with(b"--") {
            break;
        }
        // Raden efter avgränsaren.
        if !body[pos..].starts_with(b"\r\n") {
            return Err("felaktig del");
        }
        pos += 2;
        let head_len = find(&body[pos..], b"\r\n\r\n").ok_or("delen saknar huvud")?;
        let head = String::from_utf8_lossy(&body[pos..pos + head_len]).into_owned();
        pos += head_len + 4;
        let mut next_delim = Vec::with_capacity(delimiter.len() + 2);
        next_delim.extend_from_slice(b"\r\n");
        next_delim.extend_from_slice(delimiter);
        let end = find(&body[pos..], &next_delim).ok_or("delen tar aldrig slut")?;
        let data = body[pos..pos + end].to_vec();
        pos += end + next_delim.len();
        let disposition = head
            .split("\r\n")
            .find(|l| l.to_ascii_lowercase().starts_with("content-disposition:"))
            .unwrap_or("");
        let name = param(disposition, "name").unwrap_or_default();
        let filename = param(disposition, "filename").filter(|f| !f.is_empty());
        parts.push(Part {
            name,
            filename,
            data,
        });
        if parts.len() > 50 {
            return Err("för många delar");
        }
    }
    Ok(parts)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    struct Fake {
        input: io::Cursor<Vec<u8>>,
        output: Vec<u8>,
    }

    impl Read for Fake {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.input.read(buf)
        }
    }

    impl Write for Fake {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.output.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn read(raw: &[u8], limit: usize) -> (Result<Option<Request>, ReadError>, Vec<u8>) {
        let mut fake = Fake {
            input: io::Cursor::new(raw.to_vec()),
            output: Vec::new(),
        };
        let r = read_request(&mut fake, &|_, _| limit);
        (r, fake.output)
    }

    #[test]
    fn reads_length_and_chunked_bodies() {
        let (r, _) = read(
            b"POST /mcp HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\n\r\nhello",
            100,
        );
        let r = r.unwrap().unwrap();
        assert_eq!(
            (r.method.as_str(), r.path(), r.body.as_slice()),
            ("POST", "/mcp", &b"hello"[..])
        );
        let (r, _) = read(
            b"POST /mcp?a=1 HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6;x=y\r\n world\r\n0\r\n\r\n",
            100,
        );
        let r = r.unwrap().unwrap();
        assert_eq!(r.body, b"hello world");
        assert_eq!(r.query(), "a=1");
    }

    #[test]
    fn answers_expect_continue_and_refuses_large_bodies() {
        let (r, out) = read(
            b"POST /in/x HTTP/1.1\r\nExpect: 100-continue\r\nContent-Length: 3\r\n\r\n",
            100,
        );
        assert_eq!(out, b"HTTP/1.1 100 Continue\r\n\r\n");
        assert!(matches!(r, Err(ReadError::Io(_))));
        let (r, out) = read(b"POST /mcp HTTP/1.1\r\nContent-Length: 1000\r\n\r\n", 100);
        assert!(matches!(r, Err(ReadError::TooLarge)));
        assert!(out.is_empty());
        let (r, _) = read(
            b"POST /mcp HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n80\r\n",
            100,
        );
        assert!(matches!(r, Err(ReadError::TooLarge)));
        let (r, _) = read(b"", 100);
        assert!(r.unwrap().is_none());
    }

    #[test]
    fn decodes_forms_and_multipart() {
        let form = parse_form("a=1&b=h%C3%A4j+du&c=%ZZ&d");
        assert_eq!(form_value(&form, "b"), Some("häj du"));
        assert_eq!(form_value(&form, "c"), Some("%ZZ"));
        assert_eq!(form_value(&form, "d"), Some(""));
        assert_eq!(encode("a b/å"), "a%20b%2F%C3%A5");

        let body = b"--XY\r\nContent-Disposition: form-data; name=\"fil\"; filename=\"kvitto.jpg\"\r\nContent-Type: image/jpeg\r\n\r\n\xff\xd8\xff\r\n--data\r\n--XY\r\nContent-Disposition: form-data; name=\"fil\"; filename=\"\"\r\n\r\n\r\n--XY--\r\n";
        let parts = parse_multipart("multipart/form-data; boundary=XY", body).unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].filename.as_deref(), Some("kvitto.jpg"));
        assert_eq!(parts[0].data, b"\xff\xd8\xff\r\n--data");
        assert_eq!(parts[1].filename, None);
        assert!(parts[1].data.is_empty());
        assert!(parse_multipart("text/plain", body).is_err());
        assert!(parse_multipart("multipart/form-data; boundary=XY", b"--XY\r\nnope").is_err());
    }
}
