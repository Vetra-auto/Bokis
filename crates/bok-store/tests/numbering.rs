//! Gapanalysen: verifikationsnumret delades ut när utkastet skapades. Nu har
//! ett förslag inget nummer; numret tas ur räknaren när det bokförs, serien
//! får inga luckor och ett avvisat förslag förbrukar inget nummer.

mod common;

use bok_store::bok_core::signature::{SignatureError, Signer, SoftwareSigner};
use bok_store::bok_core::voucher::{Line, VoucherError};
use bok_store::bok_core::{Series, VoucherRef};
use bok_store::{Actor, StoreError};
use common::*;

fn numbers(b: &Book, series: Series) -> Vec<i64> {
    b.store
        .journal(b.year.id)
        .unwrap()
        .into_iter()
        .filter(|v| v.reference.series == series)
        .map(|v| v.reference.number)
        .collect()
}

#[test]
fn a_proposal_has_no_number_until_it_is_posted() {
    let mut b = book();
    let p1 = b.propose(&b.purchase(d(2026, 3, 1), "Ett", 5410, 100));
    let p2 = b.propose(&b.purchase(d(2026, 3, 2), "Två", 5410, 200));
    assert!(
        b.store.journal(b.year.id).unwrap().is_empty(),
        "inga verifikationer innan bokföring"
    );
    // Bokförs i omvänd ordning: numret följer bokföringen, inte förslaget.
    assert_eq!(
        b.approve(p2).reference,
        VoucherRef {
            series: Series::A,
            number: 1
        }
    );
    assert_eq!(
        b.approve(p1).reference,
        VoucherRef {
            series: Series::A,
            number: 2
        }
    );
}

#[test]
fn rejected_proposals_consume_no_number() {
    let mut b = book();
    let a = b.propose(&b.purchase(d(2026, 3, 1), "Ett", 5410, 100));
    let rejected = b.propose(&b.purchase(d(2026, 3, 2), "Dubblett", 5410, 100));
    let c = b.propose(&b.purchase(d(2026, 3, 3), "Tre", 5410, 300));
    b.approve(a);
    b.store
        .reject(&b.human, rejected, "dubblett av förslag 1")
        .unwrap();
    let posted = b.approve(c);
    assert_eq!(
        posted.reference.number, 2,
        "det avvisade förslaget tog inget nummer"
    );
    assert_eq!(numbers(&b, Series::A), vec![1, 2]);
    let decision = b.store.proposal(rejected).unwrap().decision.unwrap();
    assert_eq!(decision.reason.as_deref(), Some("dubblett av förslag 1"));
    assert!(decision.voucher_id.is_none());
    // Ett beslut är slutligt.
    assert!(matches!(
        b.store.approve(&b.human, rejected, &b.signer),
        Err(StoreError::AlreadyDecided(_))
    ));
    assert!(matches!(
        b.store.reject(&b.human, c, "ångrar"),
        Err(StoreError::AlreadyDecided(_))
    ));
    assert!(b.store.verify().unwrap().is_ok());
}

#[test]
fn rejection_needs_a_reason() {
    let mut b = book();
    let p = b.propose(&b.purchase(d(2026, 3, 1), "Ett", 5410, 100));
    assert!(matches!(
        b.store.reject(&b.human, p, "  "),
        Err(StoreError::Invalid(_))
    ));
}

struct FailingSigner;
impl Signer for FailingSigner {
    fn sign(&self, _: &str, _: &[u8]) -> Result<String, SignatureError> {
        Err(SignatureError::SigningFailed("nyckeln rördes inte".into()))
    }
}

#[test]
fn a_failed_signature_rolls_back_and_consumes_no_number() {
    let mut b = book();
    let p = b.propose(&b.purchase(d(2026, 3, 1), "Ett", 5410, 100));
    let events = b.event_count();
    let err = b.store.approve(&b.human, p, &FailingSigner).unwrap_err();
    assert!(
        matches!(err, StoreError::Signature(SignatureError::SigningFailed(_))),
        "{err}"
    );
    let stranger = SoftwareSigner::from_seed([1; 32]);
    let err = b.store.approve(&b.human, p, &stranger).unwrap_err();
    assert!(matches!(err, StoreError::UnknownKey(_)), "{err}");
    assert_eq!(
        b.event_count(),
        events,
        "inga händelser från misslyckade försök"
    );
    assert!(b.store.proposal(p).unwrap().decision.is_none());
    assert_eq!(
        b.approve(p).reference.number,
        1,
        "numret förbrukades inte av försöken"
    );
    assert!(b.store.verify().unwrap().is_ok());
}

#[test]
fn an_agent_can_propose_but_never_post() {
    let mut b = book();
    let agent = Actor::agent("claude");
    let p = b
        .store
        .create_proposal(
            &agent,
            &b.purchase(d(2026, 3, 1), "Från agenten", 5410, 100),
        )
        .unwrap();
    assert_eq!(b.store.proposal(p).unwrap().source, "agent");
    assert!(matches!(
        b.store.approve(&agent, p, &b.signer),
        Err(StoreError::NotAllowed(_))
    ));
    assert!(matches!(
        b.store.reject(&agent, p, "nej"),
        Err(StoreError::NotAllowed(_))
    ));
    assert_eq!(b.approve(p).reference.number, 1);
}

#[test]
fn series_are_separate_and_restart_every_fiscal_year() {
    let mut b = book();
    let y2027 = b
        .store
        .add_fiscal_year(&b.human, d(2027, 1, 1), d(2027, 12, 31))
        .unwrap();
    b.post_purchase(d(2026, 3, 1), "A 2026", 100);
    b.post_purchase(d(2026, 3, 2), "A 2026", 100);
    let ib = b.proposal(
        Series::I,
        d(2026, 1, 1),
        "Ingående balans",
        vec![
            Line::debit(acc(1930), kr(50000)),
            Line::credit(acc(2081), kr(50000)),
        ],
    );
    assert_eq!(b.post(&ib).reference.to_string(), "I1");
    let first_2027 = b.post_purchase(d(2027, 1, 5), "A 2027", 100);
    assert_eq!(first_2027.reference.to_string(), "A1");
    assert_eq!(first_2027.fiscal_year_id, y2027.id);
    assert_eq!(numbers(&b, Series::A), vec![1, 2]);
    assert!(b.store.verify().unwrap().is_ok());
}

#[test]
fn proposals_are_validated_before_they_are_stored() {
    let mut b = book();
    let unbalanced = b.proposal(
        Series::A,
        d(2026, 3, 1),
        "Obalanserad",
        vec![
            Line::debit(acc(5410), kr(100)),
            Line::credit(acc(1930), kr(99)),
        ],
    );
    assert!(matches!(
        b.store.create_proposal(&b.human, &unbalanced),
        Err(StoreError::Voucher(VoucherError::Unbalanced { .. }))
    ));
    let unknown = b.purchase(d(2026, 3, 1), "Okänt konto", 4010, 100);
    assert!(matches!(
        b.store.create_proposal(&b.human, &unknown),
        Err(StoreError::UnknownAccount(..))
    ));
    let no_year = b.purchase(d(2030, 3, 1), "Inget år", 5410, 100);
    assert!(matches!(
        b.store.create_proposal(&b.human, &no_year),
        Err(StoreError::NoPeriod(_))
    ));
    let series_r = b.proposal(
        Series::R,
        d(2026, 3, 1),
        "R utan original",
        b.purchase(d(2026, 3, 1), "", 5410, 1).lines,
    );
    assert!(matches!(
        b.store.create_proposal(&b.human, &series_r),
        Err(StoreError::Voucher(VoucherError::SeriesRWithoutOriginal))
    ));
    let result_in_ib = b.proposal(
        Series::I,
        d(2026, 1, 1),
        "IB med resultatkonto",
        vec![
            Line::debit(acc(1930), kr(100)),
            Line::credit(acc(3010), kr(100)),
        ],
    );
    assert!(matches!(
        b.store.create_proposal(&b.human, &result_in_ib),
        Err(StoreError::Voucher(
            VoucherError::ResultAccountInOpeningBalance { .. }
        ))
    ));
}

#[test]
fn the_same_idempotency_key_gives_back_the_same_proposal() {
    let mut b = book();
    let mut p = b.purchase(d(2026, 3, 1), "Kvitto", 5410, 100);
    p.idempotency_key = Some("mejl-123".into());
    let first = b
        .store
        .create_proposal(&Actor::agent("claude"), &p)
        .unwrap();
    let events = b.event_count();
    let again = b
        .store
        .create_proposal(&Actor::agent("claude"), &p)
        .unwrap();
    assert_eq!(first, again);
    assert_eq!(b.event_count(), events);
}

#[test]
fn an_inactive_account_cannot_be_posted_to() {
    let mut b = book();
    let p = b.propose(&b.purchase(d(2026, 6, 1), "Papper", 5410, 100));
    let mut change = chart().into_iter().find(|c| c.number == acc(5410)).unwrap();
    change.active = false;
    b.store
        .change_chart(&b.human, d(2026, 5, 1), &[change], &b.signer)
        .unwrap();
    let err = b.store.approve(&b.human, p, &b.signer).unwrap_err();
    assert!(matches!(err, StoreError::InactiveAccount(..)), "{err}");
    // Före ändringen var kontot aktivt.
    b.post_purchase(d(2026, 4, 30), "Papper i april", 100);
}
