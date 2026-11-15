//! Gapanalysen: inga transaktioner och inga triggers. Nu vägrar databasen
//! själv att ändra eller radera det bokförda, även för den som skriver förbi
//! programmet med en egen anslutning.

mod common;

use bok_store::bok_core::canonical::sha256_hex;
use bok_store::{Channel, NewAttachment};
use common::*;

/// En bok med minst en rad i varje tabell som bara får växa.
fn filled() -> Book {
    let mut b = book();
    let attachment = b
        .store
        .register_attachment(
            &b.human,
            &NewAttachment {
                sha256: sha256_hex(b"kvitto"),
                size: 6,
                channel: Channel::Manual,
                sender: None,
                original_name: Some("kvitto.pdf".into()),
                file_type: Some("application/pdf".into()),
            },
        )
        .unwrap();
    let mut p = b.purchase(d(2026, 3, 2), "Papper", 5410, 10000);
    p.attachments = vec![attachment];
    let posted = b.post(&p);
    let rejected = b.propose(&b.purchase(d(2026, 3, 3), "Fel", 5410, 1));
    b.store.reject(&b.human, rejected, "fel belopp").unwrap();
    let correction = b
        .store
        .propose_correction(&b.human, posted.voucher_id, "fel konto", &[])
        .unwrap();
    b.approve(correction);
    b
}

fn assert_refused(conn: &rusqlite::Connection, sql: &str, needle: &str) {
    match conn.execute(sql, []) {
        Ok(n) => panic!("databasen tog emot: {sql} ({n} rader)"),
        Err(e) => assert!(e.to_string().contains(needle), "{sql}: {e}"),
    }
}

#[test]
fn posted_vouchers_cannot_be_changed_or_deleted() {
    let b = filled();
    let raw = b.raw();
    assert_refused(
        &raw,
        "UPDATE verifikationer SET beskrivning = 'ändrad' WHERE id = 1",
        "kan inte ändras",
    );
    assert_refused(
        &raw,
        "UPDATE verifikationer SET nummer = 99 WHERE id = 1",
        "kan inte ändras",
    );
    assert_refused(
        &raw,
        "DELETE FROM verifikationer WHERE id = 1",
        "kan inte raderas",
    );
    assert_refused(&raw, "DELETE FROM verifikationer", "kan inte raderas");
    assert_refused(
        &raw,
        "INSERT OR REPLACE INTO verifikationer SELECT * FROM verifikationer WHERE id = 1",
        "kan inte ersättas",
    );
}

#[test]
fn lines_cannot_be_changed_deleted_or_added_afterwards() {
    let b = filled();
    let raw = b.raw();
    assert_refused(
        &raw,
        "UPDATE rader SET debet = 1 WHERE id = 1",
        "kan inte ändras",
    );
    assert_refused(
        &raw,
        "UPDATE rader SET konto = 6570 WHERE id = 1",
        "kan inte ändras",
    );
    assert_refused(&raw, "DELETE FROM rader WHERE id = 1", "kan inte raderas");
    assert_refused(
        &raw,
        "INSERT INTO rader (verifikation_id, radnr, konto, debet, kredit) VALUES (1, 3, 6570, 100, 0)",
        "i bokföringstransaktionen",
    );
    assert_refused(
        &raw,
        "INSERT OR REPLACE INTO rader (verifikation_id, radnr, konto, debet, kredit) VALUES (1, 1, 6570, 100, 0)",
        "i bokföringstransaktionen",
    );
}

#[test]
fn history_cannot_be_changed_or_deleted() {
    let b = filled();
    let raw = b.raw();
    assert_refused(
        &raw,
        "UPDATE handelser SET aktor = 'någon annan' WHERE seq = 1",
        "kan bara växa",
    );
    assert_refused(
        &raw,
        "DELETE FROM handelser WHERE seq = (SELECT MAX(seq) FROM handelser)",
        "kan bara växa",
    );
    assert_refused(
        &raw,
        "INSERT INTO handelser (seq, tid, aktor_typ, aktor, roll, handling, programversion, foregaende_hash, hash)
         VALUES (1, '2026-01-01T00:00:00Z', 'system', 'x', 'x', 'x', 'x', printf('%064d', 0), printf('%064d', 1))",
        "nästa nummer i följden",
    );
    assert_refused(
        &raw,
        "INSERT INTO handelser (seq, tid, aktor_typ, aktor, roll, handling, programversion, foregaende_hash, hash)
         VALUES ((SELECT MAX(seq) + 1 FROM handelser), '2026-01-01T00:00:00Z', 'system', 'x', 'x', 'x', 'x',
                 printf('%064d', 0), printf('%064d', 1))",
        "föregående hash",
    );
}

#[test]
fn attachments_cannot_be_changed_or_deleted() {
    let b = filled();
    let raw = b.raw();
    assert_refused(
        &raw,
        "UPDATE underlag SET originalnamn = 'annat.pdf'",
        "kan inte ändras",
    );
    assert_refused(&raw, "DELETE FROM underlag", "kan inte raderas");
    assert_refused(
        &raw,
        "UPDATE underlag_status SET text = 'ändrad'",
        "kan bara växa",
    );
    assert_refused(&raw, "DELETE FROM underlag_status", "kan bara växa");
    assert_refused(
        &raw,
        "INSERT OR REPLACE INTO underlag (sha256, storlek, mottagen, kanal)
         SELECT sha256, 0, mottagen, kanal FROM underlag WHERE id = 1",
        "redan mottagen",
    );
}

#[test]
fn decisions_corrections_proposals_and_counters_only_grow() {
    let b = filled();
    let raw = b.raw();
    assert_refused(
        &raw,
        "UPDATE beslut SET orsak = 'annan orsak'",
        "kan inte ändras",
    );
    assert_refused(&raw, "DELETE FROM beslut", "kan inte raderas");
    assert_refused(
        &raw,
        "UPDATE rattelser SET orsak = 'annan'",
        "kan bara växa",
    );
    assert_refused(&raw, "DELETE FROM rattelser", "kan bara växa");
    assert_refused(
        &raw,
        "UPDATE forslag SET beskrivning = 'ändrad'",
        "ändras inte",
    );
    assert_refused(&raw, "DELETE FROM forslag", "kan inte raderas");
    assert_refused(
        &raw,
        "UPDATE nummerserier SET nasta = 1 WHERE serie = 'A'",
        "räknas upp med ett",
    );
    assert_refused(
        &raw,
        "UPDATE nummerserier SET nasta = nasta + 2 WHERE serie = 'A'",
        "räknas upp med ett",
    );
    assert_refused(&raw, "DELETE FROM nummerserier", "kan inte raderas");
    assert_refused(
        &raw,
        "UPDATE kontoplan SET namn = 'annat'",
        "signerad version",
    );
    assert_refused(&raw, "DELETE FROM kontoplan", "signerad version");
    assert_refused(&raw, "UPDATE nycklar SET namn = 'annan'", "kan inte ändras");
    assert_refused(&raw, "DELETE FROM nycklar", "kan inte raderas");
    assert_refused(&raw, "DELETE FROM rakenskapsar", "kan inte raderas");
    assert_refused(&raw, "UPDATE foretag SET namn = 'annat'", "kan bara växa");
}

#[test]
fn rules_only_grow() {
    let b = filled();
    let raw = b.raw();
    // Regler läggs till genom signerade systemändringar från milstolpe 3;
    // tabellen vägrar redan nu ändring och radering.
    raw.execute(
        "INSERT INTO regler (namn, version, giltig_fran, innehall, andring) VALUES ('moms', 1, '2026-01-01', '{}', 1)",
        [],
    )
    .unwrap();
    assert_refused(
        &raw,
        "UPDATE regler SET innehall = '{\"x\":1}'",
        "signerad version",
    );
    assert_refused(&raw, "DELETE FROM regler", "signerad version");
    assert_refused(
        &raw,
        "INSERT OR REPLACE INTO regler (namn, version, giltig_fran, innehall, andring) VALUES ('moms', 1, '2026-01-01', '{}', 1)",
        "kan inte ersättas",
    );
}

#[test]
fn all_writes_still_verify_after_the_refusals() {
    let b = filled();
    let raw = b.raw();
    let _ = raw.execute("DELETE FROM verifikationer", []);
    let _ = raw.execute("UPDATE rader SET debet = 1", []);
    let report = b.store.verify().unwrap();
    assert!(
        report.is_ok(),
        "{:?}",
        report.problems().collect::<Vec<_>>()
    );
    assert_eq!(b.store.journal(b.year.id).unwrap().len(), 2);
}
