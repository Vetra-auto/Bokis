//! Arkivplanen som händelser i behandlingshistoriken.

mod common;

use bok_store::bok_arkivplan::ArchivePlan;
use bok_store::{ARCHIVE_PLAN_SET, Actor, StoreError};
use common::*;

fn plan(place: &str) -> ArchivePlan {
    ArchivePlan::from_json(&format!(
        r#"{{"ansvarig": "Ekonomiansvarig",
            "delar": [{{"vad": "Databasen och underlagen", "slag": "original", "plats": "{place}"}}]}}"#
    ))
    .unwrap()
}

#[test]
fn the_latest_plan_applies_and_the_history_keeps_the_old_one() {
    let mut b = book();
    assert_eq!(b.store.archive_plan().unwrap(), None);
    let first = b.store.set_archive_plan(&b.human, &plan("/srv/a")).unwrap();
    let second = b.store.set_archive_plan(&b.human, &plan("/srv/b")).unwrap();
    assert!(second > first);
    let current = b.store.archive_plan().unwrap().unwrap();
    assert_eq!(current.plan, plan("/srv/b"));
    assert_eq!(current.event_seq, second);
    assert_eq!(current.actor, "granskare");
    let event = b.store.events(Some(1)).unwrap().remove(0);
    assert_eq!(event.action, ARCHIVE_PLAN_SET);
    assert!(event.old_values.unwrap().contains("/srv/a"));
    assert!(event.new_values.unwrap().contains("/srv/b"));
    let report = b.store.verify().unwrap();
    assert!(
        report.is_ok(),
        "{:?}",
        report.problems().collect::<Vec<_>>()
    );
}

#[test]
fn only_a_human_and_only_a_change() {
    let mut b = book();
    let refused = b
        .store
        .set_archive_plan(&Actor::agent("agent"), &plan("/srv/a"));
    assert!(
        matches!(refused, Err(StoreError::NotAllowed(_))),
        "{refused:?}"
    );
    b.store.set_archive_plan(&b.human, &plan("/srv/a")).unwrap();
    let same = b.store.set_archive_plan(&b.human, &plan("/srv/a"));
    assert!(matches!(same, Err(StoreError::Invalid(_))), "{same:?}");
    let mut empty = plan("/srv/a");
    empty.parts.clear();
    assert!(b.store.set_archive_plan(&b.human, &empty).is_err());
}
