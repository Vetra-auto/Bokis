//! Momsrapporten: summor per ruta ur kontoplanens momskoder, med blankettens
//! tecken, ruta 49 framräknad och momsavräkningen bortlämnad.

mod common;

use bok_store::AccountChange;
use bok_store::bok_core::Series;
use bok_store::bok_core::voucher::Line;
use common::*;

fn with_vat_codes(b: &mut Book) {
    let change = |n: i64, name: &str, code: Option<&str>| AccountChange {
        number: acc(n),
        name: name.into(),
        active: true,
        sru: None,
        vat_code: code.map(str::to_string),
    };
    b.store
        .change_chart(
            &b.human,
            d(2026, 1, 1),
            &[
                change(3010, "Försäljning", Some("05")),
                change(2611, "Utgående moms 25 %", Some("10")),
                change(2641, "Debiterad ingående moms", Some("48")),
                change(2650, "Redovisningskonto för moms", Some("49")),
                change(6570, "Bankkostnader", Some("99")),
            ],
            &b.signer,
        )
        .unwrap();
}

#[test]
fn boxes_signs_and_box_49() {
    let mut b = book();
    with_vat_codes(&mut b);
    let sale = b.proposal(
        Series::A,
        d(2026, 3, 5),
        "Försäljning",
        vec![
            Line::debit(acc(1930), kr(125_000)),
            Line::credit(acc(3010), kr(100_000)),
            Line::credit(acc(2611), kr(25_000)),
        ],
    );
    b.post(&sale);
    let purchase = b.proposal(
        Series::A,
        d(2026, 3, 6),
        "Inköp",
        vec![
            Line::debit(acc(5410), kr(80_000)),
            Line::debit(acc(2641), kr(20_000)),
            Line::credit(acc(1930), kr(100_000)),
        ],
    );
    b.post(&purchase);
    // Momsavräkningen nollar momskontona men ska inte påverka rapporten.
    let settlement = b.proposal(
        Series::A,
        d(2026, 3, 31),
        "Momsavräkning mars",
        vec![
            Line::debit(acc(2611), kr(25_000)),
            Line::credit(acc(2641), kr(20_000)),
            Line::credit(acc(2650), kr(5_000)),
        ],
    );
    b.post(&settlement);
    // Utanför perioden.
    b.post_purchase(d(2026, 4, 2), "April", 10_000);

    let report = b.store.vat_report(d(2026, 3, 1), d(2026, 3, 31)).unwrap();
    let amounts: Vec<(&str, i64)> = report
        .boxes
        .iter()
        .map(|x| (x.code.as_str(), x.amount.ore()))
        .collect();
    assert_eq!(
        amounts,
        vec![
            ("05", 100_000),
            ("10", 25_000),
            ("48", 20_000),
            ("49", 5_000)
        ]
    );
    assert_eq!(report.to_pay, kr(5_000));
    assert_eq!(report.settlements, 1);
    assert!(report.unknown_codes.is_empty());
    assert_eq!(report.boxes[0].accounts, vec![(acc(3010), kr(100_000))]);
}

#[test]
fn refunds_unknown_codes_and_empty_periods() {
    let mut b = book();
    with_vat_codes(&mut b);
    let purchase = b.proposal(
        Series::A,
        d(2026, 3, 6),
        "Inköp",
        vec![
            Line::debit(acc(5410), kr(80_000)),
            Line::debit(acc(2641), kr(20_000)),
            Line::credit(acc(1930), kr(100_000)),
        ],
    );
    b.post(&purchase);
    b.post(&b.purchase(d(2026, 3, 7), "Avgift", 6570, 500));
    let report = b.store.vat_report(d(2026, 3, 1), d(2026, 3, 31)).unwrap();
    assert_eq!(report.to_pay, kr(-20_000), "moms att få tillbaka");
    assert_eq!(report.unknown_codes, vec![("99".to_string(), acc(6570))]);

    let empty = b.store.vat_report(d(2026, 5, 1), d(2026, 5, 31)).unwrap();
    assert_eq!(empty.boxes.len(), 1, "bara ruta 49");
    assert_eq!(empty.to_pay, kr(0));
    assert!(b.store.vat_report(d(2026, 5, 31), d(2026, 5, 1)).is_err());
}
