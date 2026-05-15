use sqlx::SqlitePool;
use std::path::Path;
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct BackupJobRow {
    pub id: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub status: String,
    pub backup_path: Option<String>,
    pub backup_hash: Option<String>,
    pub size_bytes: Option<i64>,
    pub error_message: Option<String>,
    pub initiated_by: String,
}

pub async fn create_backup(
    pool: &SqlitePool,
    backup_path: &Path,
    user_id: &str,
) -> Result<String, sqlx::Error> {
    let job_id = Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO backup_jobs (id, initiated_by) VALUES (?1, ?2)"
    )
    .bind(&job_id)
    .bind(user_id)
    .execute(pool)
    .await?;

    let path_str = backup_path.to_string_lossy().to_string();

    let result = sqlx::query(&format!("VACUUM INTO '{}'", path_str))
        .execute(pool)
        .await;

    match result {
        Ok(_) => {
            let metadata = std::fs::metadata(backup_path).ok();
            let size = metadata.map(|m| m.len() as i64);

            let hash = if let Ok(data) = std::fs::read(backup_path) {
                Some(bokforing_core::hash_chain::compute_file_hash(&data))
            } else {
                None
            };

            sqlx::query(
                "UPDATE backup_jobs SET status = 'completed', completed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), backup_path = ?2, backup_hash = ?3, size_bytes = ?4 WHERE id = ?1"
            )
            .bind(&job_id)
            .bind(&path_str)
            .bind(hash.as_deref())
            .bind(size)
            .execute(pool)
            .await?;
        }
        Err(e) => {
            sqlx::query(
                "UPDATE backup_jobs SET status = 'failed', completed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), error_message = ?2 WHERE id = ?1"
            )
            .bind(&job_id)
            .bind(e.to_string())
            .execute(pool)
            .await?;
        }
    }

    Ok(job_id)
}

pub async fn list_jobs(pool: &SqlitePool, limit: i64) -> Result<Vec<BackupJobRow>, sqlx::Error> {
    sqlx::query_as::<_, BackupJobRow>(
        "SELECT * FROM backup_jobs ORDER BY started_at DESC LIMIT ?1"
    )
    .bind(limit)
    .fetch_all(pool)
    .await
}

pub async fn record_verification(
    pool: &SqlitePool,
    backup_job_id: &str,
    user_id: &str,
    is_valid: bool,
    details: &str,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO backup_verifications (id, backup_job_id, verified_by, is_valid, details) VALUES (?1, ?2, ?3, ?4, ?5)"
    )
    .bind(&id)
    .bind(backup_job_id)
    .bind(user_id)
    .bind(is_valid)
    .bind(details)
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn get_job_by_id(pool: &SqlitePool, id: &str) -> Result<Option<BackupJobRow>, sqlx::Error> {
    sqlx::query_as::<_, BackupJobRow>(
        "SELECT * FROM backup_jobs WHERE id = ?1"
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}
