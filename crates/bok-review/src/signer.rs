//! Signering med `ssh-keygen -Y sign`, samma som i `bok`.
//!
//! Nyckelfilen pekar på en ed25519-sk-nyckel (till exempel en YubiKey);
//! ssh-keygen väntar då på en beröring av nyckeln. Lagringen kontrollerar
//! signaturen mot de registrerade nycklarna innan något bokförs.
//!
//! Koden är en kopia av `bok-cli/src/signer.rs` tills den flyttas till en
//! gemensam plats när delarna flyttas in i `~/bokforing`.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use bok_store::bok_core::signature::{SignatureError, Signer};

pub struct SshKeygenSigner {
    key: PathBuf,
}

impl SshKeygenSigner {
    pub fn new(key: PathBuf) -> Self {
        SshKeygenSigner { key }
    }
}

impl Signer for SshKeygenSigner {
    fn sign(&self, namespace: &str, message: &[u8]) -> Result<String, SignatureError> {
        let fail = |what: String| SignatureError::SigningFailed(what);
        let dir = tempfile::tempdir().map_err(|e| fail(format!("tillfällig katalog: {e}")))?;
        let message_path = dir.path().join("meddelande");
        std::fs::write(&message_path, message).map_err(|e| fail(format!("tillfällig fil: {e}")))?;
        eprintln!(
            "Signerar med {} – rör vid nyckeln om den blinkar.",
            self.key.display()
        );
        let status = Command::new("ssh-keygen")
            .arg("-q")
            .args(["-Y", "sign", "-n", namespace, "-f"])
            .arg(&self.key)
            .arg(&message_path)
            .stdin(Stdio::inherit())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|e| fail(format!("kunde inte starta ssh-keygen: {e}")))?;
        if !status.success() {
            return Err(fail(format!("ssh-keygen avslutades med {status}")));
        }
        let mut sig_path = message_path.into_os_string();
        sig_path.push(".sig");
        std::fs::read_to_string(&sig_path).map_err(|e| fail(format!("hittar ingen signatur: {e}")))
    }
}
