use leptos::prelude::*;
use crate::state::AppContext;

#[component]
pub fn DashboardPage() -> impl IntoView {
    let ctx = expect_context::<AppContext>();

    let user = move || ctx.current_user.get();
    let fy = move || ctx.active_fiscal_year.get();

    view! {
        <div class="page">
            <h1>"Översikt"</h1>

            {move || user().map(|u| view! {
                <p class="welcome">"Välkommen, " {u.display_name} "!"</p>
            })}

            {move || match fy() {
                Some(fy) => view! {
                    <div class="fy-info">
                        <h2>"Räkenskapsår: " {fy.name}</h2>
                        <p>{fy.start_date} " — " {fy.end_date}</p>
                    </div>
                }.into_any(),
                None => view! {
                    <div class="no-fy">
                        <p>"Inget räkenskapsår valt. Gå till Inställningar för att skapa ett."</p>
                    </div>
                }.into_any(),
            }}

            <div class="quick-actions">
                <h2>"Snabbåtgärder"</h2>
                <div class="action-grid">
                    <a href="/vouchers/new" class="action-card">
                        <span class="action-icon">"+"</span>
                        <span>"Ny verifikation"</span>
                    </a>
                    <a href="/journal" class="action-card">
                        <span class="action-icon">"J"</span>
                        <span>"Grundbok"</span>
                    </a>
                    <a href="/general-ledger" class="action-card">
                        <span class="action-icon">"H"</span>
                        <span>"Huvudbok"</span>
                    </a>
                    <a href="/periods" class="action-card">
                        <span class="action-icon">"P"</span>
                        <span>"Perioder"</span>
                    </a>
                </div>
            </div>
        </div>
    }
}
