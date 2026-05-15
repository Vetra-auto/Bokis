use tauri::State;

use bokforing_db::queries;
use crate::error::CommandError;
use crate::state::AppState;

#[derive(serde::Serialize)]
pub struct VoucherDto {
    pub id: String,
    pub voucher_series: String,
    pub voucher_number: i64,
    pub voucher_date: String,
    pub description: String,
    pub status: String,
    pub created_at: String,
    pub posted_at: Option<String>,
}

#[derive(serde::Serialize)]
#[allow(dead_code)]
pub struct VoucherLineDto {
    pub id: String,
    pub line_number: i64,
    pub account_id: String,
    pub debit_amount: i64,
    pub credit_amount: i64,
    pub description: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateVoucherRequest {
    pub voucher_date: String,
    pub description: String,
    pub voucher_series: Option<String>,
    pub counterparty_id: Option<String>,
    pub lines: Vec<CreateVoucherLineRequest>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateVoucherLineRequest {
    pub account_id: String,
    pub debit_amount: i64,
    pub credit_amount: i64,
    pub description: Option<String>,
}

#[tauri::command]
pub async fn create_voucher(
    state: State<'_, AppState>,
    request: CreateVoucherRequest,
) -> Result<VoucherDto, CommandError> {
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

    let fiscal_year_id = company.active_fiscal_year_id
        .ok_or(CommandError::Validation("Inget räkenskapsår valt".into()))?;

    let period = queries::period::find_for_date(&state.pool, &fiscal_year_id, &request.voucher_date)
        .await?
        .ok_or(CommandError::Validation("Ingen period matchar datumet".into()))?;

    if period.status != "open" {
        return Err(CommandError::Validation(format!("Perioden {} är inte öppen", period.year_month)));
    }

    let series = request.voucher_series.as_deref().unwrap_or("A");
    let number = queries::voucher::next_voucher_number(
        &state.pool, &company.company_id, &fiscal_year_id, series
    ).await?;

    let voucher_id = queries::voucher::create(
        &state.pool,
        &company.company_id,
        &fiscal_year_id,
        &period.id,
        series,
        number,
        &request.voucher_date,
        &request.description,
        request.counterparty_id.as_deref(),
        &user.user_id,
    ).await?;

    for (i, line) in request.lines.iter().enumerate() {
        queries::voucher::add_line(
            &state.pool,
            &voucher_id,
            (i + 1) as i64,
            &line.account_id,
            line.debit_amount,
            line.credit_amount,
            line.description.as_deref(),
            None,
            None,
            None,
        ).await?;
    }

    let v = queries::voucher::get_by_id(&state.pool, &voucher_id)
        .await?
        .ok_or(CommandError::Internal("Kunde inte hämta skapad verifikation".into()))?;

    Ok(VoucherDto {
        id: v.id,
        voucher_series: v.voucher_series,
        voucher_number: v.voucher_number,
        voucher_date: v.voucher_date,
        description: v.description,
        status: v.status,
        created_at: v.created_at,
        posted_at: v.posted_at,
    })
}

#[tauri::command]
pub async fn post_voucher(
    state: State<'_, AppState>,
    voucher_id: String,
) -> Result<VoucherDto, CommandError> {
    let user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    if !user.roles.iter().any(|r| r == "admin" || r == "bokforare") {
        return Err(CommandError::Forbidden);
    }

    let voucher = queries::voucher::get_by_id(&state.pool, &voucher_id)
        .await?
        .ok_or(CommandError::NotFound("Verifikation".into()))?;

    if voucher.status != "draft" {
        return Err(CommandError::Validation("Verifikationen är redan bokförd".into()));
    }

    let lines = queries::voucher::get_lines(&state.pool, &voucher_id).await?;
    if lines.len() < 2 {
        return Err(CommandError::Validation("Minst 2 rader krävs".into()));
    }

    let total_debit: i64 = lines.iter().map(|l| l.debit_amount).sum();
    let total_credit: i64 = lines.iter().map(|l| l.credit_amount).sum();
    if total_debit != total_credit {
        return Err(CommandError::Validation(format!(
            "Debet ({}) och kredit ({}) balanserar inte",
            total_debit, total_credit
        )));
    }

    queries::voucher::mark_posted(&state.pool, &voucher_id, &user.user_id).await?;

    queries::audit::append_event(
        &state.pool,
        "voucher_posted",
        "voucher",
        &voucher_id,
        &user.user_id,
        &serde_json::json!({"series": voucher.voucher_series, "number": voucher.voucher_number}).to_string(),
    ).await?;

    queries::audit::append_processing_history(
        &state.pool,
        "voucher_posted",
        "voucher",
        &voucher_id,
        &user.user_id,
        &format!("Verifikation {}{} bokförd", voucher.voucher_series, voucher.voucher_number),
        None,
    ).await?;

    let v = queries::voucher::get_by_id(&state.pool, &voucher_id)
        .await?
        .ok_or(CommandError::Internal("Kunde inte hämta verifikation".into()))?;

    Ok(VoucherDto {
        id: v.id,
        voucher_series: v.voucher_series,
        voucher_number: v.voucher_number,
        voucher_date: v.voucher_date,
        description: v.description,
        status: v.status,
        created_at: v.created_at,
        posted_at: v.posted_at,
    })
}

#[tauri::command]
pub async fn list_vouchers(
    state: State<'_, AppState>,
    fiscal_year_id: String,
) -> Result<Vec<VoucherDto>, CommandError> {
    let _user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    let vouchers = queries::voucher::list_by_fiscal_year(&state.pool, &fiscal_year_id).await?;

    Ok(vouchers.into_iter().map(|v| VoucherDto {
        id: v.id,
        voucher_series: v.voucher_series,
        voucher_number: v.voucher_number,
        voucher_date: v.voucher_date,
        description: v.description,
        status: v.status,
        created_at: v.created_at,
        posted_at: v.posted_at,
    }).collect())
}
