-- bokforing: initial schema
-- SQLite with WAL mode (set at connection time)
PRAGMA foreign_keys = ON;

-- ---- Company ----
CREATE TABLE companies (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    org_number TEXT NOT NULL,
    address TEXT,
    postal_code TEXT,
    city TEXT,
    country TEXT NOT NULL DEFAULT 'SE',
    currency TEXT NOT NULL DEFAULT 'SEK',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- ---- Users & Auth ----
CREATE TABLE users (
    id TEXT PRIMARY KEY,
    username TEXT UNIQUE NOT NULL,
    display_name TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    is_active INTEGER NOT NULL DEFAULT 1,
    force_password_change INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE role_assignments (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id),
    role TEXT NOT NULL CHECK (role IN ('admin', 'bokforare', 'attest', 'revisor')),
    company_id TEXT NOT NULL REFERENCES companies(id),
    assigned_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    assigned_by TEXT REFERENCES users(id),
    UNIQUE(user_id, role, company_id)
);

-- ---- Fiscal Year & Periods ----
CREATE TABLE fiscal_years (
    id TEXT PRIMARY KEY,
    company_id TEXT NOT NULL REFERENCES companies(id),
    name TEXT NOT NULL,
    start_date TEXT NOT NULL,
    end_date TEXT NOT NULL,
    is_closed INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE(company_id, start_date)
);

CREATE TABLE periods (
    id TEXT PRIMARY KEY,
    fiscal_year_id TEXT NOT NULL REFERENCES fiscal_years(id),
    year_month TEXT NOT NULL,
    start_date TEXT NOT NULL,
    end_date TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'open'
        CHECK (status IN ('open', 'under_review', 'closed', 'locked')),
    closed_at TEXT,
    closed_by TEXT REFERENCES users(id),
    locked_at TEXT,
    locked_by TEXT REFERENCES users(id),
    UNIQUE(fiscal_year_id, year_month)
);

-- ---- Chart of Accounts ----
CREATE TABLE chart_of_accounts_versions (
    id TEXT PRIMARY KEY,
    company_id TEXT NOT NULL REFERENCES companies(id),
    version_name TEXT NOT NULL,
    based_on TEXT,
    activated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    created_by TEXT REFERENCES users(id)
);

CREATE TABLE accounts (
    id TEXT PRIMARY KEY,
    company_id TEXT NOT NULL REFERENCES companies(id),
    account_number INTEGER NOT NULL,
    name TEXT NOT NULL,
    account_type TEXT NOT NULL
        CHECK (account_type IN ('asset', 'liability', 'equity', 'revenue', 'expense')),
    is_active INTEGER NOT NULL DEFAULT 1,
    sru_code TEXT,
    description TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    chart_version_id TEXT REFERENCES chart_of_accounts_versions(id),
    UNIQUE(company_id, account_number)
);
CREATE INDEX idx_accounts_company_number ON accounts(company_id, account_number);

-- ---- Counterparty ----
CREATE TABLE counterparties (
    id TEXT PRIMARY KEY,
    company_id TEXT NOT NULL REFERENCES companies(id),
    name TEXT NOT NULL,
    org_number TEXT,
    contact_info TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- ---- Vouchers (Verifikationer) ----
CREATE TABLE vouchers (
    id TEXT PRIMARY KEY,
    company_id TEXT NOT NULL REFERENCES companies(id),
    fiscal_year_id TEXT NOT NULL REFERENCES fiscal_years(id),
    period_id TEXT NOT NULL REFERENCES periods(id),
    voucher_series TEXT NOT NULL DEFAULT 'A',
    voucher_number INTEGER NOT NULL,
    voucher_date TEXT NOT NULL,
    description TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'draft'
        CHECK (status IN ('draft', 'posted')),
    counterparty_id TEXT REFERENCES counterparties(id),
    created_by TEXT NOT NULL REFERENCES users(id),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    posted_at TEXT,
    posted_by TEXT REFERENCES users(id),
    UNIQUE(company_id, voucher_series, fiscal_year_id, voucher_number)
);
CREATE INDEX idx_vouchers_period ON vouchers(period_id);
CREATE INDEX idx_vouchers_date ON vouchers(voucher_date);

-- ---- Voucher Lines (Konteringsrader) ----
CREATE TABLE voucher_lines (
    id TEXT PRIMARY KEY,
    voucher_id TEXT NOT NULL REFERENCES vouchers(id),
    line_number INTEGER NOT NULL,
    account_id TEXT NOT NULL REFERENCES accounts(id),
    debit_amount INTEGER NOT NULL DEFAULT 0,
    credit_amount INTEGER NOT NULL DEFAULT 0,
    description TEXT,
    counterparty_id TEXT REFERENCES counterparties(id),
    cost_center TEXT,
    project TEXT,
    UNIQUE(voucher_id, line_number),
    CHECK (
        (debit_amount > 0 AND credit_amount = 0) OR
        (credit_amount > 0 AND debit_amount = 0) OR
        (debit_amount = 0 AND credit_amount = 0)
    )
);
CREATE INDEX idx_voucher_lines_account ON voucher_lines(account_id);

-- ---- Attachments (Bilagor) ----
CREATE TABLE attachments (
    id TEXT PRIMARY KEY,
    voucher_id TEXT NOT NULL REFERENCES vouchers(id),
    filename TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    file_size INTEGER NOT NULL,
    file_hash TEXT NOT NULL,
    storage_path TEXT NOT NULL,
    attachment_type TEXT NOT NULL DEFAULT 'received_original'
        CHECK (attachment_type IN ('received_original', 'internal_voucher', 'shared_voucher', 'reference_voucher')),
    uploaded_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    uploaded_by TEXT NOT NULL REFERENCES users(id)
);

-- ---- Correction Links (Rättelseposter) ----
CREATE TABLE correction_links (
    id TEXT PRIMARY KEY,
    original_voucher_id TEXT NOT NULL REFERENCES vouchers(id),
    reversal_voucher_id TEXT NOT NULL REFERENCES vouchers(id),
    replacement_voucher_id TEXT REFERENCES vouchers(id),
    reason TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    created_by TEXT NOT NULL REFERENCES users(id)
);

-- ---- Period Close ----
CREATE TABLE period_closes (
    id TEXT PRIMARY KEY,
    period_id TEXT NOT NULL REFERENCES periods(id),
    action TEXT NOT NULL CHECK (action IN ('close', 'lock', 'reopen')),
    performed_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    performed_by TEXT NOT NULL REFERENCES users(id),
    reason TEXT
);

-- ---- Audit Events (SHA-256 hash chain) ----
CREATE TABLE audit_events (
    id TEXT PRIMARY KEY,
    sequence_number INTEGER NOT NULL UNIQUE,
    event_type TEXT NOT NULL,
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    user_id TEXT REFERENCES users(id),
    occurred_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    detail_json TEXT,
    previous_hash TEXT NOT NULL,
    event_hash TEXT NOT NULL
);
CREATE INDEX idx_audit_events_entity ON audit_events(entity_type, entity_id);
CREATE INDEX idx_audit_events_occurred ON audit_events(occurred_at);

-- ---- Processing History (Behandlingshistorik) ----
CREATE TABLE processing_history_events (
    id TEXT PRIMARY KEY,
    event_type TEXT NOT NULL,
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    user_id TEXT REFERENCES users(id),
    occurred_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    description TEXT NOT NULL,
    metadata_json TEXT
);
CREATE INDEX idx_processing_history_entity ON processing_history_events(entity_type, entity_id);

-- ---- System Documentation Snapshots ----
CREATE TABLE system_documentation_snapshots (
    id TEXT PRIMARY KEY,
    generated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    generated_by TEXT REFERENCES users(id),
    content_json TEXT NOT NULL,
    content_hash TEXT NOT NULL
);

-- ---- Archive & Backup ----
CREATE TABLE archive_manifests (
    id TEXT PRIMARY KEY,
    fiscal_year_id TEXT NOT NULL REFERENCES fiscal_years(id),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    created_by TEXT NOT NULL REFERENCES users(id),
    archive_path TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    manifest_json TEXT NOT NULL
);

CREATE TABLE backup_jobs (
    id TEXT PRIMARY KEY,
    started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    completed_at TEXT,
    status TEXT NOT NULL DEFAULT 'running'
        CHECK (status IN ('running', 'completed', 'failed')),
    backup_path TEXT,
    backup_hash TEXT,
    size_bytes INTEGER,
    error_message TEXT,
    initiated_by TEXT NOT NULL REFERENCES users(id)
);

CREATE TABLE backup_verifications (
    id TEXT PRIMARY KEY,
    backup_job_id TEXT NOT NULL REFERENCES backup_jobs(id),
    verified_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    verified_by TEXT NOT NULL REFERENCES users(id),
    is_valid INTEGER NOT NULL,
    details TEXT
);

-- ---- Export Jobs ----
CREATE TABLE export_jobs (
    id TEXT PRIMARY KEY,
    export_type TEXT NOT NULL CHECK (export_type IN ('sie4', 'csv', 'json', 'pdf')),
    fiscal_year_id TEXT REFERENCES fiscal_years(id),
    period_id TEXT REFERENCES periods(id),
    started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    completed_at TEXT,
    status TEXT NOT NULL DEFAULT 'running'
        CHECK (status IN ('running', 'completed', 'failed')),
    output_path TEXT,
    output_hash TEXT,
    initiated_by TEXT NOT NULL REFERENCES users(id),
    parameters_json TEXT
);
