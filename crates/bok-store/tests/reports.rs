//! Gapanalysen: grundboken sorterades på datum, ingående balans saknades,
//! saldobalansen räknade fel rader, motparten sattes aldrig och rättelser
//! syntes inte på originalet.

mod common;

use bok_store::ProposalCounterparty;
use bok_store::bok_core::voucher::{Counterparty, Line};
use bok_store::bok_core::{PeriodStatus, Series};
use common::*;

#[test]
fn trial_balance_counts_only_posted_lines_of_the_given_year() {
    let mut b = book();
    let y2027 = b
        .store
        .add_fiscal_year(&b.human, d(2027, 1, 1), d(2027, 12, 31))
        .unwrap();
    b.post_purchase(d(2026, 3, 1), "Papper", 10000);
    b.post_purchase(d(2026, 4, 1), "Pennor", 2500);
    b.post_purchase(d(2027, 1, 10), "Nästa år", 99900);
    // Ett obeslutat och ett avvisat förslag får inte räknas.
    b.propose(&b.purchase(d(2026, 5, 1), "Obeslutat", 5410, 77700));
    let rejected = b.propose(&b.purchase(d(2026, 5, 2), "Avvisat", 5410, 88800));
    b.store.reject(&b.human, rejected, "fel").unwrap();

    let tb = b.store.trial_balance(b.year.id).unwrap();
    let row = |n: i64| tb.iter().find(|r| r.account == acc(n)).unwrap().clone();
    assert_eq!(row(5410).debit, kr(12500));
    assert_eq!(row(5410).closing, kr(12500));
    assert_eq!(row(1930).credit, kr(12500));
    assert_eq!(row(1930).closing, kr(-12500));
    assert_eq!(tb.len(), 2);
    let total = tb.iter().map(|r| r.closing.ore()).sum::<i64>();
    assert_eq!(total, 0, "saldobalansen balanserar");

    let tb27 = b.store.trial_balance(y2027.id).unwrap();
    assert_eq!(
        tb27.iter().find(|r| r.account == acc(5410)).unwrap().debit,
        kr(99900)
    );
}

#[test]
fn opening_balance_from_series_i_comes_first_in_the_ledger() {
    let mut b = book();
    b.post_purchase(d(2026, 3, 1), "Papper", 10000);
    // Ingående balans bokförs senare men gäller från årets början.
    let ib = b.proposal(
        Series::I,
        d(2026, 1, 1),
        "Ingående balans 2026",
        vec![
            Line::debit(acc(1930), kr(500000)),
            Line::credit(acc(2081), kr(500000)),
        ],
    );
    b.post(&ib);
    b.post_purchase(d(2026, 3, 2), "Pennor", 2500);

    let ledger = b.store.ledger(b.year.id, Some(acc(1930))).unwrap();
    assert_eq!(ledger.len(), 1);
    let bank = &ledger[0];
    assert_eq!(bank.name, "Företagskonto");
    assert_eq!(bank.opening, kr(500000));
    let refs: Vec<String> = bank
        .entries
        .iter()
        .map(|e| e.reference.to_string())
        .collect();
    assert_eq!(
        refs,
        ["A1", "A2"],
        "I-serien står som ingående balans, inte som post"
    );
    assert_eq!(bank.entries[0].balance, kr(490000));
    assert_eq!(bank.entries[1].balance, kr(487500));
    assert_eq!(bank.closing, kr(487500));

    let tb = b.store.trial_balance(b.year.id).unwrap();
    let row = tb.iter().find(|r| r.account == acc(1930)).unwrap();
    assert_eq!(
        (row.opening, row.credit, row.closing),
        (kr(500000), kr(12500), kr(487500))
    );
}

#[test]
fn journal_is_in_registration_order_not_date_order() {
    let mut b = book();
    b.post_purchase(d(2026, 3, 20), "Senare datum, bokförd först", 100);
    b.post_purchase(d(2026, 3, 5), "Tidigare datum, bokförd sist", 200);
    let journal = b.store.journal(b.year.id).unwrap();
    let rows: Vec<(String, String)> = journal
        .iter()
        .map(|v| (v.reference.to_string(), v.date.to_string()))
        .collect();
    assert_eq!(
        rows,
        [
            ("A1".into(), "2026-03-20".into()),
            ("A2".into(), "2026-03-05".into())
        ]
    );
    assert!(
        journal[0].registered_at < journal[1].registered_at,
        "registreringstiden visas och stiger"
    );
    assert!(journal[0].registered_at.ends_with('Z'));
}

#[test]
fn a_correction_is_visible_from_the_original_in_every_view() {
    let mut b = book();
    let original = b.post_purchase(d(2026, 3, 2), "Papper på fel konto", 10000);
    let mut right = Line::debit(acc(6570), kr(10000));
    right.text = Some("Rätt konto".into());
    let proposal = b
        .store
        .propose_correction(
            &b.human,
            original.voucher_id,
            "fel konto",
            &[right, Line::credit(acc(1930), kr(10000))],
        )
        .unwrap();
    let p = b.store.proposal(proposal).unwrap();
    assert_eq!(p.series, Series::R);
    assert_eq!(p.corrects, Some(original.voucher_id));
    let correction = b.approve(proposal);
    assert_eq!(correction.reference.to_string(), "R1");
    assert_eq!(correction.period, "2026-03", "originalets period är öppen");
    assert_eq!(correction.date, d(2026, 3, 2));

    let view = b.store.voucher(original.voucher_id).unwrap();
    assert_eq!(view.corrected_by.len(), 1);
    assert_eq!(view.corrected_by[0].reference.to_string(), "R1");
    assert_eq!(view.corrected_by[0].reason, "fel konto");
    let r1 = b.store.voucher(correction.voucher_id).unwrap();
    assert_eq!(r1.corrects.as_ref().unwrap().reference.to_string(), "A1");
    assert!(r1.description.starts_with("Rättelse av A1"));

    let journal = b.store.journal(b.year.id).unwrap();
    assert_eq!(journal[0].corrected_by[0].reference.to_string(), "R1");

    let ledger = b.store.ledger(b.year.id, Some(acc(5410))).unwrap();
    let a1 = ledger[0]
        .entries
        .iter()
        .find(|e| e.reference.to_string() == "A1")
        .unwrap();
    assert_eq!(a1.corrected_by[0].reference.to_string(), "R1");
    assert_eq!(ledger[0].closing, kr(0), "omvändningen tar ut originalet");

    let tb = b.store.trial_balance(b.year.id).unwrap();
    assert_eq!(
        tb.iter().find(|r| r.account == acc(6570)).unwrap().closing,
        kr(10000)
    );
    assert!(b.store.verify().unwrap().is_ok());
}

#[test]
fn a_correction_of_a_closed_period_lands_in_the_period_open_now() {
    let mut b = book();
    let original = b.post_purchase(d(2026, 2, 10), "Papper", 10000);
    let feb = b.period_id("2026-02");
    b.store
        .transition_period(&b.human, feb, PeriodStatus::UnderReview, None)
        .unwrap();
    b.store
        .transition_period(&b.human, feb, PeriodStatus::Closed, Some(&b.signer))
        .unwrap();
    b.set_today(d(2026, 3, 16));
    let proposal = b
        .store
        .propose_correction(&b.human, original.voucher_id, "dubbelbokförd", &[])
        .unwrap();
    let correction = b.approve(proposal);
    assert_eq!(correction.period, "2026-03");
    assert_eq!(correction.date, d(2026, 3, 16));
    let decision = b.store.proposal(proposal).unwrap().decision.unwrap();
    assert!(
        decision.changes.unwrap().contains("2026-03-16"),
        "flytten syns i beslutet"
    );
    assert_eq!(
        b.store.voucher(original.voucher_id).unwrap().corrected_by[0]
            .reference
            .to_string(),
        "R1"
    );
    assert!(b.store.verify().unwrap().is_ok());
}

#[test]
fn counterparty_snapshot_survives_a_change_in_the_register() {
    let mut b = book();
    let supplier = Counterparty {
        name: "Papper & Penna AB".into(),
        org_number: Some("556000-0001".into()),
        vat_number: Some("SE556000000101".into()),
    };
    let id = b.store.add_counterparty(&b.human, &supplier).unwrap();
    let mut p = b.purchase(d(2026, 3, 2), "Papper", 5410, 10000);
    p.counterparty = Some(ProposalCounterparty::Register(id));
    let posted = b.post(&p);

    let renamed = Counterparty {
        name: "Kontorsjätten AB".into(),
        org_number: Some("556999-9997".into()),
        vat_number: None,
    };
    b.store.update_counterparty(&b.human, id, &renamed).unwrap();
    assert_eq!(b.store.counterparties().unwrap()[0].counterparty, renamed);

    let view = b.store.voucher(posted.voucher_id).unwrap();
    assert_eq!(
        view.counterparty,
        Some(supplier.clone()),
        "verifikationen behåller kopian från bokföringen"
    );
    let event = b.store.events(Some(1)).unwrap().remove(0);
    assert_eq!(event.action, "motpart.andrad");
    assert!(event.old_values.unwrap().contains("Papper & Penna AB"));
    assert!(event.new_values.unwrap().contains("Kontorsjätten AB"));
    assert!(
        b.store.verify().unwrap().is_ok(),
        "signaturen täcker kopian, inte registret"
    );

    // Ett nytt förslag mot samma motpart får det nya namnet.
    let mut p2 = b.purchase(d(2026, 3, 3), "Mer papper", 5410, 100);
    p2.counterparty = Some(ProposalCounterparty::Register(id));
    let posted2 = b.post(&p2);
    assert_eq!(
        b.store
            .voucher(posted2.voucher_id)
            .unwrap()
            .counterparty
            .unwrap()
            .name,
        "Kontorsjätten AB"
    );
}

#[test]
fn an_inline_counterparty_is_copied_as_is() {
    let mut b = book();
    let mut p = b.purchase(d(2026, 3, 2), "Lunch", 5410, 10000);
    p.counterparty = Some(ProposalCounterparty::Inline(Counterparty {
        name: "Restaurang Ö".into(),
        org_number: None,
        vat_number: Some("SE559000000001".into()),
    }));
    let posted = b.post(&p);
    let cp = b
        .store
        .voucher(posted.voucher_id)
        .unwrap()
        .counterparty
        .unwrap();
    assert_eq!(cp.name, "Restaurang Ö");
    assert_eq!(cp.vat_number.as_deref(), Some("SE559000000001"));
}
