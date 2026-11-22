//! Domänregler för Bokis AI-first.
//!
//! Biblioteket gör ingen I/O. Det äger reglerna som resten av systemet
//! lutar sig mot: belopp i öre, kontonummer, serier, periodernas
//! tillståndsmaskin, räkenskapsårets perioder, validering av verifikationer,
//! rättelser, den kanoniska formen med sin SHA-256, hashkedjorna och
//! kontrollen av ed25519-signaturer.
//!
//! Logiken är flyttad från `bokforing-core` (belopp, balans, perioder,
//! hashkedja, rättelser, periodgenerering) och utbyggd för AI-first-designen
//! i "Bokis AI-first – arkitektur".

pub mod account;
pub mod canonical;
pub mod chain;
pub mod correction;
pub mod decision;
pub mod fiscal_year;
pub mod id_number;
pub mod money;
pub mod period;
pub mod security;
pub mod series;
pub mod signature;
pub mod voucher;

pub use account::AccountNumber;
pub use id_number::{IdKind, IdNumber, IdNumberError};
pub use money::Money;
pub use period::PeriodStatus;
pub use series::{Series, VoucherRef};
