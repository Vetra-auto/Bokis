//! Bokslut: årets resultat föreslås i serie B och ingående balans för nästa
//! år i serie I. Båda bokförs först när människan godkänner dem.

mod common;

use bok_store::{AccountChange, Actor, StoreError};
use common::*;

fn with_closing_accounts(b: &mut Book) {
    let change = |n: i64, name: &str| AccountChange {
        number: acc(n),
        name: name.into(),
        active: true,
        sru: None,
        vat_code: None,
    };
    b.store
        .change_chart(
            &b.human,
            d(2026, 1, 1),
            &[
                change(2099, "Årets resultat"),
                change(8999, "Årets resultat"),
            ],
            &b.signer,
        )
        .unwrap();
}

#[test]
fn year_result_then_opening_balances() {
    let mut b = book();
    with_closing_accounts(&mut b);
    let sale = b.proposal(
        bok_store::bok_core::Series::A,
        d(2026, 3, 20),
        "Försäljning",
        vec![
            bok_store::bok_core::voucher::Line::debit(acc(1930), kr(100_000)),
            bok_store::bok_core::voucher::Line::credit(acc(3010), kr(100_000)),
        ],
    );
    b.post(&sale);
    b.post_purchase(d(2026, 3, 21), "Inköp", 30_000);
    assert_eq!(b.store.year_result(b.year.id).unwrap(), kr(70_000));

    // Utan bokfört resultat går balanskontona inte jämnt ut.
    let next = b
        .store
        .add_fiscal_year(&b.human, d(2027, 1, 1), d(2027, 12, 31))
        .unwrap();
    let err = b
        .store
        .propose_opening_balances(&b.human, b.year.id)
        .unwrap_err();
    assert!(err.to_string().contains("bokför årets resultat"), "{err}");

    let system = Actor::system("bokslut");
    let result = b
        .store
        .propose_year_result(&system, b.year.id, acc(8999), acc(2099))
        .unwrap();
    let again = b
        .store
        .propose_year_result(&system, b.year.id, acc(8999), acc(2099))
        .unwrap();
    assert_eq!(result, again, "samma förslag igen");
    let p = b.store.proposal(result).unwrap();
    assert_eq!(p.series, bok_store::bok_core::Series::B);
    assert_eq!(p.date, d(2026, 12, 31));
    let posted = b.approve(result);
    assert_eq!(posted.reference.to_string(), "B1");
    assert_eq!(b.store.year_result(b.year.id).unwrap(), kr(0));
    assert!(
        b.store
            .propose_year_result(&system, b.year.id, acc(8999), acc(2099))
            .is_err()
    );

    let ib = b
        .store
        .propose_opening_balances(&system, b.year.id)
        .unwrap();
    let p = b.store.proposal(ib).unwrap();
    assert_eq!(
        (p.series, p.date),
        (bok_store::bok_core::Series::I, d(2027, 1, 1))
    );
    assert!(p.lines.iter().all(|l| l.account.is_balance_account()));
    let posted = b.approve(ib);
    assert_eq!(posted.reference.to_string(), "I1");
    let opening: Vec<(String, i64)> = b
        .store
        .trial_balance(next.id)
        .unwrap()
        .into_iter()
        .map(|r| (r.account.to_string(), r.opening.ore()))
        .collect();
    assert_eq!(
        opening,
        vec![("1930".to_string(), 70_000), ("2099".to_string(), -70_000)]
    );
    let err = b
        .store
        .propose_opening_balances(&system, b.year.id)
        .unwrap_err();
    assert!(
        err.to_string().contains("redan en bokförd ingående balans"),
        "{err}"
    );
    assert!(b.store.verify().unwrap().is_ok());
}

#[test]
fn closing_needs_the_next_year_and_the_right_accounts() {
    let mut b = book();
    with_closing_accounts(&mut b);
    b.post_purchase(d(2026, 3, 21), "Inköp", 30_000);
    assert_eq!(
        b.store.year_result(b.year.id).unwrap(),
        kr(-30_000),
        "förlust"
    );
    let err = b
        .store
        .propose_year_result(&b.human, b.year.id, acc(2099), acc(8999))
        .unwrap_err();
    assert!(matches!(err, StoreError::Invalid(_)), "{err}");
    let loss = b
        .store
        .propose_year_result(&b.human, b.year.id, acc(8999), acc(2099))
        .unwrap();
    let lines = b.store.proposal(loss).unwrap().lines;
    assert_eq!(lines[0].credit, kr(30_000), "förlusten krediteras 8999");
    assert_eq!(lines[1].debit, kr(30_000));
    b.approve(loss);
    let err = b
        .store
        .propose_opening_balances(&b.human, b.year.id)
        .unwrap_err();
    assert!(err.to_string().contains("lägg till det först"), "{err}");
}
