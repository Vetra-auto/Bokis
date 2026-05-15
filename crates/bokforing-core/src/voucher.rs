use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::{Money, VoucherStatus};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Voucher {
    pub id: Uuid,
    pub company_id: Uuid,
    pub fiscal_year_id: Uuid,
    pub period_id: Uuid,
    pub voucher_series: String,
    pub voucher_number: i64,
    pub voucher_date: NaiveDate,
    pub description: String,
    pub status: VoucherStatus,
    pub counterparty_id: Option<Uuid>,
    pub created_by: Uuid,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub posted_at: Option<chrono::DateTime<chrono::Utc>>,
    pub posted_by: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoucherLine {
    pub id: Uuid,
    pub voucher_id: Uuid,
    pub line_number: i32,
    pub account_id: Uuid,
    pub account_number: u16,
    pub debit_amount: Money,
    pub credit_amount: Money,
    pub description: Option<String>,
    pub counterparty_id: Option<Uuid>,
    pub cost_center: Option<String>,
    pub project: Option<String>,
}

impl VoucherLine {
    pub fn net_amount(&self) -> Money {
        Money::from_ore(self.debit_amount.ore() - self.credit_amount.ore())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoucherDraft {
    pub voucher_date: NaiveDate,
    pub description: String,
    pub voucher_series: String,
    pub counterparty_id: Option<Uuid>,
    pub lines: Vec<VoucherLineDraft>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoucherLineDraft {
    pub account_number: u16,
    pub debit_amount: i64,
    pub credit_amount: i64,
    pub description: Option<String>,
    pub counterparty_id: Option<Uuid>,
    pub cost_center: Option<String>,
    pub project: Option<String>,
}
