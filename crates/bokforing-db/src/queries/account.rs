use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct AccountRow {
    pub id: String,
    pub company_id: String,
    pub account_number: i64,
    pub name: String,
    pub account_type: String,
    pub is_active: bool,
    pub sru_code: Option<String>,
    pub description: Option<String>,
    pub created_at: String,
    pub chart_version_id: Option<String>,
}

pub async fn list(pool: &SqlitePool, company_id: &str) -> Result<Vec<AccountRow>, sqlx::Error> {
    sqlx::query_as::<_, AccountRow>(
        "SELECT * FROM accounts WHERE company_id = ?1 ORDER BY account_number"
    )
    .bind(company_id)
    .fetch_all(pool)
    .await
}

pub async fn list_active(pool: &SqlitePool, company_id: &str) -> Result<Vec<AccountRow>, sqlx::Error> {
    sqlx::query_as::<_, AccountRow>(
        "SELECT * FROM accounts WHERE company_id = ?1 AND is_active = 1 ORDER BY account_number"
    )
    .bind(company_id)
    .fetch_all(pool)
    .await
}

pub async fn get_by_number(
    pool: &SqlitePool,
    company_id: &str,
    account_number: i64,
) -> Result<Option<AccountRow>, sqlx::Error> {
    sqlx::query_as::<_, AccountRow>(
        "SELECT * FROM accounts WHERE company_id = ?1 AND account_number = ?2"
    )
    .bind(company_id)
    .bind(account_number)
    .fetch_optional(pool)
    .await
}

pub async fn create(
    pool: &SqlitePool,
    company_id: &str,
    account_number: i64,
    name: &str,
    account_type: &str,
    sru_code: Option<&str>,
    description: Option<&str>,
    chart_version_id: Option<&str>,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO accounts (id, company_id, account_number, name, account_type, sru_code, description, chart_version_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"
    )
    .bind(&id)
    .bind(company_id)
    .bind(account_number)
    .bind(name)
    .bind(account_type)
    .bind(sru_code)
    .bind(description)
    .bind(chart_version_id)
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn update(
    pool: &SqlitePool,
    id: &str,
    name: &str,
    sru_code: Option<&str>,
    description: Option<&str>,
    is_active: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE accounts SET name = ?2, sru_code = ?3, description = ?4, is_active = ?5 WHERE id = ?1"
    )
    .bind(id)
    .bind(name)
    .bind(sru_code)
    .bind(description)
    .bind(is_active)
    .execute(pool)
    .await?;
    Ok(())
}
