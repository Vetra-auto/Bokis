//! Inläsning av underlag från en bevakad mapp.
//!
//! Arkiveringsplikten gäller från mottagandet (BFL 7 kap. 2 §). Därför läggs
//! varje fil i underlagsarkivet byte för byte så snart den hittas, innan någon
//! verifikation finns. Filtypen bestäms utifrån innehållet och bara i andra
//! hand utifrån filändelsen, eftersom agenten och granskningsvyn behöver veta
//! vad de visar.
//!
//! Mappen kan vara en synkad katalog från telefonen, en nedladdningsmapp eller
//! en katalog dit mejlklienten sparar bilagor och `.eml`-filer.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use bok_archive::Archive;

/// Fel vid inläsning.
#[derive(Debug, thiserror::Error)]
pub enum IntakeError {
    #[error("I/O-fel för {0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("arkivfel: {0}")]
    Archive(#[from] bok_archive::ArchiveError),
}

fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> IntakeError + '_ {
    move |e| IntakeError::Io(path.to_path_buf(), e)
}

/// En fil som har lästs in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Received {
    pub sha256: String,
    pub size: u64,
    /// Filnamnet i mappen, som originalnamn.
    pub file_name: String,
    pub media_type: String,
    /// `false` om samma innehåll redan fanns i arkivet (en dubblett).
    pub is_new: bool,
    /// Var filen ligger efter inläsningen.
    pub path_after: PathBuf,
}

/// Inställningar för en genomsökning.
#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// Filer som ändrats senare än så här hoppas över; de kan skrivas fortfarande.
    pub min_age: Duration,
    /// Om satt flyttas inlästa filer hit, så att de inte läses igen.
    pub done_dir: Option<PathBuf>,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            min_age: Duration::from_secs(2),
            done_dir: None,
        }
    }
}

/// Filnamn som hoppas över: dolda filer och filer som håller på att laddas ner.
fn should_skip(name: &str) -> bool {
    let lower = name.to_lowercase();
    name.starts_with('.')
        || name.ends_with('~')
        || [".part", ".crdownload", ".tmp", ".partial", ".download"]
            .iter()
            .any(|suffix| lower.ends_with(suffix))
}

/// Bestämmer filtyp utifrån innehållet, med filändelsen som reserv. Finns i
/// arkivet, eftersom även MCP-servern och granskningsvyn behöver den.
pub use bok_archive::media::detect as detect_media_type;

/// `true` om innehållet ser ut som ett camt.053-kontoutdrag.
pub fn looks_like_camt053(data: &[u8]) -> bool {
    let head = &data[..data.len().min(4096)];
    let text = String::from_utf8_lossy(head);
    text.contains("camt.053") || text.contains("BkToCstmrStmt")
}

fn free_name(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };
    (2..)
        .map(|n| dir.join(format!("{stem}-{n}{ext}")))
        .find(|p| !p.exists())
        .unwrap_or(candidate)
}

/// Flyttar en fil till `dir` under ett ledigt namn (`faktura-2.pdf` om
/// `faktura.pdf` redan finns) och ger den nya sökvägen. Används när filen är
/// både arkiverad och registrerad i bokföringen.
pub fn move_to(path: &Path, dir: &Path) -> Result<PathBuf, IntakeError> {
    fs::create_dir_all(dir).map_err(io_err(dir))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let target = free_name(dir, &name);
    fs::rename(path, &target).map_err(io_err(path))?;
    Ok(target)
}

/// Läser in alla färdiga filer i `folder`, i namnordning.
pub fn scan_folder(
    folder: &Path,
    archive: &Archive,
    options: &ScanOptions,
) -> Result<Vec<Received>, IntakeError> {
    let mut files: Vec<PathBuf> = fs::read_dir(folder)
        .map_err(io_err(folder))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    files.sort();

    if let Some(done) = &options.done_dir {
        fs::create_dir_all(done).map_err(io_err(done))?;
    }

    let now = SystemTime::now();
    let mut received = Vec::new();
    for path in files {
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if should_skip(&file_name) {
            continue;
        }
        let modified = fs::metadata(&path)
            .and_then(|m| m.modified())
            .map_err(io_err(&path))?;
        let age = now.duration_since(modified).unwrap_or(Duration::ZERO);
        if age < options.min_age {
            continue;
        }

        let stored = archive.put_file(&path)?;
        let head = {
            use std::io::Read;
            let mut buf = vec![0u8; 8192];
            let mut f = fs::File::open(&stored.path).map_err(io_err(&stored.path))?;
            let n = f.read(&mut buf).map_err(io_err(&stored.path))?;
            buf.truncate(n);
            buf
        };
        let media_type = detect_media_type(&head, &file_name);

        let path_after = match &options.done_dir {
            Some(done) => {
                let target = free_name(done, &file_name);
                fs::rename(&path, &target).map_err(io_err(&path))?;
                target
            }
            None => path.clone(),
        };

        received.push(Received {
            sha256: stored.sha256,
            size: stored.size,
            file_name,
            media_type,
            is_new: stored.is_new,
            path_after,
        });
    }
    Ok(received)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn media_types_from_content() {
        assert_eq!(
            detect_media_type(b"%PDF-1.7 ...", "kvitto.jpg"),
            "application/pdf"
        );
        assert_eq!(
            detect_media_type(b"\x89PNG\r\n\x1a\nrest", "x"),
            "image/png"
        );
        assert_eq!(
            detect_media_type(&[0xff, 0xd8, 0xff, 0xe0, 0, 0], "foto"),
            "image/jpeg"
        );
        assert_eq!(
            detect_media_type(b"\0\0\0\x18ftypheic\0\0\0\0", "IMG_0001.HEIC"),
            "image/heic"
        );
        assert_eq!(
            detect_media_type(b"RIFF\0\0\0\0WEBPVP8 ", "a.webp"),
            "image/webp"
        );
        assert_eq!(
            detect_media_type(b"\xef\xbb\xbf<?xml version=\"1.0\"?><Invoice/>", "f.xml"),
            "application/xml"
        );
        assert_eq!(
            detect_media_type(b"Return-Path: <a@b.se>\r\nFrom: x\r\n", "meddelande"),
            "message/rfc822"
        );
        assert_eq!(
            detect_media_type(b"Datum;Text;Belopp\n", "konto.csv"),
            "text/csv"
        );
        assert_eq!(
            detect_media_type(b"PK\x03\x04...", "avtal.docx"),
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        );
        assert_eq!(
            detect_media_type("Kvitto från Café Öst".as_bytes(), "kvitto.txt"),
            "text/plain"
        );
        assert_eq!(
            detect_media_type(&[0, 1, 2, 3, 255], "blob.bin"),
            "application/octet-stream"
        );
    }

    #[test]
    fn move_to_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let done = dir.path().join("klart");
        std::fs::create_dir_all(&done).unwrap();
        std::fs::write(done.join("kvitto.pdf"), b"gammal").unwrap();
        let source = dir.path().join("kvitto.pdf");
        std::fs::write(&source, b"ny").unwrap();
        let moved = move_to(&source, &done).unwrap();
        assert_eq!(moved, done.join("kvitto-2.pdf"));
        assert_eq!(std::fs::read(done.join("kvitto.pdf")).unwrap(), b"gammal");
        assert_eq!(std::fs::read(&moved).unwrap(), b"ny");
        assert!(!source.exists());
    }

    #[test]
    fn camt_is_recognised() {
        assert!(looks_like_camt053(b"<?xml version=\"1.0\"?><Document xmlns=\"urn:iso:std:iso:20022:tech:xsd:camt.053.001.02\">"));
        assert!(!looks_like_camt053(b"<?xml version=\"1.0\"?><Invoice/>"));
    }

    fn setup() -> (tempfile::TempDir, PathBuf, Archive) {
        let dir = tempfile::tempdir().unwrap();
        let inbox = dir.path().join("inkorg");
        fs::create_dir_all(&inbox).unwrap();
        let archive = Archive::open(dir.path().join("arkiv")).unwrap();
        (dir, inbox, archive)
    }

    fn now() -> ScanOptions {
        ScanOptions {
            min_age: Duration::ZERO,
            done_dir: None,
        }
    }

    #[test]
    fn scan_archives_files_and_skips_partial_and_hidden() {
        let (_dir, inbox, archive) = setup();
        fs::write(inbox.join("faktura.pdf"), b"%PDF-1.4 faktura").unwrap();
        fs::write(inbox.join("kvitto.png"), b"\x89PNG\r\n\x1a\nbild").unwrap();
        fs::write(inbox.join(".DS_Store"), b"skrap").unwrap();
        fs::write(inbox.join("stor.pdf.part"), b"%PDF-halv").unwrap();
        fs::write(inbox.join("utkast.txt~"), b"backup").unwrap();
        fs::create_dir_all(inbox.join("undermapp")).unwrap();

        let received = scan_folder(&inbox, &archive, &now()).unwrap();
        let names: Vec<&str> = received.iter().map(|r| r.file_name.as_str()).collect();
        assert_eq!(names, ["faktura.pdf", "kvitto.png"]);
        assert_eq!(received[0].media_type, "application/pdf");
        assert_eq!(received[1].media_type, "image/png");
        assert!(received
            .iter()
            .all(|r| r.is_new && archive.contains(&r.sha256)));
    }

    #[test]
    fn fresh_files_wait_until_they_are_finished() {
        let (_dir, inbox, archive) = setup();
        fs::write(inbox.join("nyss.pdf"), b"%PDF-1.4").unwrap();
        let options = ScanOptions {
            min_age: Duration::from_secs(3600),
            done_dir: None,
        };
        assert!(scan_folder(&inbox, &archive, &options).unwrap().is_empty());
        assert!(archive.list().unwrap().is_empty());
    }

    #[test]
    fn done_files_are_moved_and_duplicates_are_marked() {
        let (dir, inbox, archive) = setup();
        let done = dir.path().join("inlast");
        let options = ScanOptions {
            min_age: Duration::ZERO,
            done_dir: Some(done.clone()),
        };

        fs::write(inbox.join("kvitto.jpg"), [0xff, 0xd8, 0xff, 1, 2, 3]).unwrap();
        let first = scan_folder(&inbox, &archive, &options).unwrap();
        assert_eq!(first.len(), 1);
        assert!(first[0].is_new);
        assert!(!inbox.join("kvitto.jpg").exists());
        assert!(done.join("kvitto.jpg").exists());

        // Samma kvitto igen, med samma namn: dubblett, och namnet krockar inte.
        fs::write(inbox.join("kvitto.jpg"), [0xff, 0xd8, 0xff, 1, 2, 3]).unwrap();
        let second = scan_folder(&inbox, &archive, &options).unwrap();
        assert!(!second[0].is_new);
        assert_eq!(second[0].sha256, first[0].sha256);
        assert!(done.join("kvitto-2.jpg").exists());
        assert_eq!(archive.list().unwrap().len(), 1);
    }
}
