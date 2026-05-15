use tauri::State;

use bokforing_db::queries;
use crate::error::CommandError;
use crate::state::AppState;

#[derive(serde::Serialize)]
pub struct JournalEntryDto {
    pub voucher_series: String,
    pub voucher_number: i64,
    pub voucher_date: String,
    pub voucher_description: String,
    pub line_number: i64,
    pub account_number: i64,
    pub account_name: String,
    pub debit_amount: i64,
    pub credit_amount: i64,
}

#[tauri::command]
pub async fn get_journal(
    state: State<'_, AppState>,
    fiscal_year_id: String,
    from_date: Option<String>,
    to_date: Option<String>,
) -> Result<Vec<JournalEntryDto>, CommandError> {
    let _user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    let entries = queries::ledger::get_journal(
        &state.pool,
        &fiscal_year_id,
        from_date.as_deref(),
        to_date.as_deref(),
    ).await?;

    Ok(entries.into_iter().map(|e| JournalEntryDto {
        voucher_series: e.voucher_series,
        voucher_number: e.voucher_number,
        voucher_date: e.voucher_date,
        voucher_description: e.voucher_description,
        line_number: e.line_number,
        account_number: e.account_number,
        account_name: e.account_name,
        debit_amount: e.debit_amount,
        credit_amount: e.credit_amount,
    }).collect())
}

#[tauri::command]
pub async fn get_general_ledger(
    state: State<'_, AppState>,
    fiscal_year_id: String,
    account_number: Option<i64>,
) -> Result<Vec<GeneralLedgerEntryDto>, CommandError> {
    let _user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    let entries = queries::ledger::get_general_ledger(
        &state.pool,
        &fiscal_year_id,
        account_number,
    ).await?;

    Ok(entries.into_iter().map(|e| GeneralLedgerEntryDto {
        account_number: e.account_number,
        account_name: e.account_name,
        voucher_date: e.voucher_date,
        voucher_series: e.voucher_series,
        voucher_number: e.voucher_number,
        description: e.voucher_description,
        debit_amount: e.debit_amount,
        credit_amount: e.credit_amount,
    }).collect())
}

#[derive(serde::Serialize)]
pub struct GeneralLedgerEntryDto {
    pub account_number: i64,
    pub account_name: String,
    pub voucher_date: String,
    pub voucher_series: String,
    pub voucher_number: i64,
    pub description: String,
    pub debit_amount: i64,
    pub credit_amount: i64,
}

#[tauri::command]
pub async fn get_trial_balance(
    state: State<'_, AppState>,
    fiscal_year_id: String,
) -> Result<Vec<TrialBalanceDto>, CommandError> {
    let _user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    let balances = queries::ledger::get_trial_balance(&state.pool, &fiscal_year_id).await?;

    Ok(balances.into_iter().map(|b| TrialBalanceDto {
        account_number: b.account_number,
        account_name: b.account_name,
        total_debit: b.total_debit,
        total_credit: b.total_credit,
        balance: b.total_debit - b.total_credit,
    }).collect())
}

#[derive(serde::Serialize)]
pub struct TrialBalanceDto {
    pub account_number: i64,
    pub account_name: String,
    pub total_debit: i64,
    pub total_credit: i64,
    pub balance: i64,
}
