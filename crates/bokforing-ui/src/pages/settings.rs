use leptos::prelude::*;
use serde::Deserialize;
use crate::api;
use crate::state::AppContext;

#[derive(Clone, Deserialize)]
struct FiscalYearDto {
    id: String,
    name: String,
    start_date: String,
    end_date: String,
    is_closed: bool,
}

#[derive(Clone, Deserialize)]
#[allow(dead_code)]
struct UserDto {
    id: String,
    username: String,
    display_name: String,
    is_active: bool,
    roles: Vec<String>,
}

#[component]
pub fn SettingsPage() -> impl IntoView {
    let ctx = expect_context::<AppContext>();
    let refresh = RwSignal::new(0u32);

    let fiscal_years = leptos::prelude::LocalResource::new(move || {
        let _ = refresh.get();
        async move {
            api::invoke_no_args::<Vec<FiscalYearDto>>("list_fiscal_years").await.unwrap_or_default()
        }
    });

    let users = leptos::prelude::LocalResource::new(move || {
        let _ = refresh.get();
        async move {
            api::invoke_no_args::<Vec<UserDto>>("list_users").await.unwrap_or_default()
        }
    });

    let fy_name = RwSignal::new(String::new());
    let fy_start = RwSignal::new(String::new());
    let fy_end = RwSignal::new(String::new());
    let fy_error = RwSignal::new(Option::<String>::None);

    let create_fy = {
        let ctx = ctx;
        move |ev: web_sys::SubmitEvent| {
            ev.prevent_default();
            leptos::task::spawn_local(async move {
                fy_error.set(None);
                let args = serde_json::json!({
                    "name": fy_name.get_untracked(),
                    "startDate": fy_start.get_untracked(),
                    "endDate": fy_end.get_untracked()
                });
                match api::invoke::<_, FiscalYearDto>("create_fiscal_year", &args).await {
                    Ok(fy) => {
                        ctx.active_fiscal_year.set(Some(crate::state::FiscalYearInfo {
                            id: fy.id.clone(),
                            name: fy.name,
                            start_date: fy.start_date,
                            end_date: fy.end_date,
                        }));
                        let activate_args = serde_json::json!({"fiscalYearId": fy.id});
                        let _ = api::invoke::<_, FiscalYearDto>("set_active_fiscal_year", &activate_args).await;
                        fy_name.set(String::new());
                        fy_start.set(String::new());
                        fy_end.set(String::new());
                        refresh.update(|c| *c += 1);
                    }
                    Err(e) => fy_error.set(Some(e)),
                }
            });
        }
    };

    let activate_fy = move |fy: FiscalYearDto| {
        leptos::task::spawn_local(async move {
            let args = serde_json::json!({"fiscalYearId": fy.id});
            if let Ok(activated) = api::invoke::<_, FiscalYearDto>("set_active_fiscal_year", &args).await {
                ctx.active_fiscal_year.set(Some(crate::state::FiscalYearInfo {
                    id: activated.id,
                    name: activated.name,
                    start_date: activated.start_date,
                    end_date: activated.end_date,
                }));
            }
        });
    };

    let new_username = RwSignal::new(String::new());
    let new_display = RwSignal::new(String::new());
    let new_password = RwSignal::new(String::new());
    let new_role = RwSignal::new("bokforare".to_string());
    let user_error = RwSignal::new(Option::<String>::None);

    let create_user = move |ev: web_sys::SubmitEvent| {
        ev.prevent_default();
        leptos::task::spawn_local(async move {
            user_error.set(None);
            let args = serde_json::json!({
                "username": new_username.get_untracked(),
                "displayName": new_display.get_untracked(),
                "password": new_password.get_untracked(),
                "role": new_role.get_untracked()
            });
            match api::invoke::<_, UserDto>("create_user", &args).await {
                Ok(_) => {
                    new_username.set(String::new());
                    new_display.set(String::new());
                    new_password.set(String::new());
                    refresh.update(|c| *c += 1);
                }
                Err(e) => user_error.set(Some(e)),
            }
        });
    };

    view! {
        <div class="page">
            <h1>"Inställningar"</h1>

            <section class="settings-section">
                <h2>"Räkenskapsår"</h2>

                {move || fy_error.get().map(|msg| view! {
                    <div class="error-banner">{msg}</div>
                })}

                <form class="inline-form" on:submit=create_fy>
                    <div class="form-row">
                        <div class="form-group">
                            <label>"Namn"</label>
                            <input
                                type="text"
                                placeholder="t.ex. 2026"
                                prop:value=move || fy_name.get()
                                on:input=move |ev| fy_name.set(event_target_value(&ev))
                                required=true
                            />
                        </div>
                        <div class="form-group">
                            <label>"Startdatum"</label>
                            <input
                                type="date"
                                prop:value=move || fy_start.get()
                                on:change=move |ev| fy_start.set(event_target_value(&ev))
                                required=true
                            />
                        </div>
                        <div class="form-group">
                            <label>"Slutdatum"</label>
                            <input
                                type="date"
                                prop:value=move || fy_end.get()
                                on:change=move |ev| fy_end.set(event_target_value(&ev))
                                required=true
                            />
                        </div>
                        <div class="form-group form-group-btn">
                            <button type="submit" class="btn btn-primary">"Skapa"</button>
                        </div>
                    </div>
                </form>

                <Suspense fallback=|| view! { <p>"Laddar..."</p> }>
                    {move || fiscal_years.get().map(|fys| {
                        let active_id = ctx.active_fiscal_year.get().map(|f| f.id);
                        view! {
                            <table class="data-table">
                                <thead>
                                    <tr>
                                        <th>"Namn"</th>
                                        <th>"Period"</th>
                                        <th>"Status"</th>
                                        <th>"Åtgärd"</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {fys.iter().map(|fy| {
                                        let is_active = active_id.as_deref() == Some(&fy.id);
                                        let fy_clone = fy.clone();
                                        let activate_fy = activate_fy.clone();
                                        view! {
                                            <tr>
                                                <td><strong>{fy.name.clone()}</strong></td>
                                                <td>{format!("{} — {}", fy.start_date, fy.end_date)}</td>
                                                <td>
                                                    {if is_active {
                                                        view! { <span class="status-badge status-open">"Aktiv"</span> }.into_any()
                                                    } else if fy.is_closed {
                                                        view! { <span class="status-badge status-closed">"Stängt"</span> }.into_any()
                                                    } else {
                                                        view! { <span class="status-badge">"—"</span> }.into_any()
                                                    }}
                                                </td>
                                                <td>
                                                    {if !is_active && !fy.is_closed {
                                                        Some(view! {
                                                            <button class="btn btn-sm" on:click=move |_| {
                                                                activate_fy(fy_clone.clone());
                                                            }>"Aktivera"</button>
                                                        })
                                                    } else {
                                                        None
                                                    }}
                                                </td>
                                            </tr>
                                        }
                                    }).collect_view()}
                                </tbody>
                            </table>
                        }
                    })}
                </Suspense>
            </section>

            <section class="settings-section">
                <h2>"Användare"</h2>

                {move || user_error.get().map(|msg| view! {
                    <div class="error-banner">{msg}</div>
                })}

                <form class="inline-form" on:submit=create_user>
                    <div class="form-row">
                        <div class="form-group">
                            <label>"Användarnamn"</label>
                            <input
                                type="text"
                                prop:value=move || new_username.get()
                                on:input=move |ev| new_username.set(event_target_value(&ev))
                                required=true
                            />
                        </div>
                        <div class="form-group">
                            <label>"Visningsnamn"</label>
                            <input
                                type="text"
                                prop:value=move || new_display.get()
                                on:input=move |ev| new_display.set(event_target_value(&ev))
                                required=true
                            />
                        </div>
                        <div class="form-group">
                            <label>"Lösenord"</label>
                            <input
                                type="password"
                                prop:value=move || new_password.get()
                                on:input=move |ev| new_password.set(event_target_value(&ev))
                                required=true
                            />
                        </div>
                        <div class="form-group">
                            <label>"Roll"</label>
                            <select
                                prop:value=move || new_role.get()
                                on:change=move |ev| new_role.set(event_target_value(&ev))
                            >
                                <option value="admin">"Admin"</option>
                                <option value="bokforare" selected=true>"Bokförare"</option>
                                <option value="attest">"Attestant"</option>
                                <option value="revisor">"Revisor"</option>
                            </select>
                        </div>
                        <div class="form-group form-group-btn">
                            <button type="submit" class="btn btn-primary">"Skapa"</button>
                        </div>
                    </div>
                </form>

                <Suspense fallback=|| view! { <p>"Laddar..."</p> }>
                    {move || users.get().map(|us| {
                        view! {
                            <table class="data-table">
                                <thead>
                                    <tr>
                                        <th>"Användarnamn"</th>
                                        <th>"Namn"</th>
                                        <th>"Roller"</th>
                                        <th>"Status"</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {us.iter().map(|u| {
                                        let roles_str = u.roles.join(", ");
                                        view! {
                                            <tr>
                                                <td>{u.username.clone()}</td>
                                                <td>{u.display_name.clone()}</td>
                                                <td>{roles_str}</td>
                                                <td>
                                                    {if u.is_active {
                                                        view! { <span class="status-badge status-open">"Aktiv"</span> }.into_any()
                                                    } else {
                                                        view! { <span class="status-badge status-closed">"Inaktiv"</span> }.into_any()
                                                    }}
                                                </td>
                                            </tr>
                                        }
                                    }).collect_view()}
                                </tbody>
                            </table>
                        }
                    })}
                </Suspense>
            </section>
        </div>
    }
}
