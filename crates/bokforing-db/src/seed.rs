use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use sqlx::SqlitePool;
use uuid::Uuid;

pub fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2.hash_password(password.as_bytes(), &salt)?;
    Ok(hash.to_string())
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

pub async fn needs_setup(pool: &SqlitePool) -> Result<bool, sqlx::Error> {
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await?;
    Ok(count.0 == 0)
}

pub async fn initial_setup(
    pool: &SqlitePool,
    password: &str,
    company_name: &str,
    org_number: &str,
) -> Result<(), sqlx::Error> {
    let password_hash = hash_password(password).expect("Failed to hash password");
    let user_id = Uuid::new_v4().to_string();
    let company_id = Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO companies (id, name, org_number) VALUES (?1, ?2, ?3)"
    )
    .bind(&company_id)
    .bind(company_name)
    .bind(org_number)
    .execute(pool)
    .await?;

    sqlx::query(
        "INSERT INTO users (id, username, display_name, password_hash, is_active, force_password_change) VALUES (?1, ?2, ?3, ?4, 1, 0)"
    )
    .bind(&user_id)
    .bind("admin")
    .bind("Administratör")
    .bind(&password_hash)
    .execute(pool)
    .await?;

    sqlx::query(
        "INSERT INTO role_assignments (id, user_id, role, company_id) VALUES (?1, ?2, ?3, ?4)"
    )
    .bind(Uuid::new_v4().to_string())
    .bind(&user_id)
    .bind("admin")
    .bind(&company_id)
    .execute(pool)
    .await?;

    Ok(())
}
