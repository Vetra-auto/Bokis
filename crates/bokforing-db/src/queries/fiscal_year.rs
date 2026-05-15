use bokforing_core::fiscal_year::generate_monthly_periods;
use chrono::NaiveDate;
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct FiscalYearRow {
    pub id: String,
    pub company_id: String,
    pub name: String,
    pub start_date: String,
    pub end_date: String,
    pub is_closed: bool,
    pub created_at: String,
}

pub async fn list(pool: &SqlitePool, company_id: &str) -> Result<Vec<FiscalYearRow>, sqlx::Error> {
    sqlx::query_as::<_, FiscalYearRow>(
        "SELECT * FROM fiscal_years WHERE company_id = ?1 ORDER BY start_date DESC"
    )
    .bind(company_id)
    .fetch_all(pool)
    .await
}

pub async fn get_by_id(pool: &SqlitePool, id: &str) -> Result<Option<FiscalYearRow>, sqlx::Error> {
    sqlx::query_as::<_, FiscalYearRow>("SELECT * FROM fiscal_years WHERE id = ?1")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn create_with_periods(
    pool: &SqlitePool,
    company_id: &str,
    name: &str,
    start_date: &str,
    end_date: &str,
) -> Result<String, sqlx::Error> {
    let fy_id = Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO fiscal_years (id, company_id, name, start_date, end_date) VALUES (?1, ?2, ?3, ?4, ?5)"
    )
    .bind(&fy_id)
    .bind(company_id)
    .bind(name)
    .bind(start_date)
    .bind(end_date)
    .execute(pool)
    .await?;

    let start = NaiveDate::parse_from_str(start_date, "%Y-%m-%d")
        .unwrap_or_else(|_| NaiveDate::from_ymd_opt(2026, 1, 1).unwrap());
    let end = NaiveDate::parse_from_str(end_date, "%Y-%m-%d")
        .unwrap_or_else(|_| NaiveDate::from_ymd_opt(2026, 12, 31).unwrap());

    let periods = generate_monthly_periods(start, end);

    for (p_start, p_end, year_month) in periods {
        let period_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO periods (id, fiscal_year_id, year_month, start_date, end_date) VALUES (?1, ?2, ?3, ?4, ?5)"
        )
        .bind(&period_id)
        .bind(&fy_id)
        .bind(&year_month)
        .bind(p_start.to_string())
        .bind(p_end.to_string())
        .execute(pool)
        .await?;
    }

    Ok(fy_id)
}

pub async fn close(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE fiscal_years SET is_closed = 1 WHERE id = ?1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
