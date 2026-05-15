use tauri::State;

use bokforing_db::queries;
use bokforing_core::hash_chain::{verify_chain, ChainEvent};
use crate::error::CommandError;
use crate::state::AppState;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEventDto {
    pub id: String,
    pub sequence_number: i64,
    pub event_type: String,
    pub entity_type: String,
    pub entity_id: String,
    pub user_id: String,
    pub occurred_at: String,
    pub detail_json: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainVerificationResult {
    pub valid: bool,
    pub total_events: usize,
    pub error: Option<String>,
}

#[tauri::command]
pub async fn list_audit_events(
    state: State<'_, AppState>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<AuditEventDto>, CommandError> {
    let _user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    let events = queries::audit::list_events(
        &state.pool,
        limit.unwrap_or(100),
        offset.unwrap_or(0),
    ).await?;

    Ok(events.into_iter().map(|e| AuditEventDto {
        id: e.id,
        sequence_number: e.sequence_number,
        event_type: e.event_type,
        entity_type: e.entity_type,
        entity_id: e.entity_id,
        user_id: e.user_id.unwrap_or_default(),
        occurred_at: e.occurred_at,
        detail_json: e.detail_json.unwrap_or_default(),
    }).collect())
}

#[tauri::command]
pub async fn verify_audit_chain(
    state: State<'_, AppState>,
) -> Result<ChainVerificationResult, CommandError> {
    let user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    if !user.roles.iter().any(|r| r == "admin" || r == "revisor") {
        return Err(CommandError::Forbidden);
    }

    let events = queries::audit::get_all_events_ordered(&state.pool).await?;
    let total = events.len();

    let chain_events: Vec<_> = events.iter().map(|e| {
        ChainEvent {
            sequence_number: e.sequence_number,
            event_type: e.event_type.clone(),
            entity_id: e.entity_id.clone(),
            occurred_at: e.occurred_at.clone(),
            detail_json: e.detail_json.clone().unwrap_or_default(),
            previous_hash: e.previous_hash.clone(),
            event_hash: e.event_hash.clone(),
        }
    }).collect();

    match verify_chain(&chain_events) {
        Ok(()) => Ok(ChainVerificationResult {
            valid: true,
            total_events: total,
            error: None,
        }),
        Err(e) => Ok(ChainVerificationResult {
            valid: false,
            total_events: total,
            error: Some(e.to_string()),
        }),
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingHistoryDto {
    pub id: String,
    pub event_type: String,
    pub entity_type: String,
    pub entity_id: String,
    pub user_id: String,
    pub occurred_at: String,
    pub description: String,
    pub metadata_json: Option<String>,
}

#[tauri::command]
pub async fn list_processing_history(
    state: State<'_, AppState>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<ProcessingHistoryDto>, CommandError> {
    let _user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    let events = queries::audit::list_processing_history(
        &state.pool,
        limit.unwrap_or(100),
        offset.unwrap_or(0),
    ).await?;

    Ok(events.into_iter().map(|e| ProcessingHistoryDto {
        id: e.id,
        event_type: e.event_type,
        entity_type: e.entity_type,
        entity_id: e.entity_id,
        user_id: e.user_id.unwrap_or_default(),
        occurred_at: e.occurred_at,
        description: e.description,
        metadata_json: e.metadata_json,
    }).collect())
}
