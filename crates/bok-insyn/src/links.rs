//! Inloggningslänkar och sessioner för en insyn som körs som tjänst.
//!
//! `bok insyn --adress <url>` startar insynen utan länk och skriver adressen
//! i `insyn.json` bredvid databasen. `bok insyn lank --namn "Revisor X"
//! --giltig 30d` skapar en namngiven länk i behandlingshistoriken och lägger
//! dess SHA-256 här under länkens id. En inloggning med länken ger en
//! session, vars SHA-256 också sparas här med länkens id och sessionens
//! sista tid. Själva hemligheterna sparas aldrig.
//!
//! Filen avgör inte ensam om något gäller: tjänsten frågar också lagret om
//! länken är återkallad eller utgången (se `bok_store::InsynLink`), så en
//! återkallelse gäller även om filen inte hunnit skrivas om.

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use bok_store::bok_core::canonical::sha256_hex;
use serde_json::{json, Value};

/// Hur länge en session gäller som längst, i sekunder. Den slutar också
/// gälla när länken går ut eller återkallas.
pub const SESSION_TTL: i64 = 12 * 60 * 60;

/// Filen bredvid databasen.
pub fn file_beside(db: &Path) -> PathBuf {
    db.with_file_name("insyn.json")
}

fn read(path: &Path) -> io::Result<Value> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| io::Error::other(format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(e),
    }
}

fn write(path: &Path, value: &Value) -> io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(value.to_string().as_bytes())?;
    f.sync_all()?;
    fs::rename(&tmp, path)
}

/// Poster i `key` som har ett id och inte har gått ut. Länkar från före
/// namngivningen saknar id och stryks.
fn live(value: &Value, key: &str, now: i64) -> Vec<Value> {
    value[key]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|l| l["id"].as_i64().is_some())
                .filter(|l| l["utgar"].as_i64().is_some_and(|t| t > now))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn store(
    path: &Path,
    value: &mut Value,
    links: Vec<Value>,
    sessions: Vec<Value>,
) -> io::Result<()> {
    value["lankar"] = Value::Array(links);
    value["sessioner"] = Value::Array(sessions);
    write(path, value)
}

/// Sparar adressen som tjänsten nås på, och stryker utgångna länkar och
/// sessioner.
pub fn set_origin(path: &Path, origin: &str, now: i64) -> io::Result<()> {
    let value = read(path)?;
    write(
        path,
        &json!({
            "adress": origin,
            "lankar": live(&value, "lankar", now),
            "sessioner": live(&value, "sessioner", now),
        }),
    )
}

/// Adressen som tjänsten nås på, om den körs med `--adress`.
pub fn origin(path: &Path) -> io::Result<Option<String>> {
    Ok(read(path)?["adress"].as_str().map(str::to_string))
}

/// Lägger länken `id` med `token` som hemlighet, giltig till `expires`.
pub fn issue(path: &Path, id: i64, token: &str, expires: i64, now: i64) -> io::Result<()> {
    let mut value = read(path)?;
    let mut links = live(&value, "lankar", now);
    links.push(json!({"id": id, "hash": sha256_hex(token.as_bytes()), "utgar": expires}));
    let sessions = live(&value, "sessioner", now);
    store(path, &mut value, links, sessions)
}

fn find(value: &Value, key: &str, secret: &str, now: i64) -> Option<i64> {
    let hash = sha256_hex(secret.as_bytes());
    live(value, key, now)
        .iter()
        .find(|l| l["hash"].as_str() == Some(hash.as_str()))
        .and_then(|l| l["id"].as_i64())
}

/// Länkens id om `token` är en länk som inte har gått ut. Länken kan
/// användas flera gånger under sin giltighetstid; varje gång är en
/// inloggning.
pub fn link_for(path: &Path, token: &str, now: i64) -> io::Result<Option<i64>> {
    Ok(find(&read(path)?, "lankar", token, now))
}

/// Sparar en ny session för länken `id`, giltig till `expires`.
pub fn start_session(
    path: &Path,
    id: i64,
    session: &str,
    expires: i64,
    now: i64,
) -> io::Result<()> {
    let mut value = read(path)?;
    let links = live(&value, "lankar", now);
    let mut sessions = live(&value, "sessioner", now);
    sessions.push(json!({"id": id, "hash": sha256_hex(session.as_bytes()), "utgar": expires}));
    store(path, &mut value, links, sessions)
}

/// Länkens id för en session som inte har gått ut.
pub fn session_link(path: &Path, session: &str, now: i64) -> io::Result<Option<i64>> {
    Ok(find(&read(path)?, "sessioner", session, now))
}

/// Stryker länken `id` och dess sessioner ur filen.
pub fn revoke(path: &Path, id: i64, now: i64) -> io::Result<()> {
    let mut value = read(path)?;
    let mut links = live(&value, "lankar", now);
    let mut sessions = live(&value, "sessioner", now);
    links.retain(|l| l["id"].as_i64() != Some(id));
    sessions.retain(|l| l["id"].as_i64() != Some(id));
    store(path, &mut value, links, sessions)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn links_and_sessions_expire_and_can_be_revoked() {
        let dir = tempfile::tempdir().unwrap();
        let f = file_beside(&dir.path().join("bokis.db"));
        set_origin(&f, "https://bokis.example:8443", 100).unwrap();
        assert_eq!(
            origin(&f).unwrap().as_deref(),
            Some("https://bokis.example:8443")
        );
        issue(&f, 7, "aaaa", 1_000, 100).unwrap();
        issue(&f, 8, "bbbb", 1_000, 100).unwrap();
        assert_eq!(link_for(&f, "cccc", 110).unwrap(), None);
        // En länk fungerar flera gånger tills den går ut.
        assert_eq!(link_for(&f, "aaaa", 110).unwrap(), Some(7));
        assert_eq!(link_for(&f, "aaaa", 120).unwrap(), Some(7));
        assert_eq!(link_for(&f, "aaaa", 1_000).unwrap(), None);

        start_session(&f, 7, "s7", 500, 110).unwrap();
        start_session(&f, 8, "s8", 500, 110).unwrap();
        assert_eq!(session_link(&f, "s7", 200).unwrap(), Some(7));
        assert_eq!(session_link(&f, "s7", 500).unwrap(), None);
        revoke(&f, 8, 200).unwrap();
        assert_eq!(session_link(&f, "s8", 200).unwrap(), None);
        assert_eq!(link_for(&f, "bbbb", 200).unwrap(), None);
        assert_eq!(link_for(&f, "aaaa", 200).unwrap(), Some(7));

        let text = fs::read_to_string(&f).unwrap();
        assert!(!text.contains("aaaa") && !text.contains("s7"));
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&f).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn links_from_before_the_names_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let f = file_beside(&dir.path().join("bokis.db"));
        fs::write(
            &f,
            format!(
                r#"{{"adress": "https://x", "lankar": [{{"hash": "{}", "utgar": 999}}]}}"#,
                sha256_hex(b"gammal")
            ),
        )
        .unwrap();
        assert_eq!(link_for(&f, "gammal", 100).unwrap(), None);
        set_origin(&f, "https://x", 100).unwrap();
        assert!(!fs::read_to_string(&f).unwrap().contains("999"));
    }
}
