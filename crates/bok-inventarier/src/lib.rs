//! Inventarieregistret i Bokis, som ren domänlogik utan I/O.
//!
//! Ett inventarium har ett anskaffningsvärde, en nyttjandeperiod och tre
//! konton: tillgången (till exempel 1220), de ackumulerade avskrivningarna
//! (1229) och kostnaden för avskrivningen (7832). Avskrivningen är linjär
//! enligt plan, en lika stor del varje månad från och med månaden då
//! inventariet tas i bruk, som K2 (BFNAR 2016:10) föreskriver.
//!
//! Biblioteket räknar ut verifikationsrader men bokför ingenting. Raderna
//! blir förslag som en människa granskar och signerar, precis som allt annat
//! i Bokis. Själva registret lagras av `bok-store` (schemaversion 4), där det
//! bara kan växa.
//!
//! - [`asset`] – ett inventarium och dess konton, med kontroll av uppgifterna.
//! - [`plan`] – avskrivningsplanen – ackumulerat belopp, bokfört värde, årets del.
//! - [`disposal`] – försäljning, utrangering och felregistrering.
//! - [`year_end`] – årets avskrivningar för hela registret som verifikationsrader.
//! - [`threshold`] – halva prisbasbeloppet och treårsregeln för direktavdrag.

pub mod asset;
pub mod disposal;
pub mod plan;
pub mod threshold;
pub mod year_end;

#[cfg(test)]
mod testdata;

pub use asset::{Asset, AssetAccounts, AssetError, NewAsset};
pub use disposal::{Disposal, DisposalAccounts, DisposalKind, DisposalVoucher, disposal_voucher};
pub use plan::Plan;
pub use threshold::{DirectExpense, direct_expense, half_price_base_amount, price_base_amount};
pub use year_end::{DepreciationRow, depreciation_lines, register_balances, year_depreciation};
