//! OAuth 2.1 för agenterna, så som Claude, ChatGPT och Claude Code
//! ansluter till en fjärrserver:
//!
//! 1. Klienten hittar inloggningen genom ett 401-svar från `/mcp` med
//!    `resource_metadata` (RFC 9728) och serverns metadata (RFC 8414).
//! 2. Klienten registrerar sig själv (RFC 7591) och får ett klient-id.
//! 3. Användaren godkänner anslutningen i webbläsaren med en kopplingskod
//!    från `bok-fjarr kod`. Utan koden blir det inget godkännande.
//! 4. Klienten byter auktoriseringskoden mot nycklar med PKCE (S256), och
//!    förnyar dem med en förnyelsenyckel som byts ut vid varje förnyelse.

use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use bok_store::Actor;

use crate::http::{encode, form_value, parse_form, Request, Response};
use crate::state::{
    self, Client, Token, TokenKind, ACCESS_TTL, MAX_UNAPPROVED, PAIRING_ATTEMPTS, REFRESH_TTL,
};
use crate::{pages, App, AuthCode, Config};

/// Den enda behörigheten: agentens verktyg.
pub(crate) const SCOPE: &str = "bokis";
/// Hur länge en auktoriseringskod gäller, i sekunder.
const CODE_TTL: i64 = 5 * 60;

pub(crate) fn resource_metadata(c: &Config) -> Value {
    json!({
        "resource": c.mcp_url(),
        "authorization_servers": [c.origin],
        "scopes_supported": [SCOPE],
        "bearer_methods_supported": ["header"],
        "resource_name": "Bokis",
    })
}

pub(crate) fn server_metadata(c: &Config) -> Value {
    let o = &c.origin;
    json!({
        "issuer": o,
        "authorization_endpoint": format!("{o}/oauth/authorize"),
        "token_endpoint": format!("{o}/oauth/token"),
        "registration_endpoint": format!("{o}/oauth/register"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none", "client_secret_post", "client_secret_basic"],
        "scopes_supported": [SCOPE],
        "authorization_response_iss_parameter_supported": true,
    })
}

fn oauth_error(status: u16, error: &str, description: &str) -> Response {
    Response::json(
        status,
        &json!({"error": error, "error_description": description}),
    )
    .with("Pragma", "no-cache")
}

fn invalid_grant(description: &str) -> Response {
    oauth_error(400, "invalid_grant", description)
}

/// Värd och sökväg för en omdirigering till den egna datorn (RFC 8252).
fn loopback(uri: &str) -> Option<(&str, &str)> {
    let rest = uri.strip_prefix("http://")?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let host = if let Some(after) = authority.strip_prefix("[::1]") {
        if after.is_empty() || after.starts_with(':') {
            "[::1]"
        } else {
            return None;
        }
    } else {
        authority.split(':').next()?
    };
    matches!(host, "localhost" | "127.0.0.1" | "[::1]").then_some((host, path))
}

/// En omdirigering som godtas vid registrering: https, eller http till den
/// egna datorn för Claude Code och andra program som tar emot svaret lokalt.
pub(crate) fn valid_redirect(uri: &str) -> bool {
    if uri.len() > 500 || uri.contains('#') || uri.chars().any(|c| c.is_control() || c == ' ') {
        return false;
    }
    if let Some(rest) = uri.strip_prefix("https://") {
        return rest
            .split('/')
            .next()
            .is_some_and(|h| !h.is_empty() && !h.contains('@'));
    }
    loopback(uri).is_some()
}

/// Samma omdirigering: exakt, eller till den egna datorn med vilken port som helst.
pub(crate) fn redirect_matches(registered: &str, given: &str) -> bool {
    if registered == given {
        return true;
    }
    match (loopback(registered), loopback(given)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// Värden i en omdirigering, för att visa vart användaren skickas.
pub(crate) fn redirect_host(uri: &str) -> &str {
    let rest = uri.split_once("://").map_or(uri, |(_, r)| r);
    rest.split(['/', '?']).next().unwrap_or(rest)
}

fn pkce(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn valid_verifier(v: &str) -> bool {
    (43..=128).contains(&v.len())
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'))
}

/// Klientens namn som det visas: utan styrtecken, högst 60 tecken.
fn clean_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}')
        })
        .take(60)
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        "Okänd klient".into()
    } else {
        cleaned.to_string()
    }
}

/// En godkännandebegäran som har kontrollerats.
pub(crate) struct Authorize {
    pub client: Client,
    pub redirect_uri: String,
    pub challenge: String,
    pub state: Option<String>,
}

pub(crate) enum AuthorizeError {
    /// Klienten eller omdirigeringen stämmer inte: visa ett fel, skicka inte vidare.
    Page(String),
    /// Övriga fel skickas tillbaka till klienten.
    Redirect {
        redirect_uri: String,
        state: Option<String>,
        error: &'static str,
        description: String,
    },
}

/// Omdirigeringen tillbaka till klienten med parametrar.
pub(crate) fn redirect_to(uri: &str, params: &[(&str, &str)]) -> Response {
    let mut location = uri.to_string();
    for (k, v) in params {
        location.push(if location.contains('?') { '&' } else { '?' });
        location.push_str(k);
        location.push('=');
        location.push_str(&encode(v));
    }
    Response::empty(303).with("Location", location)
}

enum Pairing {
    Ok,
    Wrong(u32),
    Missing,
}

impl App {
    pub(crate) fn register(&mut self, req: &Request) -> Response {
        let now = self.now();
        let Ok(body) = serde_json::from_slice::<Value>(&req.body) else {
            return oauth_error(400, "invalid_client_metadata", "kroppen är inte JSON");
        };
        let uris: Vec<String> = body["redirect_uris"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        if uris.is_empty() || uris.len() > 10 {
            return oauth_error(
                400,
                "invalid_redirect_uri",
                "ange mellan 1 och 10 redirect_uris",
            );
        }
        if let Some(bad) = uris.iter().find(|u| !valid_redirect(u)) {
            return oauth_error(
                400,
                "invalid_redirect_uri",
                &format!(
                    "{bad} godtas inte: använd https, eller http till 127.0.0.1 eller localhost"
                ),
            );
        }
        if let Some(types) = body["grant_types"].as_array() {
            if !types.iter().any(|t| t == "authorization_code") {
                return oauth_error(
                    400,
                    "invalid_client_metadata",
                    "grant_types måste innehålla authorization_code",
                );
            }
        }
        if let Some(types) = body["response_types"].as_array() {
            if !types.iter().any(|t| t == "code") {
                return oauth_error(
                    400,
                    "invalid_client_metadata",
                    "response_types måste innehålla code",
                );
            }
        }
        let name = clean_name(body["client_name"].as_str().unwrap_or(""));
        // En klient som ber om en hemlighet får en; annars är den publik och
        // skyddas av PKCE, som Claude använder i båda fallen.
        let method = body["token_endpoint_auth_method"]
            .as_str()
            .filter(|m| matches!(*m, "client_secret_post" | "client_secret_basic"))
            .unwrap_or("none");
        let (Ok(random), Ok(new_secret)) = (state::random_secret(), state::random_secret()) else {
            return oauth_error(500, "server_error", "slumpgeneratorn svarar inte");
        };
        let id = format!("c-{}", &random[..24]);
        let client_secret = (method != "none").then_some(new_secret);
        let client = Client {
            id: id.clone(),
            name: name.clone(),
            redirect_uris: uris.clone(),
            registered: now,
            approved: None,
            revoked: None,
            last_used: None,
            secret_hash: client_secret.as_deref().map(state::hash),
        };
        let stored = self.state.update(now, |s| {
            if s.clients.iter().filter(|c| c.approved.is_none()).count() >= MAX_UNAPPROVED {
                return false;
            }
            s.clients.push(client);
            true
        });
        match stored {
            Ok(true) => {
                let mut reply = json!({
                    "client_id": id,
                    "client_id_issued_at": now,
                    "client_name": name,
                    "redirect_uris": uris,
                    "grant_types": ["authorization_code", "refresh_token"],
                    "response_types": ["code"],
                    "token_endpoint_auth_method": method,
                    "scope": SCOPE,
                });
                if let Some(secret) = client_secret {
                    reply["client_secret"] = json!(secret);
                    reply["client_secret_expires_at"] = json!(0);
                }
                Response::json(201, &reply)
            }
            Ok(false) => oauth_error(
                429,
                "invalid_client_metadata",
                "för många registreringar väntar på godkännande; försök igen om ett dygn",
            ),
            Err(e) => oauth_error(500, "server_error", &e.to_string()),
        }
    }

    pub(crate) fn check_authorize(
        &self,
        params: &[(String, String)],
    ) -> Result<Authorize, AuthorizeError> {
        let page = |t: &str| AuthorizeError::Page(t.to_string());
        let state_file = self.state.read().map_err(|e| page(&e.to_string()))?;
        let client_id = form_value(params, "client_id").ok_or_else(|| page("client_id saknas."))?;
        let client = state_file
            .client(client_id)
            .filter(|c| c.revoked.is_none())
            .cloned()
            .ok_or_else(|| {
                page("Anslutningen är okänd eller återkallad. Ta bort den i appen och lägg till den igen.")
            })?;
        let redirect_uri = match form_value(params, "redirect_uri") {
            Some(given) => {
                if !client
                    .redirect_uris
                    .iter()
                    .any(|r| redirect_matches(r, given))
                {
                    return Err(page("Omdirigeringen stämmer inte med den registrerade."));
                }
                given.to_string()
            }
            None if client.redirect_uris.len() == 1 => client.redirect_uris[0].clone(),
            None => return Err(page("redirect_uri saknas.")),
        };
        let state = form_value(params, "state").map(str::to_string);
        let fail = |error: &'static str, description: &str| AuthorizeError::Redirect {
            redirect_uri: redirect_uri.clone(),
            state: state.clone(),
            error,
            description: description.to_string(),
        };
        if form_value(params, "response_type") != Some("code") {
            return Err(fail("unsupported_response_type", "bara response_type=code"));
        }
        let challenge = form_value(params, "code_challenge")
            .filter(|c| (43..=128).contains(&c.len()))
            .ok_or_else(|| fail("invalid_request", "PKCE krävs: code_challenge saknas"))?
            .to_string();
        if form_value(params, "code_challenge_method") != Some("S256") {
            return Err(fail("invalid_request", "PKCE med S256 krävs"));
        }
        if let Some(resource) = form_value(params, "resource") {
            let r = resource.trim_end_matches('/');
            if r != self.config.mcp_url() && r != self.config.origin {
                return Err(fail("invalid_target", "resource är inte den här servern"));
            }
        }
        Ok(Authorize {
            client,
            redirect_uri,
            challenge,
            state,
        })
    }

    fn authorize_error(&self, e: AuthorizeError) -> Response {
        match e {
            AuthorizeError::Page(text) => {
                pages::message(400, "Anslutningen kan inte godkännas", &text)
            }
            AuthorizeError::Redirect {
                redirect_uri,
                state,
                error,
                description,
            } => {
                let mut params = vec![
                    ("error", error),
                    ("error_description", description.as_str()),
                ];
                if let Some(s) = &state {
                    params.push(("state", s));
                }
                params.push(("iss", &self.config.origin));
                redirect_to(&redirect_uri, &params)
            }
        }
    }

    pub(crate) fn authorize_get(&mut self, req: &Request) -> Response {
        let params = parse_form(req.query());
        match self.check_authorize(&params) {
            Ok(a) => pages::consent(&self.company, &a, &params, &self.config.origin, None),
            Err(e) => self.authorize_error(e),
        }
    }

    pub(crate) fn authorize_post(&mut self, req: &Request) -> Response {
        let params = parse_form(&String::from_utf8_lossy(&req.body));
        let a = match self.check_authorize(&params) {
            Ok(a) => a,
            Err(e) => return self.authorize_error(e),
        };
        let entered = state::normalize_code(form_value(&params, "kod").unwrap_or(""));
        let now = self.now();
        // Formuläret skickat en gång till efter ett godkännande, till exempel
        // med ett dubbelklick: säg att det redan är klart i stället för att
        // visa ett fel. Ingen ny kod lämnas ut.
        let pending = matches!(self.state.read(), Ok(s) if s.pairing.as_ref().is_some_and(|p| p.expires > now));
        if a.client.approved.is_some() && !pending {
            return pages::message(
                200,
                "Anslutningen är redan godkänd",
                "Du kan stänga den här fliken och gå tillbaka till appen.",
            );
        }
        let client_id = a.client.id.clone();
        let outcome = self.state.update(now, |s| {
            let Some(p) = s.pairing.as_mut() else {
                return Pairing::Missing;
            };
            if !entered.is_empty() && state::hash(&entered) == p.hash {
                s.pairing = None;
                if let Some(c) = s.client_mut(&client_id) {
                    c.approved.get_or_insert(now);
                }
                return Pairing::Ok;
            }
            p.attempts += 1;
            let left = PAIRING_ATTEMPTS.saturating_sub(p.attempts);
            if left == 0 {
                s.pairing = None;
            }
            Pairing::Wrong(left)
        });
        let problem = match outcome {
            Ok(Pairing::Ok) => None,
            Ok(Pairing::Wrong(0)) | Ok(Pairing::Missing) => Some(
                "Det finns ingen giltig kopplingskod. Kör bok-fjarr kod på datorn med bokföringen och skriv in den nya koden."
                    .to_string(),
            ),
            Ok(Pairing::Wrong(left)) => Some(format!(
                "Koden stämmer inte. {left} försök kvar innan koden slutar gälla."
            )),
            Err(e) => return pages::message(500, "Fel", &e.to_string()),
        };
        if let Some(text) = problem {
            return pages::consent(&self.company, &a, &params, &self.config.origin, Some(&text));
        }
        // Godkännandet loggas i behandlingshistoriken innan klienten får något.
        let actor = Actor::human(self.config.user.clone());
        if let Err(e) = self.store.record_remote_access(
            &actor,
            "fjarr.anslutning.godkand",
            &a.client.name,
            json!({
                "klient_id": a.client.id,
                "namn": a.client.name,
                "omdirigering": redirect_host(&a.redirect_uri),
            }),
        ) {
            return pages::message(500, "Fel", &format!("godkännandet kunde inte loggas: {e}"));
        }
        let Ok(code) = state::random_secret() else {
            return pages::message(500, "Fel", "slumpgeneratorn svarar inte");
        };
        self.codes.retain(|_, c| c.expires > now);
        self.codes.insert(
            state::hash(&code),
            AuthCode {
                client_id: a.client.id.clone(),
                redirect_uri: a.redirect_uri.clone(),
                challenge: a.challenge.clone(),
                expires: now + CODE_TTL,
            },
        );
        let mut params = vec![("code", code.as_str())];
        if let Some(s) = &a.state {
            params.push(("state", s));
        }
        params.push(("iss", &self.config.origin));
        redirect_to(&a.redirect_uri, &params)
    }

    fn issue(&self, client_id: &str, now: i64, rotate: Option<&str>) -> Response {
        let (Ok(access), Ok(refresh)) = (state::random_secret(), state::random_secret()) else {
            return oauth_error(500, "server_error", "slumpgeneratorn svarar inte");
        };
        let old = rotate.map(state::hash);
        let result = self.state.update(now, |s| {
            if let Some(old) = &old {
                let before = s.tokens.len();
                s.tokens
                    .retain(|t| !(t.kind == TokenKind::Refresh && &t.hash == old));
                if s.tokens.len() == before {
                    return false;
                }
            }
            s.tokens.push(Token {
                hash: state::hash(&access),
                kind: TokenKind::Access,
                client_id: client_id.to_string(),
                expires: now + ACCESS_TTL,
            });
            s.tokens.push(Token {
                hash: state::hash(&refresh),
                kind: TokenKind::Refresh,
                client_id: client_id.to_string(),
                expires: now + REFRESH_TTL,
            });
            true
        });
        match result {
            Ok(true) => Response::json(
                200,
                &json!({
                    "access_token": access,
                    "token_type": "Bearer",
                    "expires_in": ACCESS_TTL,
                    "refresh_token": refresh,
                    "scope": SCOPE,
                }),
            )
            .with("Pragma", "no-cache"),
            Ok(false) => invalid_grant("förnyelsenyckeln är redan använd"),
            Err(e) => oauth_error(500, "server_error", &e.to_string()),
        }
    }

    pub(crate) fn token(&mut self, req: &Request) -> Response {
        let now = self.now();
        let form = parse_form(&String::from_utf8_lossy(&req.body));
        // Publika klienter skickar klient-id i formuläret; klienter med
        // hemlighet skickar den i formuläret eller med Basic (RFC 6749 2.3.1).
        let basic = req
            .header("authorization")
            .and_then(|h| h.strip_prefix("Basic "))
            .and_then(|b| {
                base64::engine::general_purpose::STANDARD
                    .decode(b.trim())
                    .ok()
            })
            .and_then(|d| String::from_utf8(d).ok())
            .and_then(|d| {
                d.split_once(':')
                    .map(|(id, secret)| (crate::http::decode(id), crate::http::decode(secret)))
            });
        let client_id = form_value(&form, "client_id")
            .map(str::to_string)
            .or_else(|| basic.as_ref().map(|(id, _)| id.clone()));
        let client_secret = form_value(&form, "client_secret")
            .map(str::to_string)
            .or_else(|| basic.as_ref().map(|(_, s)| s.clone()));
        let authenticated = |client: &Client| match &client.secret_hash {
            None => true,
            Some(h) => client_secret
                .as_deref()
                .is_some_and(|s| &state::hash(s) == h),
        };
        let state_file = match self.state.read() {
            Ok(s) => s,
            Err(e) => return oauth_error(500, "server_error", &e.to_string()),
        };
        match form_value(&form, "grant_type") {
            Some("authorization_code") => {
                let Some(code) = form_value(&form, "code") else {
                    return oauth_error(400, "invalid_request", "code saknas");
                };
                // Koden tas bort vid första försöket, även om resten inte stämmer.
                let Some(entry) = self.codes.remove(&state::hash(code)) else {
                    return invalid_grant("koden är okänd, redan använd eller för gammal");
                };
                if entry.expires <= now {
                    return invalid_grant("koden är för gammal");
                }
                if client_id.as_deref() != Some(entry.client_id.as_str()) {
                    return invalid_grant("koden gäller en annan klient");
                }
                if form_value(&form, "redirect_uri").is_some_and(|r| r != entry.redirect_uri) {
                    return invalid_grant("redirect_uri stämmer inte");
                }
                let verifier = form_value(&form, "code_verifier").unwrap_or("");
                if !valid_verifier(verifier) || pkce(verifier) != entry.challenge {
                    return invalid_grant("code_verifier stämmer inte");
                }
                let Some(client) = state_file
                    .client(&entry.client_id)
                    .filter(|c| c.revoked.is_none() && c.approved.is_some())
                else {
                    return invalid_grant("anslutningen är återkallad");
                };
                if !authenticated(client) {
                    return oauth_error(401, "invalid_client", "klienthemligheten stämmer inte");
                }
                self.issue(&entry.client_id, now, None)
            }
            Some("refresh_token") => {
                let Some(refresh) = form_value(&form, "refresh_token") else {
                    return oauth_error(400, "invalid_request", "refresh_token saknas");
                };
                let h = state::hash(refresh);
                let Some(token) = state_file
                    .tokens
                    .iter()
                    .find(|t| t.kind == TokenKind::Refresh && t.hash == h && t.expires > now)
                else {
                    return invalid_grant("förnyelsenyckeln är okänd eller har gått ut");
                };
                if client_id.as_deref().is_some_and(|c| c != token.client_id) {
                    return invalid_grant("förnyelsenyckeln gäller en annan klient");
                }
                let Some(client) = state_file
                    .client(&token.client_id)
                    .filter(|c| c.revoked.is_none() && c.approved.is_some())
                else {
                    return invalid_grant("anslutningen är återkallad");
                };
                if !authenticated(client) {
                    return oauth_error(401, "invalid_client", "klienthemligheten stämmer inte");
                }
                let client = token.client_id.clone();
                self.issue(&client, now, Some(refresh))
            }
            Some(_) => oauth_error(
                400,
                "unsupported_grant_type",
                "bara authorization_code och refresh_token",
            ),
            None => oauth_error(400, "invalid_request", "grant_type saknas"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirects_are_https_or_loopback_and_match_without_port() {
        assert!(valid_redirect("https://claude.ai/api/mcp/auth_callback"));
        assert!(valid_redirect("http://localhost:3118/callback"));
        assert!(valid_redirect("http://127.0.0.1/callback"));
        assert!(valid_redirect("http://[::1]:9000/cb"));
        assert!(!valid_redirect("http://evil.example/cb"));
        assert!(!valid_redirect("http://localhost.evil.example/cb"));
        assert!(!valid_redirect("https://user@evil.example/cb"));
        assert!(!valid_redirect("https://claude.ai/cb#x"));
        assert!(!valid_redirect("javascript:alert(1)"));
        assert!(redirect_matches(
            "http://localhost/callback",
            "http://localhost:3118/callback"
        ));
        assert!(!redirect_matches(
            "http://localhost/callback",
            "http://127.0.0.1:3118/callback"
        ));
        assert!(!redirect_matches(
            "http://localhost/callback",
            "http://localhost:3118/other"
        ));
        assert!(!redirect_matches(
            "https://claude.ai/api/mcp/auth_callback",
            "https://claude.ai/api/mcp/auth_callback2"
        ));
        assert_eq!(
            redirect_host("https://claude.ai/api/mcp/auth_callback"),
            "claude.ai"
        );
    }

    #[test]
    fn pkce_matches_rfc_7636_example() {
        // RFC 7636, bilaga B.
        assert_eq!(
            pkce("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        assert!(!valid_verifier("kort"));
        assert_eq!(clean_name("Claude\u{202e}gnik"), "Claudegnik");
        assert_eq!(clean_name(" \u{7} "), "Okänd klient");
    }
}
