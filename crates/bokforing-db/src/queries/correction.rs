use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct CorrectionLinkRow {
    pub id: String,
    pub original_voucher_id: String,
    pub reversal_voucher_id: String,
    pub replacement_voucher_id: Option<String>,
    pub reason: String,
    pub created_at: String,
    pub created_by: String,
}

pub async fn create_link(
    pool: &SqlitePool,
    original_voucher_id: &str,
    reversal_voucher_id: &str,
    replacement_voucher_id: Option<&str>,
    reason: &str,
    created_by: &str,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO correction_links (id, original_voucher_id, reversal_voucher_id, replacement_voucher_id, reason, created_by) VALUES (?1, ?2, ?3, ?4, ?5, ?6)"
    )
    .bind(&id)
    .bind(original_voucher_id)
    .bind(reversal_voucher_id)
    .bind(replacement_voucher_id)
    .bind(reason)
    .bind(created_by)
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn get_links_for_voucher(
    pool: &SqlitePool,
    voucher_id: &str,
) -> Result<Vec<CorrectionLinkRow>, sqlx::Error> {
    sqlx::query_as::<_, CorrectionLinkRow>(
        "SELECT * FROM correction_links WHERE original_voucher_id = ?1 OR reversal_voucher_id = ?1 OR replacement_voucher_id = ?1 ORDER BY created_at"
    )
    .bind(voucher_id)
    .fetch_all(pool)
    .await
}
