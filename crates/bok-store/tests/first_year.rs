//! Det första räkenskapsåret börjar när bokföringsskyldigheten inträder,
//! vilken dag som helst; de följande börjar den första i en månad.

mod common;

use chrono::{TimeZone, Utc};
use common::{book, d};

use bok_store::bok_core::signature::SoftwareSigner;
use bok_store::{Actor, NewBook, Store};

#[test]
fn the_first_year_starts_on_the_day_the_company_was_formed() {
    let dir = tempfile::tempdir().unwrap();
    let signer = SoftwareSigner::from_seed([7; 32]);
    let mut store = Store::create_with_clock(
        &dir.path().join("bokis.db"),
        "bok-store-test",
        NewBook {
            company_name: "Nybildat AB",
            org_number: "559999-9991",
            key_name: "granskare",
            public_key: &signer.public_key_openssh(),
            signer: &signer,
            security_level: bok_store::bok_core::security::SecurityLevel::Software,
        },
        Box::new(|| Utc.with_ymd_and_hms(2026, 5, 20, 12, 0, 0).unwrap()),
    )
    .unwrap();
    let human = Actor::human("granskare");
    let year = store
        .add_fiscal_year(&human, d(2026, 5, 11), d(2026, 12, 31))
        .unwrap();
    let periods = store.periods(year.id).unwrap();
    assert_eq!(periods.len(), 8);
    assert_eq!(
        (periods[0].start, periods[0].end),
        (d(2026, 5, 11), d(2026, 5, 31))
    );
    assert!(
        store
            .add_fiscal_year(&human, d(2027, 1, 15), d(2027, 12, 31))
            .is_err()
    );
    assert!(
        store
            .add_fiscal_year(&human, d(2027, 1, 1), d(2027, 12, 31))
            .is_ok()
    );
}

#[test]
fn later_years_still_start_on_the_first() {
    let mut b = book();
    assert!(
        b.store
            .add_fiscal_year(&b.human, d(2027, 1, 15), d(2027, 12, 31))
            .is_err()
    );
}
