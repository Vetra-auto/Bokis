//! Underlagsarkivet, genom `bok-archive`: varje fil sparas byte för byte
//! under sin SHA-256 i `<databasens katalog>/underlag/<ab>/<hash>`, görs
//! skrivskyddad och kontrolleras mot hashen varje gång den läses.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use bok_archive::ArchiveError;

pub struct Archive {
    root: PathBuf,
}

/// Utfallet av en kontroll av en arkiverad fil.
pub enum FileCheck {
    Ok,
    Missing,
    Changed,
}

impl Archive {
    /// Arkivet ligger bredvid databasen, i katalogen `underlag`.
    pub fn beside(db: &Path) -> Archive {
        let dir = db.parent().map(Path::to_path_buf).unwrap_or_default();
        Archive {
            root: dir.join("underlag"),
        }
    }

    pub fn open(&self) -> Result<bok_archive::Archive> {
        bok_archive::Archive::open(&self.root)
            .with_context(|| format!("kan inte öppna underlagsarkivet {}", self.root.display()))
    }

    /// Sparar filen och ger dess SHA-256. Finns samma innehåll redan
    /// kontrolleras den befintliga filen i stället.
    pub fn store(&self, bytes: &[u8]) -> Result<String> {
        Ok(self.open()?.put_bytes(bytes)?.sha256)
    }

    pub fn check(&self, sha256: &str) -> Result<FileCheck> {
        match self.open()?.read(sha256) {
            Ok(_) => Ok(FileCheck::Ok),
            Err(ArchiveError::Missing(_)) => Ok(FileCheck::Missing),
            Err(ArchiveError::Changed { .. }) => Ok(FileCheck::Changed),
            Err(e) => Err(e.into()),
        }
    }

    /// Antal poster i arkivet och de som inte hör hemma där: främmande filer
    /// och rester efter avbrutna skrivningar.
    pub fn unknown_files(&self) -> Result<(usize, Vec<PathBuf>)> {
        let r = self.open()?.verify_all()?;
        Ok((r.ok + r.changed.len() + r.unknown.len(), r.unknown))
    }
}
