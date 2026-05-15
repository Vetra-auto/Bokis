use tauri::State;

use bokforing_db::queries;
use crate::error::CommandError;
use crate::state::AppState;

#[derive(serde::Serialize)]
pub struct AccountDto {
    pub id: String,
    pub account_number: i64,
    pub name: String,
    pub account_type: String,
    pub is_active: bool,
    pub sru_code: Option<String>,
}

#[tauri::command]
pub async fn list_accounts(
    state: State<'_, AppState>,
) -> Result<Vec<AccountDto>, CommandError> {
    let _user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    let company = state.current_company.read().await
        .as_ref()
        .ok_or(CommandError::Internal("Inget företag valt".into()))?
        .clone();

    let accounts = queries::account::list_active(&state.pool, &company.company_id).await?;

    Ok(accounts.into_iter().map(|a| AccountDto {
        id: a.id,
        account_number: a.account_number,
        name: a.name,
        account_type: a.account_type,
        is_active: a.is_active,
        sru_code: a.sru_code,
    }).collect())
}

#[tauri::command]
pub async fn create_account(
    state: State<'_, AppState>,
    account_number: i64,
    name: String,
    account_type: String,
    sru_code: Option<String>,
) -> Result<AccountDto, CommandError> {
    let user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    if !user.roles.iter().any(|r| r == "admin" || r == "bokforare") {
        return Err(CommandError::Forbidden);
    }

    let company = state.current_company.read().await
        .as_ref()
        .ok_or(CommandError::Internal("Inget företag valt".into()))?
        .clone();

    if !(1000..=9999).contains(&account_number) {
        return Err(CommandError::Validation("Kontonummer måste vara mellan 1000-9999".into()));
    }

    let id = queries::account::create(
        &state.pool,
        &company.company_id,
        account_number,
        &name,
        &account_type,
        sru_code.as_deref(),
        None,
        None,
    ).await?;

    Ok(AccountDto {
        id,
        account_number,
        name,
        account_type,
        is_active: true,
        sru_code,
    })
}
