use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct VoucherRow {
    pub id: String,
    pub company_id: String,
    pub fiscal_year_id: String,
    pub period_id: String,
    pub voucher_series: String,
    pub voucher_number: i64,
    pub voucher_date: String,
    pub description: String,
    pub status: String,
    pub counterparty_id: Option<String>,
    pub created_by: String,
    pub created_at: String,
    pub posted_at: Option<String>,
    pub posted_by: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct VoucherLineRow {
    pub id: String,
    pub voucher_id: String,
    pub line_number: i64,
    pub account_id: String,
    pub debit_amount: i64,
    pub credit_amount: i64,
    pub description: Option<String>,
    pub counterparty_id: Option<String>,
    pub cost_center: Option<String>,
    pub project: Option<String>,
}

pub async fn get_by_id(pool: &SqlitePool, id: &str) -> Result<Option<VoucherRow>, sqlx::Error> {
    sqlx::query_as::<_, VoucherRow>("SELECT * FROM vouchers WHERE id = ?1")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn get_lines(pool: &SqlitePool, voucher_id: &str) -> Result<Vec<VoucherLineRow>, sqlx::Error> {
    sqlx::query_as::<_, VoucherLineRow>(
        "SELECT * FROM voucher_lines WHERE voucher_id = ?1 ORDER BY line_number"
    )
    .bind(voucher_id)
    .fetch_all(pool)
    .await
}

pub async fn next_voucher_number(
    pool: &SqlitePool,
    company_id: &str,
    fiscal_year_id: &str,
    series: &str,
) -> Result<i64, sqlx::Error> {
    let row: (i64,) = sqlx::query_as(
        "SELECT COALESCE(MAX(voucher_number), 0) + 1 FROM vouchers WHERE company_id = ?1 AND fiscal_year_id = ?2 AND voucher_series = ?3"
    )
    .bind(company_id)
    .bind(fiscal_year_id)
    .bind(series)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

pub async fn create(
    pool: &SqlitePool,
    company_id: &str,
    fiscal_year_id: &str,
    period_id: &str,
    series: &str,
    voucher_number: i64,
    voucher_date: &str,
    description: &str,
    counterparty_id: Option<&str>,
    created_by: &str,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO vouchers (id, company_id, fiscal_year_id, period_id, voucher_series, voucher_number, voucher_date, description, status, counterparty_id, created_by) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'draft', ?9, ?10)"
    )
    .bind(&id)
    .bind(company_id)
    .bind(fiscal_year_id)
    .bind(period_id)
    .bind(series)
    .bind(voucher_number)
    .bind(voucher_date)
    .bind(description)
    .bind(counterparty_id)
    .bind(created_by)
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn add_line(
    pool: &SqlitePool,
    voucher_id: &str,
    line_number: i64,
    account_id: &str,
    debit_amount: i64,
    credit_amount: i64,
    description: Option<&str>,
    counterparty_id: Option<&str>,
    cost_center: Option<&str>,
    project: Option<&str>,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO voucher_lines (id, voucher_id, line_number, account_id, debit_amount, credit_amount, description, counterparty_id, cost_center, project) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"
    )
    .bind(&id)
    .bind(voucher_id)
    .bind(line_number)
    .bind(account_id)
    .bind(debit_amount)
    .bind(credit_amount)
    .bind(description)
    .bind(counterparty_id)
    .bind(cost_center)
    .bind(project)
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn mark_posted(
    pool: &SqlitePool,
    voucher_id: &str,
    user_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE vouchers SET status = 'posted', posted_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), posted_by = ?2 WHERE id = ?1 AND status = 'draft'"
    )
    .bind(voucher_id)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_by_period(
    pool: &SqlitePool,
    period_id: &str,
) -> Result<Vec<VoucherRow>, sqlx::Error> {
    sqlx::query_as::<_, VoucherRow>(
        "SELECT * FROM vouchers WHERE period_id = ?1 ORDER BY voucher_number"
    )
    .bind(period_id)
    .fetch_all(pool)
    .await
}

pub async fn list_by_fiscal_year(
    pool: &SqlitePool,
    fiscal_year_id: &str,
) -> Result<Vec<VoucherRow>, sqlx::Error> {
    sqlx::query_as::<_, VoucherRow>(
        "SELECT * FROM vouchers WHERE fiscal_year_id = ?1 ORDER BY voucher_date, voucher_number"
    )
    .bind(fiscal_year_id)
    .fetch_all(pool)
    .await
}

pub async fn delete_draft(pool: &SqlitePool, voucher_id: &str) -> Result<bool, sqlx::Error> {
    sqlx::query("DELETE FROM voucher_lines WHERE voucher_id = ?1")
        .bind(voucher_id)
        .execute(pool)
        .await?;

    let result = sqlx::query("DELETE FROM vouchers WHERE id = ?1 AND status = 'draft'")
        .bind(voucher_id)
        .execute(pool)
        .await?;

    Ok(result.rows_affected() > 0)
}
