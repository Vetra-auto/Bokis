//! Inventarieregistret i lagret (schemaversion 4): registrering, avgång,
//! spärrarna och historiken.

mod common;

use bok_store::bok_inventarier::{AssetAccounts, Disposal, DisposalKind, NewAsset};
use bok_store::{AccountChange, Actor, Store, StoreError};
use common::*;

fn with_asset_accounts(b: &mut Book) {
    let changes: Vec<AccountChange> = [
        (1220, "Inventarier och verktyg"),
        (
            1229,
            "Ackumulerade avskrivningar på inventarier och verktyg",
        ),
        (7832, "Avskrivningar på inventarier och verktyg"),
    ]
    .into_iter()
    .map(|(n, name)| AccountChange {
        number: acc(n),
        name: name.into(),
        active: true,
        sru: None,
        vat_code: None,
    })
    .collect();
    b.store
        .change_chart(&b.human, d(2026, 1, 1), &changes, &b.signer)
        .unwrap();
}

fn laptop() -> NewAsset {
    NewAsset {
        description: "Bärbar dator".into(),
        acquired: d(2026, 5, 12),
        cost: kr(3_600_000),
        life_months: 36,
        depreciation_start: d(2026, 5, 1),
        residual: kr(0),
        accounts: AssetAccounts {
            asset: acc(1220),
            accumulated: acc(1229),
            expense: acc(7832),
        },
    }
}

#[test]
fn register_and_list() {
    let mut b = book();
    with_asset_accounts(&mut b);
    let posted = b.post(&b.purchase(d(2026, 5, 12), "Dator", 1220, 3_600_000));
    let id = b
        .store
        .register_asset(&b.human, &laptop(), Some(posted.voucher_id))
        .unwrap();
    let assets = b.store.assets().unwrap();
    assert_eq!(assets.len(), 1);
    assert_eq!(assets[0].id, id);
    assert_eq!(assets[0].details, laptop());
    assert_eq!(assets[0].voucher.as_deref(), Some("A1 (2026)"));
    assert!(assets[0].disposal.is_none());
    let events = b.store.events(None).unwrap();
    assert_eq!(events.last().unwrap().action, bok_store::ASSET_REGISTERED);
    let report = b.store.verify().unwrap();
    assert!(
        report.is_ok(),
        "{:?}",
        report.problems().collect::<Vec<_>>()
    );
}

#[test]
fn dispose_once() {
    let mut b = book();
    with_asset_accounts(&mut b);
    let id = b.store.register_asset(&b.human, &laptop(), None).unwrap();
    let sold = Disposal {
        date: d(2026, 11, 2),
        kind: DisposalKind::Sold {
            price: kr(2_500_000),
        },
        note: Some("Köpare: Exempel AB".into()),
    };
    b.store.dispose_asset(&b.human, id, &sold).unwrap();
    assert_eq!(b.store.asset(id).unwrap().disposal, Some(sold.clone()));
    let again = b.store.dispose_asset(&b.human, id, &sold);
    assert!(matches!(again, Err(StoreError::Invalid(_))), "{again:?}");
}

#[test]
fn void_needs_an_explanation_and_cannot_predate_the_purchase() {
    let mut b = book();
    with_asset_accounts(&mut b);
    let id = b.store.register_asset(&b.human, &laptop(), None).unwrap();
    let mut void = Disposal {
        date: d(2026, 6, 1),
        kind: DisposalKind::Void,
        note: None,
    };
    assert!(b.store.dispose_asset(&b.human, id, &void).is_err());
    void.date = d(2026, 1, 1);
    void.note = Some("dubbelregistrerad".into());
    assert!(b.store.dispose_asset(&b.human, id, &void).is_err());
    void.date = d(2026, 6, 1);
    b.store.dispose_asset(&b.human, id, &void).unwrap();
    assert!(b.store.asset(id).unwrap().is_void());
}

#[test]
fn only_a_human_registers_and_accounts_must_exist() {
    let mut b = book();
    let refused = b
        .store
        .register_asset(&Actor::agent("agent"), &laptop(), None);
    assert!(
        matches!(refused, Err(StoreError::NotAllowed(_))),
        "{refused:?}"
    );
    // 1220, 1229 och 7832 finns inte i testets kontoplan än.
    let unknown = b.store.register_asset(&b.human, &laptop(), None);
    assert!(
        matches!(unknown, Err(StoreError::UnknownAccount(..))),
        "{unknown:?}"
    );
    with_asset_accounts(&mut b);
    let mut bad = laptop();
    bad.life_months = 0;
    assert!(matches!(
        b.store.register_asset(&b.human, &bad, None),
        Err(StoreError::Invalid(_))
    ));
}

#[test]
fn the_register_cannot_be_changed_behind_the_programs_back() {
    let mut b = book();
    with_asset_accounts(&mut b);
    let id = b.store.register_asset(&b.human, &laptop(), None).unwrap();
    b.store
        .dispose_asset(
            &b.human,
            id,
            &Disposal {
                date: d(2026, 12, 1),
                kind: DisposalKind::Scrapped,
                note: None,
            },
        )
        .unwrap();
    let raw = b.raw();
    for sql in [
        "UPDATE inventarier SET anskaffningsvarde = 1",
        "DELETE FROM inventarier",
        "UPDATE inventarieavgangar SET typ = 'felregistrering'",
        "DELETE FROM inventarieavgangar",
        "INSERT OR REPLACE INTO inventarieavgangar (id, inventarie_id, datum, typ, pris, aktor, registrerad, handelse)
         SELECT id, inventarie_id, datum, 'forsaljning', 1, aktor, registrerad, handelse FROM inventarieavgangar",
    ] {
        assert!(raw.execute(sql, []).is_err(), "{sql} gick igenom");
    }
    // Ett pris på en utrangering stoppas av schemat självt.
    let id2 = b.store.register_asset(&b.human, &laptop(), None).unwrap();
    let bad = raw.execute(
        "INSERT INTO inventarieavgangar (inventarie_id, datum, typ, pris, aktor, registrerad, handelse)
         VALUES (?1, '2026-12-01', 'utrangering', 100, 'x', 'x', 999999)",
        [id2],
    );
    assert!(bad.is_err());
    assert_eq!(b.store.missing_guards().unwrap(), Vec::<String>::new());
}

#[test]
fn an_older_book_gets_the_register_when_opened() {
    let b = book();
    let Book {
        store, path, dir, ..
    } = b;
    drop(store);
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute_batch(&format!(
        "{DROP_V5} DROP TABLE inventarieavgangar; DROP TABLE inventarier; PRAGMA user_version = 3;"
    ))
    .unwrap();
    drop(raw);
    let store = Store::open(&path, "test").unwrap();
    assert!(store.assets().unwrap().is_empty());
    assert_eq!(store.missing_guards().unwrap(), Vec::<String>::new());
    drop(dir);
}
