//! Anslutningar, nycklar och uppladdningslänkar, i `fjarr.json` bredvid
//! databasen.
//!
//! Det här är behörigheter, inte räkenskapsinformation: de ska kunna
//! återkallas och tas bort, så de ligger utanför den append-only-databasen.
//! Att en anslutning godkänns eller återkallas loggas däremot i
//! behandlingshistoriken. Nycklar, koder och länkar sparas bara som
//! SHA-256, och filen skrivs med behörigheten 0600 och ett lås, eftersom
//! servern och kommandona kan skriva samtidigt.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Hur länge en åtkomstnyckel gäller, i sekunder.
pub const ACCESS_TTL: i64 = 60 * 60;
/// Hur länge en förnyelsenyckel gäller utan att användas.
pub const REFRESH_TTL: i64 = 30 * 24 * 60 * 60;
/// Hur länge en kopplingskod gäller.
pub const PAIRING_TTL: i64 = 15 * 60;
/// Felaktiga försök innan en kopplingskod slutar gälla.
pub const PAIRING_ATTEMPTS: u32 = 5;
/// Hur länge en kod för telefonens uppladdningslänk gäller.
pub const LINK_CODE_TTL: i64 = 15 * 60;
/// Felaktiga försök innan koden för uppladdningslänken slutar gälla.
pub const LINK_CODE_ATTEMPTS: u32 = 5;
/// En registrering som aldrig godkänns tas bort efter så här lång tid.
pub const UNAPPROVED_TTL: i64 = 24 * 60 * 60;
/// Högst så många registreringar som väntar på godkännande.
pub const MAX_UNAPPROVED: usize = 20;

/// En agentklient som har registrerat sig (OAuth, RFC 7591).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Client {
    pub id: String,
    pub name: String,
    pub redirect_uris: Vec<String>,
    pub registered: i64,
    /// När anslutningen godkändes med en kopplingskod.
    pub approved: Option<i64>,
    pub revoked: Option<i64>,
    pub last_used: Option<i64>,
    /// SHA-256 av klienthemligheten, för en klient som registrerade sig med
    /// `client_secret_post` eller `client_secret_basic`. Publika klienter har ingen.
    #[serde(default)]
    pub secret_hash: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TokenKind {
    Access,
    Refresh,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
    pub hash: String,
    pub kind: TokenKind,
    pub client_id: String,
    pub expires: i64,
}

/// En kopplingskod som `bok-fjarr kod` har tagit fram.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pairing {
    pub hash: String,
    pub expires: i64,
    pub attempts: u32,
}

/// En uppladdningslänk för en telefon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub hash: String,
    pub name: String,
    pub created: i64,
    pub revoked: Option<i64>,
    pub last_used: Option<i64>,
}

/// En kod som `bok-fjarr uppladdning` har tagit fram. Telefonen skriver in
/// den på startsidan och får då en ny uppladdningslänk med det här namnet.
/// Länken skapas först då, så dess hemlighet visas aldrig på datorn och
/// behöver aldrig sparas.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkCode {
    pub hash: String,
    pub name: String,
    pub expires: i64,
    pub attempts: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// Den publika adressen som servern senast startades med.
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub clients: Vec<Client>,
    #[serde(default)]
    pub tokens: Vec<Token>,
    #[serde(default)]
    pub pairing: Option<Pairing>,
    #[serde(default)]
    pub links: Vec<Link>,
    /// Koden som telefonen hämtar sin länk med. Högst en åt gången.
    #[serde(default)]
    pub link_code: Option<LinkCode>,
}

/// SHA-256 som hexsträng.
pub fn hash(secret: &str) -> String {
    hex::encode(Sha256::digest(secret.as_bytes()))
}

/// 32 slumpade byte som hex.
pub fn random_secret() -> io::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(hex::encode(bytes))
}

/// Tecknen i en kopplingskod: 32 stycken, utan 0, 1, I och O som är lätta
/// att förväxla. 32 tecken gör att varje slumpad byte kan användas utan
/// snedfördelning.
const CODE_ALPHABET: &[u8; 32] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";

/// En kopplingskod som `7KQM-X3PD`: 8 tecken, 40 bitar.
pub fn pairing_code() -> io::Result<String> {
    let mut bytes = [0u8; 8];
    getrandom::getrandom(&mut bytes).map_err(|e| io::Error::other(e.to_string()))?;
    let chars: String = bytes
        .iter()
        .map(|b| CODE_ALPHABET[usize::from(b & 31)] as char)
        .collect();
    Ok(format!("{}-{}", &chars[..4], &chars[4..]))
}

/// Koden som den skrevs in, utan mellanslag och bindestreck, med versaler.
pub fn normalize_code(code: &str) -> String {
    code.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

impl State {
    /// Tar bort det som har gått ut: nycklar, gamla koder och registreringar
    /// som aldrig godkändes.
    pub fn tidy(&mut self, now: i64) {
        self.tokens.retain(|t| t.expires > now);
        if self.pairing.as_ref().is_some_and(|p| p.expires <= now) {
            self.pairing = None;
        }
        if self.link_code.as_ref().is_some_and(|c| c.expires <= now) {
            self.link_code = None;
        }
        self.clients
            .retain(|c| c.approved.is_some() || now - c.registered < UNAPPROVED_TTL);
    }

    pub fn client(&self, id: &str) -> Option<&Client> {
        self.clients.iter().find(|c| c.id == id)
    }

    pub fn client_mut(&mut self, id: &str) -> Option<&mut Client> {
        self.clients.iter_mut().find(|c| c.id == id)
    }

    /// Klienten bakom en giltig åtkomstnyckel.
    pub fn access(&self, token: &str, now: i64) -> Option<&Client> {
        let h = hash(token);
        let t = self
            .tokens
            .iter()
            .find(|t| t.kind == TokenKind::Access && t.hash == h && t.expires > now)?;
        self.client(&t.client_id)
            .filter(|c| c.revoked.is_none() && c.approved.is_some())
    }

    /// En giltig uppladdningslänk.
    pub fn link(&self, secret: &str) -> Option<&Link> {
        let h = hash(secret);
        self.links
            .iter()
            .find(|l| l.hash == h && l.revoked.is_none())
    }

    /// Finns det en giltig länk med det här namnet? Versaler och gemener
    /// räknas som samma.
    pub fn link_named(&self, name: &str) -> bool {
        self.links
            .iter()
            .any(|l| l.revoked.is_none() && l.name.eq_ignore_ascii_case(name))
    }
}

/// `fjarr.json` med sitt lås.
#[derive(Debug, Clone)]
pub struct StateFile {
    path: PathBuf,
    lock: PathBuf,
}

fn open_private(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

impl StateFile {
    /// Filen bredvid databasen.
    pub fn beside(db: &Path) -> Self {
        let dir = db.parent().map(Path::to_path_buf).unwrap_or_default();
        Self {
            path: dir.join("fjarr.json"),
            lock: dir.join("fjarr.lock"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn load(&self) -> io::Result<State> {
        match fs::read(&self.path) {
            Ok(data) => serde_json::from_slice(&data).map_err(|e| {
                io::Error::other(format!("{} går inte att läsa: {e}", self.path.display()))
            }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(State::default()),
            Err(e) => Err(e),
        }
    }

    /// Läser tillståndet.
    pub fn read(&self) -> io::Result<State> {
        let lock = open_private(&self.lock)?;
        lock.lock_shared()?;
        let state = self.load();
        lock.unlock()?;
        state
    }

    /// Läser, ändrar och skriver tillbaka tillståndet under ett exklusivt lås.
    /// Filen ersätts i ett steg, så att en avbruten skrivning inte lämnar en
    /// halv fil efter sig.
    pub fn update<T>(&self, now: i64, f: impl FnOnce(&mut State) -> T) -> io::Result<T> {
        let lock = open_private(&self.lock)?;
        lock.lock()?;
        let result = (|| {
            let mut state = self.load()?;
            state.tidy(now);
            let out = f(&mut state);
            let tmp = self.path.with_extension("json.tmp");
            {
                let mut file = open_private(&tmp)?;
                file.set_len(0)?;
                file.write_all(
                    serde_json::to_vec_pretty(&state)
                        .map_err(io::Error::other)?
                        .as_slice(),
                )?;
                file.sync_all()?;
            }
            fs::rename(&tmp, &self.path)?;
            Ok(out)
        })();
        lock.unlock()?;
        result
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_readable_and_normalized() {
        let code = pairing_code().unwrap();
        assert_eq!(code.len(), 9);
        assert_eq!(&code[4..5], "-");
        assert!(code
            .chars()
            .all(|c| c == '-' || CODE_ALPHABET.contains(&(c as u8))));
        assert_eq!(normalize_code(" 7kqm x3pd "), "7KQMX3PD");
        assert_eq!(normalize_code("7KQM-X3PD"), "7KQMX3PD");
        assert_ne!(random_secret().unwrap(), random_secret().unwrap());
    }

    #[test]
    fn state_is_written_privately_and_tidied() {
        let dir = tempfile::tempdir().unwrap();
        let file = StateFile::beside(&dir.path().join("bokis.db"));
        file.update(1000, |s| {
            s.clients.push(Client {
                id: "c1".into(),
                name: "Claude".into(),
                redirect_uris: vec!["https://claude.ai/api/mcp/auth_callback".into()],
                registered: 1000,
                approved: None,
                revoked: None,
                last_used: None,
                secret_hash: None,
            });
            s.tokens.push(Token {
                hash: hash("t"),
                kind: TokenKind::Access,
                client_id: "c1".into(),
                expires: 2000,
            });
        })
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(file.path()).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let s = file.read().unwrap();
        assert_eq!(s.clients.len(), 1);
        // Inte godkänd: nyckeln ger ingen åtkomst.
        assert!(s.access("t", 1500).is_none());
        // Efter ett dygn utan godkännande försvinner registreringen, och nyckeln har gått ut.
        file.update(1000 + UNAPPROVED_TTL + 1, |_| ()).unwrap();
        let s = file.read().unwrap();
        assert!(s.clients.is_empty() && s.tokens.is_empty());
    }
}
