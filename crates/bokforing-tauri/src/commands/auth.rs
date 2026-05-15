use tauri::State;

use bokforing_db::queries;
use crate::error::CommandError;
use crate::state::{AppState, AuthenticatedUser, CompanyContext};

#[tauri::command]
pub async fn login(
    state: State<'_, AppState>,
    username: String,
    password: String,
) -> Result<AuthenticatedUser, CommandError> {
    let user = queries::user::authenticate(&state.pool, &username, &password)
        .await?
        .ok_or(CommandError::Validation("Fel användarnamn eller lösenord".into()))?;

    let company: Option<(String, String)> = sqlx::query_as(
        "SELECT c.id, c.name FROM companies c JOIN role_assignments ra ON ra.company_id = c.id WHERE ra.user_id = ?1 LIMIT 1"
    )
    .bind(&user.id)
    .fetch_optional(&state.pool)
    .await?;

    let (company_id, company_name) = company.ok_or(CommandError::Validation("Ingen företagstillhörighet".into()))?;

    let roles = queries::user::get_roles(&state.pool, &user.id, &company_id).await?;

    let auth_user = AuthenticatedUser {
        user_id: user.id.clone(),
        username: user.username,
        display_name: user.display_name,
        roles,
        company_id: company_id.clone(),
    };

    *state.current_user.write().await = Some(auth_user.clone());
    *state.current_company.write().await = Some(CompanyContext {
        company_id,
        company_name,
        active_fiscal_year_id: None,
    });

    Ok(auth_user)
}

#[tauri::command]
pub async fn logout(state: State<'_, AppState>) -> Result<(), CommandError> {
    *state.current_user.write().await = None;
    *state.current_company.write().await = None;
    Ok(())
}

#[tauri::command]
pub async fn get_current_user(
    state: State<'_, AppState>,
) -> Result<Option<AuthenticatedUser>, CommandError> {
    Ok(state.current_user.read().await.clone())
}

#[tauri::command]
pub async fn change_password(
    state: State<'_, AppState>,
    new_password: String,
) -> Result<(), CommandError> {
    let user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    if new_password.len() < 8 {
        return Err(CommandError::Validation("Lösenordet måste vara minst 8 tecken".into()));
    }

    queries::user::change_password(&state.pool, &user.user_id, &new_password).await?;
    Ok(())
}
