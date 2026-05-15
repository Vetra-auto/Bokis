use leptos::prelude::*;
use serde::Deserialize;
use crate::api;
use crate::state::AppContext;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CounterpartyDto {
    id: String,
    name: String,
    org_number: Option<String>,
    contact_info: Option<String>,
    #[allow(dead_code)]
    created_at: String,
}

#[component]
pub fn CounterpartiesPage() -> impl IntoView {
    let ctx = expect_context::<AppContext>();
    let refresh = RwSignal::new(0u32);
    let error_msg = RwSignal::new(Option::<String>::None);
    let success_msg = RwSignal::new(Option::<String>::None);

    let new_name = RwSignal::new(String::new());
    let new_org_number = RwSignal::new(String::new());
    let new_contact_info = RwSignal::new(String::new());

    let editing_id = RwSignal::new(Option::<String>::None);
    let edit_name = RwSignal::new(String::new());
    let edit_org_number = RwSignal::new(String::new());
    let edit_contact_info = RwSignal::new(String::new());

    let counterparties = leptos::prelude::LocalResource::new(move || {
        let _ = refresh.get();
        async move {
            api::invoke_no_args::<Vec<CounterpartyDto>>("list_counterparties").await.unwrap_or_default()
        }
    });

    let do_create = move |ev: web_sys::SubmitEvent| {
        ev.prevent_default();
        let name = new_name.get_untracked();
        if name.trim().is_empty() {
            error_msg.set(Some("Namn krävs".into()));
            return;
        }
        error_msg.set(None);
        success_msg.set(None);

        let org = new_org_number.get_untracked();
        let contact = new_contact_info.get_untracked();

        leptos::task::spawn_local(async move {
            let args = serde_json::json!({
                "name": name,
                "orgNumber": if org.is_empty() { None } else { Some(org) },
                "contactInfo": if contact.is_empty() { None } else { Some(contact) },
            });
            match api::invoke::<_, CounterpartyDto>("create_counterparty", &args).await {
                Ok(_) => {
                    success_msg.set(Some("Motpart skapad".into()));
                    new_name.set(String::new());
                    new_org_number.set(String::new());
                    new_contact_info.set(String::new());
                    refresh.update(|c| *c += 1);
                }
                Err(e) => error_msg.set(Some(e)),
            }
        });
    };

    let start_edit = move |cp: CounterpartyDto| {
        editing_id.set(Some(cp.id));
        edit_name.set(cp.name);
        edit_org_number.set(cp.org_number.unwrap_or_default());
        edit_contact_info.set(cp.contact_info.unwrap_or_default());
    };

    let do_update = move |_| {
        let id = match editing_id.get_untracked() {
            Some(id) => id,
            None => return,
        };
        let name = edit_name.get_untracked();
        let org = edit_org_number.get_untracked();
        let contact = edit_contact_info.get_untracked();
        error_msg.set(None);
        success_msg.set(None);

        leptos::task::spawn_local(async move {
            let args = serde_json::json!({
                "id": id,
                "name": name,
                "orgNumber": if org.is_empty() { None } else { Some(org) },
                "contactInfo": if contact.is_empty() { None } else { Some(contact) },
            });
            match api::invoke::<_, ()>("update_counterparty", &args).await {
                Ok(_) => {
                    success_msg.set(Some("Motpart uppdaterad".into()));
                    editing_id.set(None);
                    refresh.update(|c| *c += 1);
                }
                Err(e) => error_msg.set(Some(e)),
            }
        });
    };

    let do_delete = move |id: String| {
        error_msg.set(None);
        success_msg.set(None);
        leptos::task::spawn_local(async move {
            let args = serde_json::json!({"id": id});
            match api::invoke::<_, ()>("delete_counterparty", &args).await {
                Ok(_) => {
                    success_msg.set(Some("Motpart borttagen".into()));
                    refresh.update(|c| *c += 1);
                }
                Err(e) => error_msg.set(Some(e)),
            }
        });
    };

    view! {
        <div class="page">
            <h1>"Motparter"</h1>

            {move || error_msg.get().map(|msg| view! {
                <div class="error-banner">{msg}</div>
            })}

            {move || success_msg.get().map(|msg| view! {
                <div class="success-banner">{msg}</div>
            })}

            <section class="settings-section">
                <h2>"Ny motpart"</h2>
                <form on:submit=do_create>
                    <div class="form-row">
                        <div class="form-group" style="flex: 2">
                            <label>"Namn"</label>
                            <input
                                type="text"
                                placeholder="Företagsnamn"
                                prop:value=move || new_name.get()
                                on:input=move |ev| new_name.set(event_target_value(&ev))
                                required=true
                            />
                        </div>
                        <div class="form-group" style="flex: 1">
                            <label>"Org.nummer"</label>
                            <input
                                type="text"
                                placeholder="556123-4567"
                                prop:value=move || new_org_number.get()
                                on:input=move |ev| new_org_number.set(event_target_value(&ev))
                            />
                        </div>
                        <div class="form-group" style="flex: 1">
                            <label>"Kontaktinfo"</label>
                            <input
                                type="text"
                                placeholder="E-post, telefon etc."
                                prop:value=move || new_contact_info.get()
                                on:input=move |ev| new_contact_info.set(event_target_value(&ev))
                            />
                        </div>
                        <div class="form-group form-group-btn">
                            <button type="submit" class="btn btn-primary">"Skapa"</button>
                        </div>
                    </div>
                </form>
            </section>

            <section class="settings-section">
                <h2>"Befintliga motparter"</h2>
                <Suspense fallback=|| view! { <p>"Laddar..."</p> }>
                    {move || counterparties.get().map(|list| {
                        if list.is_empty() {
                            return view! { <p class="text-muted">"Inga motparter registrerade."</p> }.into_any();
                        }
                        let editing = editing_id.get();
                        view! {
                            <table class="data-table">
                                <thead>
                                    <tr>
                                        <th>"Namn"</th>
                                        <th>"Org.nummer"</th>
                                        <th>"Kontaktinfo"</th>
                                        <th>"Åtgärder"</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {list.iter().map(|cp| {
                                        let cp_clone = cp.clone();
                                        let cp_id = cp.id.clone();
                                        let is_editing = editing.as_ref() == Some(&cp.id);
                                        if is_editing {
                                            view! {
                                                <tr>
                                                    <td>
                                                        <input type="text"
                                                            prop:value=move || edit_name.get()
                                                            on:input=move |ev| edit_name.set(event_target_value(&ev))
                                                        />
                                                    </td>
                                                    <td>
                                                        <input type="text"
                                                            prop:value=move || edit_org_number.get()
                                                            on:input=move |ev| edit_org_number.set(event_target_value(&ev))
                                                        />
                                                    </td>
                                                    <td>
                                                        <input type="text"
                                                            prop:value=move || edit_contact_info.get()
                                                            on:input=move |ev| edit_contact_info.set(event_target_value(&ev))
                                                        />
                                                    </td>
                                                    <td>
                                                        <button class="btn btn-sm btn-primary" on:click=do_update>"Spara"</button>
                                                        " "
                                                        <button class="btn btn-sm btn-secondary" on:click=move |_| editing_id.set(None)>"Avbryt"</button>
                                                    </td>
                                                </tr>
                                            }.into_any()
                                        } else {
                                            view! {
                                                <tr>
                                                    <td>{cp.name.clone()}</td>
                                                    <td>{cp.org_number.clone().unwrap_or_default()}</td>
                                                    <td>{cp.contact_info.clone().unwrap_or_default()}</td>
                                                    <td>
                                                        <button class="btn btn-sm btn-secondary"
                                                            on:click=move |_| start_edit(cp_clone.clone())
                                                        >"Redigera"</button>
                                                        " "
                                                        <button class="btn btn-sm btn-danger"
                                                            on:click={let id = cp_id.clone(); move |_| do_delete(id.clone())}
                                                        >"Ta bort"</button>
                                                    </td>
                                                </tr>
                                            }.into_any()
                                        }
                                    }).collect_view()}
                                </tbody>
                            </table>
                        }.into_any()
                    })}
                </Suspense>
            </section>
        </div>
    }
}
