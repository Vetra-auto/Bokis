use tauri::State;

use bokforing_db::queries;
use crate::error::CommandError;
use crate::state::AppState;
use crate::commands::voucher::VoucherDto;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrectVoucherRequest {
    pub voucher_id: String,
    pub reason: String,
    pub correction_date: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrectionResultDto {
    pub reversal: VoucherDto,
    pub correction_link_id: String,
}

#[tauri::command]
pub async fn correct_voucher(
    state: State<'_, AppState>,
    request: CorrectVoucherRequest,
) -> Result<CorrectionResultDto, CommandError> {
    let user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    if !user.roles.iter().any(|r| r == "admin" || r == "bokforare") {
        return Err(CommandError::Forbidden);
    }

    if request.reason.is_empty() {
        return Err(CommandError::Validation("Ange en anledning till rättelsen".into()));
    }

    let company = state.current_company.read().await
        .as_ref()
        .ok_or(CommandError::Internal("Inget företag valt".into()))?
        .clone();

    let fiscal_year_id = company.active_fiscal_year_id
        .ok_or(CommandError::Validation("Inget räkenskapsår valt".into()))?;

    let original = queries::voucher::get_by_id(&state.pool, &request.voucher_id)
        .await?
        .ok_or(CommandError::NotFound("Verifikation".into()))?;

    if original.status != "posted" {
        return Err(CommandError::Validation("Kan bara rätta bokförda verifikationer".into()));
    }

    let period = queries::period::find_for_date(&state.pool, &fiscal_year_id, &request.correction_date)
        .await?
        .ok_or(CommandError::Validation("Ingen period matchar rättelsedatumet".into()))?;

    if period.status != "open" {
        return Err(CommandError::Validation(format!("Perioden {} är inte öppen", period.year_month)));
    }

    let original_lines = queries::voucher::get_lines(&state.pool, &request.voucher_id).await?;

    let series = &original.voucher_series;
    let reversal_number = queries::voucher::next_voucher_number(
        &state.pool, &company.company_id, &fiscal_year_id, series
    ).await?;

    let description = format!(
        "Rättelse av ver {}{}: {}",
        original.voucher_series, original.voucher_number, request.reason
    );

    let reversal_id = queries::voucher::create(
        &state.pool,
        &company.company_id,
        &fiscal_year_id,
        &period.id,
        series,
        reversal_number,
        &request.correction_date,
        &description,
        original.counterparty_id.as_deref(),
        &user.user_id,
    ).await?;

    for (i, line) in original_lines.iter().enumerate() {
        queries::voucher::add_line(
            &state.pool,
            &reversal_id,
            (i + 1) as i64,
            &line.account_id,
            line.credit_amount,
            line.debit_amount,
            line.description.as_deref(),
            line.counterparty_id.as_deref(),
            line.cost_center.as_deref(),
            line.project.as_deref(),
        ).await?;
    }

    queries::voucher::mark_posted(&state.pool, &reversal_id, &user.user_id).await?;

    let link_id = queries::correction::create_link(
        &state.pool,
        &request.voucher_id,
        &reversal_id,
        None,
        &request.reason,
        &user.user_id,
    ).await?;

    queries::audit::append_event(
        &state.pool,
        "voucher_corrected",
        "voucher",
        &request.voucher_id,
        &user.user_id,
        &serde_json::json!({
            "reason": request.reason,
            "reversal_voucher_id": reversal_id,
            "original_series": original.voucher_series,
            "original_number": original.voucher_number
        }).to_string(),
    ).await?;

    queries::audit::append_processing_history(
        &state.pool,
        "voucher_corrected",
        "voucher",
        &request.voucher_id,
        &user.user_id,
        &format!("Verifikation {}{} rättad. Reversering: {}{}",
            original.voucher_series, original.voucher_number,
            series, reversal_number),
        None,
    ).await?;

    let v = queries::voucher::get_by_id(&state.pool, &reversal_id)
        .await?
        .ok_or(CommandError::Internal("Kunde inte hämta reverseringsverifikation".into()))?;

    Ok(CorrectionResultDto {
        reversal: VoucherDto {
            id: v.id,
            voucher_series: v.voucher_series,
            voucher_number: v.voucher_number,
            voucher_date: v.voucher_date,
            description: v.description,
            status: v.status,
            created_at: v.created_at,
            posted_at: v.posted_at,
        },
        correction_link_id: link_id,
    })
}

#[tauri::command]
pub async fn get_voucher_details(
    state: State<'_, AppState>,
    voucher_id: String,
) -> Result<VoucherWithLinesDto, CommandError> {
    let _user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    let voucher = queries::voucher::get_by_id(&state.pool, &voucher_id)
        .await?
        .ok_or(CommandError::NotFound("Verifikation".into()))?;

    let lines = queries::voucher::get_lines(&state.pool, &voucher_id).await?;

    let corrections = queries::correction::get_links_for_voucher(&state.pool, &voucher_id).await?;

    Ok(VoucherWithLinesDto {
        id: voucher.id,
        voucher_series: voucher.voucher_series,
        voucher_number: voucher.voucher_number,
        voucher_date: voucher.voucher_date,
        description: voucher.description,
        status: voucher.status,
        created_at: voucher.created_at,
        posted_at: voucher.posted_at,
        lines: lines.into_iter().map(|l| VoucherLineDetailDto {
            account_id: l.account_id,
            debit_amount: l.debit_amount,
            credit_amount: l.credit_amount,
            description: l.description,
        }).collect(),
        corrections: corrections.into_iter().map(|c| CorrectionLinkDto {
            id: c.id,
            original_voucher_id: c.original_voucher_id,
            reversal_voucher_id: c.reversal_voucher_id,
            replacement_voucher_id: c.replacement_voucher_id,
            reason: c.reason,
            created_at: c.created_at,
        }).collect(),
    })
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherWithLinesDto {
    pub id: String,
    pub voucher_series: String,
    pub voucher_number: i64,
    pub voucher_date: String,
    pub description: String,
    pub status: String,
    pub created_at: String,
    pub posted_at: Option<String>,
    pub lines: Vec<VoucherLineDetailDto>,
    pub corrections: Vec<CorrectionLinkDto>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherLineDetailDto {
    pub account_id: String,
    pub debit_amount: i64,
    pub credit_amount: i64,
    pub description: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrectionLinkDto {
    pub id: String,
    pub original_voucher_id: String,
    pub reversal_voucher_id: String,
    pub replacement_voucher_id: Option<String>,
    pub reason: String,
    pub created_at: String,
}
