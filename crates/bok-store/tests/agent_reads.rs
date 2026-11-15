//! Det som granskningsvyn och agentens verktyg läser: förslagets härkomst
//! och reglerna som gäller ett visst datum.

mod common;

use bok_store::{Actor, Provenance};
use common::*;

#[test]
fn a_proposal_carries_its_provenance() {
    let mut b = book();
    let provenance = Provenance {
        questions: Some(r#"["Är det här förbrukningsinventarier?"]"#.into()),
        flags: Some(r#"["nytt konto för känd motpart"]"#.into()),
        confidence: Some(0.82),
        model: Some("qwen3-coder-30b".into()),
        provider: Some("lokal".into()),
        region: Some("SE".into()),
        prompt_hash: Some("a".repeat(64)),
        rule_version: Some("moms.standard@2".into()),
        raw_response: Some(
            r#"{"harkomst":{"motivering":"Kvitto från Papper AB","klient":"qwen-code"}}"#.into(),
        ),
    };
    let mut p = b.purchase(d(2026, 3, 2), "Papper", 5410, 10000);
    p.provenance = provenance.clone();
    p.idempotency_key = Some("b".repeat(64));
    let id = b.store.create_proposal(&Actor::agent("qwen"), &p).unwrap();

    let row = b.store.proposal(id).unwrap();
    assert_eq!(row.provenance, provenance);
    assert_eq!((row.source.as_str(), row.actor.as_str()), ("agent", "qwen"));
    let pending = b.store.proposals(true).unwrap();
    assert_eq!(pending[0].provenance, provenance);

    let by_hand = b.propose(&b.purchase(d(2026, 3, 3), "Pennor", 5410, 500));
    assert_eq!(
        b.store.proposal(by_hand).unwrap().provenance,
        Provenance::default()
    );
}

#[test]
fn rules_are_read_as_they_apply_on_a_date() {
    let b = book();
    // Regler läggs in som signerade systemändringar från milstolpe 3; här
    // skrivs de direkt för att pröva läsningen.
    b.raw()
        .execute_batch(
            "INSERT INTO regler (namn, version, giltig_fran, giltig_till, innehall, andring) VALUES
               ('moms.standard', 1, '2025-01-01', '2025-12-31', '{\"sats\":25}', 1),
               ('moms.standard', 2, '2026-01-01', NULL, '{\"sats\":25,\"version\":2}', 1),
               ('kontering.papper', 1, '2026-06-01', '2026-12-31', '{\"konto\":5410}', 1);",
        )
        .unwrap();

    let names = |at| -> Vec<String> {
        b.store
            .rules(at)
            .unwrap()
            .into_iter()
            .map(|r| format!("{}@{}", r.name, r.version))
            .collect()
    };
    assert_eq!(names(d(2024, 12, 31)), Vec::<String>::new());
    assert_eq!(names(d(2025, 6, 1)), ["moms.standard@1"]);
    assert_eq!(names(d(2026, 3, 1)), ["moms.standard@2"]);
    assert_eq!(
        names(d(2026, 7, 1)),
        ["kontering.papper@1", "moms.standard@2"]
    );
    assert_eq!(
        names(d(2027, 1, 1)),
        ["moms.standard@2"],
        "kontering.papper gällde till och med 2026-12-31, inte dagen efter"
    );

    let papper = b.store.rules(d(2026, 12, 31)).unwrap().remove(0);
    assert_eq!(papper.valid_from, d(2026, 6, 1));
    assert_eq!(papper.valid_to, Some(d(2026, 12, 31)));
    assert_eq!(papper.content, "{\"konto\":5410}");
}
