use tauri::State;

use bokforing_db::seed;
use crate::error::CommandError;
use crate::state::AppState;

#[tauri::command]
pub async fn check_needs_setup(
    state: State<'_, AppState>,
) -> Result<bool, CommandError> {
    Ok(seed::needs_setup(&state.pool).await?)
}

#[tauri::command]
pub async fn run_initial_setup(
    state: State<'_, AppState>,
    password: String,
    company_name: String,
    org_number: String,
) -> Result<(), CommandError> {
    if !seed::needs_setup(&state.pool).await? {
        return Err(CommandError::Validation("Systemet är redan konfigurerat".into()));
    }

    if password.len() < 12 {
        return Err(CommandError::Validation("Lösenordet måste vara minst 12 tecken".into()));
    }

    seed::initial_setup(&state.pool, &password, &company_name, &org_number).await?;

    Ok(())
}
