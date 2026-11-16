//! Inloggningslänkar till en insyn som körs som tjänst.
//!
//! `bok insyn --adress <url>` startar insynen utan länk och skriver adressen
//! i `insyn.json` bredvid databasen. `bok insyn lank` lägger där en ny
//! engångslänk som gäller i 15 minuter; filen sparar bara länkens SHA-256.
//! Tjänsten läser filen när någon öppnar `/login` och stryker länken som
//! används.

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use bok_store::bok_core::canonical::sha256_hex;
use serde_json::{json, Value};

/// Hur länge en länk gäller, i sekunder.
pub const LINK_TTL: i64 = 15 * 60;

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

fn live_links(value: &Value, now: i64) -> Vec<Value> {
    value["lankar"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|l| l["utgar"].as_i64().is_some_and(|t| t > now))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// Sparar adressen som tjänsten nås på, och stryker gamla länkar.
pub fn set_origin(path: &Path, origin: &str, now: i64) -> io::Result<()> {
    let value = read(path)?;
    write(
        path,
        &json!({"adress": origin, "lankar": live_links(&value, now)}),
    )
}

/// Adressen som tjänsten nås på, om den körs med `--adress`.
pub fn origin(path: &Path) -> io::Result<Option<String>> {
    Ok(read(path)?["adress"].as_str().map(str::to_string))
}

/// Lägger en ny engångslänk med `token` som hemlighet.
pub fn issue(path: &Path, token: &str, now: i64) -> io::Result<()> {
    let mut value = read(path)?;
    let mut links = live_links(&value, now);
    links.push(json!({"hash": sha256_hex(token.as_bytes()), "utgar": now + LINK_TTL}));
    value["lankar"] = Value::Array(links);
    write(path, &value)
}

/// Sant om `token` är en giltig länk; länken stryks då så att den bara
/// fungerar en gång.
pub fn consume(path: &Path, token: &str, now: i64) -> io::Result<bool> {
    let mut value = read(path)?;
    let hash = sha256_hex(token.as_bytes());
    let mut links = live_links(&value, now);
    let before = links.len();
    links.retain(|l| l["hash"].as_str() != Some(hash.as_str()));
    let found = links.len() < before;
    value["lankar"] = Value::Array(links);
    write(path, &value)?;
    Ok(found)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_link_works_once_and_expires() {
        let dir = tempfile::tempdir().unwrap();
        let f = file_beside(&dir.path().join("bokis.db"));
        set_origin(&f, "https://bokis.example:8443", 100).unwrap();
        assert_eq!(
            origin(&f).unwrap().as_deref(),
            Some("https://bokis.example:8443")
        );
        issue(&f, "aaaa", 100).unwrap();
        assert!(!consume(&f, "bbbb", 110).unwrap());
        assert!(consume(&f, "aaaa", 110).unwrap());
        assert!(!consume(&f, "aaaa", 111).unwrap());
        issue(&f, "cccc", 200).unwrap();
        assert!(!consume(&f, "cccc", 200 + LINK_TTL).unwrap());
        assert!(!fs::read_to_string(&f).unwrap().contains("cccc"));
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&f).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
