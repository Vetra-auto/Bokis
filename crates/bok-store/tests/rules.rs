//! Regler som data: signerade versioner, den som gäller på ett datum, och
//! verifieringen av signaturerna.

mod common;

use bok_store::{Actor, StoreError};
use common::*;

#[test]
fn versions_are_signed_and_the_right_one_applies() {
    let mut b = book();
    let v1 = b
        .store
        .add_rule(
            &b.human,
            "molntjanster",
            d(2026, 1, 1),
            None,
            "Molntjänster: 5420, ingående moms 25 % på 2641.",
            &b.signer,
        )
        .unwrap();
    assert_eq!(v1.version, 1);
    assert_eq!(
        v1.content,
        r#"{"text":"Molntjänster: 5420, ingående moms 25 % på 2641."}"#
    );
    let v2 = b
        .store
        .add_rule(
            &b.human,
            "molntjanster",
            d(2026, 7, 1),
            None,
            r#"{"text": "Molntjänster: 6540 från juli.", "konto": 6540}"#,
            &b.signer,
        )
        .unwrap();
    assert_eq!(v2.version, 2);
    assert_eq!(b.store.rules(d(2026, 3, 1)).unwrap()[0].version, 1);
    assert_eq!(b.store.rules(d(2026, 8, 1)).unwrap()[0].version, 2);
    assert_eq!(b.store.all_rules().unwrap().len(), 2);
    let report = b.store.verify().unwrap();
    assert!(
        report.is_ok(),
        "{:?}",
        report.problems().collect::<Vec<_>>()
    );

    let agent = Actor::agent("modell");
    let err = b
        .store
        .add_rule(&agent, "x", d(2026, 1, 1), None, "y", &b.signer)
        .unwrap_err();
    assert!(matches!(err, StoreError::NotAllowed(_)), "{err}");
    assert!(
        b.store
            .add_rule(&b.human, " ", d(2026, 1, 1), None, "y", &b.signer)
            .is_err()
    );
    assert!(
        b.store
            .add_rule(
                &b.human,
                "x",
                d(2026, 5, 1),
                Some(d(2026, 4, 1)),
                "y",
                &b.signer
            )
            .is_err()
    );
    let stranger = bok_store::bok_core::signature::SoftwareSigner::from_seed([1; 32]);
    assert!(
        b.store
            .add_rule(&b.human, "x", d(2026, 1, 1), None, "y", &stranger)
            .is_err()
    );

    // En regel som ändras förbi programmet upptäcks av verifieringen.
    let raw = b.raw();
    assert!(
        raw.execute("UPDATE regler SET innehall = '{}'", [])
            .is_err()
    );
}
