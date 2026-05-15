mod commands;
mod error;
mod state;

use std::sync::Arc;
use tokio::sync::RwLock;
use tracing_subscriber::EnvFilter;

use tauri::Manager;

use state::AppState;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;

            let db_path = data_dir.join("bokforing.db");
            let pool = tauri::async_runtime::block_on(async {
                bokforing_db::pool::create_pool(&db_path).await
            })?;

            tauri::async_runtime::block_on(async {
                sqlx::migrate!("../bokforing-db/migrations")
                    .run(&pool)
                    .await
                    .expect("Migrationer misslyckades");
            });

            app.manage(AppState {
                pool,
                data_dir,
                current_user: Arc::new(RwLock::new(None)),
                current_company: Arc::new(RwLock::new(None)),
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::setup::check_needs_setup,
            commands::setup::run_initial_setup,
            commands::auth::login,
            commands::auth::logout,
            commands::auth::get_current_user,
            commands::auth::change_password,
            commands::voucher::create_voucher,
            commands::voucher::post_voucher,
            commands::voucher::list_vouchers,
            commands::ledger::get_journal,
            commands::ledger::get_general_ledger,
            commands::ledger::get_trial_balance,
            commands::account::list_accounts,
            commands::account::create_account,
            commands::period::list_periods,
            commands::period::transition_period,
            commands::backup::create_backup,
            commands::backup::list_backups,
            commands::backup::verify_backup,
            commands::backup::restore_backup,
            commands::backup::restore_from_file,
            commands::export::export_sie4,
            commands::export::export_csv,
            commands::export::export_json,
            commands::export::export_pdf,
            commands::admin::list_fiscal_years,
            commands::admin::create_fiscal_year,
            commands::admin::set_active_fiscal_year,
            commands::admin::list_users,
            commands::admin::create_user,
            commands::audit::list_audit_events,
            commands::audit::verify_audit_chain,
            commands::correction::correct_voucher,
            commands::correction::get_voucher_details,
            commands::attachment::upload_attachment,
            commands::attachment::list_attachments,
            commands::archive::create_archive,
            commands::archive::list_archives,
            commands::archive::generate_system_doc,
            commands::archive::list_system_docs,
            commands::counterparty::list_counterparties,
            commands::counterparty::create_counterparty,
            commands::counterparty::update_counterparty,
            commands::counterparty::delete_counterparty,
            commands::audit::list_processing_history,
        ])
        .run(tauri::generate_context!())
        .expect("Kunde inte starta applikationen");
}
