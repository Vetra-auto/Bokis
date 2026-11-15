//! Ärenden: agentens frågor och flaggor ändrar ingenting förrän granskaren
//! har svarat, och bara svaret kan ändra ett underlags status.

mod common;

use bok_store::{Actor, AttachmentVerdict, CaseKind, Channel, NewAttachment, NewCase, StoreError};
use common::*;

fn attachment(b: &mut Book, n: u8) -> i64 {
    b.store
        .register_attachment(
            &Actor::system("inkorg"),
            &NewAttachment {
                sha256: format!("{n:02x}").repeat(32),
                size: 10,
                channel: Channel::Folder,
                sender: None,
                original_name: Some(format!("fil-{n}.pdf")),
                file_type: Some("application/pdf".into()),
            },
        )
        .unwrap()
}

fn flag(attachment: i64, reason: &str) -> NewCase {
    NewCase {
        kind: CaseKind::Flag {
            reason: reason.into(),
        },
        attachment_id: Some(attachment),
        text: "Samma faktura som fil-1".into(),
        model: Some("claude-opus-5-5".into()),
        confidence: Some(0.8),
    }
}

fn status(b: &Book, id: i64) -> String {
    b.store
        .attachments()
        .unwrap()
        .into_iter()
        .find(|a| a.id == id)
        .unwrap()
        .status
}

#[test]
fn a_flag_changes_nothing_until_the_reviewer_answers() {
    let mut b = book();
    let first = attachment(&mut b, 1);
    let copy = attachment(&mut b, 2);
    let agent = Actor::agent("claude-opus-5-5");
    let case = b.store.open_case(&agent, &flag(copy, "dubblett")).unwrap();
    assert_eq!(
        status(&b, copy),
        "mottagen",
        "flaggan ändrar ingenting själv"
    );
    assert_eq!(b.store.cases(true).unwrap().len(), 1);

    let err = b
        .store
        .answer_case(&agent, case, "ja", Some(AttachmentVerdict::Duplicate))
        .unwrap_err();
    assert!(matches!(err, StoreError::NotAllowed(_)), "{err}");

    b.store
        .answer_case(
            &b.human,
            case,
            "Stämmer, samma som underlag 1",
            Some(AttachmentVerdict::Duplicate),
        )
        .unwrap();
    assert_eq!(status(&b, copy), "dubblett");
    assert_eq!(status(&b, first), "mottagen");
    assert!(b.store.cases(true).unwrap().is_empty());
    let answered = b.store.case(case).unwrap();
    assert_eq!(answered.answer.unwrap().status.as_deref(), Some("dubblett"));
    let err = b
        .store
        .answer_case(&b.human, case, "igen", None)
        .unwrap_err();
    assert!(err.to_string().contains("redan besvarat"), "{err}");

    let raw = b.raw();
    assert!(raw.execute("DELETE FROM arenden", []).is_err());
    assert!(raw.execute("UPDATE arendesvar SET svar = 'x'", []).is_err());
    assert!(b.store.verify().unwrap().is_ok());
}

#[test]
fn questions_flags_and_their_limits() {
    let mut b = book();
    let doc = attachment(&mut b, 3);
    let agent = Actor::agent("modell");
    let question = b
        .store
        .open_case(
            &agent,
            &NewCase {
                kind: CaseKind::Question,
                attachment_id: Some(doc),
                text: "Vilka deltog i luncherna?".into(),
                model: None,
                confidence: None,
            },
        )
        .unwrap();
    // En fråga kan inte ändra en status.
    let err = b
        .store
        .answer_case(
            &b.human,
            question,
            "Kund X",
            Some(AttachmentVerdict::Duplicate),
        )
        .unwrap_err();
    assert!(
        err.to_string().contains("gäller inget underlag") || err.to_string().contains("status"),
        "{err}"
    );
    b.store
        .answer_case(&b.human, question, "Kund X och jag", None)
        .unwrap();

    assert!(b.store.open_case(&agent, &flag(doc, "spam")).is_err());
    let mut no_doc = flag(doc, "dubblett");
    no_doc.attachment_id = None;
    assert!(b.store.open_case(&agent, &no_doc).is_err());
    let mut empty = flag(doc, "dubblett");
    empty.text = " ".into();
    assert!(b.store.open_case(&agent, &empty).is_err());
    assert!(matches!(
        b.store.open_case(&agent, &flag(999, "dubblett")),
        Err(StoreError::Missing { .. })
    ));

    // Ett bokfört underlag kan inte markeras som ej räkenskapsinformation.
    let mut p = b.purchase(d(2026, 3, 16), "Papper", 5410, 10_000);
    p.attachments = vec![doc];
    b.post(&p);
    let case = b
        .store
        .open_case(&agent, &flag(doc, "ej_rakenskapsinformation"))
        .unwrap();
    let err = b
        .store
        .answer_case(
            &b.human,
            case,
            "Nej",
            Some(AttachmentVerdict::NotAccountingInformation),
        )
        .unwrap_err();
    assert!(err.to_string().contains("bokford"), "{err}");
    assert_eq!(status(&b, doc), "bokford");
}

#[test]
fn a_ruled_duplicate_cannot_be_booked() {
    let mut b = book();
    let copy = attachment(&mut b, 5);
    // Ett förslag på underlaget läggs innan granskaren hunnit besluta om flaggan.
    let mut early = b.purchase(d(2026, 3, 16), "Faktura", 5410, 10_000);
    early.attachments = vec![copy];
    let early = b.propose(&early);
    let agent = Actor::agent("modell");
    let case = b.store.open_case(&agent, &flag(copy, "dubblett")).unwrap();
    b.store
        .answer_case(
            &b.human,
            case,
            "Samma som förra veckan",
            Some(AttachmentVerdict::Duplicate),
        )
        .unwrap();

    let err = b.store.approve(&b.human, early, &b.signer).unwrap_err();
    assert!(err.to_string().contains("markerat som dubblett"), "{err}");
    let mut late = b.purchase(d(2026, 3, 17), "Faktura igen", 5410, 10_000);
    late.attachments = vec![copy];
    let err = b.store.create_proposal(&agent, &late).unwrap_err();
    assert!(err.to_string().contains("markerat som dubblett"), "{err}");
    assert_eq!(status(&b, copy), "dubblett");
}
