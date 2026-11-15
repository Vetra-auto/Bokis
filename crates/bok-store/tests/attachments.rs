//! Gapanalysen: `upload_attachment` sparade typen "receipt", som bröt mot
//! databasens CHECK-villkor, så varje uppladdning misslyckades. Här tas
//! underlaget emot, länkas vid bokföring och får statusen "bokförd som A1"
//! utan att något villkor bryts.

mod common;

use bok_store::bok_core::canonical::sha256_hex;
use bok_store::{Channel, NewAttachment, StoreError};
use common::*;

fn receipt(bytes: &[u8]) -> NewAttachment {
    NewAttachment {
        sha256: sha256_hex(bytes),
        size: bytes.len() as i64,
        channel: Channel::Manual,
        sender: Some("leverantor@example.se".into()),
        original_name: Some("kvitto.pdf".into()),
        file_type: Some("application/pdf".into()),
    }
}

#[test]
fn an_attachment_is_received_linked_and_marked_as_posted() {
    let mut b = book();
    let id = b
        .store
        .register_attachment(&b.human, &receipt(b"%PDF kvitto"))
        .unwrap();
    let listed = b.store.attachments().unwrap();
    assert_eq!(listed[0].status, "mottagen");

    let mut p = b.purchase(d(2026, 3, 2), "Papper", 5410, 10000);
    p.attachments = vec![id];
    let posted = b.post(&p);
    let listed = b.store.attachments().unwrap();
    assert_eq!(listed[0].status, "bokford");
    assert_eq!(
        listed[0].status_text.as_deref(),
        Some("bokförd som A1 (2026)")
    );
    let view = b.store.voucher(posted.voucher_id).unwrap();
    assert_eq!(view.attachments, vec![sha256_hex(b"%PDF kvitto")]);
    assert!(
        b.store.verify().unwrap().is_ok(),
        "underlagets hash ingår i den signerade formen"
    );
}

#[test]
fn the_same_file_is_not_received_twice() {
    let mut b = book();
    let id = b
        .store
        .register_attachment(&b.human, &receipt(b"samma"))
        .unwrap();
    let err = b
        .store
        .register_attachment(&b.human, &receipt(b"samma"))
        .unwrap_err();
    assert!(
        matches!(err, StoreError::DuplicateAttachment(existing) if existing == id),
        "{err}"
    );
}

#[test]
fn a_proposal_cannot_point_at_a_missing_attachment() {
    let mut b = book();
    let mut p = b.purchase(d(2026, 3, 2), "Papper", 5410, 10000);
    p.attachments = vec![42];
    assert!(matches!(
        b.store.create_proposal(&b.human, &p),
        Err(StoreError::Missing { .. })
    ));
}

#[test]
fn a_bad_hash_is_refused() {
    let mut b = book();
    let mut a = receipt(b"x");
    a.sha256 = "inte-en-hash".into();
    assert!(matches!(
        b.store.register_attachment(&b.human, &a),
        Err(StoreError::Invalid(_))
    ));
}

#[test]
fn remote_access_changes_are_logged_and_only_known_ones() {
    let mut b = book();
    let before = b.event_count();
    b.store
        .record_remote_access(
            &b.human,
            "fjarr.anslutning.godkand",
            "Claude",
            serde_json::json!({"klient": "Claude", "omdirigering": "claude.ai"}),
        )
        .unwrap();
    assert_eq!(b.event_count(), before + 1);
    let last = b.store.events(Some(1)).unwrap().remove(0);
    assert_eq!(last.action, "fjarr.anslutning.godkand");
    assert_eq!(last.object.as_deref(), Some("fjarr:Claude"));
    assert!(
        b.store
            .record_remote_access(&b.human, "verifikation.bokford", "x", serde_json::json!({}))
            .is_err()
    );
    assert!(
        b.store
            .record_remote_access(&b.human, "fjarr.lank.skapad", "  ", serde_json::json!({}))
            .is_err()
    );
    assert!(b.store.verify().unwrap().is_ok());
}
