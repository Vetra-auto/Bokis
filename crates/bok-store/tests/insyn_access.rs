//! Insyn per person: namngivna länkar, återkallelse och inloggningar i
//! behandlingshistoriken.

mod common;

use chrono::Duration;

use bok_store::{
    Actor, INSYN_LINK_CREATED, INSYN_LINK_REVOKED, INSYN_LOGIN, MAX_INSYN_LINK_DAYS, StoreError,
};
use common::*;

#[test]
fn a_named_link_logs_its_logins_until_it_is_revoked() {
    let mut b = book();
    let now = b.store.now();
    let id = b
        .store
        .create_insyn_link(&b.human, " Revisor X ", now + Duration::days(30))
        .unwrap();
    let event = b.store.events(Some(1)).unwrap().remove(0);
    assert_eq!((event.seq, event.action.as_str()), (id, INSYN_LINK_CREATED));

    let session = b.store.now() + Duration::hours(12);
    b.store.record_insyn_login(id, session).unwrap();
    b.store.record_insyn_login(id, session).unwrap();
    let login = b.store.events(Some(1)).unwrap().remove(0);
    assert_eq!(login.action, INSYN_LOGIN);
    assert_eq!(login.actor, "Revisor X");
    assert_eq!(login.role, "insyn");
    assert!(login.new_values.unwrap().contains("session_till"));

    let link = b.store.insyn_link(id).unwrap().unwrap();
    assert_eq!(link.name, "Revisor X");
    assert_eq!(link.created_by, "granskare");
    assert_eq!(link.logins, 2);
    assert!(link.last_login.is_some());
    assert!(link.active_at(b.store.now()));
    assert!(!link.active_at(now + Duration::days(31)));

    let revoked = b.store.revoke_insyn_link(&b.human, id).unwrap();
    assert!(revoked.revoked.is_some());
    assert_eq!(
        b.store.events(Some(1)).unwrap()[0].action,
        INSYN_LINK_REVOKED
    );
    let refused = b.store.record_insyn_login(id, session);
    assert!(
        matches!(refused, Err(StoreError::Invalid(_))),
        "{refused:?}"
    );
    assert!(b.store.revoke_insyn_link(&b.human, id).is_err());
    assert_eq!(b.store.insyn_links().unwrap(), vec![revoked]);
    let report = b.store.verify().unwrap();
    assert!(
        report.is_ok(),
        "{:?}",
        report.problems().collect::<Vec<_>>()
    );
}

#[test]
fn only_humans_create_links_and_only_for_a_while() {
    let mut b = book();
    let now = b.store.now();
    let agent =
        b.store
            .create_insyn_link(&Actor::agent("agent"), "Revisor X", now + Duration::days(1));
    assert!(matches!(agent, Err(StoreError::NotAllowed(_))), "{agent:?}");
    for (name, until) in [
        ("", now + Duration::days(1)),
        ("Revisor\nX", now + Duration::days(1)),
        ("Revisor X", now - Duration::days(1)),
        ("Revisor X", now + Duration::days(MAX_INSYN_LINK_DAYS + 1)),
    ] {
        assert!(
            matches!(
                b.store.create_insyn_link(&b.human, name, until),
                Err(StoreError::Invalid(_))
            ),
            "{name:?} {until}"
        );
    }
    assert!(
        b.store.insyn_link(1).unwrap().is_none(),
        "händelse 1 är ingen länk"
    );
    assert!(b.store.record_insyn_login(999, now).is_err());
}

#[test]
fn a_login_with_the_start_link_is_logged_too() {
    let mut b = book();
    b.store.record_local_insyn_login("henrik").unwrap();
    let e = b.store.events(Some(1)).unwrap().remove(0);
    assert_eq!(
        (e.action.as_str(), e.actor.as_str()),
        (INSYN_LOGIN, "henrik")
    );
    assert!(e.new_values.unwrap().contains("startlänken"));
}
