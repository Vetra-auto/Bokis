use leptos::prelude::*;
use leptos_router::components::*;
use leptos_router::path;

use crate::api;
use crate::state::{AppContext, AppPhase};
use crate::pages;

#[component]
pub fn App() -> impl IntoView {
    let ctx = AppContext::new();
    provide_context(ctx);

    leptos::task::spawn_local(async move {
        let needs_setup = api::invoke_no_args::<bool>("check_needs_setup")
            .await
            .unwrap_or(false);

        if needs_setup {
            ctx.phase.set(AppPhase::Setup);
        } else {
            ctx.phase.set(AppPhase::Login);
        }
    });

    view! {
        {move || match ctx.phase.get() {
            AppPhase::Loading => view! {
                <div class="loading-screen">"Laddar..."</div>
            }.into_any(),
            AppPhase::Setup => view! {
                <pages::setup::SetupPage />
            }.into_any(),
            AppPhase::Login => view! {
                <pages::login::LoginPage />
            }.into_any(),
            AppPhase::Authenticated => view! {
                <Router>
                    <AuthenticatedShell />
                </Router>
            }.into_any(),
        }}

        {move || ctx.toast_message.get().map(|msg| {
            view! {
                <div class="toast" on:click=move |_| ctx.toast_message.set(None)>
                    {msg}
                </div>
            }
        })}
    }
}

#[component]
fn AuthenticatedShell() -> impl IntoView {
    let ctx = expect_context::<AppContext>();

    let logout = move |_| {
        leptos::task::spawn_local(async move {
            let _ = api::invoke_no_args::<()>("logout").await;
            ctx.current_user.set(None);
            ctx.active_fiscal_year.set(None);
            ctx.phase.set(AppPhase::Login);
        });
    };

    view! {
        <div class="app-layout">
            <nav class="sidebar">
                <div class="sidebar-header">
                    <h2>"Bokis"</h2>
                </div>
                <ul class="nav-links">
                    <li><a href="/">"Översikt"</a></li>
                    <li><a href="/vouchers/new">"Ny verifikation"</a></li>
                    <li><a href="/vouchers">"Verifikationer"</a></li>
                    <li><a href="/journal">"Grundbok"</a></li>
                    <li><a href="/general-ledger">"Huvudbok"</a></li>
                    <li><a href="/counterparties">"Motparter"</a></li>
                    <li><a href="/periods">"Perioder"</a></li>
                    <li><a href="/export">"Export"</a></li>
                    <li><a href="/backup">"Backup"</a></li>
                    <li><a href="/archive">"Arkiv"</a></li>
                    <li><a href="/audit">"Revisionslogg"</a></li>
                    <li><a href="/processing-history">"Behandlingshistorik"</a></li>
                    <li><a href="/settings">"Inställningar"</a></li>
                </ul>
                <div class="sidebar-footer">
                    {move || ctx.current_user.get().map(|u| view! {
                        <div class="user-info">
                            <span class="user-name">{u.display_name}</span>
                        </div>
                    })}
                    <button class="btn btn-sm btn-secondary" on:click=logout>"Logga ut"</button>
                </div>
            </nav>
            <main class="main-content">
                <Routes fallback=|| view! { <p>"Sidan hittades inte"</p> }>
                    <Route path=path!("/") view=pages::dashboard::DashboardPage />
                    <Route path=path!("/vouchers/new") view=pages::voucher_entry::VoucherEntryPage />
                    <Route path=path!("/vouchers") view=pages::vouchers::VouchersPage />
                    <Route path=path!("/journal") view=pages::journal::JournalPage />
                    <Route path=path!("/general-ledger") view=pages::general_ledger::GeneralLedgerPage />
                    <Route path=path!("/counterparties") view=pages::counterparties::CounterpartiesPage />
                    <Route path=path!("/periods") view=pages::periods::PeriodsPage />
                    <Route path=path!("/export") view=pages::export::ExportPage />
                    <Route path=path!("/backup") view=pages::backup::BackupPage />
                    <Route path=path!("/archive") view=pages::archive::ArchivePage />
                    <Route path=path!("/audit") view=pages::audit::AuditPage />
                    <Route path=path!("/processing-history") view=pages::processing_history::ProcessingHistoryPage />
                    <Route path=path!("/settings") view=pages::settings::SettingsPage />
                </Routes>
            </main>
        </div>
    }
}
