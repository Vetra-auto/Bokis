//! Signaturer med ed25519 i OpenSSH:s SSHSIG-format.
//!
//! I skarp drift signerar `ssh-keygen -Y sign`, med en ed25519-sk-nyckel
//! (till exempel en YubiKey) som kräver fysisk beröring, eller med en vanlig
//! ed25519-nyckelfil. Vilka av dem bokföringen godtar bestäms av
//! säkerhetsnivån ([`crate::security`]). Kontrollen görs här med `ssh-key`,
//! utan I/O. För hårdvarunycklar krävs alltid att nyckeln intygade beröring.

use ed25519_dalek::Signer as _;
use sha2::{Digest, Sha256};
use ssh_key::private::Ed25519Keypair;
use ssh_key::public::{Ed25519PublicKey, KeyData, SkEd25519};
use ssh_key::{Algorithm, HashAlg, LineEnding, PrivateKey, PublicKey, SshSig};
use thiserror::Error;

/// Namnrymd för signaturen över en verifikations kanoniska hash.
pub const NS_VOUCHER: &str = "bokis-verifikation";
/// Namnrymd för beslut om att stänga eller låsa en period.
pub const NS_PERIOD: &str = "bokis-period";
/// Namnrymd för systemändringar: kontoplan och regler.
pub const NS_SYSTEM: &str = "bokis-system";
/// Namnrymd för registrering av en nyckel.
pub const NS_KEY: &str = "bokis-nyckel";

/// Flaggan i en FIDO-signatur som intygar att nyckeln rördes.
pub const SK_FLAG_USER_PRESENCE: u8 = 0x01;
/// Flaggan i en FIDO-signatur som intygar att nyckelns PIN angavs. En nyckel
/// skapad med `ssh-keygen -t ed25519-sk -O verify-required` sätter den.
pub const SK_FLAG_USER_VERIFIED: u8 = 0x04;
/// Flaggor (1 byte) och räknare (4 byte) sist i en FIDO-signatur.
const SK_TRAILER_LEN: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SignatureError {
    #[error("kan inte läsa den publika nyckeln: {0}")]
    BadPublicKey(String),
    #[error("nyckeltypen {0} stöds inte; använd ed25519-sk (hårdvarunyckel) eller ed25519")]
    UnsupportedKey(String),
    #[error("kan inte läsa signaturen: {0}")]
    BadSignature(String),
    #[error("signaturen är gjord med en annan nyckel")]
    WrongKey,
    #[error("signaturen gäller namnrymden \"{found}\", inte \"{expected}\"")]
    WrongNamespace { expected: String, found: String },
    #[error("signaturen stämmer inte med innehållet")]
    Invalid,
    #[error("hårdvarunyckeln intygade ingen fysisk beröring")]
    NoUserPresence,
    #[error("signeringen misslyckades: {0}")]
    SigningFailed(String),
}

/// En publik nyckel som får signera.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyInfo {
    /// Nyckeln i OpenSSH-format, utan kommentar.
    pub openssh: String,
    /// `SHA256:…`, samma form som `ssh-keygen -l` visar.
    pub fingerprint: String,
    pub algorithm: String,
    /// Sant för ed25519-sk, där den privata nyckeln aldrig lämnar hårdvaran.
    pub hardware: bool,
}

pub fn parse_public_key(openssh: &str) -> Result<KeyInfo, SignatureError> {
    let pk = PublicKey::from_openssh(openssh.trim())
        .map_err(|e| SignatureError::BadPublicKey(e.to_string()))?;
    let algorithm = pk.algorithm();
    let hardware = match algorithm {
        Algorithm::Ed25519 => false,
        Algorithm::SkEd25519 => true,
        other => return Err(SignatureError::UnsupportedKey(other.to_string())),
    };
    let bare = PublicKey::from(pk.key_data().clone());
    Ok(KeyInfo {
        openssh: bare
            .to_openssh()
            .map_err(|e| SignatureError::BadPublicKey(e.to_string()))?,
        fingerprint: pk.fingerprint(HashAlg::Sha256).to_string(),
        algorithm: algorithm.to_string(),
        hardware,
    })
}

fn parse_sig(armored: &str) -> Result<SshSig, SignatureError> {
    SshSig::from_pem(armored.trim()).map_err(|e| SignatureError::BadSignature(e.to_string()))
}

/// Fingeravtrycket för nyckeln som gjorde signaturen. Används för att slå
/// upp nyckeln bland de registrerade; själva kontrollen görs av [`verify`].
pub fn signer_fingerprint(armored: &str) -> Result<String, SignatureError> {
    let sig = parse_sig(armored)?;
    Ok(PublicKey::from(sig.public_key().clone())
        .fingerprint(HashAlg::Sha256)
        .to_string())
}

/// Kontrollerar att `armored` är en giltig signatur av `message` i
/// `namespace`, gjord med `public_key`. En signatur från en hårdvarunyckel
/// måste dessutom intyga att nyckeln rördes.
pub fn verify(
    public_key: &str,
    namespace: &str,
    message: &[u8],
    armored: &str,
) -> Result<(), SignatureError> {
    let pk = PublicKey::from_openssh(public_key.trim())
        .map_err(|e| SignatureError::BadPublicKey(e.to_string()))?;
    let sig = parse_sig(armored)?;
    if pk.key_data() != sig.public_key() {
        return Err(SignatureError::WrongKey);
    }
    if sig.namespace() != namespace {
        return Err(SignatureError::WrongNamespace {
            expected: namespace.to_string(),
            found: sig.namespace().to_string(),
        });
    }
    pk.verify(namespace, message, &sig)
        .map_err(|_| SignatureError::Invalid)?;
    if let Some(flags) = flags_of(&sig)?
        && flags & SK_FLAG_USER_PRESENCE == 0
    {
        return Err(SignatureError::NoUserPresence);
    }
    Ok(())
}

/// FIDO-flaggorna i en signatur från en hårdvarunyckel, eller `None` för en
/// mjukvarunyckel. Säger bara vad signaturen påstår; att den är giltig
/// kontrolleras av [`verify`].
pub fn sk_flags(armored: &str) -> Result<Option<u8>, SignatureError> {
    flags_of(&parse_sig(armored)?)
}

fn flags_of(sig: &SshSig) -> Result<Option<u8>, SignatureError> {
    if sig.algorithm() != Algorithm::SkEd25519 {
        return Ok(None);
    }
    let bytes = sig.signature_bytes();
    bytes
        .len()
        .checked_sub(SK_TRAILER_LEN)
        .map(|i| Some(bytes[i]))
        .ok_or(SignatureError::Invalid)
}

/// Något som kan signera: `ssh-keygen -Y sign` i skarp drift, en
/// mjukvarunyckel i tester.
pub trait Signer {
    /// Signerar `message` i `namespace` och ger en armerad SSHSIG
    /// (`-----BEGIN SSH SIGNATURE-----`).
    fn sign(&self, namespace: &str, message: &[u8]) -> Result<String, SignatureError>;
}

/// En ed25519-nyckel i minnet. Avsedd för tester; i skarp drift används
/// hårdvarunyckeln via `ssh-keygen`.
pub struct SoftwareSigner {
    key: PrivateKey,
}

impl SoftwareSigner {
    pub fn from_seed(seed: [u8; 32]) -> Self {
        SoftwareSigner {
            key: PrivateKey::from(Ed25519Keypair::from_seed(&seed)),
        }
    }

    pub fn public_key_openssh(&self) -> String {
        self.key
            .public_key()
            .to_openssh()
            .expect("en ed25519-nyckel går alltid att skriva i OpenSSH-format")
    }
}

impl Signer for SoftwareSigner {
    fn sign(&self, namespace: &str, message: &[u8]) -> Result<String, SignatureError> {
        self.key
            .sign(namespace, HashAlg::Sha512, message)
            .and_then(|sig| sig.to_pem(LineEnding::LF))
            .map_err(|e| SignatureError::SigningFailed(e.to_string()))
    }
}

/// Efterliknar en hårdvarunyckel (ed25519-sk) med valda FIDO-flaggor, till
/// exempel beröring och PIN. Bara för tester: den privata nyckeln ligger i
/// minnet, så signaturerna bevisar ingen hårdvara. I skarp drift gör
/// `ssh-keygen` sådana signaturer med en riktig YubiKey.
pub struct SimulatedHardwareSigner {
    key: ed25519_dalek::SigningKey,
    flags: u8,
}

impl SimulatedHardwareSigner {
    /// Programmet som FIDO-nyckeln skapades för; ssh-keygen använder "ssh:".
    const APPLICATION: &str = "ssh:";

    pub fn new(seed: [u8; 32], flags: u8) -> Self {
        SimulatedHardwareSigner {
            key: ed25519_dalek::SigningKey::from_bytes(&seed),
            flags,
        }
    }

    fn key_data(&self) -> KeyData {
        let pk = Ed25519PublicKey(self.key.verifying_key().to_bytes());
        KeyData::SkEd25519(SkEd25519::new(pk, Self::APPLICATION))
    }

    pub fn public_key_openssh(&self) -> String {
        PublicKey::from(self.key_data())
            .to_openssh()
            .expect("en ed25519-sk-nyckel går alltid att skriva i OpenSSH-format")
    }
}

impl Signer for SimulatedHardwareSigner {
    fn sign(&self, namespace: &str, message: &[u8]) -> Result<String, SignatureError> {
        let failed = |e: ssh_key::Error| SignatureError::SigningFailed(e.to_string());
        let inner = SshSig::signed_data(namespace, HashAlg::Sha512, message).map_err(failed)?;
        let counter = 42u32.to_be_bytes();
        // FIDO signerar programmets hash, flaggorna, räknaren och meddelandets hash.
        let mut signed = Vec::new();
        signed.extend(Sha256::digest(Self::APPLICATION.as_bytes()));
        signed.push(self.flags);
        signed.extend(counter);
        signed.extend(Sha256::digest(&inner));
        let mut raw = self.key.sign(&signed).to_bytes().to_vec();
        raw.push(self.flags);
        raw.extend(counter);
        let signature = ssh_key::Signature::new(Algorithm::SkEd25519, raw).map_err(failed)?;
        SshSig::new(self.key_data(), namespace, HashAlg::Sha512, signature)
            .and_then(|sig| sig.to_pem(LineEnding::LF))
            .map_err(failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn software_signature_verifies() {
        let signer = SoftwareSigner::from_seed([7; 32]);
        let pk = signer.public_key_openssh();
        let sig = signer.sign(NS_VOUCHER, b"abc").unwrap();
        assert!(sig.starts_with("-----BEGIN SSH SIGNATURE-----"));
        assert_eq!(verify(&pk, NS_VOUCHER, b"abc", &sig), Ok(()));
        assert_eq!(
            signer_fingerprint(&sig).unwrap(),
            parse_public_key(&pk).unwrap().fingerprint
        );
    }

    #[test]
    fn wrong_message_key_or_namespace_fails() {
        let signer = SoftwareSigner::from_seed([7; 32]);
        let other = SoftwareSigner::from_seed([8; 32]);
        let pk = signer.public_key_openssh();
        let sig = signer.sign(NS_VOUCHER, b"abc").unwrap();
        assert_eq!(
            verify(&pk, NS_VOUCHER, b"abd", &sig),
            Err(SignatureError::Invalid)
        );
        assert_eq!(
            verify(&other.public_key_openssh(), NS_VOUCHER, b"abc", &sig),
            Err(SignatureError::WrongKey)
        );
        assert!(matches!(
            verify(&pk, NS_PERIOD, b"abc", &sig),
            Err(SignatureError::WrongNamespace { .. })
        ));
        assert!(matches!(
            verify(&pk, NS_VOUCHER, b"abc", "skräp"),
            Err(SignatureError::BadSignature(_))
        ));
    }

    #[test]
    fn only_ed25519_keys_are_accepted() {
        let info =
            parse_public_key(&SoftwareSigner::from_seed([1; 32]).public_key_openssh()).unwrap();
        assert!(!info.hardware);
        assert!(info.fingerprint.starts_with("SHA256:"));
        let rsa = "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAAAgQCs8jWvod2wlStIcgGEb0Yhqmy94ePLcF0fRb2Kot5sI/zw5b9KuC/nmBPjgKR4u91adDI2YqtZnPkcOOVy/qPHPMcqod63Grp/9PwCAJJsS8FC1xbMw0TC3ys1CysOy2E1Gue6u+OTd/6ijZ3LdK5kYOchUU/EuKBze1qQRyBTzw== x";
        assert!(matches!(
            parse_public_key(rsa),
            Err(SignatureError::UnsupportedKey(_))
        ));
    }

    /// En FIDO-signatur som en YubiKey skulle göra, med valda flaggor.
    fn sk_signature(seed: [u8; 32], flags: u8, namespace: &str, msg: &[u8]) -> (String, String) {
        let signer = SimulatedHardwareSigner::new(seed, flags);
        (
            signer.public_key_openssh(),
            signer.sign(namespace, msg).unwrap(),
        )
    }

    #[test]
    fn hardware_signature_with_touch_verifies() {
        let (pk, sig) = sk_signature([3; 32], SK_FLAG_USER_PRESENCE, NS_VOUCHER, b"hash");
        assert!(parse_public_key(&pk).unwrap().hardware);
        assert_eq!(verify(&pk, NS_VOUCHER, b"hash", &sig), Ok(()));
        assert_eq!(
            verify(&pk, NS_VOUCHER, b"hasx", &sig),
            Err(SignatureError::Invalid)
        );
    }

    #[test]
    fn hardware_signature_without_touch_is_refused() {
        let (pk, sig) = sk_signature([3; 32], 0, NS_VOUCHER, b"hash");
        assert_eq!(
            verify(&pk, NS_VOUCHER, b"hash", &sig),
            Err(SignatureError::NoUserPresence)
        );
    }

    #[test]
    fn the_flags_tell_touch_and_pin_apart() {
        let both = SK_FLAG_USER_PRESENCE | SK_FLAG_USER_VERIFIED;
        let (pk, sig) = sk_signature([4; 32], both, NS_VOUCHER, b"hash");
        assert_eq!(verify(&pk, NS_VOUCHER, b"hash", &sig), Ok(()));
        assert_eq!(sk_flags(&sig), Ok(Some(both)));
        let (_, touch) = sk_signature([4; 32], SK_FLAG_USER_PRESENCE, NS_VOUCHER, b"hash");
        assert_eq!(sk_flags(&touch), Ok(Some(SK_FLAG_USER_PRESENCE)));
        let software = SoftwareSigner::from_seed([4; 32])
            .sign(NS_VOUCHER, b"hash")
            .unwrap();
        assert_eq!(sk_flags(&software), Ok(None));
    }
}
