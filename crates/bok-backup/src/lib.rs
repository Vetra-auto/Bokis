//! Säkerhetskopior för Bokis (BFNAR 2013:2 punkt 8.2: kopian ska förvaras
//! åtskild från originalet).
//!
//! En kopia skrivs till en extern plats, till exempel en USB-disk eller en
//! monterad NAS, med den här layouten:
//!
//! ```text
//! <mål>/underlag/ab/…               underlag, delade mellan alla kopior
//! <mål>/kopior/<tid>/bokis.db        konsistent kopia av databasen (VACUUM INTO)
//! <mål>/kopior/<tid>/manifest.json   SHA-256 för databasen och varje underlag
//! <mål>/kopior/<tid>/manifest.sha256 SHA-256 för manifestet
//! ```
//!
//! [`restore_into_empty`] skriver bara till en tom katalog. Programmet skriver
//! aldrig över den levande bokföringen.

use std::fs;
use std::path::{Path, PathBuf};

use bok_archive::Archive;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Fel vid kopiering, kontroll eller återläsning.
#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("I/O-fel för {0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("databasfel: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("arkivfel: {0}")]
    Archive(#[from] bok_archive::ArchiveError),
    #[error("manifestet kan inte läsas: {0}")]
    Manifest(String),
    #[error("målkatalogen {0} är inte tom; återläsning görs bara till en tom katalog")]
    NotEmpty(PathBuf),
    #[error("kopian är skadad: {0}")]
    Damaged(String),
}

fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> BackupError + '_ {
    move |e| BackupError::Io(path.to_path_buf(), e)
}

/// Uppgifter som anroparen lägger i manifestet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupMeta {
    /// Tidpunkt i ISO 8601, till exempel `2026-11-14T02:00:00Z`.
    #[serde(rename = "skapad")]
    pub created: String,
    #[serde(rename = "programversion")]
    pub program_version: String,
    /// Senaste hash i behandlingshistorikens kedja, om den finns.
    #[serde(rename = "kedjans_huvud")]
    pub chain_head: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub sha256: String,
    #[serde(rename = "storlek")]
    pub size: u64,
}

/// Innehållet i `manifest.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    #[serde(flatten)]
    pub meta: BackupMeta,
    #[serde(rename = "databas")]
    pub database: FileEntry,
    #[serde(rename = "underlag")]
    pub documents: Vec<FileEntry>,
}

/// Vad en kopiering gjorde.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupReport {
    pub dir: PathBuf,
    pub database: FileEntry,
    pub documents_copied: usize,
    pub documents_existing: usize,
    pub manifest_sha256: String,
}

/// Resultat av en kontroll av en kopia.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CheckReport {
    pub errors: Vec<String>,
    pub documents_ok: usize,
}

impl CheckReport {
    pub fn is_intact(&self) -> bool {
        self.errors.is_empty()
    }
}

const FORMAT: &str = "bokis-kopia/1";
const DB_NAME: &str = "bokis.db";

fn hash_file(path: &Path) -> Result<FileEntry, BackupError> {
    let data = fs::read(path).map_err(io_err(path))?;
    Ok(FileEntry {
        sha256: hex::encode(Sha256::digest(&data)),
        size: data.len() as u64,
    })
}

#[cfg(unix)]
fn make_read_only(path: &Path) -> Result<(), BackupError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o444)).map_err(io_err(path))
}

#[cfg(not(unix))]
fn make_read_only(path: &Path) -> Result<(), BackupError> {
    let mut p = fs::metadata(path).map_err(io_err(path))?.permissions();
    p.set_readonly(true);
    fs::set_permissions(path, p).map_err(io_err(path))
}

fn integrity_check(db: &Path) -> Result<(), BackupError> {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let answer: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if answer == "ok" {
        Ok(())
    } else {
        Err(BackupError::Damaged(format!("integrity_check: {answer}")))
    }
}

fn dir_name(created: &str) -> String {
    created
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Skriver en kopia av databasen och alla underlag till `target`.
pub fn create_backup(
    db: &Path,
    archive: &Archive,
    target: &Path,
    meta: &BackupMeta,
) -> Result<BackupReport, BackupError> {
    let backups = target.join("kopior");
    fs::create_dir_all(&backups).map_err(io_err(&backups))?;
    let base = dir_name(&meta.created);
    let mut dir = backups.join(&base);
    let mut n = 2;
    while dir.exists() {
        dir = backups.join(format!("{base}-{n}"));
        n += 1;
    }
    fs::create_dir_all(&dir).map_err(io_err(&dir))?;

    // Konsistent ögonblicksbild, även när databasen körs i WAL-läge.
    let db_copy = dir.join(DB_NAME);
    {
        let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        conn.execute("VACUUM INTO ?1", [db_copy.to_string_lossy().as_ref()])?;
    }
    integrity_check(&db_copy)?;
    let database = hash_file(&db_copy)?;
    make_read_only(&db_copy)?;

    // Underlagen läggs i en delad pool; det som redan finns kopieras inte igen.
    let pool = Archive::open(target.join("underlag"))?;
    let mut documents = Vec::new();
    let (mut copied, mut existing) = (0, 0);
    for entry in archive.list()? {
        let source = archive.path_for(&entry.sha256)?;
        let stored = pool.put_file(&source)?;
        if stored.sha256 != entry.sha256 {
            return Err(BackupError::Damaged(format!(
                "underlaget {} i arkivet har ändrats (innehållet har hashen {})",
                entry.sha256, stored.sha256
            )));
        }
        if stored.is_new {
            copied += 1;
        } else {
            existing += 1;
        }
        documents.push(FileEntry {
            sha256: entry.sha256,
            size: entry.size,
        });
    }

    let manifest = Manifest {
        format: FORMAT.into(),
        meta: meta.clone(),
        database: database.clone(),
        documents,
    };
    let manifest_json =
        serde_json::to_vec_pretty(&manifest).map_err(|e| BackupError::Manifest(e.to_string()))?;
    let manifest_sha256 = hex::encode(Sha256::digest(&manifest_json));
    let manifest_file = dir.join("manifest.json");
    fs::write(&manifest_file, &manifest_json).map_err(io_err(&manifest_file))?;
    let hash_file_path = dir.join("manifest.sha256");
    fs::write(
        &hash_file_path,
        format!("{manifest_sha256}  manifest.json\n"),
    )
    .map_err(io_err(&hash_file_path))?;
    make_read_only(&manifest_file)?;
    make_read_only(&hash_file_path)?;

    Ok(BackupReport {
        dir,
        database,
        documents_copied: copied,
        documents_existing: existing,
        manifest_sha256,
    })
}

fn read_manifest(backup: &Path) -> Result<(Manifest, Vec<String>), BackupError> {
    let mut errors = Vec::new();
    let manifest_file = backup.join("manifest.json");
    let data = fs::read(&manifest_file).map_err(io_err(&manifest_file))?;
    let hash_path = backup.join("manifest.sha256");
    match fs::read_to_string(&hash_path) {
        Ok(line) => {
            let expected = line.split_whitespace().next().unwrap_or("");
            let actual = hex::encode(Sha256::digest(&data));
            if expected != actual {
                errors.push(format!("manifest.json har ändrats: {actual} ≠ {expected}"));
            }
        }
        Err(_) => errors.push("manifest.sha256 saknas".into()),
    }
    let manifest: Manifest =
        serde_json::from_slice(&data).map_err(|e| BackupError::Manifest(e.to_string()))?;
    if manifest.format != FORMAT {
        return Err(BackupError::Manifest(format!(
            "okänt format {}",
            manifest.format
        )));
    }
    Ok((manifest, errors))
}

/// Kontrollerar en kopia: manifestets hash, databasens hash och integritet,
/// och att varje underlag finns i poolen med rätt innehåll.
pub fn verify_backup(backup: &Path, document_pool: &Path) -> Result<CheckReport, BackupError> {
    let (manifest, errors) = read_manifest(backup)?;
    let mut report = CheckReport {
        errors,
        documents_ok: 0,
    };
    let db = backup.join(DB_NAME);
    match hash_file(&db) {
        Ok(entry) if entry == manifest.database => {
            if let Err(e) = integrity_check(&db) {
                report.errors.push(e.to_string());
            }
        }
        Ok(entry) => report.errors.push(format!(
            "databasen har ändrats: {} ≠ {}",
            entry.sha256, manifest.database.sha256
        )),
        Err(e) => report.errors.push(e.to_string()),
    }
    let pool = Archive::open(document_pool)?;
    for entry in &manifest.documents {
        match pool.read(&entry.sha256) {
            Ok(data) if data.len() as u64 == entry.size => report.documents_ok += 1,
            Ok(_) => report
                .errors
                .push(format!("underlaget {} har fel storlek", entry.sha256)),
            Err(e) => report
                .errors
                .push(format!("underlaget {}: {e}", entry.sha256)),
        }
    }
    Ok(report)
}

/// Vad en återläsning gjorde.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restored {
    pub database: PathBuf,
    pub archive: PathBuf,
    pub documents: usize,
}

/// Läser in en kopia till en ny, tom katalog: `<ny>/bokis.db` och `<ny>/underlag`,
/// samma layout som `bok` använder (arkivet bredvid databasen).
/// Nekar om katalogen finns och inte är tom. Kopian kontrolleras först.
pub fn restore_into_empty(
    backup: &Path,
    document_pool: &Path,
    new_dir: &Path,
) -> Result<Restored, BackupError> {
    if new_dir.exists()
        && fs::read_dir(new_dir)
            .map_err(io_err(new_dir))?
            .next()
            .is_some()
    {
        return Err(BackupError::NotEmpty(new_dir.to_path_buf()));
    }
    let check = verify_backup(backup, document_pool)?;
    if !check.is_intact() {
        return Err(BackupError::Damaged(check.errors.join("; ")));
    }
    let (manifest, _) = read_manifest(backup)?;
    fs::create_dir_all(new_dir).map_err(io_err(new_dir))?;
    let database = new_dir.join(DB_NAME);
    fs::copy(backup.join(DB_NAME), &database).map_err(io_err(&database))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&database, fs::Permissions::from_mode(0o600))
            .map_err(io_err(&database))?;
    }
    let pool = Archive::open(document_pool)?;
    let archive = Archive::open(new_dir.join("underlag"))?;
    for entry in &manifest.documents {
        let source = pool.path_for(&entry.sha256)?;
        archive.put_file(&source)?;
    }
    Ok(Restored {
        database,
        archive: archive.root().to_path_buf(),
        documents: manifest.documents.len(),
    })
}

/// Kopiorna under en målkatalog, äldst först.
pub fn list_backups(target: &Path) -> Result<Vec<PathBuf>, BackupError> {
    let backups = target.join("kopior");
    if !backups.exists() {
        return Ok(Vec::new());
    }
    let mut list: Vec<PathBuf> = fs::read_dir(&backups)
        .map_err(io_err(&backups))?
        .filter_map(std::result::Result::ok)
        .map(|e| e.path())
        .filter(|p| p.join("manifest.json").is_file())
        .collect();
    list.sort();
    Ok(list)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    struct Env {
        _dir: tempfile::TempDir,
        db: PathBuf,
        archive: Archive,
        target: PathBuf,
        root: PathBuf,
    }

    fn env() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let db = root.join("levande.db");
        let conn = Connection::open(&db).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.execute_batch(
            "CREATE TABLE verifikationer (nr INTEGER PRIMARY KEY, text TEXT NOT NULL);
             INSERT INTO verifikationer (text) VALUES ('A1'), ('A2'), ('A3');",
        )
        .unwrap();
        drop(conn);
        let archive = Archive::open(root.join("arkiv")).unwrap();
        archive.put_bytes(b"kvitto 1").unwrap();
        archive.put_bytes(b"faktura 2").unwrap();
        Env {
            db,
            archive,
            target: root.join("usb"),
            root,
            _dir: dir,
        }
    }

    fn meta(t: &str) -> BackupMeta {
        BackupMeta {
            created: t.into(),
            program_version: "0.1.0".into(),
            chain_head: Some("ab".repeat(32)),
        }
    }

    #[test]
    fn backup_is_created_and_verified() {
        let e = env();
        let r = create_backup(&e.db, &e.archive, &e.target, &meta("2026-11-14T02:00:00Z")).unwrap();
        assert_eq!(r.documents_copied, 2);
        assert_eq!(r.documents_existing, 0);
        let c = verify_backup(&r.dir, &e.target.join("underlag")).unwrap();
        assert!(c.is_intact(), "{:?}", c.errors);
        assert_eq!(c.documents_ok, 2);
        let conn =
            Connection::open_with_flags(r.dir.join(DB_NAME), OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM verifikationer", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 3);
        // Manifestets fältnamn är svenska och stabila.
        let manifest = fs::read_to_string(r.dir.join("manifest.json")).unwrap();
        assert!(manifest.contains("\"kedjans_huvud\"") && manifest.contains("\"underlag\""));
    }

    #[test]
    fn documents_are_copied_once() {
        let e = env();
        create_backup(&e.db, &e.archive, &e.target, &meta("2026-11-14T02:00:00Z")).unwrap();
        e.archive.put_bytes(b"nytt kvitto").unwrap();
        let r = create_backup(&e.db, &e.archive, &e.target, &meta("2026-11-14T02:00:00Z")).unwrap();
        assert_eq!(r.documents_copied, 1);
        assert_eq!(r.documents_existing, 2);
        assert!(r.dir.ends_with("2026-11-14T02-00-00Z-2"));
        assert_eq!(list_backups(&e.target).unwrap().len(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn changed_database_or_manifest_is_detected() {
        use std::os::unix::fs::PermissionsExt;
        let e = env();
        let r = create_backup(&e.db, &e.archive, &e.target, &meta("t1")).unwrap();
        let db = r.dir.join(DB_NAME);
        fs::set_permissions(&db, fs::Permissions::from_mode(0o644)).unwrap();
        let mut data = fs::read(&db).unwrap();
        let last = data.len() - 1;
        data[last] ^= 0xff;
        fs::write(&db, data).unwrap();
        let c = verify_backup(&r.dir, &e.target.join("underlag")).unwrap();
        assert!(
            c.errors.iter().any(|f| f.contains("databasen har ändrats")),
            "{:?}",
            c.errors
        );

        let r2 = create_backup(&e.db, &e.archive, &e.target, &meta("t2")).unwrap();
        let mf = r2.dir.join("manifest.json");
        fs::set_permissions(&mf, fs::Permissions::from_mode(0o644)).unwrap();
        let text = fs::read_to_string(&mf).unwrap().replace("0.1.0", "9.9.9");
        fs::write(&mf, text).unwrap();
        let c = verify_backup(&r2.dir, &e.target.join("underlag")).unwrap();
        assert!(
            c.errors
                .iter()
                .any(|f| f.contains("manifest.json har ändrats")),
            "{:?}",
            c.errors
        );
    }

    #[test]
    fn restore_only_into_an_empty_directory() {
        let e = env();
        let r = create_backup(&e.db, &e.archive, &e.target, &meta("t1")).unwrap();
        let busy = e.root.join("upptagen");
        fs::create_dir_all(&busy).unwrap();
        fs::write(busy.join("bokis.db"), b"levande data").unwrap();
        assert!(matches!(
            restore_into_empty(&r.dir, &e.target.join("underlag"), &busy),
            Err(BackupError::NotEmpty(_))
        ));
        assert_eq!(fs::read(busy.join("bokis.db")).unwrap(), b"levande data");

        let fresh = e.root.join("ny-installation");
        let restored = restore_into_empty(&r.dir, &e.target.join("underlag"), &fresh).unwrap();
        assert_eq!(restored.documents, 2);
        let conn = Connection::open(&restored.database).unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM verifikationer", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 3);
        assert!(Archive::open(&restored.archive)
            .unwrap()
            .verify_all()
            .unwrap()
            .is_intact());
    }

    #[cfg(unix)]
    #[test]
    fn damaged_backup_is_not_restored() {
        use std::os::unix::fs::PermissionsExt;
        let e = env();
        let r = create_backup(&e.db, &e.archive, &e.target, &meta("t1")).unwrap();
        let pool = Archive::open(e.target.join("underlag")).unwrap();
        let first = &pool.list().unwrap()[0];
        let file = pool.path_for(&first.sha256).unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        fs::write(&file, b"manipulerat").unwrap();
        let fresh = e.root.join("ny");
        assert!(matches!(
            restore_into_empty(&r.dir, &e.target.join("underlag"), &fresh),
            Err(BackupError::Damaged(_))
        ));
        assert!(!fresh.join("bokis.db").exists());
    }
}
