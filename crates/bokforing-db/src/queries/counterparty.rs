use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct CounterpartyRow {
    pub id: String,
    pub company_id: String,
    pub name: String,
    pub org_number: Option<String>,
    pub contact_info: Option<String>,
    pub created_at: String,
}

pub async fn list(pool: &SqlitePool, company_id: &str) -> Result<Vec<CounterpartyRow>, sqlx::Error> {
    sqlx::query_as::<_, CounterpartyRow>(
        "SELECT * FROM counterparties WHERE company_id = ?1 ORDER BY name"
    )
    .bind(company_id)
    .fetch_all(pool)
    .await
}

pub async fn get_by_id(pool: &SqlitePool, id: &str) -> Result<Option<CounterpartyRow>, sqlx::Error> {
    sqlx::query_as::<_, CounterpartyRow>(
        "SELECT * FROM counterparties WHERE id = ?1"
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

pub async fn create(
    pool: &SqlitePool,
    company_id: &str,
    name: &str,
    org_number: Option<&str>,
    contact_info: Option<&str>,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO counterparties (id, company_id, name, org_number, contact_info) VALUES (?1, ?2, ?3, ?4, ?5)"
    )
    .bind(&id)
    .bind(company_id)
    .bind(name)
    .bind(org_number)
    .bind(contact_info)
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn update(
    pool: &SqlitePool,
    id: &str,
    name: &str,
    org_number: Option<&str>,
    contact_info: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE counterparties SET name = ?1, org_number = ?2, contact_info = ?3 WHERE id = ?4"
    )
    .bind(name)
    .bind(org_number)
    .bind(contact_info)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM counterparties WHERE id = ?1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
