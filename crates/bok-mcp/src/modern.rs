//! MCP 2026-07-28: `server/discover` och anrop utan handskakning.
//!
//! Den versionen tog bort `initialize`, `notifications/initialized` och
//! sessionerna. I stället bär varje anrop sin protokollversion och klientens
//! kapabiliteter i `params._meta` (`io.modelcontextprotocol/protocolVersion`
//! och `io.modelcontextprotocol/clientCapabilities`, och gärna
//! `io.modelcontextprotocol/clientInfo`), och över HTTP ska huvudet
//! `MCP-Protocol-Version` vara samma version. En klient frågar först med
//! `server/discover`; svarar servern med ett fel som klienten inte känner
//! igen faller den tillbaka till `initialize`. Varje svar har `resultType`,
//! och svar som får sparas (`server/discover`, `tools/list`) har `ttlMs` och
//! `cacheScope`.
//!
//! Servern talar båda: ett anrop med versionen i `_meta` besvaras enligt
//! 2026-07-28, och `initialize` ger det äldre beteendet som förut, på samma
//! ändpunkt. Källa: modelcontextprotocol.io/specification/2026-07-28
//! (Versioning and Compatibility, Streamable HTTP, server/discover och
//! tools), läst 2026-11-22.

use serde_json::{json, Value};

use crate::server::PROTOCOL_VERSIONS;

/// Protokollversionen utan handskakning.
pub const MODERN_VERSION: &str = "2026-07-28";

/// Nyckeln i `_meta` för protokollversionen.
pub const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
/// Nyckeln i `_meta` för klientens namn och version.
pub const META_CLIENT_INFO: &str = "io.modelcontextprotocol/clientInfo";
/// Nyckeln i `_meta` för klientens kapabiliteter.
pub const META_CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
/// Nyckeln i `_meta` för serverns namn och version i `server/discover`.
pub const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";

/// Huvud och kropp stämmer inte överens, eller ett huvud saknas.
pub const HEADER_MISMATCH: i64 = -32020;
/// Klienten saknar en kapabilitet som anropet kräver.
pub const MISSING_CLIENT_CAPABILITY: i64 = -32021;
/// Protokollversionen stöds inte.
pub const UNSUPPORTED_VERSION: i64 = -32022;

/// Hur länge klienten får spara `server/discover` och `tools/list`, i
/// millisekunder.
pub const TTL_MS: u64 = 60 * 60 * 1000;

/// Ett fel att svara med: JSON-RPC-kod, text och eventuella data.
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    /// JSON-RPC-felkoden.
    pub code: i64,
    /// Texten till klienten.
    pub message: String,
    /// Till exempel versionerna servern stöder.
    pub data: Option<Value>,
}

impl Failure {
    fn new(code: i64, message: impl Into<String>) -> Failure {
        Failure {
            code,
            message: message.into(),
            data: None,
        }
    }

    /// Felet som JSON-RPC-svar på anropet `id`.
    pub fn response(&self, id: Value) -> String {
        let mut error = json!({"code": self.code, "message": self.message});
        if let Some(d) = &self.data {
            error["data"] = d.clone();
        }
        json!({"jsonrpc": "2.0", "id": id, "error": error}).to_string()
    }
}

/// Alla versioner servern talar, nyast först.
pub fn supported_versions() -> Vec<&'static str> {
    let mut v = vec![MODERN_VERSION];
    v.extend_from_slice(PROTOCOL_VERSIONS);
    v
}

fn unsupported(requested: &str) -> Failure {
    Failure {
        code: UNSUPPORTED_VERSION,
        message: format!(
            "Protokollversionen {requested} stöds inte. Servern talar {}.",
            supported_versions().join(", ")
        ),
        data: Some(json!({"supported": supported_versions(), "requested": requested})),
    }
}

/// Vilken generation ett anrop hör till.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Era {
    /// Utan version i `_meta`: efter `initialize`, som förut.
    Legacy,
    /// 2026-07-28, med klientens namn om det angavs.
    Modern { client: Option<String> },
}

/// Läser anropets `_meta`. Ett anrop utan version där är av den äldre
/// sorten; ett med fel version eller utan kapabiliteter avvisas.
pub fn era(params: &Value) -> Result<Era, Failure> {
    let meta = &params["_meta"];
    let Some(version) = meta.get(META_VERSION) else {
        return Ok(Era::Legacy);
    };
    let Some(version) = version.as_str() else {
        return Err(Failure::new(
            -32602,
            format!("_meta.{META_VERSION} ska vara en text"),
        ));
    };
    if version != MODERN_VERSION {
        return Err(unsupported(version));
    }
    if !meta
        .get(META_CLIENT_CAPABILITIES)
        .is_some_and(Value::is_object)
    {
        return Err(Failure::new(
            -32602,
            format!("_meta.{META_CLIENT_CAPABILITIES} saknas; ange klientens kapabiliteter i varje anrop"),
        ));
    }
    let client = meta.get(META_CLIENT_INFO).map(|info| {
        let name = info.get("name").and_then(Value::as_str).unwrap_or("okänd");
        let ver = info.get("version").and_then(Value::as_str).unwrap_or("");
        format!("{name} {ver}").trim().to_string()
    });
    Ok(Era::Modern { client })
}

/// Svaret på `server/discover`.
pub fn discover_result(instructions: &str) -> Value {
    json!({
        "resultType": "complete",
        "supportedVersions": [MODERN_VERSION],
        "capabilities": {"tools": {}},
        "instructions": instructions,
        "ttlMs": TTL_MS,
        "cacheScope": "private",
        "_meta": {META_SERVER_INFO: {"name": "bok-mcp", "version": env!("CARGO_PKG_VERSION")}}
    })
}

/// Lägger till `resultType: "complete"` om svaret inte redan har en.
pub fn complete(mut result: Value) -> Value {
    if let Some(obj) = result.as_object_mut() {
        obj.entry("resultType").or_insert_with(|| json!("complete"));
    }
    result
}

/// Lägger till hur länge svaret får sparas. Verktygen beror på vem som är
/// inloggad, så de sparas bara privat.
pub fn cacheable(mut result: Value) -> Value {
    if let Some(obj) = result.as_object_mut() {
        obj.insert("ttlMs".into(), json!(TTL_MS));
        obj.insert("cacheScope".into(), json!("private"));
    }
    complete(result)
}

/// Huvudena i en HTTP-förfrågan som rör protokollet.
#[derive(Debug, Clone, Copy, Default)]
pub struct HttpHeaders<'a> {
    /// `MCP-Protocol-Version`.
    pub protocol_version: Option<&'a str>,
    /// `Mcp-Method`.
    pub method: Option<&'a str>,
    /// `Mcp-Name`, för `tools/call`.
    pub name: Option<&'a str>,
}

/// Prövar huvudena mot kroppen. Versionen i huvudet måste vara en som
/// servern talar; ett anrop enligt 2026-07-28 måste ha samma version i
/// huvudet som i `_meta`; och `Mcp-Method` och `Mcp-Name` måste stämma när
/// de finns.
pub fn check_headers(headers: &HttpHeaders<'_>, message: &Value) -> Result<(), Failure> {
    let header = headers.protocol_version.map(str::trim);
    if let Some(h) = header {
        if !supported_versions().contains(&h) {
            return Err(unsupported(h));
        }
    }
    let body = message["params"]["_meta"][META_VERSION].as_str();
    let mismatch = |what: String| Err(Failure::new(HEADER_MISMATCH, what));
    match (header, body) {
        (Some(h), Some(b)) if h != b => {
            return mismatch(format!("MCP-Protocol-Version {h} men _meta anger {b}"));
        }
        (None, Some(b)) => {
            return mismatch(format!(
                "huvudet MCP-Protocol-Version saknas; det ska vara {b}"
            ));
        }
        (Some(MODERN_VERSION), None) => {
            return mismatch(format!(
                "MCP-Protocol-Version {MODERN_VERSION} men _meta.{META_VERSION} saknas"
            ));
        }
        _ => {}
    }
    let method = message["method"].as_str().unwrap_or("");
    if let Some(m) = headers.method.map(str::trim) {
        if m != method {
            return mismatch(format!("Mcp-Method {m} men anropet är {method}"));
        }
    }
    if let (Some(n), "tools/call") = (headers.name.map(str::trim), method) {
        let name = message["params"]["name"].as_str().unwrap_or("");
        if n != name {
            return mismatch(format!("Mcp-Name {n} men verktyget är {name}"));
        }
    }
    Ok(())
}

/// HTTP-status för ett fel. Enligt 2026-07-28 ger protokollfelen 400 och en
/// okänd metod 404; i den äldre versionen besvaras fel med 200.
pub fn http_status(modern: bool, code: i64) -> u16 {
    match code {
        -32700 | HEADER_MISMATCH | MISSING_CLIENT_CAPABILITY | UNSUPPORTED_VERSION => 400,
        -32600 | -32602 if modern => 400,
        -32601 if modern => 404,
        _ => 200,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn meta(version: &str) -> Value {
        json!({"_meta": {
            META_VERSION: version,
            META_CLIENT_INFO: {"name": "claude-ai", "version": "2"},
            META_CLIENT_CAPABILITIES: {}
        }})
    }

    #[test]
    fn the_era_follows_the_meta() {
        assert_eq!(era(&json!({})).unwrap(), Era::Legacy);
        assert_eq!(
            era(&meta(MODERN_VERSION)).unwrap(),
            Era::Modern {
                client: Some("claude-ai 2".into())
            }
        );
        let old = era(&meta("1900-01-01")).unwrap_err();
        assert_eq!(old.code, UNSUPPORTED_VERSION);
        assert_eq!(old.data.unwrap()["requested"], "1900-01-01");
        let mut no_caps = meta(MODERN_VERSION);
        no_caps["_meta"]
            .as_object_mut()
            .unwrap()
            .remove(META_CLIENT_CAPABILITIES);
        assert_eq!(era(&no_caps).unwrap_err().code, -32602);
    }

    #[test]
    fn headers_must_match_the_body() {
        let call = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "lista_underlag", "_meta": meta(MODERN_VERSION)["_meta"]}});
        let ok = HttpHeaders {
            protocol_version: Some(MODERN_VERSION),
            method: Some("tools/call"),
            name: Some("lista_underlag"),
        };
        assert!(check_headers(&ok, &call).is_ok());
        let code = |h: HttpHeaders<'_>, m: &Value| check_headers(&h, m).unwrap_err().code;
        assert_eq!(
            code(
                HttpHeaders {
                    protocol_version: None,
                    ..ok
                },
                &call
            ),
            HEADER_MISMATCH
        );
        assert_eq!(
            code(
                HttpHeaders {
                    method: Some("tools/list"),
                    ..ok
                },
                &call
            ),
            HEADER_MISMATCH
        );
        assert_eq!(
            code(
                HttpHeaders {
                    name: Some("annat"),
                    ..ok
                },
                &call
            ),
            HEADER_MISMATCH
        );
        assert_eq!(
            code(
                HttpHeaders {
                    protocol_version: Some("2099-01-01"),
                    ..ok
                },
                &call
            ),
            UNSUPPORTED_VERSION
        );
        let legacy = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"});
        assert!(check_headers(&HttpHeaders::default(), &legacy).is_ok());
        let after_init = HttpHeaders {
            protocol_version: Some("2025-06-18"),
            ..HttpHeaders::default()
        };
        assert!(check_headers(&after_init, &legacy).is_ok());
        let modern_header = HttpHeaders {
            protocol_version: Some(MODERN_VERSION),
            ..HttpHeaders::default()
        };
        assert_eq!(code(modern_header, &legacy), HEADER_MISMATCH);
    }

    #[test]
    fn statuses_per_era() {
        assert_eq!(http_status(true, -32601), 404);
        assert_eq!(http_status(false, -32601), 200);
        assert_eq!(http_status(true, -32602), 400);
        assert_eq!(http_status(false, UNSUPPORTED_VERSION), 400);
        assert_eq!(http_status(false, -32700), 400);
    }

    #[test]
    fn results_get_their_type_and_cache_time() {
        assert_eq!(complete(json!({"a": 1}))["resultType"], "complete");
        let tools = cacheable(json!({"tools": []}));
        assert_eq!(tools["ttlMs"], TTL_MS);
        assert_eq!(tools["cacheScope"], "private");
        let d = discover_result("x");
        assert_eq!(d["supportedVersions"], json!([MODERN_VERSION]));
        assert_eq!(d["_meta"][META_SERVER_INFO]["name"], "bok-mcp");
    }
}
