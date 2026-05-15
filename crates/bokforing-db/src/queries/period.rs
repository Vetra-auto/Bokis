use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct PeriodRow {
    pub id: String,
    pub fiscal_year_id: String,
    pub year_month: String,
    pub start_date: String,
    pub end_date: String,
    pub status: String,
    pub closed_at: Option<String>,
    pub closed_by: Option<String>,
    pub locked_at: Option<String>,
    pub locked_by: Option<String>,
}

pub async fn list_by_fiscal_year(
    pool: &SqlitePool,
    fiscal_year_id: &str,
) -> Result<Vec<PeriodRow>, sqlx::Error> {
    sqlx::query_as::<_, PeriodRow>(
        "SELECT * FROM periods WHERE fiscal_year_id = ?1 ORDER BY year_month"
    )
    .bind(fiscal_year_id)
    .fetch_all(pool)
    .await
}

pub async fn get_by_id(pool: &SqlitePool, id: &str) -> Result<Option<PeriodRow>, sqlx::Error> {
    sqlx::query_as::<_, PeriodRow>("SELECT * FROM periods WHERE id = ?1")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn find_for_date(
    pool: &SqlitePool,
    fiscal_year_id: &str,
    date: &str,
) -> Result<Option<PeriodRow>, sqlx::Error> {
    sqlx::query_as::<_, PeriodRow>(
        "SELECT * FROM periods WHERE fiscal_year_id = ?1 AND start_date <= ?2 AND end_date >= ?2"
    )
    .bind(fiscal_year_id)
    .bind(date)
    .fetch_optional(pool)
    .await
}

pub async fn update_status(
    pool: &SqlitePool,
    id: &str,
    new_status: &str,
    user_id: &str,
) -> Result<(), sqlx::Error> {
    match new_status {
        "closed" => {
            sqlx::query(
                "UPDATE periods SET status = 'closed', closed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), closed_by = ?2 WHERE id = ?1"
            )
            .bind(id)
            .bind(user_id)
            .execute(pool)
            .await?;
        }
        "locked" => {
            sqlx::query(
                "UPDATE periods SET status = 'locked', locked_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), locked_by = ?2 WHERE id = ?1"
            )
            .bind(id)
            .bind(user_id)
            .execute(pool)
            .await?;
        }
        _ => {
            sqlx::query("UPDATE periods SET status = ?2 WHERE id = ?1")
                .bind(id)
                .bind(new_status)
                .execute(pool)
                .await?;
        }
    }

    let action = match new_status {
        "closed" => "close",
        "locked" => "lock",
        "open" => "reopen",
        _ => new_status,
    };

    sqlx::query(
        "INSERT INTO period_closes (id, period_id, action, performed_by) VALUES (?1, ?2, ?3, ?4)"
    )
    .bind(Uuid::new_v4().to_string())
    .bind(id)
    .bind(action)
    .bind(user_id)
    .execute(pool)
    .await?;

    Ok(())
}
