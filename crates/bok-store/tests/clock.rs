//! Klockan: ingen händelse får en tid före den senaste i historiken. En dator
//! utan batteriklocka, till exempel en Raspberry Pi, kan starta med gammal tid
//! efter ett strömavbrott. Då vägrar Bokis att skriva, tills klockan är ställd
//! eller en människa godtar att det är den senaste händelsen som har fel tid.

mod common;

use chrono::Duration;

use bok_store::{Actor, CLOCK_ACCEPTED, StoreError};
use common::*;

fn events(b: &Book) -> usize {
    b.store.events(None).unwrap().len()
}

fn problems(b: &Book) -> String {
    b.store
        .verify()
        .unwrap()
        .problems()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
}

fn set_back(b: &Book, by: Duration) {
    *b.clock.lock().unwrap() -= by;
}

#[test]
fn a_clock_behind_the_last_event_stops_every_write() {
    let mut b = book();
    b.post_purchase(d(2026, 3, 1), "Papper", 10000);
    let before = events(&b);

    // Som en Pi som startar med tiden från förra timmen.
    set_back(&b, Duration::hours(1));
    let e = b
        .store
        .add_fiscal_year(&b.human, d(2027, 1, 1), d(2027, 12, 31))
        .unwrap_err();
    assert!(matches!(e, StoreError::ClockBehind { .. }), "{e}");
    assert!(e.to_string().contains("ställ klockan rätt"), "{e}");
    assert_eq!(events(&b), before);
    assert_eq!(b.store.fiscal_years().unwrap().len(), 1);

    // När klockan har kommit ikapp går det igen.
    set_back(&b, Duration::hours(-2));
    b.store
        .add_fiscal_year(&b.human, d(2027, 1, 1), d(2027, 12, 31))
        .unwrap();
    assert_eq!(events(&b), before + 1);
    assert_eq!(problems(&b), "");
}

#[test]
fn a_human_accepts_an_event_that_got_a_time_ahead() {
    let mut b = book();
    // Klockan gick en dag före när det här bokfördes.
    *b.clock.lock().unwrap() += Duration::days(1);
    b.post_purchase(d(2026, 3, 1), "Papper", 10000);
    // Nu går klockan rätt igen, och allt stannar.
    set_back(&b, Duration::days(1));
    let agent = Actor::agent("claude");
    assert!(matches!(
        b.store.accept_clock(&agent, "fel tid").unwrap_err(),
        StoreError::NotAllowed(_)
    ));
    assert!(b.store.accept_clock(&b.human, "  ").is_err());

    let (last, now) = b
        .store
        .accept_clock(&b.human, "datorns klocka gick en dag före 2026-03-16")
        .unwrap();
    assert!(now < last, "{now} {last}");
    let event = b.store.events(None).unwrap().pop().unwrap();
    assert_eq!(event.action, CLOCK_ACCEPTED);
    assert_eq!(event.actor, "granskare");
    let values = event.new_values.unwrap();
    assert!(
        values.contains(&last) && values.contains("gick en dag före"),
        "{values}"
    );

    // Därefter jämförs klockan med den nya händelsen, och historiken håller.
    b.post_purchase(d(2026, 3, 2), "Pennor", 2500);
    assert_eq!(problems(&b), "");

    // Går klockan rätt finns inget att godta.
    let e = b
        .store
        .accept_clock(&b.human, "för säkerhets skull")
        .unwrap_err();
    assert!(e.to_string().contains("inget att godta"), "{e}");
}

#[test]
fn verify_finds_a_time_that_goes_back_without_acceptance() {
    let b = book();
    let raw = b.raw();
    for t in raw
        .prepare("SELECT name FROM sqlite_master WHERE type = 'trigger'")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
    {
        raw.execute(&format!("DROP TRIGGER {}", t.unwrap()), [])
            .unwrap();
    }
    raw.execute(
        "UPDATE handelser SET tid = '2026-01-01T00:00:00.000000Z' WHERE seq = (SELECT MAX(seq) FROM handelser)",
        [],
    )
    .unwrap();
    let p = problems(&b);
    assert!(p.contains("är före föregående händelse"), "{p}");
}
