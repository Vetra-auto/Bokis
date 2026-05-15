use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use sqlx::SqlitePool;

pub struct AppState {
    pub pool: SqlitePool,
    pub data_dir: PathBuf,
    pub current_user: Arc<RwLock<Option<AuthenticatedUser>>>,
    pub current_company: Arc<RwLock<Option<CompanyContext>>>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuthenticatedUser {
    pub user_id: String,
    pub username: String,
    pub display_name: String,
    pub roles: Vec<String>,
    pub company_id: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompanyContext {
    pub company_id: String,
    pub company_name: String,
    pub active_fiscal_year_id: Option<String>,
}
