//! `bok verify`: serien obruten, kedjan intakt, varje bokförd verifikation
//! har en giltig signatur. Triggers stoppar ändringar, men den som har
//! filen kan ta bort dem; då är det verifieringen som avslöjar ändringen.

mod common;

use bok_store::VerifyReport;
use bok_store::bok_core::PeriodStatus;
use bok_store::bok_core::signature::SoftwareSigner;
use common::*;

fn ledger_with_vouchers() -> Book {
    let mut b = book();
    b.post_purchase(d(2026, 3, 1), "Papper", 10000);
    b.post_purchase(d(2026, 3, 2), "Pennor", 2500);
    b
}

fn problems(report: &VerifyReport) -> String {
    report.problems().cloned().collect::<Vec<_>>().join("\n")
}

fn tamper(b: &Book, sql: &str) {
    let raw = b.raw();
    // Den som har filen kan ta bort triggarna. Det är just det fallet
    // signaturerna och kedjorna finns för.
    let triggers: Vec<String> = raw
        .prepare("SELECT name FROM sqlite_master WHERE type = 'trigger'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    for t in triggers {
        raw.execute(&format!("DROP TRIGGER {t}"), []).unwrap();
    }
    raw.execute_batch(sql).unwrap();
}

#[test]
fn an_untouched_book_verifies() {
    let mut b = ledger_with_vouchers();
    let jan = b.period_id("2026-01");
    b.store
        .transition_period(&b.human, jan, PeriodStatus::UnderReview, None)
        .unwrap();
    b.store
        .transition_period(&b.human, jan, PeriodStatus::Closed, Some(&b.signer))
        .unwrap();
    b.store
        .transition_period(&b.human, jan, PeriodStatus::Locked, Some(&b.signer))
        .unwrap();
    let second = SoftwareSigner::from_seed([77; 32]);
    b.store
        .add_key(&b.human, "revisor", &second.public_key_openssh(), &b.signer)
        .unwrap();
    let p = b.propose(&b.purchase(d(2026, 3, 3), "Signerad med andra nyckeln", 5410, 300));
    b.store.approve(&b.human, p, &second).unwrap();
    let report = b.store.verify().unwrap();
    assert!(report.is_ok(), "{}", problems(&report));
    assert_eq!(
        report
            .checks
            .iter()
            .find(|c| c.name.contains("signatur"))
            .unwrap()
            .checked,
        3
    );
    assert!(
        report
            .checks
            .iter()
            .flat_map(|c| &c.warnings)
            .any(|w| w.contains("mjukvarunyckel")),
        "mjukvarunycklar flaggas"
    );
}

#[test]
fn a_changed_amount_is_detected() {
    let b = ledger_with_vouchers();
    tamper(
        &b,
        "UPDATE rader SET debet = 1000 WHERE verifikation_id = 1 AND radnr = 1;
                UPDATE rader SET kredit = 1000 WHERE verifikation_id = 1 AND radnr = 2;",
    );
    let report = b.store.verify().unwrap();
    assert!(!report.is_ok());
    assert!(
        problems(&report).contains("A1 (id 1): innehållet stämmer inte med den kanoniska hashen"),
        "{}",
        problems(&report)
    );
}

#[test]
fn an_added_line_is_detected() {
    let b = ledger_with_vouchers();
    tamper(
        &b,
        "INSERT INTO rader (verifikation_id, radnr, konto, debet, kredit) VALUES (2, 3, 6570, 500, 0);",
    );
    let p = problems(&b.store.verify().unwrap());
    assert!(
        p.contains("A2 (id 2): 3 rader, men verifikationen anger 2"),
        "{p}"
    );
    assert!(p.contains("A2 (id 2): innehållet stämmer inte"), "{p}");
}

#[test]
fn a_changed_event_breaks_the_chain() {
    let b = ledger_with_vouchers();
    tamper(&b, "UPDATE handelser SET aktor = 'agent' WHERE seq = 2;");
    let p = problems(&b.store.verify().unwrap());
    assert!(p.contains("händelse 2 har ändrats"), "{p}");
}

#[test]
fn a_deleted_voucher_breaks_the_series() {
    let b = ledger_with_vouchers();
    tamper(
        &b,
        "DELETE FROM beslut WHERE verifikation_id = 1; DELETE FROM rader WHERE verifikation_id = 1;
                DELETE FROM verifikationer WHERE id = 1;",
    );
    let p = problems(&b.store.verify().unwrap());
    assert!(
        p.contains("serie A i året från 2026-01-01: väntade nummer 1, fann 2"),
        "{p}"
    );
    assert!(
        p.contains("räknaren står på 3 men serien har 1 verifikationer"),
        "{p}"
    );
    assert!(p.contains("verifikationskedjan är bruten"), "{p}");
}

#[test]
fn a_voucher_signed_with_another_key_is_detected() {
    let b = ledger_with_vouchers();
    // En främling signerar samma hash; signaturen byts ut förbi programmet.
    let view = b.store.voucher(2).unwrap();
    let stranger = SoftwareSigner::from_seed([13; 32]);
    let forged = bok_store::bok_core::signature::Signer::sign(
        &stranger,
        bok_store::bok_core::signature::NS_VOUCHER,
        view.canonical_hash.as_bytes(),
    )
    .unwrap();
    tamper(
        &b,
        &format!("UPDATE verifikationer SET signatur = '{forged}' WHERE id = 2;"),
    );
    let p = problems(&b.store.verify().unwrap());
    assert!(p.contains("A2 (id 2): signaturen är inte giltig"), "{p}");
    assert!(p.contains("det signerade godkännandet saknas"), "{p}");
}

#[test]
fn a_silently_reopened_period_is_detected() {
    let mut b = ledger_with_vouchers();
    let feb = b.period_id("2026-02");
    b.store
        .transition_period(&b.human, feb, PeriodStatus::UnderReview, None)
        .unwrap();
    b.store
        .transition_period(&b.human, feb, PeriodStatus::Closed, Some(&b.signer))
        .unwrap();
    tamper(
        &b,
        &format!("UPDATE perioder SET status = 'oppen' WHERE id = {feb};"),
    );
    let p = problems(&b.store.verify().unwrap());
    assert!(
        p.contains("period 2026-02: status oppen, men historiken säger stangd"),
        "{p}"
    );
}

#[test]
fn a_chart_change_past_the_program_is_detected() {
    let b = ledger_with_vouchers();
    tamper(&b, "UPDATE kontoplan SET namn = 'Smyg' WHERE konto = 5410;");
    let p = problems(&b.store.verify().unwrap());
    assert!(p.contains("stämmer inte med de lagrade kontona"), "{p}");
}
