use tauri::State;
use std::path::PathBuf;

use bokforing_db::queries;
use bokforing_core::hash_chain::compute_file_hash;
use crate::error::CommandError;
use crate::state::AppState;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentDto {
    pub id: String,
    pub voucher_id: String,
    pub filename: String,
    pub mime_type: String,
    pub file_size: i64,
    pub uploaded_at: String,
}

#[tauri::command]
pub async fn upload_attachment(
    state: State<'_, AppState>,
    voucher_id: String,
    file_path: String,
) -> Result<AttachmentDto, CommandError> {
    let user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    if !user.roles.iter().any(|r| r == "admin" || r == "bokforare") {
        return Err(CommandError::Forbidden);
    }

    let _voucher = queries::voucher::get_by_id(&state.pool, &voucher_id)
        .await?
        .ok_or(CommandError::NotFound("Verifikation".into()))?;

    let source = PathBuf::from(&file_path);
    if !source.exists() {
        return Err(CommandError::Validation("Filen finns inte".into()));
    }

    let filename = source.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "okänd".to_string());

    let file_data = std::fs::read(&source)
        .map_err(|e| CommandError::Internal(format!("Kunde inte läsa fil: {}", e)))?;

    let file_size = file_data.len() as i64;
    let file_hash = compute_file_hash(&file_data);

    let mime_type = mime_guess::from_path(&source)
        .first_or_octet_stream()
        .to_string();

    let attach_dir = state.data_dir.join("attachments").join(&voucher_id);
    std::fs::create_dir_all(&attach_dir)
        .map_err(|e| CommandError::Internal(format!("Kunde inte skapa katalog: {}", e)))?;

    let ext = source.extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let storage_filename = format!("{}{}", uuid::Uuid::new_v4(), ext);
    let storage_path = attach_dir.join(&storage_filename);

    std::fs::write(&storage_path, &file_data)
        .map_err(|e| CommandError::Internal(format!("Kunde inte spara bilaga: {}", e)))?;

    let id = queries::attachment::create(
        &state.pool,
        &voucher_id,
        &filename,
        &mime_type,
        file_size,
        &file_hash,
        &storage_path.to_string_lossy(),
        "receipt",
        &user.user_id,
    ).await?;

    let att = queries::attachment::get_by_id(&state.pool, &id)
        .await?
        .ok_or(CommandError::Internal("Kunde inte hämta bilaga".into()))?;

    Ok(AttachmentDto {
        id: att.id,
        voucher_id: att.voucher_id,
        filename: att.filename,
        mime_type: att.mime_type,
        file_size: att.file_size,
        uploaded_at: att.uploaded_at,
    })
}

#[tauri::command]
pub async fn list_attachments(
    state: State<'_, AppState>,
    voucher_id: String,
) -> Result<Vec<AttachmentDto>, CommandError> {
    let _user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    let attachments = queries::attachment::list_for_voucher(&state.pool, &voucher_id).await?;

    Ok(attachments.into_iter().map(|a| AttachmentDto {
        id: a.id,
        voucher_id: a.voucher_id,
        filename: a.filename,
        mime_type: a.mime_type,
        file_size: a.file_size,
        uploaded_at: a.uploaded_at,
    }).collect())
}
