use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::path::Path;
use std::str::FromStr;

pub async fn create_pool(db_path: &Path) -> Result<SqlitePool, sqlx::Error> {
    let url = format!("sqlite://{}?mode=rwc", db_path.display());
    let options = SqliteConnectOptions::from_str(&url)?
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .foreign_keys(true)
        .busy_timeout(std::time::Duration::from_secs(30));

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;

    Ok(pool)
}
