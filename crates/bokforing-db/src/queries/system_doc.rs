use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct SystemDocSnapshotRow {
    pub id: String,
    pub generated_at: String,
    pub generated_by: Option<String>,
    pub content_json: String,
    pub content_hash: String,
}

pub async fn create(
    pool: &SqlitePool,
    generated_by: &str,
    content_json: &str,
    content_hash: &str,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO system_documentation_snapshots (id, generated_by, content_json, content_hash) VALUES (?1, ?2, ?3, ?4)"
    )
    .bind(&id)
    .bind(generated_by)
    .bind(content_json)
    .bind(content_hash)
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn list(pool: &SqlitePool, limit: i64) -> Result<Vec<SystemDocSnapshotRow>, sqlx::Error> {
    sqlx::query_as::<_, SystemDocSnapshotRow>(
        "SELECT * FROM system_documentation_snapshots ORDER BY generated_at DESC LIMIT ?1"
    )
    .bind(limit)
    .fetch_all(pool)
    .await
}

pub async fn get_latest(pool: &SqlitePool) -> Result<Option<SystemDocSnapshotRow>, sqlx::Error> {
    sqlx::query_as::<_, SystemDocSnapshotRow>(
        "SELECT * FROM system_documentation_snapshots ORDER BY generated_at DESC LIMIT 1"
    )
    .fetch_optional(pool)
    .await
}
