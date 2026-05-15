use tauri::State;

use bokforing_db::queries;
use crate::error::CommandError;
use crate::state::AppState;

#[derive(serde::Serialize)]
pub struct FiscalYearDto {
    pub id: String,
    pub name: String,
    pub start_date: String,
    pub end_date: String,
    pub is_closed: bool,
}

#[derive(serde::Serialize)]
pub struct UserDto {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub is_active: bool,
    pub roles: Vec<String>,
}

#[derive(serde::Serialize)]
#[allow(dead_code)]
pub struct CompanyDto {
    pub id: String,
    pub name: String,
    pub org_number: String,
}

#[tauri::command]
pub async fn list_fiscal_years(
    state: State<'_, AppState>,
) -> Result<Vec<FiscalYearDto>, CommandError> {
    let _user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    let company = state.current_company.read().await
        .as_ref()
        .ok_or(CommandError::Internal("Inget företag valt".into()))?
        .clone();

    let rows = queries::fiscal_year::list(&state.pool, &company.company_id).await?;

    Ok(rows.into_iter().map(|r| FiscalYearDto {
        id: r.id,
        name: r.name,
        start_date: r.start_date,
        end_date: r.end_date,
        is_closed: r.is_closed,
    }).collect())
}

#[tauri::command]
pub async fn create_fiscal_year(
    state: State<'_, AppState>,
    name: String,
    start_date: String,
    end_date: String,
) -> Result<FiscalYearDto, CommandError> {
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

    let id = queries::fiscal_year::create_with_periods(
        &state.pool,
        &company.company_id,
        &name,
        &start_date,
        &end_date,
    ).await?;

    Ok(FiscalYearDto {
        id,
        name,
        start_date,
        end_date,
        is_closed: false,
    })
}

#[tauri::command]
pub async fn set_active_fiscal_year(
    state: State<'_, AppState>,
    fiscal_year_id: String,
) -> Result<FiscalYearDto, CommandError> {
    let _user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    let fy = queries::fiscal_year::get_by_id(&state.pool, &fiscal_year_id).await?
        .ok_or(CommandError::NotFound("Räkenskapsår ej funnet".into()))?;

    {
        let mut company = state.current_company.write().await;
        if let Some(ref mut c) = *company {
            c.active_fiscal_year_id = Some(fiscal_year_id);
        }
    }

    Ok(FiscalYearDto {
        id: fy.id,
        name: fy.name,
        start_date: fy.start_date,
        end_date: fy.end_date,
        is_closed: fy.is_closed,
    })
}

#[tauri::command]
pub async fn list_users(
    state: State<'_, AppState>,
) -> Result<Vec<UserDto>, CommandError> {
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

    let users = queries::user::list(&state.pool).await?;
    let mut result = Vec::new();
    for u in users {
        let roles = queries::user::get_roles(&state.pool, &u.id, &company.company_id).await?;
        result.push(UserDto {
            id: u.id,
            username: u.username,
            display_name: u.display_name,
            is_active: u.is_active,
            roles,
        });
    }
    Ok(result)
}

#[tauri::command]
pub async fn create_user(
    state: State<'_, AppState>,
    username: String,
    display_name: String,
    password: String,
    role: String,
) -> Result<UserDto, CommandError> {
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

    let id = queries::user::create(&state.pool, &username, &display_name, &password).await?;
    queries::user::assign_role(&state.pool, &id, &role, &company.company_id, &user.user_id).await?;

    Ok(UserDto {
        id,
        username,
        display_name,
        is_active: true,
        roles: vec![role],
    })
}
