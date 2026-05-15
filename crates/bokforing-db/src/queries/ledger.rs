use sqlx::SqlitePool;

#[derive(Debug, sqlx::FromRow)]
pub struct JournalEntry {
    pub voucher_id: String,
    pub voucher_series: String,
    pub voucher_number: i64,
    pub voucher_date: String,
    pub voucher_description: String,
    pub voucher_status: String,
    pub line_id: String,
    pub line_number: i64,
    pub account_number: i64,
    pub account_name: String,
    pub debit_amount: i64,
    pub credit_amount: i64,
    pub line_description: Option<String>,
}

pub async fn get_journal(
    pool: &SqlitePool,
    fiscal_year_id: &str,
    from_date: Option<&str>,
    to_date: Option<&str>,
) -> Result<Vec<JournalEntry>, sqlx::Error> {
    let query = r#"
        SELECT
            v.id as voucher_id,
            v.voucher_series,
            v.voucher_number,
            v.voucher_date,
            v.description as voucher_description,
            v.status as voucher_status,
            vl.id as line_id,
            vl.line_number,
            a.account_number,
            a.name as account_name,
            vl.debit_amount,
            vl.credit_amount,
            vl.description as line_description
        FROM vouchers v
        JOIN voucher_lines vl ON vl.voucher_id = v.id
        JOIN accounts a ON a.id = vl.account_id
        WHERE v.fiscal_year_id = ?1
          AND v.status = 'posted'
          AND (?2 IS NULL OR v.voucher_date >= ?2)
          AND (?3 IS NULL OR v.voucher_date <= ?3)
        ORDER BY v.voucher_date, v.voucher_series, v.voucher_number, vl.line_number
    "#;

    sqlx::query_as::<_, JournalEntry>(query)
        .bind(fiscal_year_id)
        .bind(from_date)
        .bind(to_date)
        .fetch_all(pool)
        .await
}

#[derive(Debug, sqlx::FromRow)]
pub struct GeneralLedgerEntry {
    pub account_number: i64,
    pub account_name: String,
    pub voucher_date: String,
    pub voucher_series: String,
    pub voucher_number: i64,
    pub voucher_description: String,
    pub debit_amount: i64,
    pub credit_amount: i64,
}

pub async fn get_general_ledger(
    pool: &SqlitePool,
    fiscal_year_id: &str,
    account_number: Option<i64>,
) -> Result<Vec<GeneralLedgerEntry>, sqlx::Error> {
    let query = r#"
        SELECT
            a.account_number,
            a.name as account_name,
            v.voucher_date,
            v.voucher_series,
            v.voucher_number,
            v.description as voucher_description,
            vl.debit_amount,
            vl.credit_amount
        FROM voucher_lines vl
        JOIN vouchers v ON v.id = vl.voucher_id
        JOIN accounts a ON a.id = vl.account_id
        WHERE v.fiscal_year_id = ?1
          AND v.status = 'posted'
          AND (?2 IS NULL OR a.account_number = ?2)
        ORDER BY a.account_number, v.voucher_date, v.voucher_number
    "#;

    sqlx::query_as::<_, GeneralLedgerEntry>(query)
        .bind(fiscal_year_id)
        .bind(account_number)
        .fetch_all(pool)
        .await
}

#[derive(Debug, sqlx::FromRow)]
pub struct AccountBalance {
    pub account_id: String,
    pub account_number: i64,
    pub account_name: String,
    pub total_debit: i64,
    pub total_credit: i64,
}

pub async fn get_trial_balance(
    pool: &SqlitePool,
    fiscal_year_id: &str,
) -> Result<Vec<AccountBalance>, sqlx::Error> {
    let query = r#"
        SELECT
            a.id as account_id,
            a.account_number,
            a.name as account_name,
            COALESCE(SUM(vl.debit_amount), 0) as total_debit,
            COALESCE(SUM(vl.credit_amount), 0) as total_credit
        FROM accounts a
        LEFT JOIN voucher_lines vl ON vl.account_id = a.id
        LEFT JOIN vouchers v ON v.id = vl.voucher_id AND v.fiscal_year_id = ?1 AND v.status = 'posted'
        WHERE a.company_id = (SELECT company_id FROM fiscal_years WHERE id = ?1)
          AND a.is_active = 1
        GROUP BY a.id, a.account_number, a.name
        HAVING total_debit > 0 OR total_credit > 0
        ORDER BY a.account_number
    "#;

    sqlx::query_as::<_, AccountBalance>(query)
        .bind(fiscal_year_id)
        .fetch_all(pool)
        .await
}
