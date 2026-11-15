//! `bok` mot en historik vars senaste händelse ligger före datorns klocka:
//! inget skrivs förrän klockan går rätt igen eller en människa godtar det.

use std::path::Path;
use std::process::{Command, Output};

use chrono::{Duration, Utc};

use bok_store::bok_core::signature::SoftwareSigner;
use bok_store::{NewBook, Store};

fn bok(db: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bok"))
        .args(args)
        .env("BOKIS_DB", db)
        .env("BOKIS_USER", "henrik")
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn a_history_ahead_of_the_clock_stops_bok_until_it_is_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("bokis.db");
    let signer = SoftwareSigner::from_seed([7; 32]);
    // Bokföringen skapades när datorns klocka gick ett dygn före.
    let ahead = Utc::now() + Duration::days(1);
    Store::create_with_clock(
        &db,
        "test",
        NewBook {
            company_name: "Klockbolaget AB",
            org_number: "559999-0003",
            key_name: "granskare",
            public_key: &signer.public_key_openssh(),
            signer: &signer,
            security_level: bok_store::bok_core::security::SecurityLevel::Software,
        },
        Box::new(move || ahead),
    )
    .unwrap();

    let refused = bok(&db, &["ar", "lagg-till", "2026"]);
    assert!(!refused.status.success());
    let err = text(&refused.stderr);
    assert!(err.contains("ställ klockan rätt"), "{err}");
    assert!(err.contains("bok klocka godta"), "{err}");

    let accepted = bok(
        &db,
        &[
            "klocka",
            "godta",
            "--reason",
            "databasen skapades med klockan ett dygn före",
        ],
    );
    assert!(accepted.status.success(), "{}", text(&accepted.stderr));
    assert!(text(&accepted.stdout).starts_with("Godtaget."));

    let added = bok(&db, &["ar", "lagg-till", "2026"]);
    assert!(added.status.success(), "{}", text(&added.stderr));
    let verified = bok(&db, &["verify"]);
    assert!(verified.status.success(), "{}", text(&verified.stdout));
    let history = text(&bok(&db, &["historik"]).stdout);
    assert!(history.contains("klocka.godtagen"), "{history}");
}
