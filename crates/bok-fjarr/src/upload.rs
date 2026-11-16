//! Uppladdning från telefonen: en personlig länk `/in/<hemlighet>` med ett
//! formulär för foton och PDF. Varje fil sparas oförändrad i arkivet och
//! registreras som underlag med länkens namn som avsändare. Sidan visar
//! också vad som har hänt med de senast skickade: väntar på agenten,
//! förslag väntar på granskning, bokfört.
//!
//! Telefonen får sin länk på startsidan: den skriver koden från
//! `bok-fjarr uppladdning`, och servern skapar länken och skickar telefonen
//! dit. Ingen behöver föra över en lång adress för hand.

use std::collections::HashSet;

use serde_json::json;

use bok_store::bok_core::period::ActorKind;
use bok_store::{Actor, AttachmentRow, Channel, NewAttachment, StoreError};

use crate::http::{self, form_value, parse_form, Request, Response};
use crate::pages::{self, Outcome, Recent};
use crate::state::{self, Link, LINK_CODE_ATTEMPTS};
use crate::{App, MAX_FILE};

/// Vad som hände när telefonen skrev in en kod på startsidan.
enum Redeemed {
    /// Länken är skapad. Hemligheten finns bara i omdirigeringen.
    Link,
    /// Fel kod, ingen kod eller en kod som har gått ut.
    Wrong,
    /// Namnet har hunnit få en annan länk.
    Taken(String),
    /// Länken kunde inte loggas i behandlingshistoriken och skapades därför inte.
    NotLogged(String),
}

/// Hur många av de senast skickade som visas.
const RECENT: usize = 6;

/// Avsändaren som underlagen från en länk får.
pub(crate) fn sender_label(link_name: &str) -> String {
    format!("uppladdningslänken {link_name}")
}

/// Filnamnet utan sökväg och styrtecken, högst 120 tecken.
fn clean_filename(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String = base.chars().filter(|c| !c.is_control()).take(120).collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        "underlag".into()
    } else {
        cleaned.to_string()
    }
}

fn ids(list: Option<&str>) -> Vec<i64> {
    list.unwrap_or("")
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .take(50)
        .collect()
}

fn recent_status(a: &AttachmentRow, pending: &HashSet<i64>) -> (String, bool) {
    match a.status.as_str() {
        "bokford" => (
            a.status_text
                .clone()
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| "bokfört".into()),
            true,
        ),
        "dubblett" => ("dubblett".into(), false),
        "ej_rakenskapsinformation" => ("inte räkenskapsinformation".into(), false),
        "komplettering" => ("behöver komplettering".into(), false),
        _ if pending.contains(&a.id) => ("förslag väntar på dig".into(), false),
        _ => ("väntar på agenten".into(), false),
    }
}

/// Kakan med länken som webbläsaren nyss löste in, och hur länge den gäller.
const REDEEMED_COOKIE: &str = "bok_lank";
const REDEEMED_COOKIE_TTL: i64 = 900;

impl App {
    /// Startsidan: formuläret för koden, och inlösen av den.
    pub(crate) fn start(&mut self, req: &Request) -> Response {
        match req.method.as_str() {
            "GET" => pages::start(&self.config.origin, None),
            "POST" => self.redeem(req),
            _ => Response::empty(405).with("Allow", "GET, POST"),
        }
    }

    /// Byter koden mot en ny uppladdningslänk. Länkens hemlighet tas fram här
    /// och finns sedan bara i telefonens adressfält; tillståndet sparar bara
    /// dess SHA-256.
    fn redeem(&mut self, req: &Request) -> Response {
        let form = parse_form(&String::from_utf8_lossy(&req.body));
        let entered = state::normalize_code(form_value(&form, "kod").unwrap_or(""));
        let now = self.now();
        let Ok(secret) = state::random_secret() else {
            return pages::message(500, "Fel", "slumpgeneratorn svarar inte");
        };
        let actor = Actor::human(self.config.user.clone());
        let store = &mut self.store;
        let outcome = self.state.update(now, |s| {
            let Some(code) = s.link_code.as_mut() else {
                return Redeemed::Wrong;
            };
            if entered.is_empty() || state::hash(&entered) != code.hash {
                code.attempts += 1;
                if code.attempts >= LINK_CODE_ATTEMPTS {
                    s.link_code = None;
                }
                return Redeemed::Wrong;
            }
            let name = code.name.clone();
            s.link_code = None;
            if s.link_named(&name) {
                return Redeemed::Taken(name);
            }
            // Länken loggas i behandlingshistoriken innan den finns, under
            // samma lås, så att det aldrig finns en länk utan händelse.
            if let Err(e) = store.record_remote_access(
                &actor,
                "fjarr.lank.skapad",
                &name,
                json!({"namn": name, "via": "kod på startsidan"}),
            ) {
                return Redeemed::NotLogged(e.to_string());
            }
            s.links.push(Link {
                hash: state::hash(&secret),
                name,
                created: now,
                revoked: None,
                last_used: None,
            });
            Redeemed::Link
        });
        match outcome {
            // Kakan låter samma webbläsare komma tillbaka till sin nya länk om
            // formuläret skickas en gång till, till exempel efter ett dubbelklick
            // eller med bakåtknappen, i stället för att få ett fel. Den bär samma
            // hemlighet som adressen och gäller bara en kort stund.
            Ok(Redeemed::Link) => Response::empty(303)
                .with("Location", format!("/in/{secret}"))
                .with(
                    "Set-Cookie",
                    format!(
                        "{REDEEMED_COOKIE}={secret}; Path=/; Max-Age={REDEEMED_COOKIE_TTL}; \
                         HttpOnly; Secure; SameSite=Strict"
                    ),
                ),
            Ok(Redeemed::Wrong) => match self.redeemed_here(req) {
                Some(secret) => Response::empty(303).with("Location", format!("/in/{secret}")),
                None => pages::start(&self.config.origin, Some(pages::WRONG_LINK_CODE)),
            },
            Ok(Redeemed::Taken(name)) => pages::start(
                &self.config.origin,
                Some(&format!(
                    "Det finns redan en uppladdningslänk som heter {name}. Återkalla den med \
                     bok-fjarr aterkalla, eller ta fram en kod med ett annat namn."
                )),
            ),
            Ok(Redeemed::NotLogged(e)) => pages::message(
                500,
                "Fel",
                &format!("länken kunde inte loggas i behandlingshistoriken och skapades inte: {e}"),
            ),
            Err(e) => pages::message(500, "Fel", &e.to_string()),
        }
    }

    /// Länken som den här webbläsaren nyss löste in, om den fortfarande gäller.
    fn redeemed_here(&self, req: &Request) -> Option<String> {
        let secret = req
            .header("Cookie")?
            .split(';')
            .filter_map(|c| c.trim().split_once('='))
            .find(|(k, _)| *k == REDEEMED_COOKIE)
            .map(|(_, v)| v.to_string())?;
        let s = self.state.read().ok()?;
        s.link(&secret).is_some().then_some(secret)
    }

    pub(crate) fn upload(&mut self, req: &Request) -> Response {
        let secret = req.path().trim_start_matches("/in/");
        if secret.is_empty() || secret.contains('/') {
            return pages::message(404, "Finns inte", "Sidan finns inte.");
        }
        let link = match self.state.read() {
            Ok(s) => s.link(secret).cloned(),
            Err(e) => return pages::message(500, "Fel", &e.to_string()),
        };
        let Some(link) = link else {
            return pages::link_gone();
        };
        match req.method.as_str() {
            "GET" => match self.upload_page(&link.name, link.last_used.is_none(), req.query()) {
                Ok(r) => r,
                Err(e) => pages::message(500, "Fel", &e),
            },
            "POST" => self.receive(req, secret, &link.name),
            _ => Response::empty(405).with("Allow", "GET, POST"),
        }
    }

    fn upload_page(&self, link_name: &str, fresh: bool, query: &str) -> Result<Response, String> {
        let sender = sender_label(link_name);
        let attachments = self.store.attachments().map_err(|e| e.to_string())?;
        // Bara underlag från den här länken, även om någon skriver andra nummer i adressen.
        let own = |id: i64| {
            attachments
                .iter()
                .find(|a| a.id == id && a.sender.as_deref() == Some(sender.as_str()))
        };
        let form = parse_form(query);
        let outcome = Outcome {
            received: ids(form_value(&form, "mottaget"))
                .into_iter()
                .filter_map(own)
                .map(|a| {
                    (
                        a.id,
                        a.original_name.clone().unwrap_or_default(),
                        usize::try_from(a.size).unwrap_or(0),
                    )
                })
                .collect(),
            existing: ids(form_value(&form, "fanns"))
                .into_iter()
                .filter_map(|id| attachments.iter().find(|a| a.id == id))
                .map(|a| (a.id, a.original_name.clone().unwrap_or_default()))
                .collect(),
            refused: form_value(&form, "nekade")
                .and_then(|n| n.parse().ok())
                .unwrap_or(0usize)
                .min(50),
        };
        let pending: HashSet<i64> = self
            .store
            .proposals(true)
            .map_err(|e| e.to_string())?
            .iter()
            .flat_map(|p| p.attachments.iter().copied())
            .collect();
        let recent: Vec<Recent> = attachments
            .iter()
            .rev()
            .filter(|a| a.sender.as_deref() == Some(sender.as_str()))
            .take(RECENT)
            .map(|a| {
                let (status, booked) = recent_status(a, &pending);
                Recent {
                    id: a.id,
                    name: a.original_name.clone().unwrap_or_else(|| "underlag".into()),
                    status,
                    booked,
                }
            })
            .collect();
        Ok(pages::upload(
            &self.company,
            link_name,
            fresh,
            &outcome,
            &recent,
        ))
    }

    fn receive(&mut self, req: &Request, secret: &str, link_name: &str) -> Response {
        let now = self.now();
        let content_type = req.header("content-type").unwrap_or("");
        let parts = match http::parse_multipart(content_type, &req.body) {
            Ok(p) => p,
            Err(e) => return pages::message(400, "Uppladdningen gick inte att läsa", e),
        };
        let sender = sender_label(link_name);
        let actor = Actor {
            kind: ActorKind::Human,
            name: self.config.user.clone(),
            role: "mobil".into(),
        };
        let (mut received, mut existing, mut refused) = (Vec::new(), Vec::new(), 0usize);
        for part in parts
            .into_iter()
            .filter(|p| p.name == "fil" && !p.data.is_empty())
        {
            let filename = clean_filename(part.filename.as_deref().unwrap_or(""));
            if part.data.len() > MAX_FILE {
                refused += 1;
                continue;
            }
            // Filtypen avgörs av innehållet, inte av vad webbläsaren säger.
            let media = bok_archive::media::detect(&part.data, &filename);
            if !(media.starts_with("image/") || media == "application/pdf") {
                refused += 1;
                continue;
            }
            let stored = match self.archive.put_bytes(&part.data) {
                Ok(s) => s,
                Err(e) => return pages::message(500, "Fel", &e.to_string()),
            };
            let size = i64::try_from(part.data.len()).unwrap_or(i64::MAX);
            match self.store.register_attachment(
                &actor,
                &NewAttachment {
                    sha256: stored.sha256,
                    size,
                    channel: Channel::Manual,
                    sender: Some(sender.clone()),
                    original_name: Some(filename),
                    file_type: Some(media),
                },
            ) {
                Ok(id) => received.push(id),
                Err(StoreError::DuplicateAttachment(id)) => existing.push(id),
                Err(e) => return pages::message(500, "Fel", &e.to_string()),
            }
        }
        // Efter hemligheten, inte namnet: en återkallad länk kan ha samma namn.
        let hash = state::hash(secret);
        let _ = self.state.update(now, |s| {
            if let Some(l) = s.links.iter_mut().find(|l| l.hash == hash) {
                l.last_used = Some(now);
            }
        });
        // Tillbaka till sidan, så att en omladdning inte skickar filerna igen.
        let join = |v: &[i64]| v.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
        let mut query = Vec::new();
        if !received.is_empty() {
            query.push(format!("mottaget={}", join(&received)));
        }
        if !existing.is_empty() {
            query.push(format!("fanns={}", join(&existing)));
        }
        if refused > 0 {
            query.push(format!("nekade={refused}"));
        }
        let location = if query.is_empty() {
            format!("/in/{secret}")
        } else {
            format!("/in/{secret}?{}", query.join("&"))
        };
        Response::empty(303).with("Location", location)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filenames_lose_paths_and_control_characters() {
        assert_eq!(clean_filename("C:\\Users\\x\\IMG_1.jpg"), "IMG_1.jpg");
        assert_eq!(clean_filename("../../etc/passwd"), "passwd");
        assert_eq!(clean_filename("a\u{0}b.jpg"), "ab.jpg");
        assert_eq!(clean_filename(""), "underlag");
        assert_eq!(ids(Some("1,2,x,3")), vec![1, 2, 3]);
    }
}
