//! Ett inventarium i registret: vad det är, när det skaffades, vad det
//! kostade, hur länge det ska skrivas av och på vilka konton.

use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use bok_core::{AccountNumber, Money};

use crate::disposal::Disposal;

/// Längsta nyttjandeperiod som registret godtar, i månader (50 år).
pub const MAX_LIFE_MONTHS: u32 = 600;

/// Fel i uppgifterna om ett inventarium.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AssetError {
    /// Beskrivningen är tom.
    #[error("inventariet saknar beskrivning")]
    NoDescription,
    /// Anskaffningsvärdet är noll eller negativt.
    #[error("anskaffningsvärdet måste vara större än noll, inte {0}")]
    NonPositiveCost(Money),
    /// Restvärdet är negativt eller minst lika stort som anskaffningsvärdet.
    #[error("restvärdet {residual} måste vara minst 0 och mindre än anskaffningsvärdet {cost}")]
    BadResidual {
        /// Restvärdet.
        residual: Money,
        /// Anskaffningsvärdet.
        cost: Money,
    },
    /// Nyttjandeperioden är noll eller orimligt lång.
    #[error("nyttjandeperioden måste vara 1–{MAX_LIFE_MONTHS} månader, inte {0}")]
    BadLife(u32),
    /// Avskrivningen börjar före anskaffningen.
    #[error(
        "avskrivningen kan inte börja ({start}) före månaden då inventariet skaffades ({acquired})"
    )]
    StartsBeforeAcquisition {
        /// Avskrivningens första dag.
        start: NaiveDate,
        /// Anskaffningsdagen.
        acquired: NaiveDate,
    },
    /// Avskrivningen börjar inte på en månads första dag.
    #[error("avskrivningen börjar på en månads första dag, inte {0}")]
    StartNotFirstOfMonth(NaiveDate),
    /// Ett konto är av fel slag.
    #[error("{what} ska vara ett {kind}, inte {account}")]
    WrongAccountKind {
        /// Kontots roll, till exempel "tillgångskontot".
        what: &'static str,
        /// "balanskonto" eller "resultatkonto".
        kind: &'static str,
        /// Kontot som angavs.
        account: AccountNumber,
    },
    /// Två av kontona är samma.
    #[error("tillgångskontot och kontot för ackumulerade avskrivningar måste vara olika")]
    SameAccounts,
    /// Avgången är daterad före anskaffningen.
    #[error("avgången {0} är före anskaffningen")]
    DisposalBeforeAcquisition(NaiveDate),
    /// Ett belopp blev för stort.
    #[error("beloppet blev för stort")]
    Overflow,
}

/// Kontona ett inventarium bokförs på.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetAccounts {
    /// Tillgången, till exempel 1220 Inventarier och verktyg.
    pub asset: AccountNumber,
    /// Ackumulerade avskrivningar, till exempel 1229.
    pub accumulated: AccountNumber,
    /// Kostnaden för avskrivningen, till exempel 7832.
    pub expense: AccountNumber,
}

impl AssetAccounts {
    /// Kontrollerar att tillgång och ackumulerade avskrivningar är
    /// balanskonton och kostnaden ett resultatkonto.
    pub fn validate(&self) -> Result<(), AssetError> {
        let balance = |what, account: AccountNumber| {
            if account.is_balance_account() {
                Ok(())
            } else {
                Err(AssetError::WrongAccountKind {
                    what,
                    kind: "balanskonto",
                    account,
                })
            }
        };
        balance("tillgångskontot", self.asset)?;
        balance("kontot för ackumulerade avskrivningar", self.accumulated)?;
        if !self.expense.is_result_account() {
            return Err(AssetError::WrongAccountKind {
                what: "kostnadskontot",
                kind: "resultatkonto",
                account: self.expense,
            });
        }
        if self.asset == self.accumulated {
            return Err(AssetError::SameAccounts);
        }
        Ok(())
    }
}

/// Uppgifterna när ett inventarium registreras.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewAsset {
    /// Vad inventariet är, till exempel "Bärbar dator".
    pub description: String,
    /// Anskaffningsdagen (leveransen eller fakturan).
    pub acquired: NaiveDate,
    /// Anskaffningsvärdet, utan moms som får dras av.
    pub cost: Money,
    /// Nyttjandeperioden i månader.
    pub life_months: u32,
    /// Första dagen i den första månaden som skrivs av, normalt
    /// anskaffningsmånaden (eller månaden då inventariet togs i bruk).
    pub depreciation_start: NaiveDate,
    /// Värdet som inte skrivs av, normalt 0.
    pub residual: Money,
    /// Kontona.
    pub accounts: AssetAccounts,
}

impl NewAsset {
    /// Första dagen i anskaffningsmånaden, som är standard för när
    /// avskrivningen börjar.
    pub fn default_start(acquired: NaiveDate) -> NaiveDate {
        acquired.with_day(1).unwrap_or(acquired)
    }

    /// Kontrollerar uppgifterna.
    pub fn validate(&self) -> Result<(), AssetError> {
        if self.description.trim().is_empty() {
            return Err(AssetError::NoDescription);
        }
        if self.cost.ore() <= 0 {
            return Err(AssetError::NonPositiveCost(self.cost));
        }
        if self.residual.is_negative() || self.residual >= self.cost {
            return Err(AssetError::BadResidual {
                residual: self.residual,
                cost: self.cost,
            });
        }
        if self.life_months == 0 || self.life_months > MAX_LIFE_MONTHS {
            return Err(AssetError::BadLife(self.life_months));
        }
        if self.depreciation_start.day() != 1 {
            return Err(AssetError::StartNotFirstOfMonth(self.depreciation_start));
        }
        if self.depreciation_start < Self::default_start(self.acquired) {
            return Err(AssetError::StartsBeforeAcquisition {
                start: self.depreciation_start,
                acquired: self.acquired,
            });
        }
        self.accounts.validate()
    }

    /// Beloppet som skrivs av under hela nyttjandeperioden.
    pub fn depreciable(&self) -> Money {
        Money::from_ore(self.cost.ore() - self.residual.ore())
    }
}

/// Ett registrerat inventarium, med sin avgång om det har lämnat företaget.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asset {
    /// Numret i registret.
    pub id: i64,
    /// Uppgifterna från registreringen.
    #[serde(flatten)]
    pub details: NewAsset,
    /// Verifikationen som anskaffningen bokfördes på, till exempel "A12 (2026)".
    pub voucher: Option<String>,
    /// Försäljning, utrangering eller felregistrering.
    pub disposal: Option<Disposal>,
}

impl Asset {
    /// Sant om inventariet är struket ur registret som felregistrerat. Det
    /// har då aldrig funnits och skrivs inte av.
    pub fn is_void(&self) -> bool {
        self.disposal.as_ref().is_some_and(Disposal::is_void)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    use crate::testdata::{acc, laptop};

    #[test]
    fn a_normal_asset_is_valid() {
        assert_eq!(laptop().validate(), Ok(()));
        assert_eq!(
            NewAsset::default_start(laptop().acquired),
            laptop().depreciation_start
        );
    }

    #[test]
    fn bad_values_are_refused() {
        let mut a = laptop();
        a.cost = Money::ZERO;
        assert!(matches!(a.validate(), Err(AssetError::NonPositiveCost(_))));
        let mut a = laptop();
        a.residual = a.cost;
        assert!(matches!(a.validate(), Err(AssetError::BadResidual { .. })));
        let mut a = laptop();
        a.life_months = 0;
        assert_eq!(a.validate(), Err(AssetError::BadLife(0)));
        let mut a = laptop();
        a.depreciation_start = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        assert!(matches!(
            a.validate(),
            Err(AssetError::StartsBeforeAcquisition { .. })
        ));
        let mut a = laptop();
        a.depreciation_start = NaiveDate::from_ymd_opt(2026, 6, 2).unwrap();
        assert!(matches!(
            a.validate(),
            Err(AssetError::StartNotFirstOfMonth(_))
        ));
        let mut a = laptop();
        a.description = "  ".into();
        assert_eq!(a.validate(), Err(AssetError::NoDescription));
    }

    #[test]
    fn accounts_must_be_of_the_right_kind() {
        let mut a = laptop();
        a.accounts.expense = acc(1930);
        assert!(matches!(
            a.validate(),
            Err(AssetError::WrongAccountKind { .. })
        ));
        let mut a = laptop();
        a.accounts.asset = acc(7832);
        assert!(matches!(
            a.validate(),
            Err(AssetError::WrongAccountKind { .. })
        ));
        let mut a = laptop();
        a.accounts.accumulated = acc(1220);
        assert_eq!(a.validate(), Err(AssetError::SameAccounts));
    }
}
