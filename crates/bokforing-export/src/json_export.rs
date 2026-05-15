use serde::Serialize;

#[derive(Serialize)]
pub struct FullExport {
    pub export_version: String,
    pub exported_at: String,
    pub company: CompanyExport,
    pub fiscal_year: FiscalYearExport,
    pub accounts: Vec<AccountExport>,
    pub vouchers: Vec<VoucherExport>,
    pub audit_events: Vec<AuditEventExport>,
}

#[derive(Serialize)]
pub struct CompanyExport {
    pub name: String,
    pub org_number: String,
    pub address: Option<String>,
    pub city: Option<String>,
}

#[derive(Serialize)]
pub struct FiscalYearExport {
    pub name: String,
    pub start_date: String,
    pub end_date: String,
}

#[derive(Serialize)]
pub struct AccountExport {
    pub number: u16,
    pub name: String,
    pub account_type: String,
    pub sru_code: Option<String>,
}

#[derive(Serialize)]
pub struct VoucherExport {
    pub series: String,
    pub number: i64,
    pub date: String,
    pub description: String,
    pub posted_at: Option<String>,
    pub lines: Vec<VoucherLineExport>,
}

#[derive(Serialize)]
pub struct VoucherLineExport {
    pub account_number: u16,
    pub debit_ore: i64,
    pub credit_ore: i64,
    pub description: Option<String>,
}

#[derive(Serialize)]
pub struct AuditEventExport {
    pub sequence_number: i64,
    pub event_type: String,
    pub entity_type: String,
    pub entity_id: String,
    pub occurred_at: String,
    pub event_hash: String,
}

pub fn export_to_json(export: &FullExport) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(export)
}
