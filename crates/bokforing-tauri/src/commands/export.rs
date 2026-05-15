use tauri::State;

use bokforing_db::queries;
use bokforing_export::sie4::{Sie4Account, Sie4Company, Sie4Voucher, Sie4VoucherLine, Sie4Writer};
use bokforing_export::csv_export::{CsvExporter, JournalRow};
use bokforing_export::json_export::{
    FullExport, CompanyExport, FiscalYearExport, AccountExport,
    VoucherExport, VoucherLineExport, AuditEventExport, export_to_json,
};
use bokforing_export::pdf_export::PdfExporter;
use bokforing_core::types::Money;
use crate::error::CommandError;
use crate::state::AppState;

#[tauri::command]
pub async fn export_sie4(
    state: State<'_, AppState>,
    fiscal_year_id: String,
    output_path: String,
) -> Result<String, CommandError> {
    let user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    let company = state.current_company.read().await
        .as_ref()
        .ok_or(CommandError::Internal("Inget företag valt".into()))?
        .clone();

    let fy = queries::fiscal_year::get_by_id(&state.pool, &fiscal_year_id)
        .await?
        .ok_or(CommandError::NotFound("Räkenskapsår".into()))?;

    let accounts = queries::account::list(&state.pool, &company.company_id).await?;
    let vouchers = queries::voucher::list_by_fiscal_year(&state.pool, &fiscal_year_id).await?;

    let mut writer = Sie4Writer::new();

    let company_row: Option<(String, String)> = sqlx::query_as(
        "SELECT name, org_number FROM companies WHERE id = ?1"
    )
    .bind(&company.company_id)
    .fetch_optional(&state.pool)
    .await?;

    let (company_name, org_number) = company_row.unwrap_or(("".to_string(), "".to_string()));

    writer.write_header(
        &Sie4Company {
            name: company_name,
            org_number,
            fiscal_year_start: fy.start_date.replace("-", ""),
            fiscal_year_end: fy.end_date.replace("-", ""),
        },
        "Bokforing",
        "0.1.0",
    );

    writer.write_accounts(
        &accounts.iter().map(|a| Sie4Account {
            number: a.account_number as u16,
            name: a.name.clone(),
            sru_code: a.sru_code.clone(),
        }).collect::<Vec<_>>()
    );

    let account_map: std::collections::HashMap<String, u16> = accounts.iter()
        .map(|a| (a.id.clone(), a.account_number as u16))
        .collect();

    let mut sie_vouchers = Vec::new();
    for v in &vouchers {
        if v.status != "posted" {
            continue;
        }
        let lines = queries::voucher::get_lines(&state.pool, &v.id).await?;
        let sie_lines: Vec<Sie4VoucherLine> = lines.iter().map(|l| {
            let amount = if l.debit_amount > 0 {
                Money::from_ore(l.debit_amount)
            } else {
                Money::from_ore(-l.credit_amount)
            };
            Sie4VoucherLine {
                account_number: account_map.get(&l.account_id).copied().unwrap_or(0),
                amount,
                date: v.voucher_date.replace("-", ""),
                description: l.description.clone().unwrap_or_default(),
            }
        }).collect();

        sie_vouchers.push(Sie4Voucher {
            series: v.voucher_series.clone(),
            number: v.voucher_number,
            date: v.voucher_date.replace("-", ""),
            description: v.description.clone(),
            lines: sie_lines,
        });
    }

    writer.write_vouchers(&sie_vouchers);

    let output = writer.finish();
    std::fs::write(&output_path, &output)
        .map_err(|e| CommandError::Internal(format!("Kunde inte skriva SIE4-fil: {}", e)))?;

    queries::audit::append_event(
        &state.pool,
        "export_sie4",
        "fiscal_year",
        &fiscal_year_id,
        &user.user_id,
        &serde_json::json!({"path": output_path}).to_string(),
    ).await?;

    Ok(output_path)
}

#[tauri::command]
pub async fn export_csv(
    state: State<'_, AppState>,
    fiscal_year_id: String,
    output_path: String,
) -> Result<String, CommandError> {
    let user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    let company = state.current_company.read().await
        .as_ref()
        .ok_or(CommandError::Internal("Inget företag valt".into()))?
        .clone();

    let accounts = queries::account::list(&state.pool, &company.company_id).await?;
    let account_map: std::collections::HashMap<String, u16> = accounts.iter()
        .map(|a| (a.id.clone(), a.account_number as u16))
        .collect();

    let vouchers = queries::voucher::list_by_fiscal_year(&state.pool, &fiscal_year_id).await?;

    let mut rows = Vec::new();
    for v in &vouchers {
        if v.status != "posted" {
            continue;
        }
        let lines = queries::voucher::get_lines(&state.pool, &v.id).await?;
        for line in &lines {
            rows.push(JournalRow {
                date: v.voucher_date.clone(),
                series: v.voucher_series.clone(),
                number: v.voucher_number,
                description: v.description.clone(),
                account_number: account_map.get(&line.account_id).copied().unwrap_or(0),
                debit_ore: line.debit_amount,
                credit_ore: line.credit_amount,
            });
        }
    }

    let exporter = CsvExporter::new();
    let mut output = Vec::new();
    exporter.export_journal(&mut output, &rows)
        .map_err(|e| CommandError::Internal(format!("CSV-export misslyckades: {}", e)))?;

    std::fs::write(&output_path, &output)
        .map_err(|e| CommandError::Internal(format!("Kunde inte skriva CSV-fil: {}", e)))?;

    queries::audit::append_event(
        &state.pool,
        "export_csv",
        "fiscal_year",
        &fiscal_year_id,
        &user.user_id,
        &serde_json::json!({"path": output_path}).to_string(),
    ).await?;

    Ok(output_path)
}

#[tauri::command]
pub async fn export_json(
    state: State<'_, AppState>,
    fiscal_year_id: String,
    output_path: String,
) -> Result<String, CommandError> {
    let user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    let company = state.current_company.read().await
        .as_ref()
        .ok_or(CommandError::Internal("Inget företag valt".into()))?
        .clone();

    let fy = queries::fiscal_year::get_by_id(&state.pool, &fiscal_year_id)
        .await?
        .ok_or(CommandError::NotFound("Räkenskapsår".into()))?;

    let accounts = queries::account::list(&state.pool, &company.company_id).await?;
    let account_map: std::collections::HashMap<String, u16> = accounts.iter()
        .map(|a| (a.id.clone(), a.account_number as u16))
        .collect();

    let vouchers = queries::voucher::list_by_fiscal_year(&state.pool, &fiscal_year_id).await?;
    let audit_events = queries::audit::get_all_events_ordered(&state.pool).await?;

    let company_row: Option<(String, String)> = sqlx::query_as(
        "SELECT name, org_number FROM companies WHERE id = ?1"
    )
    .bind(&company.company_id)
    .fetch_optional(&state.pool)
    .await?;
    let (company_name, org_number) = company_row.unwrap_or(("".to_string(), "".to_string()));

    let mut voucher_exports = Vec::new();
    for v in &vouchers {
        if v.status != "posted" {
            continue;
        }
        let lines = queries::voucher::get_lines(&state.pool, &v.id).await?;
        voucher_exports.push(VoucherExport {
            series: v.voucher_series.clone(),
            number: v.voucher_number,
            date: v.voucher_date.clone(),
            description: v.description.clone(),
            posted_at: v.posted_at.clone(),
            lines: lines.iter().map(|l| VoucherLineExport {
                account_number: account_map.get(&l.account_id).copied().unwrap_or(0),
                debit_ore: l.debit_amount,
                credit_ore: l.credit_amount,
                description: l.description.clone(),
            }).collect(),
        });
    }

    let full_export = FullExport {
        export_version: "1.0".to_string(),
        exported_at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        company: CompanyExport {
            name: company_name,
            org_number,
            address: None,
            city: None,
        },
        fiscal_year: FiscalYearExport {
            name: fy.name.clone(),
            start_date: fy.start_date.clone(),
            end_date: fy.end_date.clone(),
        },
        accounts: accounts.iter().map(|a| AccountExport {
            number: a.account_number as u16,
            name: a.name.clone(),
            account_type: a.account_type.clone(),
            sru_code: a.sru_code.clone(),
        }).collect(),
        vouchers: voucher_exports,
        audit_events: audit_events.iter().map(|e| AuditEventExport {
            sequence_number: e.sequence_number,
            event_type: e.event_type.clone(),
            entity_type: e.entity_type.clone(),
            entity_id: e.entity_id.clone(),
            occurred_at: e.occurred_at.clone(),
            event_hash: e.event_hash.clone(),
        }).collect(),
    };

    let json = export_to_json(&full_export)
        .map_err(|e| CommandError::Internal(format!("JSON-serialisering misslyckades: {}", e)))?;

    std::fs::write(&output_path, &json)
        .map_err(|e| CommandError::Internal(format!("Kunde inte skriva JSON-fil: {}", e)))?;

    queries::audit::append_event(
        &state.pool,
        "export_json",
        "fiscal_year",
        &fiscal_year_id,
        &user.user_id,
        &serde_json::json!({"path": output_path}).to_string(),
    ).await?;

    Ok(output_path)
}

#[tauri::command]
pub async fn export_pdf(
    state: State<'_, AppState>,
    fiscal_year_id: String,
    report_type: String,
    output_path: String,
) -> Result<String, CommandError> {
    let user = state.current_user.read().await
        .as_ref()
        .ok_or(CommandError::NotAuthenticated)?
        .clone();

    let company = state.current_company.read().await
        .as_ref()
        .ok_or(CommandError::Internal("Inget företag valt".into()))?
        .clone();

    let fy = queries::fiscal_year::get_by_id(&state.pool, &fiscal_year_id)
        .await?
        .ok_or(CommandError::NotFound("Räkenskapsår".into()))?;

    let company_row: Option<(String, String)> = sqlx::query_as(
        "SELECT name, org_number FROM companies WHERE id = ?1"
    )
    .bind(&company.company_id)
    .fetch_optional(&state.pool)
    .await?;
    let (company_name, _org_number) = company_row.unwrap_or(("".to_string(), "".to_string()));

    let font_dir = std::path::Path::new("/usr/share/fonts/truetype/liberation");
    let exporter = PdfExporter::new(font_dir)
        .map_err(|e| CommandError::Internal(format!("PDF-initiering misslyckades: {}", e)))?;

    let accounts = queries::account::list(&state.pool, &company.company_id).await?;
    let account_map: std::collections::HashMap<String, u16> = accounts.iter()
        .map(|a| (a.id.clone(), a.account_number as u16))
        .collect();
    let account_name_map: std::collections::HashMap<u16, String> = accounts.iter()
        .map(|a| (a.account_number as u16, a.name.clone()))
        .collect();

    let pdf_bytes = match report_type.as_str() {
        "journal" => {
            let vouchers = queries::voucher::list_by_fiscal_year(&state.pool, &fiscal_year_id).await?;
            let mut rows = Vec::new();
            for v in &vouchers {
                if v.status != "posted" { continue; }
                let lines = queries::voucher::get_lines(&state.pool, &v.id).await?;
                for line in &lines {
                    rows.push(JournalRow {
                        date: v.voucher_date.clone(),
                        series: v.voucher_series.clone(),
                        number: v.voucher_number,
                        description: v.description.clone(),
                        account_number: account_map.get(&line.account_id).copied().unwrap_or(0),
                        debit_ore: line.debit_amount,
                        credit_ore: line.credit_amount,
                    });
                }
            }
            exporter.export_journal(&company_name, &fy.name, &rows)
                .map_err(|e| CommandError::Internal(format!("PDF-generering misslyckades: {}", e)))?
        }
        "trial_balance" => {
            let vouchers = queries::voucher::list_by_fiscal_year(&state.pool, &fiscal_year_id).await?;
            let mut account_totals: std::collections::HashMap<u16, (i64, i64)> = std::collections::HashMap::new();

            for v in &vouchers {
                if v.status != "posted" { continue; }
                let lines = queries::voucher::get_lines(&state.pool, &v.id).await?;
                for line in &lines {
                    let acct_num = account_map.get(&line.account_id).copied().unwrap_or(0);
                    let entry = account_totals.entry(acct_num).or_insert((0, 0));
                    entry.0 += line.debit_amount;
                    entry.1 += line.credit_amount;
                }
            }

            let mut trial_rows: Vec<bokforing_export::pdf_export::TrialBalanceRow> = account_totals
                .iter()
                .filter(|(_, (d, c))| *d != 0 || *c != 0)
                .map(|(num, (d, c))| bokforing_export::pdf_export::TrialBalanceRow {
                    account_number: *num,
                    account_name: account_name_map.get(num).cloned().unwrap_or_default(),
                    total_debit_ore: *d,
                    total_credit_ore: *c,
                    balance_ore: *d - *c,
                })
                .collect();
            trial_rows.sort_by_key(|r| r.account_number);

            exporter.export_trial_balance(&company_name, &fy.name, &trial_rows)
                .map_err(|e| CommandError::Internal(format!("PDF-generering misslyckades: {}", e)))?
        }
        _ => return Err(CommandError::Validation(format!("Okänd rapporttyp: {}", report_type))),
    };

    std::fs::write(&output_path, &pdf_bytes)
        .map_err(|e| CommandError::Internal(format!("Kunde inte skriva PDF-fil: {}", e)))?;

    queries::audit::append_event(
        &state.pool,
        "export_pdf",
        "fiscal_year",
        &fiscal_year_id,
        &user.user_id,
        &serde_json::json!({"path": output_path, "type": report_type}).to_string(),
    ).await?;

    Ok(output_path)
}
