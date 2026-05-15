use tauri::State;

use bokforing_db::queries;
use crate::error::CommandError;
use crate::state::AppState;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CounterpartyDto {
    pub id: String,
    pub name: String,
    pub org_number: Option<String>,
    pub contact_info: Option<String>,
    pub created_at: String,
}

#[tauri::command]
pub async fn list_counterparties(
    state: State<'_, AppState>,
) -> Result<Vec<CounterpartyDto>, CommandError> {
    let _user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    let company = state.current_company.read().await
        .as_ref()
        .ok_or(CommandError::Internal("Inget företag valt".into()))?
        .clone();

    let rows = queries::counterparty::list(&state.pool, &company.company_id).await?;

    Ok(rows.into_iter().map(|r| CounterpartyDto {
        id: r.id,
        name: r.name,
        org_number: r.org_number,
        contact_info: r.contact_info,
        created_at: r.created_at,
    }).collect())
}

#[tauri::command]
pub async fn create_counterparty(
    state: State<'_, AppState>,
    name: String,
    org_number: Option<String>,
    contact_info: Option<String>,
) -> Result<CounterpartyDto, CommandError> {
    let user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    let company = state.current_company.read().await
        .as_ref()
        .ok_or(CommandError::Internal("Inget företag valt".into()))?
        .clone();

    if name.trim().is_empty() {
        return Err(CommandError::Validation("Namn krävs".into()));
    }

    let id = queries::counterparty::create(
        &state.pool,
        &company.company_id,
        name.trim(),
        org_number.as_deref(),
        contact_info.as_deref(),
    ).await?;

    queries::audit::append_event(
        &state.pool,
        "counterparty_created",
        "counterparty",
        &id,
        &user.user_id,
        &serde_json::json!({"name": name}).to_string(),
    ).await?;

    Ok(CounterpartyDto {
        id,
        name,
        org_number,
        contact_info,
        created_at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    })
}

#[tauri::command]
pub async fn update_counterparty(
    state: State<'_, AppState>,
    id: String,
    name: String,
    org_number: Option<String>,
    contact_info: Option<String>,
) -> Result<(), CommandError> {
    let user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    if name.trim().is_empty() {
        return Err(CommandError::Validation("Namn krävs".into()));
    }

    queries::counterparty::update(
        &state.pool,
        &id,
        name.trim(),
        org_number.as_deref(),
        contact_info.as_deref(),
    ).await?;

    queries::audit::append_event(
        &state.pool,
        "counterparty_updated",
        "counterparty",
        &id,
        &user.user_id,
        &serde_json::json!({"name": name}).to_string(),
    ).await?;

    Ok(())
}

#[tauri::command]
pub async fn delete_counterparty(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), CommandError> {
    let user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    queries::counterparty::delete(&state.pool, &id).await?;

    queries::audit::append_event(
        &state.pool,
        "counterparty_deleted",
        "counterparty",
        &id,
        &user.user_id,
        "{}",
    ).await?;

    Ok(())
}
