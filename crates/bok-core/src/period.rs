//! Periodernas tillståndsmaskin.
//!
//! öppen → under avstämning → stängd → låst. En period under avstämning kan
//! öppnas igen, men en stängd eller låst period kan aldrig öppnas. Stängning
//! och låsning är signerade beslut, och bara en människa får fatta dem.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PeriodStatus {
    Open,
    UnderReview,
    Closed,
    Locked,
}

/// Vem som utför en handling. Agentens behörighet slutar vid förslag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ActorKind {
    Human,
    Agent,
    System,
}

impl ActorKind {
    pub fn as_db_str(self) -> &'static str {
        match self {
            ActorKind::Human => "manniska",
            ActorKind::Agent => "agent",
            ActorKind::System => "system",
        }
    }

    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "manniska" => Some(ActorKind::Human),
            "agent" => Some(ActorKind::Agent),
            "system" => Some(ActorKind::System),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PeriodError {
    #[error("en låst period kan inte ändras")]
    LockedPermanently,
    #[error("en stängd period kan inte öppnas igen")]
    ClosedCannotReopen,
    #[error("övergången från {from} till {to} är inte tillåten")]
    InvalidTransition {
        from: PeriodStatus,
        to: PeriodStatus,
    },
    #[error("bara en människa med nyckeln får ändra en periods status")]
    NotAllowedForActor,
    #[error("okänd periodstatus \"{0}\"")]
    UnknownStatus(String),
}

impl PeriodStatus {
    pub fn as_db_str(self) -> &'static str {
        match self {
            PeriodStatus::Open => "oppen",
            PeriodStatus::UnderReview => "under_avstamning",
            PeriodStatus::Closed => "stangd",
            PeriodStatus::Locked => "last",
        }
    }

    pub fn from_db_str(s: &str) -> Result<Self, PeriodError> {
        match s {
            "oppen" => Ok(PeriodStatus::Open),
            "under_avstamning" => Ok(PeriodStatus::UnderReview),
            "stangd" => Ok(PeriodStatus::Closed),
            "last" => Ok(PeriodStatus::Locked),
            other => Err(PeriodError::UnknownStatus(other.to_string())),
        }
    }

    pub fn allows_posting(self) -> bool {
        self == PeriodStatus::Open
    }

    /// Stängning och låsning är beslut som måste signeras med nyckeln.
    pub fn requires_signature(self) -> bool {
        matches!(self, PeriodStatus::Closed | PeriodStatus::Locked)
    }
}

impl fmt::Display for PeriodStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PeriodStatus::Open => "öppen",
            PeriodStatus::UnderReview => "under avstämning",
            PeriodStatus::Closed => "stängd",
            PeriodStatus::Locked => "låst",
        })
    }
}

impl FromStr for PeriodStatus {
    type Err = PeriodError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        PeriodStatus::from_db_str(s)
    }
}

/// Kontrollerar en statusövergång. Samma regler finns som trigger i
/// databasen, så de gäller även för skrivningar förbi programmet.
pub fn check_transition(
    from: PeriodStatus,
    to: PeriodStatus,
    actor: ActorKind,
) -> Result<(), PeriodError> {
    if actor != ActorKind::Human {
        return Err(PeriodError::NotAllowedForActor);
    }
    match (from, to) {
        (PeriodStatus::Locked, _) => Err(PeriodError::LockedPermanently),
        (PeriodStatus::Closed, PeriodStatus::Open | PeriodStatus::UnderReview) => {
            Err(PeriodError::ClosedCannotReopen)
        }
        (PeriodStatus::Open, PeriodStatus::UnderReview)
        | (PeriodStatus::UnderReview, PeriodStatus::Open)
        | (PeriodStatus::UnderReview, PeriodStatus::Closed)
        | (PeriodStatus::Closed, PeriodStatus::Locked) => Ok(()),
        _ => Err(PeriodError::InvalidTransition { from, to }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ActorKind::*;
    use PeriodStatus::*;

    #[test]
    fn allowed_transitions() {
        assert!(check_transition(Open, UnderReview, Human).is_ok());
        assert!(check_transition(UnderReview, Open, Human).is_ok());
        assert!(check_transition(UnderReview, Closed, Human).is_ok());
        assert!(check_transition(Closed, Locked, Human).is_ok());
    }

    #[test]
    fn closed_and_locked_never_reopen() {
        assert_eq!(
            check_transition(Closed, Open, Human),
            Err(PeriodError::ClosedCannotReopen)
        );
        assert_eq!(
            check_transition(Closed, UnderReview, Human),
            Err(PeriodError::ClosedCannotReopen)
        );
        for to in [Open, UnderReview, Closed, Locked] {
            assert_eq!(
                check_transition(Locked, to, Human),
                Err(PeriodError::LockedPermanently)
            );
        }
    }

    #[test]
    fn skipping_steps_is_refused() {
        assert!(matches!(
            check_transition(Open, Closed, Human),
            Err(PeriodError::InvalidTransition { .. })
        ));
        assert!(matches!(
            check_transition(Open, Locked, Human),
            Err(PeriodError::InvalidTransition { .. })
        ));
        assert!(matches!(
            check_transition(Open, Open, Human),
            Err(PeriodError::InvalidTransition { .. })
        ));
    }

    #[test]
    fn agent_cannot_change_periods() {
        assert_eq!(
            check_transition(Open, UnderReview, Agent),
            Err(PeriodError::NotAllowedForActor)
        );
        assert_eq!(
            check_transition(UnderReview, Closed, System),
            Err(PeriodError::NotAllowedForActor)
        );
    }

    #[test]
    fn only_open_allows_posting_and_closing_is_signed() {
        assert!(Open.allows_posting());
        assert!(
            !UnderReview.allows_posting() && !Closed.allows_posting() && !Locked.allows_posting()
        );
        assert!(Closed.requires_signature() && Locked.requires_signature());
        assert!(!Open.requires_signature() && !UnderReview.requires_signature());
        for s in [Open, UnderReview, Closed, Locked] {
            assert_eq!(PeriodStatus::from_db_str(s.as_db_str()), Ok(s));
        }
    }
}
