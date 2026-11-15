//! Rutter och skydd för granskningsvyn.
//!
//! Skydden, i den ordning de prövas:
//! 1. Host-huvudet måste vara 127.0.0.1 eller localhost med rätt port (mot DNS-rebinding).
//! 2. Varje sida kräver sessionskakan, som sätts av engångslänken i terminalen.
//! 3. Varje POST kräver formulärets CSRF-token, och Origin måste vara sidans egen.
//! 4. Underlag visas inline bara som bild eller PDF; allt annat skickas som
//!    text att ladda ner, så att ett underlag aldrig körs som HTML i vyn.

use crate::backend::{ReviewBackend, ReviewError};
use crate::html;

/// En HTTP-förfrågan.
#[derive(Debug, Clone, Default)]
pub struct Request {
    pub method: String,
    /// Sökväg med eventuell query, till exempel `/login?token=…`.
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

    fn path(&self) -> &str {
        self.target
            .split_once('?')
            .map_or(self.target.as_str(), |(p, _)| p)
    }

    fn query(&self) -> &str {
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

const PAGE_CSP: &str =
    "default-src 'none'; img-src 'self'; frame-src 'self'; style-src 'unsafe-inline'; \
form-action 'self'; base-uri 'none'; frame-ancestors 'none'";

impl Response {
    fn html(status: u16, body: String) -> Self {
        Self {
            status,
            headers: vec![
                ("Content-Type".into(), "text/html; charset=utf-8".into()),
                ("Content-Security-Policy".into(), PAGE_CSP.into()),
                ("X-Frame-Options".into(), "DENY".into()),
                ("X-Content-Type-Options".into(), "nosniff".into()),
                // Inte no-referrer: då skickar webbläsare `Origin: null` på POST,
                // även inom samma sida, och Origin-kontrollen nekar varje formulär.
                ("Referrer-Policy".into(), "same-origin".into()),
                ("Cache-Control".into(), "no-store".into()),
            ],
            body: body.into_bytes(),
        }
    }

    fn redirect(location: &str, cookie: Option<String>) -> Self {
        let mut headers = vec![
            ("Location".into(), location.into()),
            ("Cache-Control".into(), "no-store".into()),
        ];
        if let Some(c) = cookie {
            headers.push(("Set-Cookie".into(), c));
        }
        Self {
            status: 303,
            headers,
            body: Vec::new(),
        }
    }

    fn message(status: u16, title: &str, text: &str) -> Self {
        Self::html(status, html::message_page(title, text))
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Avkodar `application/x-www-form-urlencoded`.
pub fn parse_form(input: &str) -> Vec<(String, String)> {
    fn decode(s: &str) -> String {
        let bytes = s.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'+' => out.push(b' '),
                b'%' if i + 2 < bytes.len() => {
                    match u8::from_str_radix(
                        std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("zz"),
                        16,
                    ) {
                        Ok(b) => {
                            out.push(b);
                            i += 2;
                        }
                        Err(_) => out.push(b'%'),
                    }
                }
                b => out.push(b),
            }
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }
    input
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => (decode(k), decode(v)),
            None => (decode(p), String::new()),
        })
        .collect()
}

fn form_value<'a>(form: &'a [(String, String)], key: &str) -> Option<&'a str> {
    form.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

/// Jämför två strängar utan att läcka var de skiljer sig.
fn same_secret(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

/// Underlag som får visas inline i vyn.
fn inline_media(media_type: &str) -> bool {
    matches!(
        media_type,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "application/pdf"
    )
}

/// Underlag som visas som text direkt på förslagets sida.
fn text_media(media_type: &str) -> bool {
    media_type.starts_with("text/")
        || media_type.ends_with("xml")
        || media_type.ends_with("json")
        || media_type == "message/rfc822"
}

const MAX_INLINE_TEXT: usize = 200 * 1024;

/// Granskningsvyns rutter.
pub struct Router<B: ReviewBackend> {
    backend: B,
    /// Token i inloggningslänken. Den fungerar en gång och är inte kakans värde,
    /// så den som ser länken efteråt (i historiken) kan inte logga in med den.
    login_token: Option<String>,
    session_token: String,
    csrf_token: String,
    allowed_hosts: Vec<String>,
}

impl<B: ReviewBackend> Router<B> {
    /// `port` är porten vyn lyssnar på; bara 127.0.0.1 och localhost på den porten godtas.
    /// De tre nycklarna ska vara olika och slumpmässiga.
    pub fn new(
        backend: B,
        login_token: String,
        session_token: String,
        csrf_token: String,
        port: u16,
    ) -> Self {
        Self {
            backend,
            login_token: Some(login_token),
            session_token,
            csrf_token,
            allowed_hosts: vec![format!("127.0.0.1:{port}"), format!("localhost:{port}")],
        }
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    fn has_session(&self, req: &Request) -> bool {
        req.header("cookie")
            .map(|c| {
                c.split(';')
                    .filter_map(|part| part.trim().split_once('='))
                    .any(|(k, v)| k == "bok_session" && same_secret(v, &self.session_token))
            })
            .unwrap_or(false)
    }

    fn post_allowed(&self, req: &Request, form: &[(String, String)]) -> bool {
        let csrf_ok = form_value(form, "csrf").is_some_and(|t| same_secret(t, &self.csrf_token));
        let origin_ok = match req.header("origin") {
            None => true,
            Some(o) => self
                .allowed_hosts
                .iter()
                .any(|h| o == format!("http://{h}")),
        };
        csrf_ok && origin_ok
    }

    pub fn handle(&mut self, req: &Request) -> Response {
        let host = req.header("host").unwrap_or("");
        if !self.allowed_hosts.iter().any(|h| h == host) {
            return Response::message(
                403,
                "Fel adress",
                "Granskningsvyn svarar bara på 127.0.0.1 och localhost.",
            );
        }

        if req.method == "GET" && req.path() == "/login" {
            let form = parse_form(req.query());
            let valid = match (form_value(&form, "token"), &self.login_token) {
                (Some(t), Some(expected)) => same_secret(t, expected),
                _ => false,
            };
            if valid {
                self.login_token = None;
                return Response::redirect(
                    "/",
                    Some(format!(
                        "bok_session={}; HttpOnly; SameSite=Strict; Path=/",
                        self.session_token
                    )),
                );
            }
            return Response::message(
                403,
                "Fel länk",
                "Länken stämmer inte eller är redan använd. Starta granskningsvyn igen för en ny länk.",
            );
        }

        if !self.has_session(req) {
            return Response::message(
                403,
                "Inte inloggad",
                "Öppna länken som visades i terminalen när granskningsvyn startade.",
            );
        }

        let segments: Vec<&str> = req.path().split('/').filter(|s| !s.is_empty()).collect();
        match (req.method.as_str(), segments.as_slice()) {
            ("GET", []) => match self.backend.open_proposals() {
                Ok(list) => Response::html(200, html::list_page(&list, None)),
                Err(e) => Response::message(500, "Fel", &e.to_string()),
            },
            ("GET", ["forslag", id]) => self.detail(id),
            ("GET", ["underlag", id]) => self.document(id),
            ("GET", ["bank"]) => match self.backend.bank_rows() {
                Ok(rows) => Response::html(200, html::bank_page(&rows, &self.csrf_token)),
                Err(e) => Response::message(500, "Fel", &e.to_string()),
            },
            ("GET", ["arenden"]) => match self.backend.cases() {
                Ok(cases) => Response::html(200, html::cases_page(&cases, &self.csrf_token)),
                Err(e) => Response::message(500, "Fel", &e.to_string()),
            },
            ("POST", ["bank", id, "stam-av"]) => {
                let form = parse_form(&String::from_utf8_lossy(&req.body));
                if !self.post_allowed(req, &form) {
                    return Response::message(
                        403,
                        "Nekat",
                        "Formuläret kom inte från granskningsvyn.",
                    );
                }
                let voucher = form_value(&form, "verifikation")
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let explanation = form_value(&form, "forklaring")
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                if voucher.is_none() == explanation.is_none() {
                    return Response::message(
                        400,
                        "Välj ett",
                        "Stäm av mot en verifikation eller skriv en förklaring.",
                    );
                }
                match self.backend.reconcile(id, voucher, explanation) {
                    Ok(()) => Response::redirect("/bank", None),
                    Err(e) => Response::message(error_status(&e), "Inte avstämd", &e.to_string()),
                }
            }
            ("POST", ["arenden", id, "svara"]) => {
                let form = parse_form(&String::from_utf8_lossy(&req.body));
                if !self.post_allowed(req, &form) {
                    return Response::message(
                        403,
                        "Nekat",
                        "Formuläret kom inte från granskningsvyn.",
                    );
                }
                let Some(answer) = form_value(&form, "svar")
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                else {
                    return Response::message(400, "Svar saknas", "Skriv ett svar.");
                };
                let mark = form_value(&form, "markera")
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                if mark.is_some_and(|m| m != "dubblett" && m != "ej_rakenskapsinformation") {
                    return Response::message(
                        400,
                        "Okänt val",
                        "Underlaget kan bara markeras som dubblett eller ej räkenskapsinformation.",
                    );
                }
                match self.backend.answer_case(id, answer, mark) {
                    Ok(()) => Response::redirect("/arenden", None),
                    Err(e) => Response::message(error_status(&e), "Inte besvarat", &e.to_string()),
                }
            }
            ("POST", ["forslag", id, action]) => {
                let form = parse_form(&String::from_utf8_lossy(&req.body));
                if !self.post_allowed(req, &form) {
                    return Response::message(
                        403,
                        "Nekat",
                        "Formuläret kom inte från granskningsvyn.",
                    );
                }
                match *action {
                    "godkann" => match self.backend.approve(id) {
                        Ok(number) => Response::message(
                            200,
                            "Bokförd",
                            &format!("Förslaget {id} är bokfört som verifikation {number}."),
                        ),
                        Err(e) => {
                            Response::message(error_status(&e), "Inte bokförd", &e.to_string())
                        }
                    },
                    "avvisa" => match form_value(&form, "orsak")
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        None => {
                            Response::message(400, "Orsak saknas", "Ange varför förslaget avvisas.")
                        }
                        Some(reason) => match self.backend.reject(id, reason) {
                            Ok(()) => Response::redirect("/", None),
                            Err(e) => {
                                Response::message(error_status(&e), "Inte avvisat", &e.to_string())
                            }
                        },
                    },
                    _ => Response::message(404, "Finns inte", "Sidan finns inte."),
                }
            }
            _ => Response::message(404, "Finns inte", "Sidan finns inte."),
        }
    }

    fn detail(&self, id: &str) -> Response {
        let proposal = match self.backend.proposal(id) {
            Ok(p) => p,
            Err(e) => return Response::message(error_status(&e), "Finns inte", &e.to_string()),
        };
        let mut texts = Vec::new();
        for d in &proposal.documents {
            if text_media(&d.media_type) {
                if let Ok((_, data)) = self.backend.document(&d.id) {
                    if data.len() <= MAX_INLINE_TEXT {
                        texts.push((d.id.clone(), String::from_utf8_lossy(&data).into_owned()));
                    }
                }
            }
        }
        Response::html(200, html::detail_page(&proposal, &self.csrf_token, &texts))
    }

    fn document(&self, id: &str) -> Response {
        match self.backend.document(id) {
            Ok((media_type, data)) => {
                let mut headers = vec![
                    ("X-Content-Type-Options".to_string(), "nosniff".to_string()),
                    ("X-Frame-Options".to_string(), "SAMEORIGIN".to_string()),
                    ("Cache-Control".to_string(), "no-store".to_string()),
                ];
                if inline_media(&media_type) {
                    headers.push(("Content-Type".into(), media_type));
                } else {
                    // Aldrig som HTML eller SVG i vyn: som text att ladda ner.
                    headers.push(("Content-Type".into(), "text/plain; charset=utf-8".into()));
                    headers.push(("Content-Disposition".into(), "attachment".into()));
                    headers.push((
                        "Content-Security-Policy".into(),
                        "default-src 'none'; sandbox".into(),
                    ));
                }
                Response {
                    status: 200,
                    headers,
                    body: data,
                }
            }
            Err(e) => Response::message(error_status(&e), "Finns inte", &e.to_string()),
        }
    }
}

fn error_status(e: &ReviewError) -> u16 {
    match e {
        ReviewError::NotFound(_) => 404,
        ReviewError::Refused(_) => 409,
        ReviewError::Internal(_) => 500,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::backend::MemoryReviewBackend;

    const PORT: u16 = 8484;

    fn router() -> Router<MemoryReviewBackend> {
        Router::new(
            MemoryReviewBackend::demo(),
            "l".repeat(64),
            "s".repeat(64),
            "c".repeat(64),
            PORT,
        )
    }

    fn get(path: &str, cookie: bool) -> Request {
        let mut headers = vec![("Host".to_string(), format!("127.0.0.1:{PORT}"))];
        if cookie {
            headers.push((
                "Cookie".into(),
                format!("annat=1; bok_session={}", "s".repeat(64)),
            ));
        }
        Request {
            method: "GET".into(),
            target: path.into(),
            headers,
            body: vec![],
        }
    }

    fn post(path: &str, body: &str, origin: Option<&str>) -> Request {
        let mut r = get(path, true);
        r.method = "POST".into();
        r.body = body.as_bytes().to_vec();
        if let Some(o) = origin {
            r.headers.push(("Origin".into(), o.into()));
        }
        r
    }

    fn text(r: &Response) -> String {
        String::from_utf8_lossy(&r.body).into_owned()
    }

    #[test]
    fn pages_let_the_browser_send_its_origin_on_forms() {
        // Med no-referrer skickar webbläsaren `Origin: null` och varje formulär nekas.
        let mut r = router();
        let page = r.handle(&get("/", true));
        assert_eq!(page.header("Referrer-Policy"), Some("same-origin"));
        let csrf = format!("csrf={}", "c".repeat(64));
        assert_eq!(
            r.handle(&post("/forslag/F1/godkann", &csrf, Some("null")))
                .status,
            403,
            "Origin null godtas inte"
        );
    }

    #[test]
    fn bank_and_case_pages_need_session_csrf_and_valid_input() {
        let mut r = router();
        assert_eq!(r.handle(&get("/bank", false)).status, 403);
        let bank = r.handle(&get("/bank", true));
        assert_eq!(bank.status, 200);
        assert!(text(&bank).contains("Alla bankrader är avstämda"));
        assert!(
            text(&bank).contains("href=\"/arenden\""),
            "navigeringen finns"
        );
        assert_eq!(r.handle(&get("/arenden", true)).status, 200);

        let csrf = format!("csrf={}", "c".repeat(64));
        assert_eq!(
            r.handle(&post("/bank/1/stam-av", "forklaring=x", None))
                .status,
            403
        );
        assert_eq!(r.handle(&post("/bank/1/stam-av", &csrf, None)).status, 400);
        assert_eq!(
            r.handle(&post(
                "/bank/1/stam-av",
                &format!("{csrf}&verifikation=3&forklaring=x"),
                None
            ))
            .status,
            400,
            "inte båda"
        );
        // Demolagret har ingen bankavstämning: nekat med 409, inte ett krasch.
        assert_eq!(
            r.handle(&post(
                "/bank/1/stam-av",
                &format!("{csrf}&forklaring=Utl%C3%A4gg"),
                None
            ))
            .status,
            409
        );
        assert_eq!(
            r.handle(&post("/arenden/1/svara", "svar=ja", None)).status,
            403
        );
        assert_eq!(r.handle(&post("/arenden/1/svara", &csrf, None)).status, 400);
        assert_eq!(
            r.handle(&post(
                "/arenden/1/svara",
                &format!("{csrf}&svar=ja&markera=spam"),
                None
            ))
            .status,
            400
        );
        assert_eq!(
            r.handle(&post(
                "/arenden/1/svara",
                &format!("{csrf}&svar=ja&markera=dubblett"),
                Some("http://evil.example")
            ))
            .status,
            403
        );
    }

    #[test]
    fn bank_and_case_pages_escape_everything() {
        use crate::backend::{BankRowView, CandidateView, CaseView};
        let rows = vec![BankRowView {
            id: "1\"><script>".into(),
            date: "2026-03-01".into(),
            amount_ore: -100,
            text: "<img src=x onerror=alert(1)>".into(),
            reference: Some("<b>".into()),
            candidates: vec![CandidateView {
                voucher_id: "7".into(),
                reference: "A7 (2026)".into(),
                date: "2026-03-01".into(),
                description: "<script>alert(2)</script>".into(),
            }],
            suggestions: vec!["<svg onload=alert(3)>".into()],
        }];
        let page = html::bank_page(&rows, "c");
        assert!(
            !page.contains("<script>") && !page.contains("<img") && !page.contains("<svg"),
            "{page}"
        );
        let cases = vec![CaseView {
            id: "2".into(),
            kind: "Flagga (dubblett)".into(),
            is_flag: true,
            document_id: Some("<x>".into()),
            text: "<script>alert(4)</script>".into(),
            actor: "<agent>".into(),
            created_at: "nu".into(),
        }];
        let page = html::cases_page(&cases, "c");
        assert!(
            !page.contains("<script>") && !page.contains("<agent>") && !page.contains("<x>"),
            "{page}"
        );
        assert!(
            page.contains("ej_rakenskapsinformation"),
            "flaggan kan markera underlaget"
        );
    }

    #[test]
    fn wrong_host_is_refused() {
        let mut r = router();
        let mut req = get("/", true);
        req.headers[0].1 = "evil.example:8484".into();
        assert_eq!(r.handle(&req).status, 403);
    }

    #[test]
    fn login_link_sets_the_session_cookie() {
        let mut r = router();
        assert_eq!(r.handle(&get("/", false)).status, 403);
        assert_eq!(r.handle(&get("/login?token=fel", false)).status, 403);
        // Kakans värde fungerar inte som länk, och länkens värde inte som kaka.
        assert_eq!(
            r.handle(&get(&format!("/login?token={}", "s".repeat(64)), false))
                .status,
            403
        );
        let resp = r.handle(&get(&format!("/login?token={}", "l".repeat(64)), false));
        assert_eq!(resp.status, 303);
        let cookie = resp.header("Set-Cookie").unwrap();
        assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
        assert!(cookie.contains(&"s".repeat(64)) && !cookie.contains(&"l".repeat(64)));
        assert_eq!(r.handle(&get("/", true)).status, 200);
        // Länken fungerar bara en gång.
        assert_eq!(
            r.handle(&get(&format!("/login?token={}", "l".repeat(64)), false))
                .status,
            403
        );
    }

    #[test]
    fn list_and_detail_escape_content() {
        let mut r = router();
        let list = r.handle(&get("/", true));
        let body = text(&list);
        assert!(body.contains("Molntjänst, månadsabonnemang mars"));
        assert!(body.contains("Lunch med kund &lt;okänd&gt;"));
        assert!(!body.contains("<okänd>"));
        assert!(list
            .header("Content-Security-Policy")
            .unwrap()
            .contains("frame-ancestors 'none'"));

        let detail = text(&r.handle(&get("/forslag/F1", true)));
        assert!(detail.contains("Exempel Hosting AB"));
        assert!(detail.contains("500,00"));
        assert!(detail.contains("Godkänn och signera med nyckeln"));
        assert!(
            detail.contains("Faktura 2026-311"),
            "textunderlag visas direkt på sidan"
        );
        let detail2 = text(&r.handle(&get("/forslag/F2", true)));
        assert!(detail2.contains("deltagare och syfte saknas"));
        assert_eq!(r.handle(&get("/forslag/F99", true)).status, 404);
    }

    #[test]
    fn documents_are_never_served_as_active_content() {
        let mut r = router();
        let svg = r.handle(&get("/underlag/INK-9", true));
        assert_eq!(svg.status, 200);
        assert_eq!(
            svg.header("Content-Type"),
            Some("text/plain; charset=utf-8")
        );
        assert_eq!(svg.header("Content-Disposition"), Some("attachment"));
        assert_eq!(svg.header("X-Content-Type-Options"), Some("nosniff"));
        assert_eq!(r.handle(&get("/underlag/INK-404", true)).status, 404);
    }

    #[test]
    fn approval_requires_csrf_and_own_origin() {
        let mut r = router();
        let csrf = format!("csrf={}", "c".repeat(64));
        assert_eq!(
            r.handle(&post("/forslag/F1/godkann", "csrf=fel", None))
                .status,
            403
        );
        assert_eq!(
            r.handle(&post(
                "/forslag/F1/godkann",
                &csrf,
                Some("http://evil.example")
            ))
            .status,
            403
        );
        assert!(
            r.backend().approved.is_empty(),
            "inget får bokföras utan giltigt formulär"
        );

        let ok = r.handle(&post(
            "/forslag/F1/godkann",
            &csrf,
            Some(&format!("http://127.0.0.1:{PORT}")),
        ));
        assert_eq!(ok.status, 200);
        assert!(text(&ok).contains("verifikation A1"));
        assert_eq!(
            r.backend().approved,
            vec![("F1".to_string(), "A1".to_string())]
        );

        let again = r.handle(&post("/forslag/F1/godkann", &csrf, None));
        assert_eq!(
            again.status, 409,
            "ett avgjort förslag kan inte bokföras igen"
        );
        let list = text(&r.handle(&get("/", true)));
        assert!(!list.contains("/forslag/F1\""));
    }

    #[test]
    fn rejection_requires_a_reason() {
        let mut r = router();
        let csrf = "c".repeat(64);
        assert_eq!(
            r.handle(&post(
                "/forslag/F2/avvisa",
                &format!("csrf={csrf}&orsak=+"),
                None
            ))
            .status,
            400
        );
        let resp = r.handle(&post(
            "/forslag/F2/avvisa",
            &format!("csrf={csrf}&orsak=Privat+k%C3%B6p"),
            None,
        ));
        assert_eq!(resp.status, 303);
        assert_eq!(
            r.backend().rejected,
            vec![("F2".to_string(), "Privat köp".to_string())]
        );
    }

    #[test]
    fn form_decoding() {
        let f = parse_form("a=1&b=h%C3%A4j+du&c&d=%zz&e=100%25");
        assert_eq!(form_value(&f, "a"), Some("1"));
        assert_eq!(form_value(&f, "b"), Some("häj du"));
        assert_eq!(form_value(&f, "c"), Some(""));
        assert_eq!(form_value(&f, "d"), Some("%zz"));
        assert_eq!(form_value(&f, "e"), Some("100%"));
    }
}
