use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct ArchiveManifestRow {
    pub id: String,
    pub fiscal_year_id: String,
    pub created_at: String,
    pub created_by: String,
    pub archive_path: String,
    pub content_hash: String,
    pub manifest_json: String,
}

pub async fn create(
    pool: &SqlitePool,
    fiscal_year_id: &str,
    created_by: &str,
    archive_path: &str,
    content_hash: &str,
    manifest_json: &str,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO archive_manifests (id, fiscal_year_id, created_by, archive_path, content_hash, manifest_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)"
    )
    .bind(&id)
    .bind(fiscal_year_id)
    .bind(created_by)
    .bind(archive_path)
    .bind(content_hash)
    .bind(manifest_json)
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn list_for_fiscal_year(
    pool: &SqlitePool,
    fiscal_year_id: &str,
) -> Result<Vec<ArchiveManifestRow>, sqlx::Error> {
    sqlx::query_as::<_, ArchiveManifestRow>(
        "SELECT * FROM archive_manifests WHERE fiscal_year_id = ?1 ORDER BY created_at DESC"
    )
    .bind(fiscal_year_id)
    .fetch_all(pool)
    .await
}

pub async fn list_all(pool: &SqlitePool) -> Result<Vec<ArchiveManifestRow>, sqlx::Error> {
    sqlx::query_as::<_, ArchiveManifestRow>(
        "SELECT * FROM archive_manifests ORDER BY created_at DESC"
    )
    .fetch_all(pool)
    .await
}
