//! Kontoavstämning mot ett externt saldo: bokfört saldo på en dag, skillnaden
//! och en händelse i historiken. Bara en människa stämmer av.

mod common;

use bok_store::bok_core::Series;
use bok_store::bok_core::voucher::Line;
use bok_store::{AccountChange, Actor, StoreError};
use common::*;

#[test]
fn balance_difference_and_history() {
    let mut b = book();
    b.store
        .change_chart(
            &b.human,
            d(2026, 1, 1),
            &[AccountChange {
                number: acc(1630),
                name: "Skattekonto".into(),
                active: true,
                sru: None,
                vat_code: None,
            }],
            &b.signer,
        )
        .unwrap();
    let deposit = b.proposal(
        Series::A,
        d(2026, 3, 10),
        "Insättning på skattekontot",
        vec![
            Line::debit(acc(1630), kr(500_000)),
            Line::credit(acc(1930), kr(500_000)),
        ],
    );
    b.post(&deposit);
    let later = b.proposal(
        Series::A,
        d(2026, 4, 2),
        "Insättning i april",
        vec![
            Line::debit(acc(1630), kr(100_000)),
            Line::credit(acc(1930), kr(100_000)),
        ],
    );
    b.post(&later);
    assert_eq!(
        b.store.account_balance(acc(1630), d(2026, 3, 31)).unwrap(),
        kr(500_000)
    );
    assert_eq!(
        b.store.account_balance(acc(1630), d(2026, 4, 30)).unwrap(),
        kr(600_000)
    );

    let agent = Actor::agent("modell");
    let err = b
        .store
        .record_reconciliation(&agent, acc(1630), d(2026, 3, 31), kr(500_000), None)
        .unwrap_err();
    assert!(matches!(err, StoreError::NotAllowed(_)), "{err}");

    let ok = b
        .store
        .record_reconciliation(
            &b.human,
            acc(1630),
            d(2026, 3, 31),
            kr(500_000),
            Some("Skattekontot mars"),
        )
        .unwrap();
    assert_eq!(ok.difference, kr(0));
    let off = b
        .store
        .record_reconciliation(&b.human, acc(1630), d(2026, 4, 30), kr(587_500), None)
        .unwrap();
    assert_eq!(
        off.difference,
        kr(12_500),
        "räntan eller en avgift är inte bokförd"
    );

    let done = b.store.reconciliations().unwrap();
    assert_eq!(done.len(), 2);
    assert_eq!(done[1].2, off);
    assert_eq!(done[0].1, "granskare");
    assert!(b.store.account_balance(acc(1630), d(2030, 1, 1)).is_err());
    assert!(b.store.verify().unwrap().is_ok());
}
