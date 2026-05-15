use leptos::prelude::*;
use serde::Deserialize;
use crate::api;
use crate::state::{AppContext, AppPhase, UserInfo, FiscalYearInfo};

#[derive(Clone, Deserialize)]
struct FyDto {
    id: String,
    name: String,
    start_date: String,
    end_date: String,
}

#[component]
pub fn LoginPage() -> impl IntoView {
    let ctx = expect_context::<AppContext>();
    let username = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());
    let error_msg = RwSignal::new(Option::<String>::None);
    let loading = RwSignal::new(false);

    let on_submit = move |ev: web_sys::SubmitEvent| {
        ev.prevent_default();
        let ctx = ctx.clone();
        leptos::task::spawn_local(async move {
            loading.set(true);
            error_msg.set(None);

            let args = serde_json::json!({
                "username": username.get_untracked(),
                "password": password.get_untracked()
            });

            match api::invoke::<_, UserInfo>("login", &args).await {
                Ok(user) => {
                    ctx.current_user.set(Some(user));
                    if let Ok(fys) = api::invoke_no_args::<Vec<FyDto>>("list_fiscal_years").await {
                        if let Some(fy) = fys.into_iter().next() {
                            let activate_args = serde_json::json!({"fiscalYearId": fy.id});
                            let _ = api::invoke::<_, FyDto>("set_active_fiscal_year", &activate_args).await;
                            ctx.active_fiscal_year.set(Some(FiscalYearInfo {
                                id: fy.id,
                                name: fy.name,
                                start_date: fy.start_date,
                                end_date: fy.end_date,
                            }));
                        }
                    }
                    ctx.phase.set(AppPhase::Authenticated);
                }
                Err(e) => {
                    error_msg.set(Some(e));
                }
            }
            loading.set(false);
        });
    };

    view! {
        <div class="login-container">
            <div class="login-card">
                <h1>"Bokis"</h1>
                <p class="login-subtitle">"Logga in för att fortsätta"</p>

                <form on:submit=on_submit>
                    {move || error_msg.get().map(|msg| view! {
                        <div class="error-banner">{msg}</div>
                    })}

                    <div class="form-group">
                        <label for="username">"Användarnamn"</label>
                        <input
                            id="username"
                            type="text"
                            prop:value=move || username.get()
                            on:input=move |ev| username.set(event_target_value(&ev))
                            placeholder="admin"
                            autofocus=true
                        />
                    </div>

                    <div class="form-group">
                        <label for="password">"Lösenord"</label>
                        <input
                            id="password"
                            type="password"
                            prop:value=move || password.get()
                            on:input=move |ev| password.set(event_target_value(&ev))
                        />
                    </div>

                    <button type="submit" class="btn btn-primary" disabled=move || loading.get()>
                        {move || if loading.get() { "Loggar in..." } else { "Logga in" }}
                    </button>
                </form>
            </div>
        </div>
    }
}
