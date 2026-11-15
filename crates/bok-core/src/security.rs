//! Säkerhetsnivån: vilka signaturer bokföringen godtar.
//!
//! Bokföringslagen kräver ingen viss nivå. Den kräver att det syns vem som
//! gjorde vad och när, och att bokföringen inte kan ändras i efterhand utan
//! att det märks. Nivån är ett val av hur svårt det ska vara för någon annan,
//! till exempel en agent på samma dator, att signera i ditt namn.
//!
//! Bokis kan bara kontrollera det som signaturen själv visar: nyckeltypen och
//! FIDO-flaggorna för beröring och PIN. En lösenfras på en nyckelfil syns inte
//! i signaturen och är därför ett råd, ingen nivå.

use std::fmt;
use std::str::FromStr;

use thiserror::Error;

use crate::signature::{SK_FLAG_USER_PRESENCE, SK_FLAG_USER_VERIFIED};

/// Nivåerna i stigande ordning: en högre nivå godtar färre signaturer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SecurityLevel {
    /// Varje registrerad nyckel, också en nyckelfil på datorn. Allt som kör
    /// som du kan signera.
    Software,
    /// Bara hårdvarunycklar (ed25519-sk), och varje signatur intygar att
    /// nyckeln rördes.
    Hardware,
    /// Som [`SecurityLevel::Hardware`], och varje signatur intygar dessutom att
    /// nyckelns PIN angavs.
    HardwarePin,
}

impl SecurityLevel {
    pub const ALL: [SecurityLevel; 3] = [Self::Software, Self::Hardware, Self::HardwarePin];

    /// Namnet i databasen och på kommandoraden.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Software => "mjukvara",
            Self::Hardware => "hardvara",
            Self::HardwarePin => "hardvara-pin",
        }
    }

    /// Namnet för människor.
    pub fn title(self) -> &'static str {
        match self {
            Self::Software => "mjukvarunyckel",
            Self::Hardware => "hårdvarunyckel",
            Self::HardwarePin => "hårdvarunyckel med PIN",
        }
    }

    /// Vad nivån innebär.
    pub fn meaning(self) -> &'static str {
        match self {
            Self::Software => {
                "Alla registrerade nycklar godtas, också en nyckelfil på datorn. Allt som kör \
                 som du kan signera."
            }
            Self::Hardware => {
                "Bara hårdvarunycklar godtas, och varje signatur kräver att någon rör nyckeln."
            }
            Self::HardwarePin => {
                "Bara hårdvarunycklar godtas, och varje signatur kräver både beröring och \
                 nyckelns PIN."
            }
        }
    }

    /// Kontrollerar en signatur mot nivån. `hardware` är nyckelns typ och
    /// `sk_flags` flaggorna i signaturen, `None` för en mjukvarunyckel (se
    /// [`crate::signature::sk_flags`]).
    pub fn check(self, hardware: bool, sk_flags: Option<u8>) -> Result<(), LevelError> {
        if self == Self::Software {
            return Ok(());
        }
        let Some(flags) = sk_flags.filter(|_| hardware) else {
            return Err(LevelError::SoftwareKey(self));
        };
        if flags & SK_FLAG_USER_PRESENCE == 0 {
            return Err(LevelError::NoTouch(self));
        }
        if self == Self::HardwarePin && flags & SK_FLAG_USER_VERIFIED == 0 {
            return Err(LevelError::NoPin);
        }
        Ok(())
    }

    /// Om en nyckel av den här typen alls kan signera på nivån.
    pub fn admits_key(self, hardware: bool) -> bool {
        self == Self::Software || hardware
    }
}

impl fmt::Display for SecurityLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SecurityLevel {
    type Err = LevelError;

    /// Tar emot namnen med eller utan å: `hardvara` och `hårdvara`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().replace('å', "a").as_str() {
            "mjukvara" => Ok(Self::Software),
            "hardvara" => Ok(Self::Hardware),
            "hardvara-pin" => Ok(Self::HardwarePin),
            _ => Err(LevelError::Unknown(s.trim().to_string())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LevelError {
    #[error("säkerhetsnivån {0:?} finns inte; välj mjukvara, hardvara eller hardvara-pin")]
    Unknown(String),
    #[error(
        "säkerhetsnivån är {}, men signaturen kommer från en mjukvarunyckel",
        .0.title()
    )]
    SoftwareKey(SecurityLevel),
    #[error("säkerhetsnivån är {}, men nyckeln intygade ingen beröring", .0.title())]
    NoTouch(SecurityLevel),
    #[error(
        "säkerhetsnivån är hårdvarunyckel med PIN, men nyckeln intygade ingen PIN; skapa \
         nyckeln med ssh-keygen -t ed25519-sk -O verify-required"
    )]
    NoPin,
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOUCH: u8 = SK_FLAG_USER_PRESENCE;
    const PIN: u8 = SK_FLAG_USER_PRESENCE | SK_FLAG_USER_VERIFIED;

    #[test]
    fn the_levels_are_ordered_and_named() {
        assert!(SecurityLevel::Software < SecurityLevel::Hardware);
        assert!(SecurityLevel::Hardware < SecurityLevel::HardwarePin);
        for level in SecurityLevel::ALL {
            assert_eq!(level.as_str().parse::<SecurityLevel>(), Ok(level));
        }
        assert_eq!("Hårdvara".parse(), Ok(SecurityLevel::Hardware));
        assert_eq!(" hårdvara-pin ".parse(), Ok(SecurityLevel::HardwarePin));
        assert!(matches!(
            "yubikey".parse::<SecurityLevel>(),
            Err(LevelError::Unknown(_))
        ));
    }

    #[test]
    fn each_level_accepts_only_its_signatures() {
        use SecurityLevel::*;
        // Mjukvara godtar allt som har en giltig signatur.
        assert_eq!(Software.check(false, None), Ok(()));
        assert_eq!(Software.check(true, Some(TOUCH)), Ok(()));
        // Hårdvara kräver en hårdvarunyckel som rördes.
        assert_eq!(
            Hardware.check(false, None),
            Err(LevelError::SoftwareKey(Hardware))
        );
        assert_eq!(
            Hardware.check(true, Some(0)),
            Err(LevelError::NoTouch(Hardware))
        );
        assert_eq!(Hardware.check(true, Some(TOUCH)), Ok(()));
        assert_eq!(Hardware.check(true, Some(PIN)), Ok(()));
        // Hårdvara med PIN kräver dessutom PIN-flaggan.
        assert_eq!(
            HardwarePin.check(false, None),
            Err(LevelError::SoftwareKey(HardwarePin))
        );
        assert_eq!(HardwarePin.check(true, Some(TOUCH)), Err(LevelError::NoPin));
        assert_eq!(HardwarePin.check(true, Some(PIN)), Ok(()));
        assert!(LevelError::NoPin.to_string().contains("verify-required"));
    }

    #[test]
    fn only_software_level_admits_a_key_file() {
        assert!(SecurityLevel::Software.admits_key(false));
        assert!(!SecurityLevel::Hardware.admits_key(false));
        assert!(SecurityLevel::HardwarePin.admits_key(true));
    }
}
