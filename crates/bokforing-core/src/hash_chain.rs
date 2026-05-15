use sha2::{Digest, Sha256};

use crate::error::HashChainError;

pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

pub fn compute_event_hash(
    sequence_number: i64,
    event_type: &str,
    entity_id: &str,
    occurred_at: &str,
    detail_json: &str,
    previous_hash: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(sequence_number.to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(event_type.as_bytes());
    hasher.update(b"|");
    hasher.update(entity_id.as_bytes());
    hasher.update(b"|");
    hasher.update(occurred_at.as_bytes());
    hasher.update(b"|");
    hasher.update(detail_json.as_bytes());
    hasher.update(b"|");
    hasher.update(previous_hash.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn compute_file_hash(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

pub struct ChainEvent {
    pub sequence_number: i64,
    pub event_type: String,
    pub entity_id: String,
    pub occurred_at: String,
    pub detail_json: String,
    pub previous_hash: String,
    pub event_hash: String,
}

pub fn verify_chain(events: &[ChainEvent]) -> Result<(), HashChainError> {
    if events.is_empty() {
        return Err(HashChainError::EmptyChain);
    }

    let first = &events[0];
    if first.previous_hash != GENESIS_HASH {
        return Err(HashChainError::InvalidGenesis);
    }

    for (i, event) in events.iter().enumerate() {
        let computed = compute_event_hash(
            event.sequence_number,
            &event.event_type,
            &event.entity_id,
            &event.occurred_at,
            &event.detail_json,
            &event.previous_hash,
        );

        if computed != event.event_hash {
            return Err(HashChainError::BrokenChain {
                position: event.sequence_number,
                expected: event.event_hash.clone(),
                actual: computed,
            });
        }

        if i > 0 {
            let prev = &events[i - 1];
            if event.previous_hash != prev.event_hash {
                return Err(HashChainError::BrokenChain {
                    position: event.sequence_number,
                    expected: prev.event_hash.clone(),
                    actual: event.previous_hash.clone(),
                });
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_deterministic() {
        let h1 = compute_event_hash(1, "voucher_posted", "abc-123", "2026-01-15T10:00:00Z", "{}", GENESIS_HASH);
        let h2 = compute_event_hash(1, "voucher_posted", "abc-123", "2026-01-15T10:00:00Z", "{}", GENESIS_HASH);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
    }

    #[test]
    fn different_inputs_produce_different_hashes() {
        let h1 = compute_event_hash(1, "voucher_posted", "abc-123", "2026-01-15T10:00:00Z", "{}", GENESIS_HASH);
        let h2 = compute_event_hash(2, "voucher_posted", "abc-123", "2026-01-15T10:00:00Z", "{}", GENESIS_HASH);
        assert_ne!(h1, h2);
    }

    #[test]
    fn valid_chain_verifies() {
        let hash1 = compute_event_hash(1, "create", "e1", "2026-01-01T00:00:00Z", "{}", GENESIS_HASH);
        let hash2 = compute_event_hash(2, "update", "e2", "2026-01-02T00:00:00Z", "{\"x\":1}", &hash1);

        let events = vec![
            ChainEvent {
                sequence_number: 1,
                event_type: "create".to_string(),
                entity_id: "e1".to_string(),
                occurred_at: "2026-01-01T00:00:00Z".to_string(),
                detail_json: "{}".to_string(),
                previous_hash: GENESIS_HASH.to_string(),
                event_hash: hash1.clone(),
            },
            ChainEvent {
                sequence_number: 2,
                event_type: "update".to_string(),
                entity_id: "e2".to_string(),
                occurred_at: "2026-01-02T00:00:00Z".to_string(),
                detail_json: "{\"x\":1}".to_string(),
                previous_hash: hash1,
                event_hash: hash2,
            },
        ];

        assert!(verify_chain(&events).is_ok());
    }

    #[test]
    fn tampered_hash_detected() {
        let hash1 = compute_event_hash(1, "create", "e1", "2026-01-01T00:00:00Z", "{}", GENESIS_HASH);

        let events = vec![ChainEvent {
            sequence_number: 1,
            event_type: "create".to_string(),
            entity_id: "e1".to_string(),
            occurred_at: "2026-01-01T00:00:00Z".to_string(),
            detail_json: "{}".to_string(),
            previous_hash: GENESIS_HASH.to_string(),
            event_hash: "tampered_hash".to_string(),
        }];

        assert!(matches!(verify_chain(&events), Err(HashChainError::BrokenChain { .. })));
    }

    #[test]
    fn broken_link_detected() {
        let hash1 = compute_event_hash(1, "create", "e1", "2026-01-01T00:00:00Z", "{}", GENESIS_HASH);
        let wrong_prev = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let hash2 = compute_event_hash(2, "update", "e2", "2026-01-02T00:00:00Z", "{}", wrong_prev);

        let events = vec![
            ChainEvent {
                sequence_number: 1,
                event_type: "create".to_string(),
                entity_id: "e1".to_string(),
                occurred_at: "2026-01-01T00:00:00Z".to_string(),
                detail_json: "{}".to_string(),
                previous_hash: GENESIS_HASH.to_string(),
                event_hash: hash1,
            },
            ChainEvent {
                sequence_number: 2,
                event_type: "update".to_string(),
                entity_id: "e2".to_string(),
                occurred_at: "2026-01-02T00:00:00Z".to_string(),
                detail_json: "{}".to_string(),
                previous_hash: wrong_prev.to_string(),
                event_hash: hash2,
            },
        ];

        assert!(matches!(verify_chain(&events), Err(HashChainError::BrokenChain { .. })));
    }

    #[test]
    fn file_hash_works() {
        let hash = compute_file_hash(b"hello world");
        assert_eq!(hash.len(), 64);
        assert_eq!(hash, "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9");
    }
}
