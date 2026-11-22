//! Gapanalysen: `restore_backup` och `restore_from_file` skrev över den
//! levande databasen. Nu skapar `Store::create` bara nya filer, och
//! `Store::open` skapar eller skriver aldrig om en fil som inte är en
//! Bokis-databas. (Att CLI:t saknar kommandon som återställer, nollställer
//! eller raderar prövas i bok-cli.)

mod common;

use bok_store::bok_core::signature::{SignatureError, Signer, SoftwareSigner};
use bok_store::{NewBook, Store, StoreError};
use common::*;

fn new_book<'a>(signer: &'a dyn Signer, public_key: &'a str) -> NewBook<'a> {
    NewBook {
        company_name: "Nytt AB",
        org_number: "559000-0005",
        key_name: "granskare",
        public_key,
        signer,
        security_level: bok_store::bok_core::security::SecurityLevel::Software,
    }
}

#[test]
fn create_refuses_an_existing_database_and_leaves_it_untouched() {
    let mut b = book();
    b.post_purchase(d(2026, 3, 1), "Papper", 100);
    let before = b.store.journal(b.year.id).unwrap();
    let signer = SoftwareSigner::from_seed([5; 32]);
    let pk = signer.public_key_openssh();
    let err = Store::create(&b.path, "test", new_book(&signer, &pk))
        .err()
        .unwrap();
    assert!(matches!(err, StoreError::AlreadyExists(_)), "{err}");
    let reopened = Store::open(&b.path, "test").unwrap();
    assert_eq!(reopened.journal(b.year.id).unwrap(), before);
    assert_eq!(reopened.company().unwrap().name, "Testbolaget AB");
}

#[test]
fn create_refuses_any_existing_file_or_leftover_wal() {
    let dir = tempfile::tempdir().unwrap();
    let signer = SoftwareSigner::from_seed([5; 32]);
    let pk = signer.public_key_openssh();

    let plain = dir.path().join("viktig.txt");
    std::fs::write(&plain, b"inte en databas").unwrap();
    assert!(matches!(
        Store::create(&plain, "test", new_book(&signer, &pk)),
        Err(StoreError::AlreadyExists(_))
    ));
    assert_eq!(std::fs::read(&plain).unwrap(), b"inte en databas");

    let fresh = dir.path().join("ny.db");
    std::fs::write(dir.path().join("ny.db-wal"), b"gammal wal").unwrap();
    assert!(matches!(
        Store::create(&fresh, "test", new_book(&signer, &pk)),
        Err(StoreError::AlreadyExists(_))
    ));
    assert!(!fresh.exists());
}

#[test]
fn open_never_creates_or_rewrites_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("saknas.db");
    assert!(matches!(
        Store::open(&missing, "test"),
        Err(StoreError::NotFound(_))
    ));
    assert!(!missing.exists());

    let other = dir.path().join("annan.db");
    std::fs::write(&other, b"").unwrap();
    assert!(matches!(
        Store::open(&other, "test"),
        Err(StoreError::NotABokisDatabase(_))
    ));
    assert_eq!(
        std::fs::metadata(&other).unwrap().len(),
        0,
        "filen lämnades orörd"
    );
}

struct BrokenSigner;
impl Signer for BrokenSigner {
    fn sign(&self, _: &str, _: &[u8]) -> Result<String, SignatureError> {
        Err(SignatureError::SigningFailed("ingen nyckel".into()))
    }
}

#[test]
fn a_failed_create_leaves_nothing_behind() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bokis.db");
    let pk = SoftwareSigner::from_seed([5; 32]).public_key_openssh();
    let err = Store::create(&path, "test", new_book(&BrokenSigner, &pk))
        .err()
        .unwrap();
    assert!(matches!(err, StoreError::Signature(_)), "{err}");
    assert!(!path.exists());
    // Ett nytt försök går att göra.
    let signer = SoftwareSigner::from_seed([5; 32]);
    Store::create(&path, "test", new_book(&signer, &pk)).unwrap();
}

#[test]
fn the_first_key_must_sign_its_own_registration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bokis.db");
    let pk = SoftwareSigner::from_seed([5; 32]).public_key_openssh();
    let other = SoftwareSigner::from_seed([6; 32]);
    let err = Store::create(&path, "test", new_book(&other, &pk))
        .err()
        .unwrap();
    assert!(
        matches!(err, StoreError::Signature(SignatureError::WrongKey)),
        "{err}"
    );
    assert!(!path.exists());
}
