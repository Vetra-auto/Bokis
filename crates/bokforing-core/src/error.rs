use thiserror::Error;

use crate::types::{PeriodStatus, VoucherStatus};

#[derive(Debug, Error)]
pub enum LedgerError {
    #[error("Verifikationen måste ha minst 2 rader")]
    TooFewLines,

    #[error("Debet och kredit balanserar inte (debet: {debit}, kredit: {credit})")]
    Unbalanced { debit: i64, credit: i64 },

    #[error("Rad {line} har varken debet eller kredit")]
    EmptyLine { line: usize },

    #[error("Rad {line} har både debet och kredit")]
    BothDebitAndCredit { line: usize },

    #[error("Verifikationsdatum {date} ligger inte inom perioden {period_start} - {period_end}")]
    DateOutsidePeriod {
        date: String,
        period_start: String,
        period_end: String,
    },

    #[error("Perioden är {status:?} — bokföring tillåts bara i öppna perioder")]
    PeriodNotOpen { status: PeriodStatus },

    #[error("Verifikationen har redan status {status:?} och kan inte bokföras")]
    AlreadyPosted { status: VoucherStatus },

    #[error("Konto {account} är inaktivt")]
    InactiveAccount { account: u16 },
}

#[derive(Debug, Error)]
pub enum CorrectionError {
    #[error("Kan bara rätta bokförda verifikationer")]
    NotPosted,

    #[error("Motivering krävs för rättelse")]
    MissingReason,
}

#[derive(Debug, Error)]
pub enum PeriodError {
    #[error("Övergång från {from:?} till {to:?} är inte tillåten")]
    InvalidTransition { from: PeriodStatus, to: PeriodStatus },

    #[error("Rollen saknar behörighet för denna periodövergång")]
    InsufficientRole,

    #[error("Låst period kan inte öppnas igen")]
    LockedPermanently,
}

#[derive(Debug, Error)]
pub enum HashChainError {
    #[error("Kedjan är bruten vid position {position}: förväntad hash {expected}, fick {actual}")]
    BrokenChain {
        position: i64,
        expected: String,
        actual: String,
    },

    #[error("Felaktig genesis-hash vid position 0")]
    InvalidGenesis,

    #[error("Tom kedja")]
    EmptyChain,
}
