use tauri::State;

use bokforing_db::queries;
use bokforing_core::hash_chain::compute_file_hash;
use crate::error::CommandError;
use crate::state::AppState;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveManifestDto {
    pub id: String,
    pub fiscal_year_id: String,
    pub created_at: String,
    pub archive_path: String,
    pub content_hash: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemDocDto {
    pub id: String,
    pub generated_at: String,
    pub content_hash: String,
}

#[tauri::command]
pub async fn create_archive(
    state: State<'_, AppState>,
    fiscal_year_id: String,
) -> Result<ArchiveManifestDto, CommandError> {
    let user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    if !user.roles.iter().any(|r| r == "admin") {
        return Err(CommandError::Forbidden);
    }

    let company = state.current_company.read().await
        .as_ref()
        .ok_or(CommandError::Internal("Inget företag valt".into()))?
        .clone();

    let fy = queries::fiscal_year::get_by_id(&state.pool, &fiscal_year_id)
        .await?
        .ok_or(CommandError::NotFound("Räkenskapsår".into()))?;

    if !fy.is_closed {
        return Err(CommandError::Validation("Räkenskapsåret måste vara stängt innan arkivering".into()));
    }

    let archive_dir = state.data_dir.join("archives");
    std::fs::create_dir_all(&archive_dir)
        .map_err(|e| CommandError::Internal(format!("Kunde inte skapa arkivkatalog: {}", e)))?;

    let archive_filename = format!("archive_{}_{}.db", fy.name, chrono::Utc::now().format("%Y%m%d"));
    let archive_path = archive_dir.join(&archive_filename);

    let result = sqlx::query(&format!("VACUUM INTO '{}'", archive_path.to_string_lossy()))
        .execute(&state.pool)
        .await;

    if let Err(e) = result {
        return Err(CommandError::Internal(format!("Arkivering misslyckades: {}", e)));
    }

    let file_data = std::fs::read(&archive_path)
        .map_err(|e| CommandError::Internal(format!("Kunde inte läsa arkivfil: {}", e)))?;
    let content_hash = compute_file_hash(&file_data);

    let accounts = queries::account::list(&state.pool, &company.company_id).await?;
    let vouchers = queries::voucher::list_by_fiscal_year(&state.pool, &fiscal_year_id).await?;
    let posted_count = vouchers.iter().filter(|v| v.status == "posted").count();

    let manifest = serde_json::json!({
        "fiscal_year": fy.name,
        "start_date": fy.start_date,
        "end_date": fy.end_date,
        "accounts_count": accounts.len(),
        "vouchers_posted": posted_count,
        "archive_file": archive_filename,
        "content_hash": content_hash,
        "archived_at": chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        "retention_years": 7,
    });

    let id = queries::archive::create(
        &state.pool,
        &fiscal_year_id,
        &user.user_id,
        &archive_path.to_string_lossy(),
        &content_hash,
        &manifest.to_string(),
    ).await?;

    queries::audit::append_event(
        &state.pool,
        "archive_created",
        "fiscal_year",
        &fiscal_year_id,
        &user.user_id,
        &manifest.to_string(),
    ).await?;

    Ok(ArchiveManifestDto {
        id,
        fiscal_year_id,
        created_at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        archive_path: archive_path.to_string_lossy().to_string(),
        content_hash,
    })
}

#[tauri::command]
pub async fn list_archives(
    state: State<'_, AppState>,
) -> Result<Vec<ArchiveManifestDto>, CommandError> {
    let _user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    let archives = queries::archive::list_all(&state.pool).await?;

    Ok(archives.into_iter().map(|a| ArchiveManifestDto {
        id: a.id,
        fiscal_year_id: a.fiscal_year_id,
        created_at: a.created_at,
        archive_path: a.archive_path,
        content_hash: a.content_hash,
    }).collect())
}

#[tauri::command]
pub async fn generate_system_doc(
    state: State<'_, AppState>,
) -> Result<SystemDocDto, CommandError> {
    let user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    if !user.roles.iter().any(|r| r == "admin") {
        return Err(CommandError::Forbidden);
    }

    let company = state.current_company.read().await
        .as_ref()
        .ok_or(CommandError::Internal("Inget företag valt".into()))?
        .clone();

    let company_row: Option<(String, String)> = sqlx::query_as(
        "SELECT name, org_number FROM companies WHERE id = ?1"
    )
    .bind(&company.company_id)
    .fetch_optional(&state.pool)
    .await?;
    let (company_name, org_number) = company_row.unwrap_or(("".to_string(), "".to_string()));

    let accounts = queries::account::list(&state.pool, &company.company_id).await?;
    let fiscal_years = queries::fiscal_year::list(&state.pool, &company.company_id).await?;
    let users = queries::user::list(&state.pool).await?;
    let archives = queries::archive::list_all(&state.pool).await?;
    let backups = queries::backup::list_jobs(&state.pool, 10).await?;

    let doc = serde_json::json!({
        "system": "Bokis",
        "version": "0.1.0",
        "generated_at": chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        "company": {
            "name": company_name,
            "org_number": org_number,
        },
        "description": "Systemdokumentation enligt BFNAR 2013:2. Bokis är ett lokalt bokföringssystem med append-only verifikationer, SHA-256 hashkedja för revisionsloggen, periodlåsning och SIE4/CSV/JSON-export.",
        "data_storage": {
            "type": "SQLite (WAL-mode)",
            "location": "Lokal appdata-katalog",
            "encryption": "Ingen (lokal desktop-app)",
        },
        "accounts": {
            "count": accounts.len(),
            "standard": "BAS 2024",
        },
        "fiscal_years": fiscal_years.iter().map(|fy| serde_json::json!({
            "name": fy.name,
            "start_date": fy.start_date,
            "end_date": fy.end_date,
            "is_closed": fy.is_closed,
        })).collect::<Vec<_>>(),
        "users": users.iter().map(|u| serde_json::json!({
            "username": u.username,
            "display_name": u.display_name,
            "is_active": u.is_active,
        })).collect::<Vec<_>>(),
        "archives": archives.len(),
        "recent_backups": backups.len(),
        "retention_policy": "7 år enligt BFL 7 kap. 2§",
        "audit_trail": "SHA-256 hashkedja, append-only",
    });

    let content_json = serde_json::to_string_pretty(&doc)
        .map_err(|e| CommandError::Internal(format!("JSON-serialisering misslyckades: {}", e)))?;
    let content_hash = compute_file_hash(content_json.as_bytes());

    let id = queries::system_doc::create(
        &state.pool,
        &user.user_id,
        &content_json,
        &content_hash,
    ).await?;

    queries::audit::append_event(
        &state.pool,
        "system_doc_generated",
        "system",
        &id,
        &user.user_id,
        "{}",
    ).await?;

    Ok(SystemDocDto {
        id,
        generated_at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        content_hash,
    })
}

#[tauri::command]
pub async fn list_system_docs(
    state: State<'_, AppState>,
) -> Result<Vec<SystemDocDto>, CommandError> {
    let _user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    let docs = queries::system_doc::list(&state.pool, 20).await?;
    Ok(docs.into_iter().map(|d| SystemDocDto {
        id: d.id,
        generated_at: d.generated_at,
        content_hash: d.content_hash,
    }).collect())
}
