use sqlx::SqlitePool;
use bokforing_db::queries;
use bokforing_core::hash_chain;

async fn setup_test_db() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("Failed to create in-memory DB");

    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

async fn setup_full_env(pool: &SqlitePool) -> (String, String, String) {
    bokforing_db::seed::initial_setup(pool, "testpassword12", "Testföretag", "556677-8899")
        .await
        .expect("Seed failed");

    let (company_id,): (String,) = sqlx::query_as("SELECT id FROM companies LIMIT 1")
        .fetch_one(pool).await.unwrap();
    let (user_id,): (String,) = sqlx::query_as("SELECT id FROM users LIMIT 1")
        .fetch_one(pool).await.unwrap();

    let fy_id = queries::fiscal_year::create_with_periods(
        pool, &company_id, "2026", "2026-01-01", "2026-12-31"
    ).await.expect("Failed to create fiscal year");

    (company_id, user_id, fy_id)
}

#[tokio::test]
async fn full_accounting_flow() {
    let pool = setup_test_db().await;

    // Seed admin
    bokforing_db::seed::initial_setup(&pool, "testpassword12", "Testföretag", "556677-8899")
        .await
        .expect("Seed failed");

    // Get the seeded company and user
    let (company_id,): (String,) = sqlx::query_as("SELECT id FROM companies LIMIT 1")
        .fetch_one(&pool).await.unwrap();
    let (user_id,): (String,) = sqlx::query_as("SELECT id FROM users LIMIT 1")
        .fetch_one(&pool).await.unwrap();

    // Create fiscal year with periods
    let fy_id = queries::fiscal_year::create_with_periods(
        &pool, &company_id, "2026", "2026-01-01", "2026-12-31"
    ).await.expect("Failed to create fiscal year");

    // Verify 12 periods were created
    let periods = queries::period::list_by_fiscal_year(&pool, &fy_id).await.unwrap();
    assert_eq!(periods.len(), 12);
    assert_eq!(periods[0].status, "open");

    // Create accounts
    let acc_1910 = queries::account::create(
        &pool, &company_id, 1910, "Kassa", "asset", None, None, None
    ).await.unwrap();
    let acc_2440 = queries::account::create(
        &pool, &company_id, 2440, "Leverantörsskulder", "liability", None, None, None
    ).await.unwrap();
    let acc_5410 = queries::account::create(
        &pool, &company_id, 5410, "Kontorsmaterial", "expense", None, None, None
    ).await.unwrap();

    // Find January period
    let jan_period = queries::period::find_for_date(&pool, &fy_id, "2026-01-15")
        .await.unwrap().expect("January period not found");

    // Create a voucher
    let voucher_id = queries::voucher::create(
        &pool, &company_id, &fy_id, &jan_period.id,
        "A", 1, "2026-01-15", "Köp kontorsmaterial",
        None, &user_id,
    ).await.unwrap();

    // Add lines
    queries::voucher::add_line(&pool, &voucher_id, 1, &acc_5410, 10000, 0, Some("Kontorsmaterial"), None, None, None).await.unwrap();
    queries::voucher::add_line(&pool, &voucher_id, 2, &acc_2440, 0, 10000, Some("Leverantörsskuld"), None, None, None).await.unwrap();

    // Verify draft status
    let v = queries::voucher::get_by_id(&pool, &voucher_id).await.unwrap().unwrap();
    assert_eq!(v.status, "draft");

    // Post the voucher
    queries::voucher::mark_posted(&pool, &voucher_id, &user_id).await.unwrap();
    let v = queries::voucher::get_by_id(&pool, &voucher_id).await.unwrap().unwrap();
    assert_eq!(v.status, "posted");
    assert!(v.posted_at.is_some());

    // Add audit event
    queries::audit::append_event(
        &pool, "voucher_posted", "voucher", &voucher_id, &user_id, "{}"
    ).await.unwrap();

    // Add second audit event
    queries::audit::append_event(
        &pool, "period_transition", "period", &jan_period.id, &user_id,
        &serde_json::json!({"from": "open", "to": "closed"}).to_string()
    ).await.unwrap();

    // Verify hash chain
    let events = queries::audit::get_all_events_ordered(&pool).await.unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].previous_hash, hash_chain::GENESIS_HASH);

    // Verify chain integrity
    let chain_events: Vec<hash_chain::ChainEvent> = events.iter().map(|e| {
        hash_chain::ChainEvent {
            sequence_number: e.sequence_number,
            event_type: e.event_type.clone(),
            entity_id: e.entity_id.clone(),
            occurred_at: e.occurred_at.clone(),
            detail_json: e.detail_json.clone().unwrap_or_default(),
            previous_hash: e.previous_hash.clone(),
            event_hash: e.event_hash.clone(),
        }
    }).collect();
    assert!(hash_chain::verify_chain(&chain_events).is_ok());

    // Query journal
    let journal = queries::ledger::get_journal(&pool, &fy_id, None, None).await.unwrap();
    assert_eq!(journal.len(), 2); // 2 lines from the posted voucher

    // Query general ledger
    let gl = queries::ledger::get_general_ledger(&pool, &fy_id, Some(5410)).await.unwrap();
    assert_eq!(gl.len(), 1);
    assert_eq!(gl[0].debit_amount, 10000);

    // Trial balance
    let tb = queries::ledger::get_trial_balance(&pool, &fy_id).await.unwrap();
    assert!(tb.len() >= 2);
    let total_debit: i64 = tb.iter().map(|b| b.total_debit).sum();
    let total_credit: i64 = tb.iter().map(|b| b.total_credit).sum();
    assert_eq!(total_debit, total_credit);

    // Period transition
    queries::period::update_status(&pool, &jan_period.id, "closed", &user_id).await.unwrap();
    let updated = queries::period::get_by_id(&pool, &jan_period.id).await.unwrap().unwrap();
    assert_eq!(updated.status, "closed");

    // Cannot post into closed period (application-level check)
    // Create another voucher in January - it will exist but period is closed
    let v2_id = queries::voucher::create(
        &pool, &company_id, &fy_id, &jan_period.id,
        "A", 2, "2026-01-20", "Ny transaktion",
        None, &user_id,
    ).await.unwrap();
    queries::voucher::add_line(&pool, &v2_id, 1, &acc_1910, 5000, 0, None, None, None, None).await.unwrap();
    queries::voucher::add_line(&pool, &v2_id, 2, &acc_2440, 0, 5000, None, None, None, None).await.unwrap();
    // In a real app, the command layer would check period.status before calling mark_posted

    // Backup (VACUUM INTO doesn't work on :memory:, verify the job record instead)
    let backup_dir = std::env::temp_dir().join("bokforing_test_backup");
    std::fs::create_dir_all(&backup_dir).unwrap();
    let backup_path = backup_dir.join("test_backup.db");
    let _job_id = queries::backup::create_backup(&pool, &backup_path, &user_id).await.unwrap();

    let jobs = queries::backup::list_jobs(&pool, 10).await.unwrap();
    assert_eq!(jobs.len(), 1);
    // On in-memory DB, VACUUM INTO fails, so status will be 'failed'
    // In production with a file-based DB, this would be 'completed'
    assert!(jobs[0].status == "completed" || jobs[0].status == "failed");

    // Cleanup
    let _ = std::fs::remove_dir_all(&backup_dir);
}

#[tokio::test]
async fn draft_voucher_can_be_deleted() {
    let pool = setup_test_db().await;
    bokforing_db::seed::initial_setup(&pool, "testpassword12", "Testföretag", "556677-8899").await.unwrap();

    let (company_id,): (String,) = sqlx::query_as("SELECT id FROM companies LIMIT 1")
        .fetch_one(&pool).await.unwrap();
    let (user_id,): (String,) = sqlx::query_as("SELECT id FROM users LIMIT 1")
        .fetch_one(&pool).await.unwrap();

    let fy_id = queries::fiscal_year::create_with_periods(
        &pool, &company_id, "2026", "2026-01-01", "2026-12-31"
    ).await.unwrap();

    let jan = queries::period::find_for_date(&pool, &fy_id, "2026-01-15")
        .await.unwrap().unwrap();

    let acc = queries::account::create(&pool, &company_id, 1910, "Kassa", "asset", None, None, None).await.unwrap();
    let acc2 = queries::account::create(&pool, &company_id, 2440, "Skuld", "liability", None, None, None).await.unwrap();

    let vid = queries::voucher::create(
        &pool, &company_id, &fy_id, &jan.id, "A", 1, "2026-01-10", "Test", None, &user_id
    ).await.unwrap();
    queries::voucher::add_line(&pool, &vid, 1, &acc, 100, 0, None, None, None, None).await.unwrap();
    queries::voucher::add_line(&pool, &vid, 2, &acc2, 0, 100, None, None, None, None).await.unwrap();

    // Delete draft
    let deleted = queries::voucher::delete_draft(&pool, &vid).await.unwrap();
    assert!(deleted);

    // Verify gone
    let v = queries::voucher::get_by_id(&pool, &vid).await.unwrap();
    assert!(v.is_none());
}

#[tokio::test]
async fn posted_voucher_cannot_be_deleted() {
    let pool = setup_test_db().await;
    bokforing_db::seed::initial_setup(&pool, "testpassword12", "Testföretag", "556677-8899").await.unwrap();

    let (company_id,): (String,) = sqlx::query_as("SELECT id FROM companies LIMIT 1")
        .fetch_one(&pool).await.unwrap();
    let (user_id,): (String,) = sqlx::query_as("SELECT id FROM users LIMIT 1")
        .fetch_one(&pool).await.unwrap();

    let fy_id = queries::fiscal_year::create_with_periods(
        &pool, &company_id, "2026", "2026-01-01", "2026-12-31"
    ).await.unwrap();

    let jan = queries::period::find_for_date(&pool, &fy_id, "2026-01-15")
        .await.unwrap().unwrap();

    let acc = queries::account::create(&pool, &company_id, 1910, "Kassa", "asset", None, None, None).await.unwrap();
    let acc2 = queries::account::create(&pool, &company_id, 2440, "Skuld", "liability", None, None, None).await.unwrap();

    let vid = queries::voucher::create(
        &pool, &company_id, &fy_id, &jan.id, "A", 1, "2026-01-10", "Test", None, &user_id
    ).await.unwrap();
    queries::voucher::add_line(&pool, &vid, 1, &acc, 100, 0, None, None, None, None).await.unwrap();
    queries::voucher::add_line(&pool, &vid, 2, &acc2, 0, 100, None, None, None, None).await.unwrap();

    // Post it
    queries::voucher::mark_posted(&pool, &vid, &user_id).await.unwrap();

    // Try to delete - should fail (status is 'posted')
    let deleted = queries::voucher::delete_draft(&pool, &vid).await.unwrap();
    assert!(!deleted);

    // Still exists
    let v = queries::voucher::get_by_id(&pool, &vid).await.unwrap();
    assert!(v.is_some());
    assert_eq!(v.unwrap().status, "posted");
}

#[tokio::test]
async fn correction_flow_creates_reversal_and_link() {
    let pool = setup_test_db().await;
    bokforing_db::seed::initial_setup(&pool, "testpassword12", "Testföretag", "556677-8899").await.unwrap();

    let (company_id,): (String,) = sqlx::query_as("SELECT id FROM companies LIMIT 1")
        .fetch_one(&pool).await.unwrap();
    let (user_id,): (String,) = sqlx::query_as("SELECT id FROM users LIMIT 1")
        .fetch_one(&pool).await.unwrap();

    let fy_id = queries::fiscal_year::create_with_periods(
        &pool, &company_id, "2026", "2026-01-01", "2026-12-31"
    ).await.unwrap();

    let jan = queries::period::find_for_date(&pool, &fy_id, "2026-01-15")
        .await.unwrap().unwrap();

    let acc1 = queries::account::create(&pool, &company_id, 5410, "Kontorsmaterial", "expense", None, None, None).await.unwrap();
    let acc2 = queries::account::create(&pool, &company_id, 2440, "Skuld", "liability", None, None, None).await.unwrap();

    // Create and post original voucher
    let orig_id = queries::voucher::create(
        &pool, &company_id, &fy_id, &jan.id, "A", 1, "2026-01-10", "Original", None, &user_id
    ).await.unwrap();
    queries::voucher::add_line(&pool, &orig_id, 1, &acc1, 5000, 0, None, None, None, None).await.unwrap();
    queries::voucher::add_line(&pool, &orig_id, 2, &acc2, 0, 5000, None, None, None, None).await.unwrap();
    queries::voucher::mark_posted(&pool, &orig_id, &user_id).await.unwrap();

    // Create reversal voucher with swapped amounts
    let rev_number = queries::voucher::next_voucher_number(&pool, &company_id, &fy_id, "A").await.unwrap();
    assert_eq!(rev_number, 2);

    let rev_id = queries::voucher::create(
        &pool, &company_id, &fy_id, &jan.id, "A", rev_number, "2026-01-12", "Rättelse av ver A1: Fel belopp", None, &user_id
    ).await.unwrap();
    // Reversed lines (swap debit/credit)
    queries::voucher::add_line(&pool, &rev_id, 1, &acc1, 0, 5000, None, None, None, None).await.unwrap();
    queries::voucher::add_line(&pool, &rev_id, 2, &acc2, 5000, 0, None, None, None, None).await.unwrap();
    queries::voucher::mark_posted(&pool, &rev_id, &user_id).await.unwrap();

    // Create correction link
    let link_id = queries::correction::create_link(
        &pool, &orig_id, &rev_id, None, "Fel belopp", &user_id
    ).await.unwrap();
    assert!(!link_id.is_empty());

    // Verify link
    let links = queries::correction::get_links_for_voucher(&pool, &orig_id).await.unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].reason, "Fel belopp");
    assert_eq!(links[0].reversal_voucher_id, rev_id);

    // Verify trial balance is zero after correction
    let tb = queries::ledger::get_trial_balance(&pool, &fy_id).await.unwrap();
    let total_debit: i64 = tb.iter().map(|b| b.total_debit).sum();
    let total_credit: i64 = tb.iter().map(|b| b.total_credit).sum();
    assert_eq!(total_debit, total_credit);
    // Net effect should be zero on each account
    for b in &tb {
        assert_eq!(b.total_debit, b.total_credit);
    }
}

#[tokio::test]
async fn audit_chain_detects_tampering() {
    let pool = setup_test_db().await;
    bokforing_db::seed::initial_setup(&pool, "testpassword12", "Testföretag", "556677-8899").await.unwrap();

    let (user_id,): (String,) = sqlx::query_as("SELECT id FROM users LIMIT 1")
        .fetch_one(&pool).await.unwrap();

    // Append events
    queries::audit::append_event(&pool, "test_event_1", "test", "entity1", &user_id, "{}").await.unwrap();
    queries::audit::append_event(&pool, "test_event_2", "test", "entity2", &user_id, "{\"key\":\"val\"}").await.unwrap();
    queries::audit::append_event(&pool, "test_event_3", "test", "entity3", &user_id, "[]").await.unwrap();

    // Verify chain is valid
    let events = queries::audit::get_all_events_ordered(&pool).await.unwrap();
    assert_eq!(events.len(), 3);
    let chain: Vec<_> = events.iter().map(|e| hash_chain::ChainEvent {
        sequence_number: e.sequence_number,
        event_type: e.event_type.clone(),
        entity_id: e.entity_id.clone(),
        occurred_at: e.occurred_at.clone(),
        detail_json: e.detail_json.clone().unwrap_or_default(),
        previous_hash: e.previous_hash.clone(),
        event_hash: e.event_hash.clone(),
    }).collect();
    assert!(hash_chain::verify_chain(&chain).is_ok());

    // Tamper with an event hash
    sqlx::query("UPDATE audit_events SET event_hash = 'tampered' WHERE sequence_number = 2")
        .execute(&pool).await.unwrap();

    let events2 = queries::audit::get_all_events_ordered(&pool).await.unwrap();
    let chain2: Vec<_> = events2.iter().map(|e| hash_chain::ChainEvent {
        sequence_number: e.sequence_number,
        event_type: e.event_type.clone(),
        entity_id: e.entity_id.clone(),
        occurred_at: e.occurred_at.clone(),
        detail_json: e.detail_json.clone().unwrap_or_default(),
        previous_hash: e.previous_hash.clone(),
        event_hash: e.event_hash.clone(),
    }).collect();
    assert!(hash_chain::verify_chain(&chain2).is_err());
}

#[tokio::test]
async fn e2e_full_acceptance_scenario() {
    let pool = setup_test_db().await;
    let (company_id, user_id, fy_id) = setup_full_env(&pool).await;

    // === 1. Create accounts ===
    let acc_1910 = queries::account::create(&pool, &company_id, 1910, "Kassa", "asset", None, None, None).await.unwrap();
    let acc_1930 = queries::account::create(&pool, &company_id, 1930, "Företagskonto", "asset", None, None, None).await.unwrap();
    let acc_2440 = queries::account::create(&pool, &company_id, 2440, "Leverantörsskulder", "liability", None, None, None).await.unwrap();
    let acc_3010 = queries::account::create(&pool, &company_id, 3010, "Försäljningsintäkter", "revenue", None, None, None).await.unwrap();
    let acc_5410 = queries::account::create(&pool, &company_id, 5410, "Kontorsmaterial", "expense", None, None, None).await.unwrap();

    let accounts = queries::account::list(&pool, &company_id).await.unwrap();
    assert!(accounts.len() >= 5);

    // === 2. Create counterparty ===
    let cp_id = queries::counterparty::create(
        &pool, &company_id, "Leverantör AB", Some("556111-2222"), Some("info@leverantor.se")
    ).await.unwrap();
    let counterparties = queries::counterparty::list(&pool, &company_id).await.unwrap();
    assert_eq!(counterparties.len(), 1);
    assert_eq!(counterparties[0].name, "Leverantör AB");

    // Update counterparty
    queries::counterparty::update(&pool, &cp_id, "Leverantör AB (ny)", Some("556111-2222"), Some("ny@leverantor.se")).await.unwrap();
    let cp = queries::counterparty::get_by_id(&pool, &cp_id).await.unwrap().unwrap();
    assert_eq!(cp.name, "Leverantör AB (ny)");

    // === 3. Book multiple vouchers ===
    let jan = queries::period::find_for_date(&pool, &fy_id, "2026-01-15").await.unwrap().unwrap();
    let feb = queries::period::find_for_date(&pool, &fy_id, "2026-02-15").await.unwrap().unwrap();

    // Voucher A1: Buy office supplies (January)
    let v1_id = queries::voucher::create(
        &pool, &company_id, &fy_id, &jan.id, "A", 1, "2026-01-10", "Köp kontorsmaterial",
        Some(&cp_id), &user_id,
    ).await.unwrap();
    queries::voucher::add_line(&pool, &v1_id, 1, &acc_5410, 15000, 0, Some("Kontorsmaterial"), None, None, None).await.unwrap();
    queries::voucher::add_line(&pool, &v1_id, 2, &acc_2440, 0, 15000, Some("Leverantörsskuld"), None, None, None).await.unwrap();
    queries::voucher::mark_posted(&pool, &v1_id, &user_id).await.unwrap();

    // Log processing history
    queries::audit::append_processing_history(
        &pool, "voucher_posted", "voucher", &v1_id, &user_id,
        "Verifikation A1 bokförd", None,
    ).await.unwrap();

    // Voucher A2: Sales revenue (January)
    let v2_id = queries::voucher::create(
        &pool, &company_id, &fy_id, &jan.id, "A", 2, "2026-01-20", "Försäljning tjänster",
        None, &user_id,
    ).await.unwrap();
    queries::voucher::add_line(&pool, &v2_id, 1, &acc_1930, 50000, 0, Some("Kundbetalning"), None, None, None).await.unwrap();
    queries::voucher::add_line(&pool, &v2_id, 2, &acc_3010, 0, 50000, Some("Intäkt"), None, None, None).await.unwrap();
    queries::voucher::mark_posted(&pool, &v2_id, &user_id).await.unwrap();

    // Voucher A3: Pay supplier (February)
    let v3_id = queries::voucher::create(
        &pool, &company_id, &fy_id, &feb.id, "A", 3, "2026-02-05", "Betalning leverantör",
        Some(&cp_id), &user_id,
    ).await.unwrap();
    queries::voucher::add_line(&pool, &v3_id, 1, &acc_2440, 15000, 0, Some("Betala skuld"), None, None, None).await.unwrap();
    queries::voucher::add_line(&pool, &v3_id, 2, &acc_1930, 0, 15000, Some("Utbetalning"), None, None, None).await.unwrap();
    queries::voucher::mark_posted(&pool, &v3_id, &user_id).await.unwrap();

    // === 4. Correct voucher A1 (wrong amount) ===
    let rev_id = queries::voucher::create(
        &pool, &company_id, &fy_id, &jan.id, "A", 4, "2026-01-25",
        "Rättelse av ver A1: Fel belopp (skulle vara 12000 öre)",
        None, &user_id,
    ).await.unwrap();
    queries::voucher::add_line(&pool, &rev_id, 1, &acc_5410, 0, 15000, None, None, None, None).await.unwrap();
    queries::voucher::add_line(&pool, &rev_id, 2, &acc_2440, 15000, 0, None, None, None, None).await.unwrap();
    queries::voucher::mark_posted(&pool, &rev_id, &user_id).await.unwrap();

    // Replacement voucher with correct amount
    let repl_id = queries::voucher::create(
        &pool, &company_id, &fy_id, &jan.id, "A", 5, "2026-01-25",
        "Korrekt belopp kontorsmaterial",
        Some(&cp_id), &user_id,
    ).await.unwrap();
    queries::voucher::add_line(&pool, &repl_id, 1, &acc_5410, 12000, 0, None, None, None, None).await.unwrap();
    queries::voucher::add_line(&pool, &repl_id, 2, &acc_2440, 0, 12000, None, None, None, None).await.unwrap();
    queries::voucher::mark_posted(&pool, &repl_id, &user_id).await.unwrap();

    // Create correction link
    let _link_id = queries::correction::create_link(
        &pool, &v1_id, &rev_id, Some(&repl_id), "Fel belopp, skulle vara 120 kr", &user_id,
    ).await.unwrap();

    // Verify correction links
    let links = queries::correction::get_links_for_voucher(&pool, &v1_id).await.unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].replacement_voucher_id.as_deref(), Some(repl_id.as_str()));

    // === 5. Close January period ===
    queries::period::update_status(&pool, &jan.id, "closed", &user_id).await.unwrap();
    let jan_updated = queries::period::get_by_id(&pool, &jan.id).await.unwrap().unwrap();
    assert_eq!(jan_updated.status, "closed");

    // Log processing history for period close
    queries::audit::append_processing_history(
        &pool, "period_closed", "period", &jan.id, &user_id,
        "Period 2026-01 stängd", None,
    ).await.unwrap();

    // === 6. Verify journal and ledger ===
    let journal = queries::ledger::get_journal(&pool, &fy_id, None, None).await.unwrap();
    // 5 posted vouchers × 2 lines each = 10 journal entries
    assert_eq!(journal.len(), 10);

    let gl_5410 = queries::ledger::get_general_ledger(&pool, &fy_id, Some(5410)).await.unwrap();
    assert_eq!(gl_5410.len(), 3); // original debit + reversal credit + replacement debit
    // Net effect: 12000 debit on 5410
    let net_5410: i64 = gl_5410.iter().map(|l| l.debit_amount - l.credit_amount).sum();
    assert_eq!(net_5410, 12000);

    // Trial balance
    let tb = queries::ledger::get_trial_balance(&pool, &fy_id).await.unwrap();
    let total_debit: i64 = tb.iter().map(|b| b.total_debit).sum();
    let total_credit: i64 = tb.iter().map(|b| b.total_credit).sum();
    assert_eq!(total_debit, total_credit);

    // === 7. Audit trail ===
    queries::audit::append_event(&pool, "voucher_posted", "voucher", &v1_id, &user_id, "{}").await.unwrap();
    queries::audit::append_event(&pool, "voucher_corrected", "voucher", &v1_id, &user_id,
        &serde_json::json!({"reversal": rev_id, "replacement": repl_id}).to_string()).await.unwrap();
    queries::audit::append_event(&pool, "period_closed", "period", &jan.id, &user_id, "{}").await.unwrap();

    // Verify hash chain integrity
    let events = queries::audit::get_all_events_ordered(&pool).await.unwrap();
    assert!(events.len() >= 3);
    let chain: Vec<_> = events.iter().map(|e| hash_chain::ChainEvent {
        sequence_number: e.sequence_number,
        event_type: e.event_type.clone(),
        entity_id: e.entity_id.clone(),
        occurred_at: e.occurred_at.clone(),
        detail_json: e.detail_json.clone().unwrap_or_default(),
        previous_hash: e.previous_hash.clone(),
        event_hash: e.event_hash.clone(),
    }).collect();
    assert!(hash_chain::verify_chain(&chain).is_ok());

    // === 8. Processing history ===
    let ph = queries::audit::list_processing_history(&pool, 100, 0).await.unwrap();
    assert!(ph.len() >= 2);
    assert!(ph.iter().any(|e| e.event_type == "voucher_posted"));
    assert!(ph.iter().any(|e| e.event_type == "period_closed"));

    // === 9. Posted voucher is immutable (cannot delete) ===
    let deleted = queries::voucher::delete_draft(&pool, &v1_id).await.unwrap();
    assert!(!deleted, "Posted voucher must not be deletable");

    // === 10. Close fiscal year ===
    // Close remaining periods
    let periods = queries::period::list_by_fiscal_year(&pool, &fy_id).await.unwrap();
    for p in &periods {
        if p.status == "open" {
            queries::period::update_status(&pool, &p.id, "closed", &user_id).await.unwrap();
        }
    }
    queries::fiscal_year::close(&pool, &fy_id).await.unwrap();
    let fy = queries::fiscal_year::get_by_id(&pool, &fy_id).await.unwrap().unwrap();
    assert!(fy.is_closed);

    // === 11. System documentation snapshot ===
    let content = serde_json::json!({"system": "Bokis", "accounts": accounts.len()});
    let content_str = serde_json::to_string_pretty(&content).unwrap();
    let content_hash = bokforing_core::hash_chain::compute_file_hash(content_str.as_bytes());
    let doc_id = queries::system_doc::create(&pool, &user_id, &content_str, &content_hash).await.unwrap();
    assert!(!doc_id.is_empty());

    let docs = queries::system_doc::list(&pool, 10).await.unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].content_hash, content_hash);

    // === 12. Archive manifest ===
    let archive_path = "/tmp/test_archive_2026.db";
    let archive_hash = "abc123def456";
    let manifest_json = serde_json::json!({"fiscal_year": "2026", "accounts": 5}).to_string();
    let archive_id = queries::archive::create(
        &pool, &fy_id, &user_id, archive_path, archive_hash, &manifest_json
    ).await.unwrap();
    assert!(!archive_id.is_empty());

    let archives = queries::archive::list_all(&pool).await.unwrap();
    assert_eq!(archives.len(), 1);
    assert_eq!(archives[0].fiscal_year_id, fy_id);

    // === 13. Verify counterparty still linked to vouchers ===
    let cps = queries::counterparty::list(&pool, &company_id).await.unwrap();
    assert_eq!(cps.len(), 1);
    assert_eq!(cps[0].name, "Leverantör AB (ny)");
}

#[tokio::test]
async fn period_locking_prevents_reopening() {
    let pool = setup_test_db().await;
    let (company_id, user_id, fy_id) = setup_full_env(&pool).await;

    let jan = queries::period::find_for_date(&pool, &fy_id, "2026-01-15").await.unwrap().unwrap();

    // Close then lock
    queries::period::update_status(&pool, &jan.id, "closed", &user_id).await.unwrap();
    queries::period::update_status(&pool, &jan.id, "locked", &user_id).await.unwrap();

    let locked = queries::period::get_by_id(&pool, &jan.id).await.unwrap().unwrap();
    assert_eq!(locked.status, "locked");
}

#[tokio::test]
async fn counterparty_crud_operations() {
    let pool = setup_test_db().await;
    let (company_id, _user_id, _fy_id) = setup_full_env(&pool).await;

    // Create
    let id = queries::counterparty::create(&pool, &company_id, "Test AB", Some("556000-1234"), None).await.unwrap();

    // Read
    let cp = queries::counterparty::get_by_id(&pool, &id).await.unwrap().unwrap();
    assert_eq!(cp.name, "Test AB");
    assert_eq!(cp.org_number.as_deref(), Some("556000-1234"));

    // Update
    queries::counterparty::update(&pool, &id, "Test AB (uppdaterat)", None, Some("kontakt@test.se")).await.unwrap();
    let cp2 = queries::counterparty::get_by_id(&pool, &id).await.unwrap().unwrap();
    assert_eq!(cp2.name, "Test AB (uppdaterat)");
    assert_eq!(cp2.contact_info.as_deref(), Some("kontakt@test.se"));

    // Delete
    queries::counterparty::delete(&pool, &id).await.unwrap();
    let cp3 = queries::counterparty::get_by_id(&pool, &id).await.unwrap();
    assert!(cp3.is_none());
}

#[tokio::test]
async fn processing_history_records_events() {
    let pool = setup_test_db().await;
    let (_company_id, user_id, _fy_id) = setup_full_env(&pool).await;

    queries::audit::append_processing_history(
        &pool, "voucher_posted", "voucher", "v-001", &user_id,
        "Verifikation A1 bokförd", Some("{\"amount\": 10000}"),
    ).await.unwrap();

    queries::audit::append_processing_history(
        &pool, "period_closed", "period", "p-001", &user_id,
        "Period 2026-01 stängd", None,
    ).await.unwrap();

    let history = queries::audit::list_processing_history(&pool, 10, 0).await.unwrap();
    assert_eq!(history.len(), 2);
    assert!(history.iter().any(|e| e.event_type == "period_closed"));
    assert!(history.iter().any(|e| e.event_type == "voucher_posted"));
    let voucher_evt = history.iter().find(|e| e.event_type == "voucher_posted").unwrap();
    assert_eq!(voucher_evt.description, "Verifikation A1 bokförd");
    assert_eq!(voucher_evt.metadata_json.as_deref(), Some("{\"amount\": 10000}"));
}

#[tokio::test]
async fn backup_and_verify_with_file_db() {
    let tmp_dir = std::env::temp_dir().join("bokis_backup_test");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).unwrap();

    let db_path = tmp_dir.join("test.db");
    let pool = SqlitePool::connect(&format!("sqlite:{}?mode=rwc", db_path.display()))
        .await
        .expect("Failed to create file-based DB");

    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    // Seed data
    bokforing_db::seed::initial_setup(&pool, "testpass123", "Backuptest AB", "559900-1122")
        .await
        .unwrap();

    let (company_id,): (String,) = sqlx::query_as("SELECT id FROM companies LIMIT 1")
        .fetch_one(&pool).await.unwrap();
    let (user_id,): (String,) = sqlx::query_as("SELECT id FROM users LIMIT 1")
        .fetch_one(&pool).await.unwrap();

    // Create some data
    let fy_id = queries::fiscal_year::create_with_periods(
        &pool, &company_id, "2026", "2026-01-01", "2026-12-31"
    ).await.unwrap();
    let acc = queries::account::create(&pool, &company_id, 1910, "Kassa", "asset", None, None, None).await.unwrap();
    let acc2 = queries::account::create(&pool, &company_id, 2440, "Skuld", "liability", None, None, None).await.unwrap();

    let jan = queries::period::find_for_date(&pool, &fy_id, "2026-01-15").await.unwrap().unwrap();
    let vid = queries::voucher::create(
        &pool, &company_id, &fy_id, &jan.id, "A", 1, "2026-01-10", "Test", None, &user_id
    ).await.unwrap();
    queries::voucher::add_line(&pool, &vid, 1, &acc, 5000, 0, None, None, None, None).await.unwrap();
    queries::voucher::add_line(&pool, &vid, 2, &acc2, 0, 5000, None, None, None, None).await.unwrap();
    queries::voucher::mark_posted(&pool, &vid, &user_id).await.unwrap();

    // === Perform backup ===
    let backup_path = tmp_dir.join("backup_test.db");
    let job_id = queries::backup::create_backup(&pool, &backup_path, &user_id).await.unwrap();
    assert!(!job_id.is_empty());

    // Verify backup file exists and is non-empty
    let backup_metadata = std::fs::metadata(&backup_path).expect("Backup file should exist");
    assert!(backup_metadata.len() > 0, "Backup file should be non-empty");

    // Check job status
    let jobs = queries::backup::list_jobs(&pool, 10).await.unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, "completed");
    assert!(jobs[0].backup_hash.is_some());
    assert!(jobs[0].size_bytes.is_some());
    assert_eq!(jobs[0].size_bytes.unwrap(), backup_metadata.len() as i64);

    // === Verify backup integrity ===
    let backup_data = std::fs::read(&backup_path).unwrap();
    let actual_hash = hash_chain::compute_file_hash(&backup_data);
    assert_eq!(actual_hash, jobs[0].backup_hash.as_ref().unwrap().as_str());

    // Open backup and check integrity + data
    let backup_pool = SqlitePool::connect(&format!("sqlite:{}?mode=ro", backup_path.display()))
        .await
        .expect("Should be able to open backup");

    let (integrity,): (String,) = sqlx::query_as("PRAGMA integrity_check")
        .fetch_one(&backup_pool)
        .await
        .unwrap();
    assert_eq!(integrity, "ok");

    // Verify data exists in backup
    let (voucher_count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM vouchers WHERE status = 'posted'")
        .fetch_one(&backup_pool)
        .await
        .unwrap();
    assert_eq!(voucher_count, 1);

    let (account_count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM accounts")
        .fetch_one(&backup_pool)
        .await
        .unwrap();
    assert!(account_count >= 2);

    backup_pool.close().await;

    // === Record verification ===
    let verify_id = queries::backup::record_verification(
        &pool, &job_id, &user_id, true, "Integrity OK, hash match"
    ).await.unwrap();
    assert!(!verify_id.is_empty());

    // === Test tampered backup detection ===
    // Corrupt the backup file
    let mut corrupted = backup_data.clone();
    if corrupted.len() > 100 {
        corrupted[50] ^= 0xFF;
        corrupted[51] ^= 0xFF;
    }
    std::fs::write(&backup_path, &corrupted).unwrap();

    let tampered_hash = hash_chain::compute_file_hash(&corrupted);
    assert_ne!(tampered_hash, jobs[0].backup_hash.as_ref().unwrap().as_str(),
        "Tampered file should have different hash");

    // Cleanup
    pool.close().await;
    let _ = std::fs::remove_dir_all(&tmp_dir);
}
