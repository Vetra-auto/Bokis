use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct AttachmentRow {
    pub id: String,
    pub voucher_id: String,
    pub filename: String,
    pub mime_type: String,
    pub file_size: i64,
    pub file_hash: String,
    pub storage_path: String,
    pub attachment_type: String,
    pub uploaded_at: String,
    pub uploaded_by: String,
}

pub async fn list_for_voucher(
    pool: &SqlitePool,
    voucher_id: &str,
) -> Result<Vec<AttachmentRow>, sqlx::Error> {
    sqlx::query_as::<_, AttachmentRow>(
        "SELECT * FROM attachments WHERE voucher_id = ?1 ORDER BY uploaded_at"
    )
    .bind(voucher_id)
    .fetch_all(pool)
    .await
}

pub async fn create(
    pool: &SqlitePool,
    voucher_id: &str,
    filename: &str,
    mime_type: &str,
    file_size: i64,
    file_hash: &str,
    storage_path: &str,
    attachment_type: &str,
    uploaded_by: &str,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO attachments (id, voucher_id, filename, mime_type, file_size, file_hash, storage_path, attachment_type, uploaded_by) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
    )
    .bind(&id)
    .bind(voucher_id)
    .bind(filename)
    .bind(mime_type)
    .bind(file_size)
    .bind(file_hash)
    .bind(storage_path)
    .bind(attachment_type)
    .bind(uploaded_by)
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn get_by_id(pool: &SqlitePool, id: &str) -> Result<Option<AttachmentRow>, sqlx::Error> {
    sqlx::query_as::<_, AttachmentRow>("SELECT * FROM attachments WHERE id = ?1")
        .bind(id)
        .fetch_optional(pool)
        .await
}
