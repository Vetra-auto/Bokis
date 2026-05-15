use tauri::State;

use bokforing_db::queries;
use bokforing_core::hash_chain::compute_file_hash;
use crate::error::CommandError;
use crate::state::AppState;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreResultDto {
    pub message: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupJobDto {
    pub id: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub status: String,
    pub backup_path: Option<String>,
    pub size_bytes: Option<i64>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupVerifyResultDto {
    pub is_valid: bool,
    pub details: String,
}

#[tauri::command]
pub async fn create_backup(
    state: State<'_, AppState>,
) -> Result<BackupJobDto, CommandError> {
    let user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    if !user.roles.iter().any(|r| r == "admin") {
        return Err(CommandError::Forbidden);
    }

    let backup_dir = state.data_dir.join("backups");
    std::fs::create_dir_all(&backup_dir)
        .map_err(|e| CommandError::Internal(format!("Kunde inte skapa backup-katalog: {}", e)))?;

    let filename = format!("backup_{}.db", chrono::Utc::now().format("%Y%m%d_%H%M%S"));
    let backup_path = backup_dir.join(&filename);

    let _job_id = queries::backup::create_backup(&state.pool, &backup_path, &user.user_id).await?;

    let jobs = queries::backup::list_jobs(&state.pool, 1).await?;
    let job = jobs.into_iter().next()
        .ok_or(CommandError::Internal("Backup-jobb hittades inte".into()))?;

    Ok(BackupJobDto {
        id: job.id,
        started_at: job.started_at,
        completed_at: job.completed_at,
        status: job.status,
        backup_path: job.backup_path,
        size_bytes: job.size_bytes,
    })
}

#[tauri::command]
pub async fn list_backups(
    state: State<'_, AppState>,
) -> Result<Vec<BackupJobDto>, CommandError> {
    let _user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    let jobs = queries::backup::list_jobs(&state.pool, 50).await?;

    Ok(jobs.into_iter().map(|j| BackupJobDto {
        id: j.id,
        started_at: j.started_at,
        completed_at: j.completed_at,
        status: j.status,
        backup_path: j.backup_path,
        size_bytes: j.size_bytes,
    }).collect())
}

#[tauri::command]
pub async fn verify_backup(
    state: State<'_, AppState>,
    backup_job_id: String,
) -> Result<BackupVerifyResultDto, CommandError> {
    let user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    if !user.roles.iter().any(|r| r == "admin") {
        return Err(CommandError::Forbidden);
    }

    let jobs = queries::backup::list_jobs(&state.pool, 100).await?;
    let job = jobs.into_iter()
        .find(|j| j.id == backup_job_id)
        .ok_or(CommandError::NotFound("Backup-jobb".into()))?;

    let backup_path = job.backup_path
        .ok_or(CommandError::Validation("Backup har ingen fil".into()))?;

    let file_data = std::fs::read(&backup_path)
        .map_err(|e| CommandError::Internal(format!("Kunde inte läsa backup-fil: {}", e)))?;

    let mut issues = Vec::new();

    if let Some(expected_hash) = &job.backup_hash {
        let actual_hash = compute_file_hash(&file_data);
        if &actual_hash != expected_hash {
            issues.push(format!("Hash-mismatch: förväntad {}, fick {}", expected_hash, actual_hash));
        }
    }

    let backup_pool = sqlx::SqlitePool::connect(&format!("sqlite:{}?mode=ro", backup_path))
        .await
        .map_err(|e| CommandError::Internal(format!("Kunde inte öppna backup: {}", e)))?;

    let integrity: (String,) = sqlx::query_as("PRAGMA integrity_check")
        .fetch_one(&backup_pool)
        .await
        .map_err(|e| CommandError::Internal(format!("Integrity check misslyckades: {}", e)))?;

    if integrity.0 != "ok" {
        issues.push(format!("Integrity check: {}", integrity.0));
    }

    backup_pool.close().await;

    let is_valid = issues.is_empty();
    let details = if is_valid {
        "Backup verifierad: integritet OK, hash stämmer".to_string()
    } else {
        issues.join("; ")
    };

    queries::backup::record_verification(
        &state.pool,
        &backup_job_id,
        &user.user_id,
        is_valid,
        &details,
    ).await?;

    Ok(BackupVerifyResultDto { is_valid, details })
}

#[tauri::command]
pub async fn restore_backup(
    state: State<'_, AppState>,
    backup_job_id: String,
) -> Result<RestoreResultDto, CommandError> {
    let user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    if !user.roles.iter().any(|r| r == "admin") {
        return Err(CommandError::Forbidden);
    }

    let job = queries::backup::get_job_by_id(&state.pool, &backup_job_id).await?
        .ok_or(CommandError::NotFound("Backup-jobb".into()))?;

    if job.status != "completed" {
        return Err(CommandError::Validation("Kan bara återställa slutförda backuper".into()));
    }

    let backup_path_str = job.backup_path
        .ok_or(CommandError::Validation("Backup har ingen fil".into()))?;

    let backup_path = std::path::Path::new(&backup_path_str);
    if !backup_path.exists() {
        return Err(CommandError::NotFound("Backup-filen saknas på disk".into()));
    }

    let file_data = std::fs::read(backup_path)
        .map_err(|e| CommandError::Internal(format!("Kunde inte läsa backup-fil: {}", e)))?;

    if let Some(expected_hash) = &job.backup_hash {
        let actual_hash = compute_file_hash(&file_data);
        if &actual_hash != expected_hash {
            return Err(CommandError::Validation(format!(
                "Hash-verifiering misslyckades. Backup-filen kan vara korrupt."
            )));
        }
    }

    let backup_pool = sqlx::SqlitePool::connect(&format!("sqlite:{}?mode=ro", backup_path_str))
        .await
        .map_err(|e| CommandError::Internal(format!("Kunde inte öppna backup: {}", e)))?;

    let integrity: (String,) = sqlx::query_as("PRAGMA integrity_check")
        .fetch_one(&backup_pool)
        .await
        .map_err(|e| CommandError::Internal(format!("Integrity check misslyckades: {}", e)))?;

    backup_pool.close().await;

    if integrity.0 != "ok" {
        return Err(CommandError::Validation(format!(
            "Backup-filen klarade inte integritetskontroll: {}", integrity.0
        )));
    }

    let db_path = state.data_dir.join("bokforing.db");

    state.pool.close().await;

    let wal_path = db_path.with_extension("db-wal");
    let shm_path = db_path.with_extension("db-shm");
    let _ = std::fs::remove_file(&wal_path);
    let _ = std::fs::remove_file(&shm_path);

    std::fs::copy(backup_path, &db_path)
        .map_err(|e| CommandError::Internal(format!("Kunde inte kopiera backup: {}", e)))?;

    Ok(RestoreResultDto {
        message: "Backup återställd. Starta om applikationen för att använda den återställda databasen.".into(),
    })
}

#[tauri::command]
pub async fn restore_from_file(
    state: State<'_, AppState>,
    file_path: String,
) -> Result<RestoreResultDto, CommandError> {
    let user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    if !user.roles.iter().any(|r| r == "admin") {
        return Err(CommandError::Forbidden);
    }

    let backup_path = std::path::Path::new(&file_path);
    if !backup_path.exists() {
        return Err(CommandError::NotFound("Filen finns inte".into()));
    }

    let ext = backup_path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if ext != "db" && ext != "sqlite" && ext != "sqlite3" {
        return Err(CommandError::Validation(
            "Filen måste vara en SQLite-databas (.db, .sqlite, .sqlite3)".into()
        ));
    }

    let backup_pool = sqlx::SqlitePool::connect(&format!("sqlite:{}?mode=ro", file_path))
        .await
        .map_err(|e| CommandError::Validation(format!("Kunde inte öppna filen som databas: {}", e)))?;

    let integrity: (String,) = sqlx::query_as("PRAGMA integrity_check")
        .fetch_one(&backup_pool)
        .await
        .map_err(|e| CommandError::Internal(format!("Integrity check misslyckades: {}", e)))?;

    let has_vouchers: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='vouchers')")
        .fetch_one(&backup_pool)
        .await
        .unwrap_or(false);

    backup_pool.close().await;

    if integrity.0 != "ok" {
        return Err(CommandError::Validation(format!(
            "Filen klarade inte integritetskontroll: {}", integrity.0
        )));
    }

    if !has_vouchers {
        return Err(CommandError::Validation(
            "Filen verkar inte vara en Bokis-databas (saknar tabellen 'vouchers')".into()
        ));
    }

    let db_path = state.data_dir.join("bokforing.db");

    state.pool.close().await;

    let wal_path = db_path.with_extension("db-wal");
    let shm_path = db_path.with_extension("db-shm");
    let _ = std::fs::remove_file(&wal_path);
    let _ = std::fs::remove_file(&shm_path);

    std::fs::copy(backup_path, &db_path)
        .map_err(|e| CommandError::Internal(format!("Kunde inte kopiera fil: {}", e)))?;

    Ok(RestoreResultDto {
        message: "Databas återställd från fil. Starta om applikationen.".into(),
    })
}
