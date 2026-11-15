//! Kanonisk serialisering.
//!
//! Formen är textbaserad och längdprefixad: först en domänrad med version,
//! sedan ett fält per rad som `namn:längd:värde`, där längden är värdets
//! antal byte i UTF-8. Ett saknat värde skrivs `namn:-`. Längdprefixet gör
//! formen entydig utan escape-regler, och den är läsbar nog för att visas
//! för den som ska signera. Formatet får aldrig ändras i efterhand; en ny
//! form får ett nytt versionsnamn, så att gamla signaturer går att pröva.

use sha2::{Digest, Sha256};

pub struct CanonicalWriter {
    buf: Vec<u8>,
}

impl CanonicalWriter {
    pub fn new(domain: &str) -> Self {
        debug_assert!(!domain.contains('\n'));
        let mut buf = Vec::with_capacity(512);
        buf.extend_from_slice(domain.as_bytes());
        buf.push(b'\n');
        Self { buf }
    }

    pub fn field(&mut self, name: &str, value: &str) {
        debug_assert!(!name.contains([':', '\n']));
        self.buf.extend_from_slice(name.as_bytes());
        self.buf.push(b':');
        self.buf
            .extend_from_slice(value.len().to_string().as_bytes());
        self.buf.push(b':');
        self.buf.extend_from_slice(value.as_bytes());
        self.buf.push(b'\n');
    }

    pub fn opt(&mut self, name: &str, value: Option<&str>) {
        match value {
            Some(v) => self.field(name, v),
            None => {
                debug_assert!(!name.contains([':', '\n']));
                self.buf.extend_from_slice(name.as_bytes());
                self.buf.extend_from_slice(b":-\n");
            }
        }
    }

    pub fn int(&mut self, name: &str, value: i64) {
        self.field(name, &value.to_string());
    }

    pub fn finish(self) -> Vec<u8> {
        self.buf
    }
}

/// SHA-256 som 64 hexadecimala gemener.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_are_length_prefixed_and_unambiguous() {
        let mut a = CanonicalWriter::new("test-v1");
        a.field("x", "a\nb:2:c");
        a.opt("y", None);
        let a = a.finish();
        assert_eq!(a, b"test-v1\nx:7:a\nb:2:c\ny:-\n");

        // Ett värde som ser ut som två fält ger inte samma form som två fält.
        let mut b = CanonicalWriter::new("test-v1");
        b.field("x", "a");
        b.field("b", "c");
        b.opt("y", None);
        assert_ne!(a, b.finish());
    }

    #[test]
    fn empty_and_missing_differ() {
        let mut a = CanonicalWriter::new("t");
        a.field("x", "");
        let mut b = CanonicalWriter::new("t");
        b.opt("x", None);
        assert_ne!(a.finish(), b.finish());
    }

    #[test]
    fn sha256_known_value() {
        assert_eq!(
            sha256_hex(b"hello world"),
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }
}
