use crate::error::PeriodError;
use crate::types::{PeriodStatus, Role};

pub fn can_transition(from: PeriodStatus, to: PeriodStatus, role: &Role) -> Result<(), PeriodError> {
    if from == PeriodStatus::Locked {
        return Err(PeriodError::LockedPermanently);
    }

    let valid = matches!(
        (&from, &to),
        (PeriodStatus::Open, PeriodStatus::UnderReview)
            | (PeriodStatus::UnderReview, PeriodStatus::Closed)
            | (PeriodStatus::UnderReview, PeriodStatus::Open)
            | (PeriodStatus::Closed, PeriodStatus::Locked)
    );

    if !valid {
        return Err(PeriodError::InvalidTransition { from, to });
    }

    let has_role = match (&from, &to) {
        (PeriodStatus::Open, PeriodStatus::UnderReview) => {
            matches!(role, Role::Bokforare | Role::Admin)
        }
        (PeriodStatus::UnderReview, PeriodStatus::Closed) => matches!(role, Role::Admin),
        (PeriodStatus::UnderReview, PeriodStatus::Open) => matches!(role, Role::Admin),
        (PeriodStatus::Closed, PeriodStatus::Locked) => matches!(role, Role::Admin),
        _ => false,
    };

    if !has_role {
        return Err(PeriodError::InsufficientRole);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_transitions() {
        assert!(can_transition(PeriodStatus::Open, PeriodStatus::UnderReview, &Role::Bokforare).is_ok());
        assert!(can_transition(PeriodStatus::Open, PeriodStatus::UnderReview, &Role::Admin).is_ok());
        assert!(can_transition(PeriodStatus::UnderReview, PeriodStatus::Closed, &Role::Admin).is_ok());
        assert!(can_transition(PeriodStatus::UnderReview, PeriodStatus::Open, &Role::Admin).is_ok());
        assert!(can_transition(PeriodStatus::Closed, PeriodStatus::Locked, &Role::Admin).is_ok());
    }

    #[test]
    fn invalid_transitions() {
        assert!(can_transition(PeriodStatus::Open, PeriodStatus::Closed, &Role::Admin).is_err());
        assert!(can_transition(PeriodStatus::Open, PeriodStatus::Locked, &Role::Admin).is_err());
        assert!(can_transition(PeriodStatus::Closed, PeriodStatus::Open, &Role::Admin).is_err());
    }

    #[test]
    fn locked_is_permanent() {
        assert!(matches!(
            can_transition(PeriodStatus::Locked, PeriodStatus::Open, &Role::Admin),
            Err(PeriodError::LockedPermanently)
        ));
    }

    #[test]
    fn insufficient_role() {
        assert!(matches!(
            can_transition(PeriodStatus::UnderReview, PeriodStatus::Closed, &Role::Bokforare),
            Err(PeriodError::InsufficientRole)
        ));
        assert!(matches!(
            can_transition(PeriodStatus::Open, PeriodStatus::UnderReview, &Role::Revisor),
            Err(PeriodError::InsufficientRole)
        ));
    }
}
