use leptos::prelude::*;
use crate::api;
use crate::state::{AppContext, AppPhase};

#[component]
pub fn SetupPage() -> impl IntoView {
    let ctx = expect_context::<AppContext>();

    let company_name = RwSignal::new(String::new());
    let org_number = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());
    let password_confirm = RwSignal::new(String::new());
    let error_msg = RwSignal::new(Option::<String>::None);
    let loading = RwSignal::new(false);

    let password_len = move || password.get().len();
    let passwords_match = move || {
        let p = password.get();
        let c = password_confirm.get();
        !p.is_empty() && p == c
    };
    let can_submit = move || {
        password_len() >= 12
            && passwords_match()
            && !company_name.get().is_empty()
            && !loading.get()
    };

    let on_submit = move |ev: web_sys::SubmitEvent| {
        ev.prevent_default();
        leptos::task::spawn_local(async move {
            loading.set(true);
            error_msg.set(None);

            let args = serde_json::json!({
                "password": password.get_untracked(),
                "companyName": company_name.get_untracked(),
                "orgNumber": org_number.get_untracked()
            });

            match api::invoke::<_, ()>("run_initial_setup", &args).await {
                Ok(_) => ctx.phase.set(AppPhase::Login),
                Err(e) => error_msg.set(Some(e)),
            }
            loading.set(false);
        });
    };

    view! {
        <div class="login-container">
            <div class="setup-card">
                <h1>"Bokis"</h1>
                <p class="login-subtitle">"Konfigurera ditt bokföringssystem"</p>

                {move || error_msg.get().map(|msg| view! {
                    <div class="error-banner">{msg}</div>
                })}

                <form on:submit=on_submit>
                    <fieldset class="setup-section">
                        <legend>"Företagsuppgifter"</legend>
                        <div class="form-group">
                            <label>"Företagsnamn"</label>
                            <input
                                type="text"
                                prop:value=move || company_name.get()
                                on:input=move |ev| company_name.set(event_target_value(&ev))
                                placeholder="Mitt Företag AB"
                                required=true
                            />
                        </div>
                        <div class="form-group">
                            <label>"Organisationsnummer"</label>
                            <input
                                type="text"
                                prop:value=move || org_number.get()
                                on:input=move |ev| org_number.set(event_target_value(&ev))
                                placeholder="556677-8899"
                            />
                        </div>
                    </fieldset>

                    <fieldset class="setup-section">
                        <legend>"Admin-konto"</legend>
                        <p class="field-hint">"Användarnamn: " <strong>"admin"</strong></p>
                        <div class="form-group">
                            <label>"Lösenord"</label>
                            <input
                                type="password"
                                prop:value=move || password.get()
                                on:input=move |ev| password.set(event_target_value(&ev))
                                required=true
                            />
                            <span class={move || if password_len() == 0 { "field-hint" } else if password_len() >= 12 { "field-hint field-ok" } else { "field-hint field-err" }}>
                                {move || format!("{}/12 tecken", password_len())}
                            </span>
                        </div>
                        <div class="form-group">
                            <label>"Bekräfta lösenord"</label>
                            <input
                                type="password"
                                prop:value=move || password_confirm.get()
                                on:input=move |ev| password_confirm.set(event_target_value(&ev))
                                required=true
                            />
                            {move || {
                                let c = password_confirm.get();
                                if c.is_empty() {
                                    view! { <span class="field-hint"></span> }.into_any()
                                } else if passwords_match() {
                                    view! { <span class="field-hint field-ok">"Matchar"</span> }.into_any()
                                } else {
                                    view! { <span class="field-hint field-err">"Matchar inte"</span> }.into_any()
                                }
                            }}
                        </div>
                    </fieldset>

                    <button
                        type="submit"
                        class="btn btn-primary"
                        disabled=move || !can_submit()
                    >
                        {move || if loading.get() { "Skapar..." } else { "Skapa och starta" }}
                    </button>
                </form>
            </div>
        </div>
    }
}
