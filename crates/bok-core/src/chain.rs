//! Hashkedjor: behandlingshistoriken (`handelser`) och verifikationskedjan.
//!
//! Idén är flyttad från `bokforing-core::hash_chain`, men hashen räknas nu
//! över en längdprefixad kanonisk form i stället för fält skilda med `|`,
//! så att ett fält som innehåller avgränsaren inte kan förväxlas med två.

use thiserror::Error;

use crate::canonical::{CanonicalWriter, sha256_hex};

/// Föregående hash för den första länken i en kedja.
pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// En händelse i behandlingshistoriken, med alla fält som hashen täcker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRecord {
    pub seq: i64,
    pub time: String,
    pub actor_kind: String,
    pub actor: String,
    pub role: String,
    pub action: String,
    pub object: Option<String>,
    pub old_values: Option<String>,
    pub new_values: Option<String>,
    pub program_version: String,
    pub rule_version: Option<String>,
    pub signature: Option<String>,
    pub prev_hash: String,
}

impl EventRecord {
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut w = CanonicalWriter::new("bokis-handelse-v1");
        w.int("seq", self.seq);
        w.field("tid", &self.time);
        w.field("aktor_typ", &self.actor_kind);
        w.field("aktor", &self.actor);
        w.field("roll", &self.role);
        w.field("handling", &self.action);
        w.opt("objekt", self.object.as_deref());
        w.opt("gamla", self.old_values.as_deref());
        w.opt("nya", self.new_values.as_deref());
        w.field("programversion", &self.program_version);
        w.opt("regelversion", self.rule_version.as_deref());
        w.opt("signatur", self.signature.as_deref());
        w.field("foregaende_hash", &self.prev_hash);
        w.finish()
    }

    pub fn compute_hash(&self) -> String {
        sha256_hex(&self.canonical_bytes())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ChainError {
    #[error("historiken har en lucka: väntade händelse {expected}, fann {found}")]
    Gap { expected: i64, found: i64 },
    #[error("händelse {seq} pekar inte på föregående händelses hash")]
    WrongPrevious { seq: i64 },
    #[error("händelse {seq} har ändrats: hashen stämmer inte med innehållet")]
    WrongHash { seq: i64 },
    #[error("verifikationskedjan bryts vid {at}")]
    VoucherChainBroken { at: String },
}

/// Kontrollerar hela historiken: sekvensen börjar på 1 och saknar luckor,
/// varje händelse pekar på föregående, och varje hash stämmer med innehållet.
/// Ger antalet kontrollerade händelser.
pub fn verify_events<'a, I>(events: I) -> Result<usize, ChainError>
where
    I: IntoIterator<Item = (&'a EventRecord, &'a str)>,
{
    let mut prev = GENESIS.to_string();
    let mut count = 0;
    for (expected_seq, (event, stored_hash)) in (1..).zip(events) {
        if event.seq != expected_seq {
            return Err(ChainError::Gap {
                expected: expected_seq,
                found: event.seq,
            });
        }
        if event.prev_hash != prev {
            return Err(ChainError::WrongPrevious { seq: event.seq });
        }
        if event.compute_hash() != stored_hash {
            return Err(ChainError::WrongHash { seq: event.seq });
        }
        prev = stored_hash.to_string();
        count += 1;
    }
    Ok(count)
}

/// Kedjehashen för en bokförd verifikation: föregående verifikations
/// kedjehash (i registreringsordning) och den här verifikationens kanoniska
/// hash.
pub fn voucher_chain_hash(prev_chain_hash: &str, canonical_hash: &str) -> String {
    let mut w = CanonicalWriter::new("bokis-kedja-v1");
    w.field("foregaende", prev_chain_hash);
    w.field("verifikation", canonical_hash);
    sha256_hex(&w.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(seq: i64, prev: &str, action: &str) -> EventRecord {
        EventRecord {
            seq,
            time: format!("2026-01-0{seq}T00:00:00.000000Z"),
            actor_kind: "manniska".into(),
            actor: "henrik".into(),
            role: "granskare".into(),
            action: action.into(),
            object: Some("verifikation:1".into()),
            old_values: None,
            new_values: Some("{\"x\":1}".into()),
            program_version: "test".into(),
            rule_version: None,
            signature: None,
            prev_hash: prev.into(),
        }
    }

    fn chain(n: i64) -> Vec<(EventRecord, String)> {
        let mut out = Vec::new();
        let mut prev = GENESIS.to_string();
        for seq in 1..=n {
            let e = event(seq, &prev, "skapad");
            let h = e.compute_hash();
            prev = h.clone();
            out.push((e, h));
        }
        out
    }

    fn check(c: &[(EventRecord, String)]) -> Result<usize, ChainError> {
        verify_events(c.iter().map(|(e, h)| (e, h.as_str())))
    }

    #[test]
    fn intact_chain_verifies() {
        assert_eq!(check(&chain(3)), Ok(3));
        assert_eq!(check(&[]), Ok(0));
    }

    #[test]
    fn changed_content_is_detected() {
        let mut c = chain(3);
        c[1].0.action = "annat".into();
        assert_eq!(check(&c), Err(ChainError::WrongHash { seq: 2 }));
    }

    #[test]
    fn recomputed_hash_breaks_the_next_link() {
        let mut c = chain(3);
        c[1].0.action = "annat".into();
        c[1].1 = c[1].0.compute_hash();
        assert_eq!(check(&c), Err(ChainError::WrongPrevious { seq: 3 }));
    }

    #[test]
    fn removed_event_is_a_gap() {
        let mut c = chain(3);
        c.remove(1);
        assert_eq!(
            check(&c),
            Err(ChainError::Gap {
                expected: 2,
                found: 3
            })
        );
    }

    #[test]
    fn separator_in_a_field_does_not_collide() {
        let mut a = event(1, GENESIS, "a|b");
        a.object = None;
        let mut b = event(1, GENESIS, "a");
        b.object = Some("b".into());
        assert_ne!(a.compute_hash(), b.compute_hash());
    }

    #[test]
    fn voucher_chain_depends_on_order() {
        let x = voucher_chain_hash(GENESIS, &"a".repeat(64));
        let y = voucher_chain_hash(&x, &"b".repeat(64));
        let y2 = voucher_chain_hash(GENESIS, &"b".repeat(64));
        assert_ne!(y, y2);
        assert_eq!(x.len(), 64);
    }
}
