use sqlx::SqlitePool;
use uuid::Uuid;

use crate::seed::{hash_password, verify_password};

#[derive(Debug, sqlx::FromRow)]
pub struct UserRow {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub password_hash: String,
    pub is_active: bool,
    pub force_password_change: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, sqlx::FromRow)]
pub struct RoleRow {
    pub role: String,
}

pub async fn authenticate(
    pool: &SqlitePool,
    username: &str,
    password: &str,
) -> Result<Option<UserRow>, sqlx::Error> {
    let user = sqlx::query_as::<_, UserRow>(
        "SELECT * FROM users WHERE username = ?1 AND is_active = 1"
    )
    .bind(username)
    .fetch_optional(pool)
    .await?;

    match user {
        Some(u) if verify_password(password, &u.password_hash) => Ok(Some(u)),
        _ => Ok(None),
    }
}

pub async fn get_roles(pool: &SqlitePool, user_id: &str, company_id: &str) -> Result<Vec<String>, sqlx::Error> {
    let rows = sqlx::query_as::<_, RoleRow>(
        "SELECT role FROM role_assignments WHERE user_id = ?1 AND company_id = ?2"
    )
    .bind(user_id)
    .bind(company_id)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(|r| r.role).collect())
}

pub async fn create(
    pool: &SqlitePool,
    username: &str,
    display_name: &str,
    password: &str,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let password_hash = hash_password(password).expect("Failed to hash password");
    sqlx::query(
        "INSERT INTO users (id, username, display_name, password_hash) VALUES (?1, ?2, ?3, ?4)"
    )
    .bind(&id)
    .bind(username)
    .bind(display_name)
    .bind(&password_hash)
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn change_password(
    pool: &SqlitePool,
    user_id: &str,
    new_password: &str,
) -> Result<(), sqlx::Error> {
    let password_hash = hash_password(new_password).expect("Failed to hash password");
    sqlx::query(
        "UPDATE users SET password_hash = ?2, force_password_change = 0, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1"
    )
    .bind(user_id)
    .bind(&password_hash)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<UserRow>, sqlx::Error> {
    sqlx::query_as::<_, UserRow>("SELECT * FROM users ORDER BY username")
        .fetch_all(pool)
        .await
}

pub async fn set_active(pool: &SqlitePool, user_id: &str, is_active: bool) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET is_active = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1")
        .bind(user_id)
        .bind(is_active)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn assign_role(
    pool: &SqlitePool,
    user_id: &str,
    role: &str,
    company_id: &str,
    assigned_by: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT OR IGNORE INTO role_assignments (id, user_id, role, company_id, assigned_by) VALUES (?1, ?2, ?3, ?4, ?5)"
    )
    .bind(Uuid::new_v4().to_string())
    .bind(user_id)
    .bind(role)
    .bind(company_id)
    .bind(assigned_by)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn remove_role(
    pool: &SqlitePool,
    user_id: &str,
    role: &str,
    company_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM role_assignments WHERE user_id = ?1 AND role = ?2 AND company_id = ?3")
        .bind(user_id)
        .bind(role)
        .bind(company_id)
        .execute(pool)
        .await?;
    Ok(())
}
