//! Gapanalysen: `post_voucher` kontrollerade inte periodens status, och
//! periodövergångar loggades med värden som bröt mot CHECK-villkoret.

mod common;

use bok_store::bok_core::PeriodStatus;
use bok_store::bok_core::period::PeriodError;
use bok_store::bok_core::signature::SoftwareSigner;
use bok_store::{Actor, StoreError};
use common::*;

fn close(b: &mut Book, label: &str) {
    let id = b.period_id(label);
    b.store
        .transition_period(&b.human, id, PeriodStatus::UnderReview, None)
        .unwrap();
    b.store
        .transition_period(&b.human, id, PeriodStatus::Closed, Some(&b.signer))
        .unwrap();
}

#[test]
fn posting_in_a_closed_period_is_refused() {
    let mut b = book();
    let p = b.purchase(d(2026, 2, 10), "Papper", 5410, 10000);
    let id = b.propose(&p);
    close(&mut b, "2026-02");
    let err = b.store.approve(&b.human, id, &b.signer).unwrap_err();
    assert!(
        matches!(err, StoreError::PeriodNotOpen { ref period, status: PeriodStatus::Closed } if period == "2026-02"),
        "{err}"
    );
    assert!(b.store.journal(b.year.id).unwrap().is_empty());
    assert!(
        b.store.proposal(id).unwrap().decision.is_none(),
        "förslaget ligger kvar obeslutat"
    );
}

#[test]
fn posting_in_a_locked_period_or_under_review_is_refused() {
    let mut b = book();
    let feb = b.purchase(d(2026, 2, 10), "Papper", 5410, 10000);
    let jan = b.purchase(d(2026, 1, 10), "Pennor", 5410, 5000);
    let feb_id = b.propose(&feb);
    let jan_id = b.propose(&jan);
    close(&mut b, "2026-02");
    let p = b.period_id("2026-02");
    b.store
        .transition_period(&b.human, p, PeriodStatus::Locked, Some(&b.signer))
        .unwrap();
    let err = b.store.approve(&b.human, feb_id, &b.signer).unwrap_err();
    assert!(
        matches!(
            err,
            StoreError::PeriodNotOpen {
                status: PeriodStatus::Locked,
                ..
            }
        ),
        "{err}"
    );

    let jan_period = b.period_id("2026-01");
    b.store
        .transition_period(&b.human, jan_period, PeriodStatus::UnderReview, None)
        .unwrap();
    let err = b.store.approve(&b.human, jan_id, &b.signer).unwrap_err();
    assert!(
        matches!(
            err,
            StoreError::PeriodNotOpen {
                status: PeriodStatus::UnderReview,
                ..
            }
        ),
        "{err}"
    );
}

#[test]
fn database_refuses_a_voucher_in_a_closed_period_even_past_the_program() {
    let mut b = book();
    b.post_purchase(d(2026, 2, 3), "Papper", 10000);
    close(&mut b, "2026-02");
    let raw = b.raw();
    let feb = b.period_id("2026-02");
    // Räkna upp räknaren som programmet gör och försök skriva förbi det.
    raw.execute(
        "UPDATE nummerserier SET nasta = nasta + 1 WHERE rakenskapsar_id = ?1 AND serie = 'A'",
        [b.year.id],
    )
    .unwrap();
    let err = raw
        .execute(
            "INSERT INTO verifikationer (rakenskapsar_id, serie, nummer, period_id, datum, sammanstallningsdag,
                registrerad, beskrivning, forslag_id, antal_rader, kanonisk_hash, signatur, nyckel_id, kedjehash)
             VALUES (?1, 'A', 2, ?2, '2026-02-20', '2026-02-20', '2026-03-16T12:00:00.000000Z', 'Smygpost', 1, 2,
                     printf('%064d', 0), 'x', 1, printf('%064d', 0))",
            rusqlite::params![b.year.id, feb],
        )
        .unwrap_err();
    assert!(
        err.to_string().contains("perioden tar inte emot poster"),
        "{err}"
    );
}

#[test]
fn closed_and_locked_periods_never_reopen() {
    let mut b = book();
    close(&mut b, "2026-01");
    let jan = b.period_id("2026-01");
    for to in [PeriodStatus::Open, PeriodStatus::UnderReview] {
        let err = b
            .store
            .transition_period(&b.human, jan, to, Some(&b.signer))
            .unwrap_err();
        assert!(
            matches!(err, StoreError::Period(PeriodError::ClosedCannotReopen)),
            "{err}"
        );
    }
    b.store
        .transition_period(&b.human, jan, PeriodStatus::Locked, Some(&b.signer))
        .unwrap();
    for to in [
        PeriodStatus::Open,
        PeriodStatus::UnderReview,
        PeriodStatus::Closed,
    ] {
        let err = b
            .store
            .transition_period(&b.human, jan, to, Some(&b.signer))
            .unwrap_err();
        assert!(
            matches!(err, StoreError::Period(PeriodError::LockedPermanently)),
            "{err}"
        );
    }

    // Databasen vägrar också, om någon går förbi programmet.
    let raw = b.raw();
    let err = raw
        .execute("UPDATE perioder SET status = 'oppen' WHERE id = ?1", [jan])
        .unwrap_err();
    assert!(
        err.to_string().contains("en låst period kan inte ändras"),
        "{err}"
    );
    close(&mut b, "2026-02");
    let feb = b.period_id("2026-02");
    let err = raw
        .execute("UPDATE perioder SET status = 'oppen' WHERE id = ?1", [feb])
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("en stängd period kan inte öppnas igen"),
        "{err}"
    );
    let err = raw
        .execute(
            "UPDATE perioder SET status = 'under_avstamning' WHERE id = ?1",
            [feb],
        )
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("en stängd period kan inte öppnas igen"),
        "{err}"
    );
    let err = raw
        .execute("DELETE FROM perioder WHERE id = ?1", [feb])
        .unwrap_err();
    assert!(err.to_string().contains("kan inte raderas"), "{err}");
    let err = raw
        .execute(
            "INSERT OR REPLACE INTO perioder (rakenskapsar_id, start, slut) VALUES (?1, '2026-02-01', '2026-02-28')",
            [b.year.id],
        )
        .unwrap_err();
    assert!(err.to_string().contains("kan inte ersättas"), "{err}");
    assert_eq!(b.store.period(feb).unwrap().status, PeriodStatus::Closed);
}

#[test]
fn every_transition_is_logged_without_check_failures() {
    let mut b = book();
    let mar = b.period_id("2026-03");
    let before = b.event_count();
    let steps = [
        (PeriodStatus::UnderReview, "period.under_avstamning", false),
        (PeriodStatus::Open, "period.oppnad", false),
        (PeriodStatus::UnderReview, "period.under_avstamning", false),
        (PeriodStatus::Closed, "period.stangd", true),
        (PeriodStatus::Locked, "period.last", true),
    ];
    let mut from = PeriodStatus::Open;
    for (to, action, signed) in steps {
        let t = b
            .store
            .transition_period(&b.human, mar, to, Some(&b.signer))
            .unwrap();
        assert_eq!((t.from, t.to), (from, to));
        let events = b.store.events(Some(1)).unwrap();
        let e = &events[0];
        assert_eq!(e.action, action);
        assert_eq!(e.object.as_deref(), Some(format!("period:{mar}").as_str()));
        assert!(e.old_values.as_deref().unwrap().contains(from.as_db_str()));
        assert!(e.new_values.as_deref().unwrap().contains(to.as_db_str()));
        assert_eq!(e.signed, signed, "{action}");
        assert_eq!(e.actor_kind, "manniska");
        from = to;
    }
    assert_eq!(b.event_count(), before + steps.len());
    assert!(b.store.verify().unwrap().is_ok());
}

#[test]
fn closing_requires_a_signature_from_a_registered_key() {
    let mut b = book();
    let apr = b.period_id("2026-04");
    b.store
        .transition_period(&b.human, apr, PeriodStatus::UnderReview, None)
        .unwrap();
    let err = b
        .store
        .transition_period(&b.human, apr, PeriodStatus::Closed, None)
        .unwrap_err();
    assert!(matches!(err, StoreError::Invalid(_)), "{err}");
    let stranger = SoftwareSigner::from_seed([9; 32]);
    let err = b
        .store
        .transition_period(&b.human, apr, PeriodStatus::Closed, Some(&stranger))
        .unwrap_err();
    assert!(matches!(err, StoreError::UnknownKey(_)), "{err}");
    assert_eq!(
        b.store.period(apr).unwrap().status,
        PeriodStatus::UnderReview
    );
}

#[test]
fn an_agent_cannot_change_a_period() {
    let mut b = book();
    let may = b.period_id("2026-05");
    let err = b
        .store
        .transition_period(
            &Actor::agent("claude"),
            may,
            PeriodStatus::UnderReview,
            None,
        )
        .unwrap_err();
    assert!(
        matches!(err, StoreError::Period(PeriodError::NotAllowedForActor)),
        "{err}"
    );
}

#[test]
fn closing_reports_pending_proposals_in_the_period() {
    let mut b = book();
    let p = b.purchase(d(2026, 6, 5), "Papper", 5410, 10000);
    b.propose(&p);
    let jun = b.period_id("2026-06");
    b.store
        .transition_period(&b.human, jun, PeriodStatus::UnderReview, None)
        .unwrap();
    let t = b
        .store
        .transition_period(&b.human, jun, PeriodStatus::Closed, Some(&b.signer))
        .unwrap();
    assert_eq!(t.pending_proposals, 1);
}
