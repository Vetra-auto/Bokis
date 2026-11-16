//! Lagringen för Bokis AI-first: SQLite med STRICT-tabeller och triggers.
//!
//! Allt som är räkenskapsinformation kan bara växa: triggers i databasen
//! stoppar UPDATE och DELETE på bokförda verifikationer, rader, underlag,
//! beslut, rättelser och historiken. Varje skrivning sker i en egen
//! transaktion (`BEGIN IMMEDIATE`) som också lägger en hashkedjad händelse i
//! `handelser`. Ett förslag har inget nummer; numret tas ur räknaren i
//! `nummerserier` i samma transaktion som verifikationen skrivs, så ett
//! avvisat förslag förbrukar inget nummer.
//!
//! Enligt arkitekturen ska bara `bokd` använda det här biblioteket. I
//! milstolpe 1 finns ingen `bokd` än, och CLI:t `bok` anropar biblioteket
//! direkt.
//!
//! Ingen funktion här raderar, nollställer eller återställer en databas.
//! [`Store::create`] vägrar om filen redan finns.

mod error;
mod guards;
mod model;
mod ops;
mod reports;
mod vat;
mod verify;
mod write;

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, NaiveDate, Utc};
use rusqlite::config::DbConfig;
use rusqlite::{Connection, OpenFlags};

pub use bok_core;
pub use bok_inventarier;
pub use error::StoreError;
pub use guards::{APPEND_ONLY_TABLES, Guard};
pub use model::*;
pub use ops::assets::{ASSET_DISPOSED, ASSET_REGISTERED};
pub use ops::security::SECURITY_LEVEL_DECIDED;
pub use vat::{VAT_BOXES, VatBox, VatReport, box_title};
pub use verify::{Check, VerifyReport};
pub use write::Actor;

/// Schemaversionen som programmet skriver och förstår.
pub const SCHEMA_VERSION: i64 = 4;

/// Migreringarna i ordning. En publicerad migrering ändras aldrig; nya
/// tabeller och kolumner kommer i nästa version.
const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("schema_v1.sql")),
    // Bankrader, avstämning och matchningsförslag (steg 4).
    (2, include_str!("schema_v2.sql")),
    // Säkerhetsnivån: vilka signaturer bokföringen godtar.
    (3, include_str!("schema_v3.sql")),
    // Inventarieregistret: anskaffningar och avgångar.
    (4, include_str!("schema_v4.sql")),
];

/// Klocka som ger tiden i UTC. Kan bytas ut i tester.
pub type Clock = Box<dyn Fn() -> DateTime<Utc> + Send>;

pub type Result<T> = std::result::Result<T, StoreError>;

/// En öppen bokföringsdatabas.
pub struct Store {
    conn: Connection,
    path: PathBuf,
    clock: Clock,
    program_version: String,
}

/// Uppgifterna som behövs för att skapa en ny databas.
pub struct NewBook<'a> {
    pub company_name: &'a str,
    pub org_number: &'a str,
    /// Den som äger nyckeln, till exempel "henrik".
    pub key_name: &'a str,
    /// Den publika nyckeln i OpenSSH-format.
    pub public_key: &'a str,
    /// Nyckeln signerar sin egen registrering, vilket visar att den fungerar.
    pub signer: &'a dyn bok_core::signature::Signer,
    /// Vilka signaturer bokföringen ska godta. Nyckeln signerar också det
    /// beslutet och måste själv klara nivån.
    pub security_level: bok_core::security::SecurityLevel,
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

impl Store {
    /// Skapar en ny, tom bokföringsdatabas. Vägrar om filen eller dess
    /// WAL-filer redan finns: programmet skriver aldrig över en databas.
    pub fn create(path: &Path, program_version: &str, book: NewBook<'_>) -> Result<Store> {
        Self::create_with_clock(path, program_version, book, Box::new(Utc::now))
    }

    pub fn create_with_clock(
        path: &Path,
        program_version: &str,
        book: NewBook<'_>,
        clock: Clock,
    ) -> Result<Store> {
        for p in [
            sidecar(path, "-wal"),
            sidecar(path, "-shm"),
            sidecar(path, "-journal"),
        ] {
            if p.exists() {
                return Err(StoreError::AlreadyExists(p));
            }
        }
        // create_new skapar filen atomiskt och misslyckas om den finns.
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(path) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(StoreError::AlreadyExists(path.to_path_buf()));
            }
            Err(e) => return Err(e.into()),
        }
        let result = (|| {
            let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
            let mut store = Store {
                conn,
                path: path.to_path_buf(),
                clock,
                program_version: program_version.to_string(),
            };
            store.configure()?;
            store.migrate()?;
            store.initialize(book)?;
            Ok(store)
        })();
        if result.is_err() {
            // Filen skapades nyss av det här anropet och innehåller ingen
            // bokföring; ta bort den så att ett nytt försök kan göras.
            for p in [
                sidecar(path, "-wal"),
                sidecar(path, "-shm"),
                path.to_path_buf(),
            ] {
                let _ = std::fs::remove_file(p);
            }
        }
        result
    }

    /// Öppnar en befintlig databas. Skapar aldrig en ny.
    pub fn open(path: &Path, program_version: &str) -> Result<Store> {
        if !path.is_file() {
            return Err(StoreError::NotFound(path.to_path_buf()));
        }
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        // Versionen läses innan något skrivs, så att en fil som inte är en
        // Bokis-databas lämnas orörd.
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(|_| StoreError::NotABokisDatabase(path.to_path_buf()))?;
        if version == 0 {
            return Err(StoreError::NotABokisDatabase(path.to_path_buf()));
        }
        if version > SCHEMA_VERSION {
            return Err(StoreError::NewerSchema {
                found: version,
                known: SCHEMA_VERSION,
            });
        }
        let mut store = Store {
            conn,
            path: path.to_path_buf(),
            clock: Box::new(Utc::now),
            program_version: program_version.to_string(),
        };
        store.configure()?;
        store.migrate()?;
        Ok(store)
    }

    /// Byter klockan. Tester använder det för att styra registreringstid och
    /// "dagens datum".
    pub fn set_clock(&mut self, clock: Clock) {
        self.clock = clock;
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn program_version(&self) -> &str {
        &self.program_version
    }

    fn configure(&self) -> Result<()> {
        self.conn.pragma_update(None, "foreign_keys", true)?;
        self.conn.pragma_update(None, "recursive_triggers", true)?;
        self.conn.pragma_update(None, "journal_mode", "WAL")?;
        self.conn.pragma_update(None, "synchronous", "FULL")?;
        self.conn.busy_timeout(std::time::Duration::from_secs(10))?;
        self.conn
            .set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
        Ok(())
    }

    fn migrate(&mut self) -> Result<()> {
        let current: i64 = self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))?;
        for (version, sql) in MIGRATIONS {
            if *version <= current {
                continue;
            }
            let tx = self
                .conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", version)?;
            tx.commit()?;
        }
        Ok(())
    }

    pub(crate) fn now(&self) -> DateTime<Utc> {
        (self.clock)()
    }

    /// Dagens datum i lokal tid.
    pub fn today(&self) -> NaiveDate {
        local_date(self.now())
    }

    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }
}

/// Tidpunkt i UTC med mikrosekunder, i den form som lagras och hashas.
pub fn format_timestamp(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}

/// Läser en lagrad tidpunkt.
pub(crate) fn parse_timestamp(stored: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(stored)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|_| StoreError::Invalid(format!("tidpunkten {stored} går inte att läsa")))
}

pub fn local_date(t: DateTime<Utc>) -> NaiveDate {
    t.with_timezone(&Local).date_naive()
}

/// Lokal tid för visning, till exempel "2026-03-16 11:22:33".
pub fn display_timestamp(stored: &str) -> String {
    DateTime::parse_from_rfc3339(stored)
        .map(|t| {
            t.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|_| stored.to_string())
}

pub(crate) fn parse_date(s: &str) -> rusqlite::Result<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}
