use tauri::State;

use bokforing_core::period::can_transition;
use bokforing_core::types::{PeriodStatus, Role};
use bokforing_db::queries;
use crate::error::CommandError;
use crate::state::AppState;

#[derive(serde::Serialize)]
pub struct PeriodDto {
    pub id: String,
    pub year_month: String,
    pub start_date: String,
    pub end_date: String,
    pub status: String,
    pub closed_at: Option<String>,
    pub locked_at: Option<String>,
}

#[tauri::command]
pub async fn list_periods(
    state: State<'_, AppState>,
    fiscal_year_id: String,
) -> Result<Vec<PeriodDto>, CommandError> {
    let _user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    let periods = queries::period::list_by_fiscal_year(&state.pool, &fiscal_year_id).await?;

    Ok(periods.into_iter().map(|p| PeriodDto {
        id: p.id,
        year_month: p.year_month,
        start_date: p.start_date,
        end_date: p.end_date,
        status: p.status,
        closed_at: p.closed_at,
        locked_at: p.locked_at,
    }).collect())
}

#[tauri::command]
pub async fn transition_period(
    state: State<'_, AppState>,
    period_id: String,
    new_status: String,
) -> Result<PeriodDto, CommandError> {
    let user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    let period = queries::period::get_by_id(&state.pool, &period_id)
        .await?
        .ok_or(CommandError::NotFound("Period".into()))?;

    let from = parse_period_status(&period.status)?;
    let to = parse_period_status(&new_status)?;
    let role = parse_highest_role(&user.roles);

    can_transition(from, to, &role).map_err(|e| CommandError::Validation(e.to_string()))?;

    queries::period::update_status(&state.pool, &period_id, &new_status, &user.user_id).await?;

    queries::audit::append_event(
        &state.pool,
        "period_transition",
        "period",
        &period_id,
        &user.user_id,
        &serde_json::json!({"from": period.status, "to": new_status}).to_string(),
    ).await?;

    let updated = queries::period::get_by_id(&state.pool, &period_id)
        .await?
        .ok_or(CommandError::Internal("Period försvann".into()))?;

    Ok(PeriodDto {
        id: updated.id,
        year_month: updated.year_month,
        start_date: updated.start_date,
        end_date: updated.end_date,
        status: updated.status,
        closed_at: updated.closed_at,
        locked_at: updated.locked_at,
    })
}

fn parse_period_status(s: &str) -> Result<PeriodStatus, CommandError> {
    match s {
        "open" => Ok(PeriodStatus::Open),
        "under_review" => Ok(PeriodStatus::UnderReview),
        "closed" => Ok(PeriodStatus::Closed),
        "locked" => Ok(PeriodStatus::Locked),
        _ => Err(CommandError::Validation(format!("Okänd periodstatus: {}", s))),
    }
}

fn parse_highest_role(roles: &[String]) -> Role {
    if roles.iter().any(|r| r == "admin") {
        Role::Admin
    } else if roles.iter().any(|r| r == "bokforare") {
        Role::Bokforare
    } else if roles.iter().any(|r| r == "attest") {
        Role::Attest
    } else {
        Role::Revisor
    }
}
