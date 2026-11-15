//! Underlagsarkiv för Bokis.
//!
//! Varje mottaget underlag sparas byte för byte i en fil vars namn är
//! filens SHA-256 (BFL 7 kap. 1 §: mottaget format och oförändrat innehåll).
//! Arkivet har inget API för att ändra eller radera en fil. Filerna görs
//! skrivskyddade, och [`Archive::verify_all`] räknar om varje hash så att
//! en ändring gjord utanför programmet upptäcks.
//!
//! Layout under roten, samma som `bok`-kommandots arkiv i milstolpe 1
//! (`<databasens katalog>/underlag`), så att arkivet kan ta över utan att
//! filer flyttas:
//!
//! ```text
//! <rot>/ab/abcdef…   en fil per underlag, namnet är hela hashen
//! <rot>/.tmp/        tillfälliga filer under skrivning
//! ```
//!
//! Originalnamn, mottagningstid, kanal och avsändare hör inte hit; de sparas
//! i databasen tillsammans med hashen.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

pub mod media;

/// Fel från arkivet.
#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("I/O-fel för {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("ogiltig SHA-256: {0:?}")]
    InvalidHash(String),
    #[error("underlaget {0} finns inte i arkivet")]
    Missing(String),
    #[error("filen för {expected} har ändrats: innehållet har hashen {actual}")]
    Changed { expected: String, actual: String },
}

/// Resultat från arkivet.
pub type Result<T> = std::result::Result<T, ArchiveError>;

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> ArchiveError + '_ {
    move |source| ArchiveError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Ett underlag som finns i arkivet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    /// SHA-256 som 64 hexadecimala gemener.
    pub sha256: String,
    /// Storlek i byte.
    pub size: u64,
    /// Sökväg till filen i arkivet.
    pub path: PathBuf,
    /// `true` om filen lades in nu, `false` om samma innehåll redan fanns.
    pub is_new: bool,
}

/// En post i arkivets förteckning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub sha256: String,
    pub size: u64,
}

/// Resultat av [`Archive::verify_all`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct VerifyReport {
    /// Antal filer vars innehåll stämmer med namnet.
    pub ok: usize,
    /// Filer vars innehåll inte längre stämmer med hashen i namnet:
    /// (hash i namnet, faktisk hash).
    pub changed: Vec<(String, String)>,
    /// Filer eller kataloger som inte hör hemma i arkivet.
    pub unknown: Vec<PathBuf>,
}

impl VerifyReport {
    /// `true` om inga avvikelser hittades.
    pub fn is_intact(&self) -> bool {
        self.changed.is_empty() && self.unknown.is_empty()
    }
}

/// Katalogen för filer under skrivning.
const TMP_DIR: &str = ".tmp";

/// `true` för en katalog vars namn är två gemena hexadecimala tecken.
fn is_prefix_dir(path: &Path) -> bool {
    path.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
        n.len() == 2
            && n.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// Ett underlagsarkiv på disk.
#[derive(Debug, Clone)]
pub struct Archive {
    root: PathBuf,
}

/// Kontrollerar att en sträng är en SHA-256 i gemena hexadecimala tecken.
pub fn validate_hash(sha256: &str) -> Result<()> {
    let valid = sha256.len() == 64
        && sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if valid {
        Ok(())
    } else {
        Err(ArchiveError::InvalidHash(sha256.to_string()))
    }
}

/// Räknar SHA-256 för en byte-sekvens.
pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

fn sha256_of_file(path: &Path) -> Result<(String, u64)> {
    let mut file = File::open(path).map_err(io_err(path))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut size = 0u64;
    loop {
        let n = file.read(&mut buf).map_err(io_err(path))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    Ok((hex::encode(hasher.finalize()), size))
}

#[cfg(unix)]
fn make_read_only(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o444)).map_err(io_err(path))
}

#[cfg(not(unix))]
fn make_read_only(path: &Path) -> Result<()> {
    let mut perms = fs::metadata(path).map_err(io_err(path))?.permissions();
    perms.set_readonly(true);
    fs::set_permissions(path, perms).map_err(io_err(path))
}

impl Archive {
    /// Öppnar ett arkiv och skapar katalogerna om de saknas.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let tmp = root.join(TMP_DIR);
        fs::create_dir_all(&tmp).map_err(io_err(&tmp))?;
        Ok(Self { root })
    }

    /// Arkivets rotkatalog.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Sökvägen där ett underlag med given hash ligger (eller skulle ligga).
    pub fn path_for(&self, sha256: &str) -> Result<PathBuf> {
        validate_hash(sha256)?;
        Ok(self.root.join(&sha256[..2]).join(sha256))
    }

    /// `true` om ett underlag med given hash finns i arkivet.
    pub fn contains(&self, sha256: &str) -> bool {
        self.path_for(sha256).map(|p| p.is_file()).unwrap_or(false)
    }

    /// Lägger in en byte-sekvens. Samma innehåll lagras bara en gång.
    pub fn put_bytes(&self, data: &[u8]) -> Result<Stored> {
        let sha256 = sha256_hex(data);
        self.write(&sha256, data.len() as u64, |file, path| {
            file.write_all(data).map_err(io_err(path))
        })
    }

    /// Lägger in en fil från disk utan att ändra källfilen.
    pub fn put_file(&self, source: &Path) -> Result<Stored> {
        let (sha256, size) = sha256_of_file(source)?;
        self.write(&sha256, size, |file, path| {
            let mut input = File::open(source).map_err(io_err(source))?;
            io::copy(&mut input, file).map_err(io_err(path))?;
            Ok(())
        })
    }

    fn write(
        &self,
        sha256: &str,
        size: u64,
        write_content: impl FnOnce(&mut File, &Path) -> Result<()>,
    ) -> Result<Stored> {
        let target = self.path_for(sha256)?;
        if target.is_file() {
            // Samma innehåll finns redan. Kontrollera att filen är orörd.
            let (actual, actual_size) = sha256_of_file(&target)?;
            if actual != sha256 {
                return Err(ArchiveError::Changed {
                    expected: sha256.to_string(),
                    actual,
                });
            }
            return Ok(Stored {
                sha256: sha256.to_string(),
                size: actual_size,
                path: target,
                is_new: false,
            });
        }

        let dir = target
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.root.clone());
        fs::create_dir_all(&dir).map_err(io_err(&dir))?;

        let tmp = self
            .root
            .join(TMP_DIR)
            .join(format!("{sha256}.{}.tmp", std::process::id()));
        {
            let mut file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&tmp)
                .map_err(io_err(&tmp))?;
            write_content(&mut file, &tmp)?;
            file.sync_all().map_err(io_err(&tmp))?;
        }

        // Kontrollera det som faktiskt hamnade på disk innan det får sitt namn.
        let (written, written_size) = sha256_of_file(&tmp)?;
        if written != sha256 || written_size != size {
            let _ = fs::remove_file(&tmp);
            return Err(ArchiveError::Changed {
                expected: sha256.to_string(),
                actual: written,
            });
        }

        make_read_only(&tmp)?;
        fs::rename(&tmp, &target).map_err(io_err(&target))?;
        if let Ok(d) = File::open(&dir) {
            let _ = d.sync_all();
        }

        Ok(Stored {
            sha256: sha256.to_string(),
            size,
            path: target,
            is_new: true,
        })
    }

    /// Läser ett underlag och kontrollerar att innehållet stämmer med hashen.
    pub fn read(&self, sha256: &str) -> Result<Vec<u8>> {
        let path = self.path_for(sha256)?;
        if !path.is_file() {
            return Err(ArchiveError::Missing(sha256.to_string()));
        }
        let data = fs::read(&path).map_err(io_err(&path))?;
        let actual = sha256_hex(&data);
        if actual != sha256 {
            return Err(ArchiveError::Changed {
                expected: sha256.to_string(),
                actual,
            });
        }
        Ok(data)
    }

    /// Förteckning över alla underlag, sorterad på hash.
    pub fn list(&self) -> Result<Vec<Entry>> {
        let mut entries = Vec::new();
        let base = &self.root;
        for prefix in fs::read_dir(base).map_err(io_err(base))? {
            let prefix = prefix.map_err(io_err(base))?.path();
            if !prefix.is_dir() || !is_prefix_dir(&prefix) {
                continue;
            }
            for file in fs::read_dir(&prefix).map_err(io_err(&prefix))? {
                let file = file.map_err(io_err(&prefix))?;
                let name = file.file_name().to_string_lossy().to_string();
                if validate_hash(&name).is_ok() {
                    let size = file.metadata().map_err(io_err(&file.path()))?.len();
                    entries.push(Entry { sha256: name, size });
                }
            }
        }
        entries.sort_by(|a, b| a.sha256.cmp(&b.sha256));
        Ok(entries)
    }

    /// Räknar om hashen för varje fil och rapporterar avvikelser. Filer som
    /// blivit kvar i `.tmp` efter en avbruten skrivning räknas som okända.
    pub fn verify_all(&self) -> Result<VerifyReport> {
        let mut report = VerifyReport::default();
        let base = &self.root;
        for prefix in fs::read_dir(base).map_err(io_err(base))? {
            let prefix = prefix.map_err(io_err(base))?.path();
            let prefix_name = prefix
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if prefix_name == TMP_DIR && prefix.is_dir() {
                for leftover in fs::read_dir(&prefix).map_err(io_err(&prefix))? {
                    report
                        .unknown
                        .push(leftover.map_err(io_err(&prefix))?.path());
                }
                continue;
            }
            if !prefix.is_dir() || !is_prefix_dir(&prefix) {
                report.unknown.push(prefix);
                continue;
            }
            for file in fs::read_dir(&prefix).map_err(io_err(&prefix))? {
                let path = file.map_err(io_err(&prefix))?.path();
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                if validate_hash(&name).is_err()
                    || !name.starts_with(&prefix_name)
                    || !path.is_file()
                {
                    report.unknown.push(path);
                    continue;
                }
                let (actual, _) = sha256_of_file(&path)?;
                if actual == name {
                    report.ok += 1;
                } else {
                    report.changed.push((name, actual));
                }
            }
        }
        report.changed.sort();
        report.unknown.sort();
        Ok(report)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn new_archive() -> (tempfile::TempDir, Archive) {
        let dir = tempfile::tempdir().unwrap();
        let archive = Archive::open(dir.path().join("arkiv")).unwrap();
        (dir, archive)
    }

    #[test]
    fn same_content_is_stored_once() {
        let (_dir, archive) = new_archive();
        let first = archive.put_bytes(b"kvitto 125 kr").unwrap();
        let second = archive.put_bytes(b"kvitto 125 kr").unwrap();
        assert!(first.is_new);
        assert!(!second.is_new);
        assert_eq!(first.sha256, second.sha256);
        assert_eq!(first.path, second.path);
        assert_eq!(archive.list().unwrap().len(), 1);
    }

    #[test]
    fn content_is_kept_byte_for_byte() {
        let (_dir, archive) = new_archive();
        // CRLF och bytes utanför UTF-8 ska överleva oförändrade.
        let data = b"From: lev@example.se\r\nSubject: Faktura\r\n\r\n\xff\xfe\x00rad\r\n";
        let stored = archive.put_bytes(data).unwrap();
        assert_eq!(archive.read(&stored.sha256).unwrap(), data.to_vec());
        assert_eq!(stored.sha256, sha256_hex(data));
    }

    #[test]
    fn file_is_stored_without_touching_the_source() {
        let (dir, archive) = new_archive();
        let source = dir.path().join("faktura.pdf");
        fs::write(&source, b"%PDF-1.7 testinnehall").unwrap();
        let stored = archive.put_file(&source).unwrap();
        assert_eq!(fs::read(&source).unwrap(), b"%PDF-1.7 testinnehall");
        assert_eq!(stored.size, 21);
        assert!(archive.contains(&stored.sha256));
    }

    #[cfg(unix)]
    #[test]
    fn stored_files_are_read_only() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, archive) = new_archive();
        let stored = archive.put_bytes(b"skrivskydd").unwrap();
        let mode = fs::metadata(&stored.path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o444);
    }

    #[cfg(unix)]
    #[test]
    fn changes_outside_the_program_are_detected() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, archive) = new_archive();
        let stored = archive.put_bytes(b"originalet").unwrap();
        archive.put_bytes(b"ett annat underlag").unwrap();

        fs::set_permissions(&stored.path, fs::Permissions::from_mode(0o644)).unwrap();
        fs::write(&stored.path, b"manipulerat").unwrap();

        let report = archive.verify_all().unwrap();
        assert!(!report.is_intact());
        assert_eq!(report.ok, 1);
        assert_eq!(report.changed.len(), 1);
        assert_eq!(report.changed[0].0, stored.sha256);
        assert!(matches!(
            archive.read(&stored.sha256),
            Err(ArchiveError::Changed { .. })
        ));
        // Att lägga in originalet igen reparerar inte tyst en ändrad fil.
        assert!(matches!(
            archive.put_bytes(b"originalet"),
            Err(ArchiveError::Changed { .. })
        ));
    }

    #[test]
    fn unknown_files_are_reported() {
        let (_dir, archive) = new_archive();
        let stored = archive.put_bytes(b"ok").unwrap();
        fs::write(archive.root().join("smuggel.txt"), b"x").unwrap();
        fs::create_dir(archive.root().join("zz")).unwrap();
        fs::write(
            stored.path.parent().unwrap().join(".tmpAbC123"),
            b"halvskriven",
        )
        .unwrap();
        fs::write(archive.root().join(".tmp").join("avbruten.tmp"), b"y").unwrap();
        let report = archive.verify_all().unwrap();
        assert_eq!(report.unknown.len(), 4, "{:?}", report.unknown);
        assert_eq!(report.ok, 1);
        assert_eq!(archive.list().unwrap().len(), 1);
    }

    #[test]
    fn layout_matches_the_milestone_1_cli() {
        // `bok underlag lagg-till` sparar som <underlag>/<två första tecknen>/<hash>.
        let (_dir, archive) = new_archive();
        let data = b"kvitto fran bok-cli";
        let sha = sha256_hex(data);
        let dir = archive.root().join(&sha[..2]);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(&sha), data).unwrap();
        assert_eq!(archive.read(&sha).unwrap(), data.to_vec());
        assert!(!archive.put_bytes(data).unwrap().is_new);
        assert!(archive.verify_all().unwrap().is_intact());
    }

    #[test]
    fn invalid_hash_is_refused() {
        let (_dir, archive) = new_archive();
        assert!(matches!(
            archive.path_for("../../etc/passwd"),
            Err(ArchiveError::InvalidHash(_))
        ));
        assert!(matches!(
            archive.read(&"A".repeat(64)),
            Err(ArchiveError::InvalidHash(_))
        ));
        assert!(matches!(
            archive.read(&"a".repeat(64)),
            Err(ArchiveError::Missing(_))
        ));
    }

    #[test]
    fn listing_is_sorted() {
        let (_dir, archive) = new_archive();
        for i in 0..20 {
            archive
                .put_bytes(format!("underlag {i}").as_bytes())
                .unwrap();
        }
        let list = archive.list().unwrap();
        assert_eq!(list.len(), 20);
        assert!(list.windows(2).all(|w| w[0].sha256 < w[1].sha256));
    }
}
