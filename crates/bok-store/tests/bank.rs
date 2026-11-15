//! Bankrader och avstämning: inläsning utan dubbletter, avstämning bara av
//! människa och med rätt belopp, agentens matchningsförslag, kandidater och
//! spärren mot att stänga en period med rader som inte är avstämda.

mod common;

use bok_store::bok_core::PeriodStatus;
use bok_store::{
    Actor, BankImport, Channel, MatchTarget, NewAttachment, NewBankRow, NewMatchSuggestion,
    StoreError, SuggestedMatch,
};
use common::*;

fn post_id(n: u8) -> String {
    format!("{:064x}", u64::from(n) + 1)
}

fn row(n: u8, day: u32, ore: i64, text: &str) -> NewBankRow {
    NewBankRow {
        post_id: post_id(n),
        bank_account: "SE4550000000058398257466".into(),
        booking_date: d(2026, 3, day),
        value_date: Some(d(2026, 3, day)),
        amount: kr(ore),
        currency: "SEK".into(),
        text: text.into(),
        reference: None,
        counterparty: None,
        balance: None,
    }
}

/// Ett mottaget kontoutdrag med tre rader i mars.
fn statement(b: &mut Book) -> i64 {
    let attachment = b
        .store
        .register_attachment(
            &b.human,
            &NewAttachment {
                sha256: "ab".repeat(32),
                size: 1000,
                channel: Channel::Bank,
                sender: None,
                original_name: Some("utdrag-mars.xml".into()),
                file_type: Some("application/xml".into()),
            },
        )
        .unwrap();
    let mut papper = row(1, 10, -12_500, "Kortköp Papper AB");
    papper.reference = Some("Faktura 2026-311".into());
    let rows = [
        papper,
        row(2, 15, -4_000, "Bankavgift"),
        row(3, 20, 100_000, "Insättning från ägaren"),
    ];
    let first = b
        .store
        .import_bank_rows(&b.human, attachment, acc(1930), &rows)
        .unwrap();
    assert_eq!(
        first,
        BankImport {
            new: 3,
            existing: 0
        }
    );
    attachment
}

#[test]
fn import_is_idempotent_and_append_only() {
    let mut b = book();
    let attachment = statement(&mut b);
    let again = b
        .store
        .import_bank_rows(
            &b.human,
            attachment,
            acc(1930),
            &[row(1, 10, -12_500, "x"), row(4, 25, -100, "Ränta")],
        )
        .unwrap();
    assert_eq!(
        again,
        BankImport {
            new: 1,
            existing: 1
        }
    );
    let rows = b.store.bank_rows(false).unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!(
        rows[0].text, "Kortköp Papper AB",
        "den gamla raden är orörd"
    );

    let raw = b.raw();
    assert!(raw.execute("UPDATE bankrader SET belopp = 0", []).is_err());
    assert!(raw.execute("DELETE FROM bankrader", []).is_err());
    assert_eq!(b.store.bank_rows(false).unwrap().len(), 4);
}

#[test]
fn import_needs_a_received_statement_and_a_known_account() {
    let mut b = book();
    let attachment = statement(&mut b);
    let err = b
        .store
        .import_bank_rows(&b.human, 999, acc(1930), &[row(5, 1, 1, "x")])
        .unwrap_err();
    assert!(matches!(err, StoreError::Missing { .. }), "{err}");
    let err = b
        .store
        .import_bank_rows(&b.human, attachment, acc(1940), &[row(5, 1, 1, "x")])
        .unwrap_err();
    assert!(matches!(err, StoreError::UnknownAccount(..)), "{err}");
    let mut bad = row(5, 1, 1, "x");
    bad.post_id = "kort".into();
    assert!(
        b.store
            .import_bank_rows(&b.human, attachment, acc(1930), &[bad])
            .is_err()
    );
}

#[test]
fn reconciliation_needs_a_human_and_the_exact_amount() {
    let mut b = book();
    statement(&mut b);
    let rows = b.store.bank_rows(true).unwrap();
    let (papper, avgift) = (rows[0].id, rows[1].id);
    let wrong = b.post_purchase(d(2026, 3, 9), "Papper", 12_400);
    let right = b.post_purchase(d(2026, 3, 9), "Papper, faktura 2026-311", 12_500);

    let agent = Actor::agent("modell");
    let err = b
        .store
        .reconcile_bank_row(&agent, papper, &MatchTarget::Voucher(right.voucher_id))
        .unwrap_err();
    assert!(matches!(err, StoreError::NotAllowed(_)), "{err}");
    let err = b
        .store
        .reconcile_bank_row(&b.human, papper, &MatchTarget::Voucher(wrong.voucher_id))
        .unwrap_err();
    assert!(err.to_string().contains("rör konto 1930"), "{err}");

    b.store
        .reconcile_bank_row(&b.human, papper, &MatchTarget::Voucher(right.voucher_id))
        .unwrap();
    let err = b
        .store
        .reconcile_bank_row(&b.human, papper, &MatchTarget::Explanation("igen".into()))
        .unwrap_err();
    assert!(err.to_string().contains("redan avstämd"), "{err}");

    // En verifikation kan bara stämma av en rad.
    let mut same = row(9, 11, -12_500, "Kortköp Papper AB igen");
    same.post_id = post_id(9);
    let attachment = b.store.bank_row(papper).unwrap().attachment_id;
    b.store
        .import_bank_rows(&b.human, attachment, acc(1930), &[same])
        .unwrap();
    let other = b
        .store
        .bank_rows(true)
        .unwrap()
        .into_iter()
        .find(|r| r.text.ends_with("igen"))
        .unwrap();
    let err = b
        .store
        .reconcile_bank_row(&b.human, other.id, &MatchTarget::Voucher(right.voucher_id))
        .unwrap_err();
    assert!(
        err.to_string().contains("redan avstämd mot bankrad"),
        "{err}"
    );

    assert!(
        b.store
            .reconcile_bank_row(&b.human, avgift, &MatchTarget::Explanation("  ".into()))
            .is_err()
    );
    b.store
        .reconcile_bank_row(
            &b.human,
            avgift,
            &MatchTarget::Explanation("Bokförs samlat i april".into()),
        )
        .unwrap();
    let done = b.store.bank_row(avgift).unwrap().reconciliation.unwrap();
    assert_eq!(done.explanation.as_deref(), Some("Bokförs samlat i april"));
    assert_eq!(done.actor, "granskare");

    let raw = b.raw();
    assert!(raw.execute("DELETE FROM bankmatchningar", []).is_err());
    assert!(
        raw.execute("UPDATE bankmatchningar SET forklaring = 'x'", [])
            .is_err()
    );
    assert!(b.store.verify().unwrap().is_ok());
}

#[test]
fn candidates_prefer_amount_date_and_reference() {
    let mut b = book();
    statement(&mut b);
    let papper = b.store.bank_rows(true).unwrap()[0].id;
    let far = b.post_purchase(d(2026, 3, 1), "Kontorsmaterial", 12_500);
    let near = b.post_purchase(d(2026, 3, 9), "Papper, faktura 2026-311", 12_500);
    b.post_purchase(d(2026, 3, 10), "Annat belopp", 12_600);
    b.post_purchase(d(2026, 1, 10), "Utanför fönstret", 12_500);

    let candidates = b.store.match_candidates(papper, 14).unwrap();
    let ids: Vec<i64> = candidates.iter().map(|c| c.voucher_id).collect();
    assert_eq!(ids, vec![near.voucher_id, far.voucher_id], "{candidates:?}");
    assert!(candidates[0].score > candidates[1].score);
    assert_eq!(candidates[0].days_apart, -1);

    b.store
        .reconcile_bank_row(&b.human, papper, &MatchTarget::Voucher(near.voucher_id))
        .unwrap();
    let second = b.store.bank_rows(true).unwrap()[0].id;
    assert!(
        b.store
            .match_candidates(second, 14)
            .unwrap()
            .iter()
            .all(|c| c.voucher_id != near.voucher_id),
        "en avstämd verifikation föreslås inte igen"
    );
}

#[test]
fn agent_suggestions_reconcile_nothing() {
    let mut b = book();
    statement(&mut b);
    let rows = b.store.bank_rows(true).unwrap();
    let papper = rows[0].id;
    let insattning = rows[2].id;
    let posted = b.post_purchase(d(2026, 3, 9), "Papper", 12_500);
    let agent = Actor::agent("claude-opus-5-5");
    let suggestion = |target| NewMatchSuggestion {
        target,
        confidence: Some(0.9),
        model: Some("claude-opus-5-5".into()),
        rationale: Some("Samma belopp och dag före.".into()),
    };
    b.store
        .suggest_match(
            &agent,
            papper,
            &suggestion(SuggestedMatch::Voucher(posted.voucher_id)),
        )
        .unwrap();
    let err = b
        .store
        .suggest_match(
            &agent,
            insattning,
            &suggestion(SuggestedMatch::Voucher(posted.voucher_id)),
        )
        .unwrap_err();
    assert!(
        err.to_string().contains("rör konto 1930"),
        "fel belopp föreslås inte: {err}"
    );
    let proposal = b.propose(&b.proposal(
        bok_store::bok_core::Series::A,
        d(2026, 3, 20),
        "Insättning från ägaren",
        vec![
            bok_store::bok_core::voucher::Line::debit(acc(1930), kr(100_000)),
            bok_store::bok_core::voucher::Line::credit(acc(2091), kr(100_000)),
        ],
    ));
    b.store
        .suggest_match(
            &agent,
            insattning,
            &suggestion(SuggestedMatch::Proposal(proposal)),
        )
        .unwrap();

    assert_eq!(b.store.match_suggestions(None).unwrap().len(), 2);
    let for_row = b.store.match_suggestions(Some(papper)).unwrap();
    assert_eq!(
        for_row[0].target,
        SuggestedMatch::Voucher(posted.voucher_id)
    );
    assert_eq!(for_row[0].source, "agent");
    assert_eq!(
        b.store.bank_rows(true).unwrap().len(),
        3,
        "förslag avstämmer ingenting"
    );
}

#[test]
fn a_period_with_unreconciled_bank_rows_cannot_be_closed() {
    let mut b = book();
    statement(&mut b);
    // Bokföringen sker medan perioden är öppen; under avstämning tar den inte emot poster.
    let posted = b.post_purchase(d(2026, 3, 9), "Papper", 12_500);
    let posted_avgift = b.post(&b.purchase(d(2026, 3, 15), "Bankavgift", 6570, 4_000));
    let mar = b.period_id("2026-03");
    b.store
        .transition_period(&b.human, mar, PeriodStatus::UnderReview, None)
        .unwrap();
    let err = b
        .store
        .transition_period(&b.human, mar, PeriodStatus::Closed, Some(&b.signer))
        .unwrap_err();
    match err {
        StoreError::UnreconciledBankRows { period, count } => {
            assert_eq!((period.as_str(), count), ("2026-03", 3));
        }
        other => panic!("fel fel: {other}"),
    }

    let rows = b.store.bank_rows(true).unwrap();
    b.store
        .reconcile_bank_row(
            &b.human,
            rows[0].id,
            &MatchTarget::Voucher(posted.voucher_id),
        )
        .unwrap();
    b.store
        .reconcile_bank_row(
            &b.human,
            rows[1].id,
            &MatchTarget::Voucher(posted_avgift.voucher_id),
        )
        .unwrap();
    b.store
        .reconcile_bank_row(
            &b.human,
            rows[2].id,
            &MatchTarget::Explanation("Ägarens insättning, bokförs i april".into()),
        )
        .unwrap();
    b.store
        .transition_period(&b.human, mar, PeriodStatus::Closed, Some(&b.signer))
        .unwrap();
    assert!(b.store.verify().unwrap().is_ok());

    // En rad som kommer in efter stängningen syns i verifieringen.
    let attachment = b.store.bank_row(rows[0].id).unwrap().attachment_id;
    b.store
        .import_bank_rows(
            &b.human,
            attachment,
            acc(1930),
            &[row(7, 31, -50, "Sen avgift")],
        )
        .unwrap();
    let report = b.store.verify().unwrap();
    assert!(!report.is_ok());
    assert!(
        report
            .problems()
            .any(|p| p.contains("stängd period men är inte avstämd")),
        "{report:?}"
    );
    let err = b
        .store
        .transition_period(&b.human, mar, PeriodStatus::Locked, Some(&b.signer))
        .unwrap_err();
    assert!(
        matches!(err, StoreError::UnreconciledBankRows { count: 1, .. }),
        "{err}"
    );
}

#[test]
fn insert_or_replace_cannot_move_a_reconciliation() {
    let mut b = book();
    statement(&mut b);
    let rows = b.store.bank_rows(true).unwrap();
    let posted = b.post_purchase(d(2026, 3, 9), "Papper", 12_500);
    b.store
        .reconcile_bank_row(
            &b.human,
            rows[0].id,
            &MatchTarget::Voucher(posted.voucher_id),
        )
        .unwrap();
    // Ett annat verktyg försöker flytta avstämningen till en annan rad med REPLACE.
    let raw = b.raw();
    let err = raw.execute(
        "INSERT OR REPLACE INTO bankmatchningar (bankrad_id, verifikation_id, aktor, tid, handelse)
         VALUES (?1, ?2, 'någon', '2026-03-31T00:00:00.000000Z', 1)",
        rusqlite::params![rows[1].id, posted.voucher_id],
    );
    assert!(err.is_err(), "REPLACE gick igenom");
    let first = b.store.bank_row(rows[0].id).unwrap();
    assert_eq!(
        first.reconciliation.unwrap().voucher_id,
        Some(posted.voucher_id)
    );
    assert!(
        b.store
            .bank_row(rows[1].id)
            .unwrap()
            .reconciliation
            .is_none()
    );
}

#[test]
fn foreign_currency_and_zero_rows_are_handled() {
    let mut b = book();
    let attachment = statement(&mut b);
    let mut eur = row(20, 12, -12_500, "Hotell Berlin");
    eur.currency = "EUR".into();
    let err = b
        .store
        .import_bank_rows(&b.human, attachment, acc(1930), &[eur])
        .unwrap_err();
    assert!(err.to_string().contains("bara rader i SEK"), "{err}");

    b.store
        .import_bank_rows(
            &b.human,
            attachment,
            acc(1930),
            &[row(21, 13, 0, "Nollrad")],
        )
        .unwrap();
    let zero = b
        .store
        .bank_rows(true)
        .unwrap()
        .into_iter()
        .find(|r| r.text == "Nollrad")
        .unwrap();
    // En verifikation som inte rör 1930 har nettot 0 men får inte stämma av en nollrad.
    let other = b.post(&b.proposal(
        bok_store::bok_core::Series::A,
        d(2026, 3, 13),
        "Leverantörsskuld",
        vec![
            bok_store::bok_core::voucher::Line::debit(acc(5410), kr(1_000)),
            bok_store::bok_core::voucher::Line::credit(acc(2440), kr(1_000)),
        ],
    ));
    let err = b
        .store
        .reconcile_bank_row(&b.human, zero.id, &MatchTarget::Voucher(other.voucher_id))
        .unwrap_err();
    assert!(err.to_string().contains("0 kr"), "{err}");
    assert!(b.store.match_candidates(zero.id, 14).unwrap().is_empty());
    b.store
        .reconcile_bank_row(
            &b.human,
            zero.id,
            &MatchTarget::Explanation("Nollrad från banken".into()),
        )
        .unwrap();

    // En verifikation som inte rör bankens konto avvisas med ett tydligt fel.
    let papper = b.store.bank_rows(true).unwrap()[0].id;
    let err = b
        .store
        .reconcile_bank_row(&b.human, papper, &MatchTarget::Voucher(other.voucher_id))
        .unwrap_err();
    assert!(err.to_string().contains("rör inte konto 1930"), "{err}");
}
