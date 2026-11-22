//! Säkerhetsnivån: bokföringen godtar bara signaturer som klarar nivån. Den
//! som höjer nivån signerar med en nyckel för den nya nivån, den som sänker
//! den med en nyckel för den gamla, och `verify` prövar hela historiken.

mod common;

use bok_store::bok_core::security::{LevelError, SecurityLevel};
use bok_store::bok_core::signature::{
    SK_FLAG_USER_PRESENCE, SK_FLAG_USER_VERIFIED, Signer, SimulatedHardwareSigner, SoftwareSigner,
};
use bok_store::{Actor, NewBook, SECURITY_LEVEL_DECIDED, Store, StoreError};
use common::*;

const TOUCH: u8 = SK_FLAG_USER_PRESENCE;
const PIN: u8 = SK_FLAG_USER_PRESENCE | SK_FLAG_USER_VERIFIED;

/// Samma simulerade YubiKey varje gång; flaggorna säger om PIN angavs.
fn yubikey(flags: u8) -> SimulatedHardwareSigner {
    SimulatedHardwareSigner::new([9; 32], flags)
}

fn problems(b: &Book) -> String {
    b.store
        .verify()
        .unwrap()
        .problems()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
}

fn warnings(store: &Store) -> String {
    store
        .verify()
        .unwrap()
        .checks
        .into_iter()
        .flat_map(|c| c.warnings)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Registrerar YubiKeyn, signerad av bokens nyckelfil.
fn add_yubikey(b: &mut Book) {
    b.store
        .add_key(
            &b.human,
            "yubikey",
            &yubikey(TOUCH).public_key_openssh(),
            &b.signer,
        )
        .unwrap();
}

fn create(
    dir: &tempfile::TempDir,
    level: SecurityLevel,
    signer: &dyn Signer,
    public_key: &str,
) -> Result<Store, StoreError> {
    Store::create(
        &dir.path().join("bokis.db"),
        "test",
        NewBook {
            company_name: "Nivåbolaget AB",
            org_number: "559999-0008",
            key_name: "granskare",
            public_key,
            signer,
            security_level: level,
        },
    )
}

#[test]
fn a_new_book_records_its_level_and_verify_explains_it() {
    let b = book();
    let decisions = b.store.security_decisions().unwrap();
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].level, SecurityLevel::Software);
    assert_eq!(decisions[0].previous, None);
    assert_eq!(decisions[0].actor, "granskare");
    assert_eq!(b.store.security_level().unwrap(), SecurityLevel::Software);
    let events = b.store.events(None).unwrap();
    assert_eq!(events[1].action, SECURITY_LEVEL_DECIDED);
    assert_eq!(problems(&b), "");
    let w = warnings(&b.store);
    assert!(w.contains("säkerhetsnivån är mjukvarunyckel sedan"), "{w}");
}

#[test]
fn a_hardware_book_needs_a_hardware_key_from_the_start() {
    let dir = tempfile::tempdir().unwrap();
    let file = SoftwareSigner::from_seed([1; 32]);
    let e = create(
        &dir,
        SecurityLevel::Hardware,
        &file,
        &file.public_key_openssh(),
    )
    .err()
    .unwrap();
    assert!(e.to_string().contains("kräver en hårdvarunyckel"), "{e}");
    assert!(!dir.path().join("bokis.db").exists());

    // Hårdvara med PIN kräver PIN redan när bokföringen skapas.
    let touch = yubikey(TOUCH);
    let e = create(
        &dir,
        SecurityLevel::HardwarePin,
        &touch,
        &touch.public_key_openssh(),
    )
    .err()
    .unwrap();
    assert!(
        matches!(e, StoreError::SecurityLevel(LevelError::NoPin)),
        "{e}"
    );
    assert!(!dir.path().join("bokis.db").exists());

    let pin = yubikey(PIN);
    let mut store = create(
        &dir,
        SecurityLevel::HardwarePin,
        &pin,
        &pin.public_key_openssh(),
    )
    .unwrap();
    assert_eq!(store.security_level().unwrap(), SecurityLevel::HardwarePin);
    let human = Actor::human("granskare");
    store
        .change_chart(&human, d(2025, 1, 1), &chart(), &pin)
        .unwrap();
    assert!(matches!(
        store.change_chart(&human, d(2026, 1, 1), &chart(), &touch),
        Err(StoreError::SecurityLevel(LevelError::NoPin))
    ));
    assert!(store.verify().unwrap().is_ok());
    assert_eq!(warnings(&store), "");
}

#[test]
fn raising_needs_a_key_for_the_new_level_and_lowering_one_for_the_old() {
    let mut b = book();
    add_yubikey(&mut b);
    let key = yubikey(TOUCH);

    // Nyckelfilen kan inte höja nivån: den klarar inte den nya.
    let e = b
        .store
        .set_security_level(&b.human, SecurityLevel::Hardware, &b.signer)
        .unwrap_err();
    assert!(
        matches!(
            e,
            StoreError::SecurityLevel(LevelError::SoftwareKey(SecurityLevel::Hardware))
        ),
        "{e}"
    );
    let raised = b
        .store
        .set_security_level(&b.human, SecurityLevel::Hardware, &key)
        .unwrap();
    assert_eq!(raised.previous, Some(SecurityLevel::Software));

    // Nyckelfilen kan inte längre bokföra, men YubiKeyn kan.
    let p = b.purchase(d(2026, 3, 1), "Papper", 5410, 10000);
    let id = b.propose(&p);
    let e = b.store.approve(&b.human, id, &b.signer).unwrap_err();
    assert!(matches!(e, StoreError::SecurityLevel(_)), "{e}");
    b.store.approve(&b.human, id, &key).unwrap();

    // Nyckelfilen kan inte heller sänka nivån: den klarar inte den gamla.
    assert!(matches!(
        b.store
            .set_security_level(&b.human, SecurityLevel::Software, &b.signer),
        Err(StoreError::SecurityLevel(LevelError::SoftwareKey(
            SecurityLevel::Hardware
        )))
    ));
    assert!(matches!(
        b.store
            .set_security_level(&b.human, SecurityLevel::Hardware, &key),
        Err(StoreError::Invalid(_))
    ));
    b.store
        .set_security_level(&b.human, SecurityLevel::Software, &key)
        .unwrap();
    let levels: Vec<_> = b
        .store
        .security_decisions()
        .unwrap()
        .iter()
        .map(|d| (d.previous, d.level))
        .collect();
    assert_eq!(
        levels,
        [
            (None, SecurityLevel::Software),
            (Some(SecurityLevel::Software), SecurityLevel::Hardware),
            (Some(SecurityLevel::Hardware), SecurityLevel::Software),
        ]
    );
    // Efter sänkningen godtas nyckelfilen igen.
    b.post_purchase(d(2026, 3, 2), "Pennor", 2500);
    assert_eq!(problems(&b), "");
}

#[test]
fn the_pin_level_needs_the_pin_on_every_signature() {
    let mut b = book();
    add_yubikey(&mut b);
    assert!(matches!(
        b.store
            .set_security_level(&b.human, SecurityLevel::HardwarePin, &yubikey(TOUCH)),
        Err(StoreError::SecurityLevel(LevelError::NoPin))
    ));
    b.store
        .set_security_level(&b.human, SecurityLevel::HardwarePin, &yubikey(PIN))
        .unwrap();
    let p = b.purchase(d(2026, 3, 1), "Papper", 5410, 10000);
    let id = b.propose(&p);
    assert!(matches!(
        b.store.approve(&b.human, id, &yubikey(TOUCH)),
        Err(StoreError::SecurityLevel(LevelError::NoPin))
    ));
    b.store.approve(&b.human, id, &yubikey(PIN)).unwrap();
    assert_eq!(problems(&b), "");
    assert!(!warnings(&b.store).contains("säkerhetsnivån är mjukvarunyckel"));
}

#[test]
fn a_key_file_is_not_registered_at_a_hardware_level_but_a_spare_yubikey_is() {
    let mut b = book();
    add_yubikey(&mut b);
    let key = yubikey(TOUCH);
    b.store
        .set_security_level(&b.human, SecurityLevel::Hardware, &key)
        .unwrap();
    let file = SoftwareSigner::from_seed([11; 32]);
    let e = b
        .store
        .add_key(&b.human, "fil", &file.public_key_openssh(), &key)
        .unwrap_err();
    assert!(e.to_string().contains("mjukvarunyckel"), "{e}");
    let spare = SimulatedHardwareSigner::new([12; 32], TOUCH);
    b.store
        .add_key(&b.human, "reserv", &spare.public_key_openssh(), &key)
        .unwrap();
    b.store
        .set_security_level(&b.human, SecurityLevel::Software, &spare)
        .unwrap();
}

#[test]
fn an_agent_cannot_change_the_level() {
    let mut b = book();
    add_yubikey(&mut b);
    assert!(matches!(
        b.store.set_security_level(
            &Actor::agent("claude"),
            SecurityLevel::Hardware,
            &yubikey(TOUCH)
        ),
        Err(StoreError::NotAllowed(_))
    ));
}

#[test]
fn verify_finds_a_signature_below_the_level_in_force() {
    let mut b = book();
    b.post_purchase(d(2026, 3, 1), "Papper", 10000);
    let raw = b.raw();
    for t in raw
        .prepare("SELECT name FROM sqlite_master WHERE type = 'trigger'")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
    {
        raw.execute(&format!("DROP TRIGGER {}", t.unwrap()), [])
            .unwrap();
    }
    // Som om bokföringen hade skapats på hårdvarunivå.
    raw.execute("UPDATE sakerhetsniva SET niva = 'hardvara'", [])
        .unwrap();
    let p = problems(&b);
    assert!(p.contains("stämmer inte med sitt signerade beslut"), "{p}");
    assert!(
        p.contains("men signaturen kommer från en mjukvarunyckel"),
        "{p}"
    );
}

#[test]
fn a_book_from_before_the_levels_accepts_every_key() {
    let b = book();
    let Book {
        store, path, dir, ..
    } = b;
    drop(store);
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute_batch(
        &format!(
            "{DROP_V5} DROP TABLE inventarieavgangar; DROP TABLE inventarier; DROP TABLE sakerhetsniva; \
             PRAGMA user_version = 2;"
        ),
    )
        .unwrap();
    drop(raw);

    let store = Store::open(&path, "test").unwrap();
    let version: i64 = rusqlite::Connection::open(&path)
        .unwrap()
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, bok_store::SCHEMA_VERSION);
    assert!(store.security_decisions().unwrap().is_empty());
    assert_eq!(store.security_level().unwrap(), SecurityLevel::Software);
    let w = warnings(&store);
    assert!(
        w.contains("eftersom bokföringen skapades före nivåerna"),
        "{w}"
    );
    drop(dir);
}
