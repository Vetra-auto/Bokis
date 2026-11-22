//! Motpartens adress (schemaversion 5): i registret, på förslaget, på den
//! bokförda verifikationen och i rättelsen, och spärrarna mot ändringar.

mod common;

use bok_store::bok_core::voucher::Counterparty;
use bok_store::{ProposalCounterparty, Store};
use common::*;

fn private_person() -> Counterparty {
    Counterparty {
        name: "Anna Exempel".into(),
        org_number: None,
        vat_number: None,
        address: Some("Storgatan 1\n123 45 Exempelstad".into()),
    }
}

#[test]
fn the_address_follows_the_voucher_and_its_correction() {
    let mut b = book();
    let mut p = b.purchase(d(2026, 3, 2), "Begagnad hylla", 5410, 50_000);
    p.counterparty = Some(ProposalCounterparty::Inline(private_person()));
    let id = b.propose(&p);
    let proposal = b.store.proposal(id).unwrap();
    let Some(ProposalCounterparty::Inline(c)) = proposal.counterparty else {
        panic!("motparten saknas på förslaget");
    };
    assert_eq!(
        c.address.as_deref(),
        Some("Storgatan 1, 123 45 Exempelstad")
    );
    let posted = b.approve(id);
    let view = b.store.voucher(posted.voucher_id).unwrap();
    assert_eq!(
        view.counterparty.unwrap().address.as_deref(),
        Some("Storgatan 1, 123 45 Exempelstad")
    );

    let correction = b
        .store
        .propose_correction(&b.human, posted.voucher_id, "dubbelbokförd", &[])
        .unwrap();
    let r = b.approve(correction);
    let view = b.store.voucher(r.voucher_id).unwrap();
    assert_eq!(
        view.counterparty.unwrap().address.as_deref(),
        Some("Storgatan 1, 123 45 Exempelstad")
    );
    let report = b.store.verify().unwrap();
    assert!(
        report.is_ok(),
        "{:?}",
        report.problems().collect::<Vec<_>>()
    );

    // Adressen täcks av signaturen: en ändring förbi programmet syns.
    let raw = b.raw();
    raw.execute_batch("DROP TRIGGER verifikationer_ingen_andring;")
        .unwrap();
    raw.execute(
        "UPDATE verifikationer SET motpart_adress = 'Annan väg 2' WHERE id = ?1",
        [posted.voucher_id],
    )
    .unwrap();
    let report = b.store.verify().unwrap();
    assert!(!report.is_ok());
}

#[test]
fn the_register_keeps_the_address_and_logs_changes() {
    let mut b = book();
    let id = b
        .store
        .add_counterparty(&b.human, &private_person())
        .unwrap();
    let mut moved = private_person();
    moved.address = Some("Nygatan 3, 123 46 Exempelstad".into());
    b.store.update_counterparty(&b.human, id, &moved).unwrap();
    let rows = b.store.counterparties().unwrap();
    assert_eq!(
        rows[0].counterparty.address.as_deref(),
        Some("Nygatan 3, 123 46 Exempelstad")
    );
    let event = b.store.events(Some(1)).unwrap().remove(0);
    assert!(event.old_values.unwrap().contains("Storgatan 1"));

    let mut p = b.purchase(d(2026, 3, 2), "Begagnad hylla", 5410, 50_000);
    p.counterparty = Some(ProposalCounterparty::Register(id));
    let posted = b.post(&p);
    let view = b.store.voucher(posted.voucher_id).unwrap();
    assert_eq!(
        view.counterparty.unwrap().address.as_deref(),
        Some("Nygatan 3, 123 46 Exempelstad")
    );
}

#[test]
fn the_address_columns_are_guarded() {
    let mut b = book();
    let mut p = b.purchase(d(2026, 3, 2), "Begagnad hylla", 5410, 50_000);
    p.counterparty = Some(ProposalCounterparty::Inline(private_person()));
    let posted = b.post(&p);
    b.store
        .add_counterparty(&b.human, &private_person())
        .unwrap();
    let raw = b.raw();
    for sql in [
        "UPDATE verifikationer SET motpart_adress = 'x'",
        "UPDATE forslag SET motpart_adress = 'x'",
        "DELETE FROM motparter",
    ] {
        assert!(raw.execute(sql, []).is_err(), "{sql} gick igenom");
    }
    let blank = raw.execute(
        "INSERT INTO motparter (namn, registrerad, adress) VALUES ('X', 'x', ' ')",
        [],
    );
    assert!(blank.is_err(), "en tom adress ska stoppas av schemat");
    assert_eq!(b.store.missing_guards().unwrap(), Vec::<String>::new());
    assert!(b.store.voucher(posted.voucher_id).is_ok());
}

#[test]
fn a_version_4_book_gets_the_columns_and_keeps_its_signatures() {
    let mut b = book();
    let mut p = b.purchase(d(2026, 3, 2), "Papper", 5410, 10_000);
    p.counterparty = Some(ProposalCounterparty::Inline(Counterparty {
        name: "Papper AB".into(),
        org_number: Some("556000-0001".into()),
        vat_number: None,
        address: None,
    }));
    let posted = b.post(&p);
    let Book {
        store, path, dir, ..
    } = b;
    drop(store);
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute_batch(&format!("{DROP_V5} PRAGMA user_version = 4;"))
        .unwrap();
    drop(raw);
    let store = Store::open(&path, "test").unwrap();
    let version: i64 = rusqlite::Connection::open(&path)
        .unwrap()
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 5);
    assert_eq!(
        store
            .voucher(posted.voucher_id)
            .unwrap()
            .counterparty
            .unwrap()
            .address,
        None
    );
    let report = store.verify().unwrap();
    assert!(
        report.is_ok(),
        "{:?}",
        report.problems().collect::<Vec<_>>()
    );
    assert_eq!(store.missing_guards().unwrap(), Vec::<String>::new());
    drop(dir);
}
