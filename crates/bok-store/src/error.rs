use std::path::PathBuf;

use chrono::NaiveDate;
use thiserror::Error;

use bok_core::account::AccountError;
use bok_core::correction::CorrectionError;
use bok_core::fiscal_year::FiscalYearError;
use bok_core::money::MoneyError;
use bok_core::period::PeriodError;
use bok_core::series::SeriesError;
use bok_core::signature::SignatureError;
use bok_core::voucher::VoucherError;
use bok_core::{AccountNumber, PeriodStatus};

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("{0} finns redan; bok skriver aldrig över en befintlig databas")]
    AlreadyExists(PathBuf),
    #[error("hittar ingen databas på {0}")]
    NotFound(PathBuf),
    #[error("{0} är ingen Bokis-databas")]
    NotABokisDatabase(PathBuf),
    #[error(
        "databasen har schemaversion {found}, programmet känner bara till {known}; uppdatera programmet"
    )]
    NewerSchema { found: i64, known: i64 },
    #[error("databasfel: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("ogiltig JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Voucher(#[from] VoucherError),
    #[error(transparent)]
    Period(#[from] PeriodError),
    #[error(transparent)]
    FiscalYear(#[from] FiscalYearError),
    #[error(transparent)]
    Correction(#[from] CorrectionError),
    #[error(transparent)]
    Signature(#[from] SignatureError),
    #[error(transparent)]
    SecurityLevel(#[from] bok_core::security::LevelError),
    #[error(transparent)]
    Money(#[from] MoneyError),
    #[error(transparent)]
    Account(#[from] AccountError),
    #[error(transparent)]
    Series(#[from] SeriesError),
    #[error("{what} {id} finns inte")]
    Missing { what: &'static str, id: String },
    #[error("förslag {0} har redan ett beslut")]
    AlreadyDecided(i64),
    #[error("perioden {period} är {status} och tar inte emot poster")]
    PeriodNotOpen {
        period: String,
        status: PeriodStatus,
    },
    #[error("det finns ingen period för {0}; lägg till räkenskapsåret först")]
    NoPeriod(NaiveDate),
    #[error("konto {0} finns inte i kontoplanen per {1}")]
    UnknownAccount(AccountNumber, NaiveDate),
    #[error("konto {0} är inte aktivt per {1}")]
    InactiveAccount(AccountNumber, NaiveDate),
    #[error("nyckeln {0} är inte registrerad")]
    UnknownKey(String),
    #[error("bara en människa med nyckeln får {0}")]
    NotAllowed(&'static str),
    #[error("filen finns redan som underlag {0}")]
    DuplicateAttachment(i64),
    #[error(
        "perioden {period} har {count} bankrader som inte är avstämda; matcha eller förklara dem först"
    )]
    UnreconciledBankRows { period: String, count: i64 },
    #[error(
        "klockan visar {now}, men den senaste händelsen i historiken är från {last}; ställ klockan rätt innan något skrivs"
    )]
    ClockBehind { now: String, last: String },
    #[error("{0}")]
    Invalid(String),
}

impl StoreError {
    pub(crate) fn missing(what: &'static str, id: impl ToString) -> StoreError {
        StoreError::Missing {
            what,
            id: id.to_string(),
        }
    }
}
