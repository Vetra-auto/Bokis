use bokforing_core::hash_chain::{compute_event_hash, GENESIS_HASH};
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct AuditEventRow {
    pub id: String,
    pub sequence_number: i64,
    pub event_type: String,
    pub entity_type: String,
    pub entity_id: String,
    pub user_id: Option<String>,
    pub occurred_at: String,
    pub detail_json: Option<String>,
    pub previous_hash: String,
    pub event_hash: String,
}

#[derive(Debug, sqlx::FromRow)]
pub struct ProcessingHistoryRow {
    pub id: String,
    pub event_type: String,
    pub entity_type: String,
    pub entity_id: String,
    pub user_id: Option<String>,
    pub occurred_at: String,
    pub description: String,
    pub metadata_json: Option<String>,
}

pub async fn append_event(
    pool: &SqlitePool,
    event_type: &str,
    entity_type: &str,
    entity_id: &str,
    user_id: &str,
    detail_json: &str,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();

    let last: Option<(i64, String)> = sqlx::query_as(
        "SELECT sequence_number, event_hash FROM audit_events ORDER BY sequence_number DESC LIMIT 1"
    )
    .fetch_optional(pool)
    .await?;

    let (seq, prev_hash) = match last {
        Some((seq, hash)) => (seq + 1, hash),
        None => (1, GENESIS_HASH.to_string()),
    };

    let occurred_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();

    let event_hash = compute_event_hash(seq, event_type, entity_id, &occurred_at, detail_json, &prev_hash);

    sqlx::query(
        "INSERT INTO audit_events (id, sequence_number, event_type, entity_type, entity_id, user_id, occurred_at, detail_json, previous_hash, event_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"
    )
    .bind(&id)
    .bind(seq)
    .bind(event_type)
    .bind(entity_type)
    .bind(entity_id)
    .bind(user_id)
    .bind(&occurred_at)
    .bind(detail_json)
    .bind(&prev_hash)
    .bind(&event_hash)
    .execute(pool)
    .await?;

    Ok(id)
}

pub async fn append_processing_history(
    pool: &SqlitePool,
    event_type: &str,
    entity_type: &str,
    entity_id: &str,
    user_id: &str,
    description: &str,
    metadata_json: Option<&str>,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO processing_history_events (id, event_type, entity_type, entity_id, user_id, description, metadata_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"
    )
    .bind(&id)
    .bind(event_type)
    .bind(entity_type)
    .bind(entity_id)
    .bind(user_id)
    .bind(description)
    .bind(metadata_json)
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn list_events(
    pool: &SqlitePool,
    limit: i64,
    offset: i64,
) -> Result<Vec<AuditEventRow>, sqlx::Error> {
    sqlx::query_as::<_, AuditEventRow>(
        "SELECT * FROM audit_events ORDER BY sequence_number DESC LIMIT ?1 OFFSET ?2"
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
}

pub async fn list_events_for_entity(
    pool: &SqlitePool,
    entity_type: &str,
    entity_id: &str,
) -> Result<Vec<AuditEventRow>, sqlx::Error> {
    sqlx::query_as::<_, AuditEventRow>(
        "SELECT * FROM audit_events WHERE entity_type = ?1 AND entity_id = ?2 ORDER BY sequence_number"
    )
    .bind(entity_type)
    .bind(entity_id)
    .fetch_all(pool)
    .await
}

pub async fn get_all_events_ordered(pool: &SqlitePool) -> Result<Vec<AuditEventRow>, sqlx::Error> {
    sqlx::query_as::<_, AuditEventRow>(
        "SELECT * FROM audit_events ORDER BY sequence_number ASC"
    )
    .fetch_all(pool)
    .await
}

pub async fn list_processing_history(
    pool: &SqlitePool,
    limit: i64,
    offset: i64,
) -> Result<Vec<ProcessingHistoryRow>, sqlx::Error> {
    sqlx::query_as::<_, ProcessingHistoryRow>(
        "SELECT * FROM processing_history_events ORDER BY occurred_at DESC LIMIT ?1 OFFSET ?2"
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
}
