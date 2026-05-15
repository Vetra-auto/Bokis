use leptos::prelude::*;
use serde::Deserialize;
use crate::api;
use crate::state::AppContext;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuditEventDto {
    id: String,
    sequence_number: i64,
    event_type: String,
    entity_type: String,
    entity_id: String,
    user_id: String,
    occurred_at: String,
    detail_json: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChainVerificationResult {
    valid: bool,
    total_events: usize,
    error: Option<String>,
}

#[component]
pub fn AuditPage() -> impl IntoView {
    let ctx = expect_context::<AppContext>();
    let refresh = RwSignal::new(0u32);
    let verifying = RwSignal::new(false);
    let verify_result = RwSignal::new(Option::<ChainVerificationResult>::None);
    let verify_error = RwSignal::new(Option::<String>::None);

    let events = leptos::prelude::LocalResource::new(move || {
        let _ = refresh.get();
        async move {
            let args = serde_json::json!({"limit": 200});
            api::invoke::<_, Vec<AuditEventDto>>("list_audit_events", &args)
                .await
                .unwrap_or_default()
        }
    });

    let is_admin_or_revisor = move || {
        ctx.current_user.get()
            .map(|u| u.roles.iter().any(|r| r == "admin" || r == "revisor"))
            .unwrap_or(false)
    };

    let do_verify = move |_| {
        verifying.set(true);
        verify_result.set(None);
        verify_error.set(None);
        leptos::task::spawn_local(async move {
            match api::invoke_no_args::<ChainVerificationResult>("verify_audit_chain").await {
                Ok(result) => verify_result.set(Some(result)),
                Err(e) => verify_error.set(Some(e)),
            }
            verifying.set(false);
        });
    };

    view! {
        <div class="page">
            <h1>"Revisionslogg"</h1>

            {move || if is_admin_or_revisor() {
                view! {
                    <section class="settings-section">
                        <h2>"Hashkedjeverifiering"</h2>
                        <p class="section-desc">"Verifiera att revisionsloggens SHA-256-kedja är intakt. Alla händelser kontrolleras."</p>

                        {move || verify_error.get().map(|msg| view! {
                            <div class="error-banner">{msg}</div>
                        })}

                        {move || verify_result.get().map(|r| {
                            if r.valid {
                                view! {
                                    <div class="success-banner">
                                        {format!("Kedjan är intakt. {} händelser verifierade.", r.total_events)}
                                    </div>
                                }.into_any()
                            } else {
                                view! {
                                    <div class="error-banner">
                                        {format!("Kedjan är bruten! {} händelser kontrollerade. Fel: {}",
                                            r.total_events,
                                            r.error.unwrap_or_default()
                                        )}
                                    </div>
                                }.into_any()
                            }
                        })}

                        <button
                            class="btn btn-primary"
                            on:click=do_verify
                            disabled=move || verifying.get()
                        >
                            {move || if verifying.get() { "Verifierar..." } else { "Verifiera hashkedja" }}
                        </button>
                    </section>
                }.into_any()
            } else {
                view! { <div></div> }.into_any()
            }}

            <section class="settings-section">
                <h2>"Händelselogg"</h2>

                <button class="btn btn-sm btn-secondary" style="margin-bottom: 1rem"
                    on:click=move |_| refresh.update(|c| *c += 1)
                >
                    "Uppdatera"
                </button>

                <Suspense fallback=|| view! { <p>"Laddar..."</p> }>
                    {move || events.get().map(|evts| {
                        if evts.is_empty() {
                            return view! { <p>"Inga händelser registrerade."</p> }.into_any();
                        }
                        view! {
                            <table class="data-table">
                                <thead>
                                    <tr>
                                        <th>"#"</th>
                                        <th>"Tid"</th>
                                        <th>"Typ"</th>
                                        <th>"Entitet"</th>
                                        <th>"Användare"</th>
                                        <th>"Detaljer"</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {evts.iter().map(|evt| {
                                        let detail_short = if evt.detail_json.len() > 60 {
                                            format!("{}...", &evt.detail_json[..60])
                                        } else {
                                            evt.detail_json.clone()
                                        };
                                        view! {
                                            <tr>
                                                <td>{evt.sequence_number}</td>
                                                <td>{evt.occurred_at.clone()}</td>
                                                <td><span class="status-badge">{evt.event_type.clone()}</span></td>
                                                <td>{format!("{}:{}", evt.entity_type, &evt.entity_id[..8.min(evt.entity_id.len())])}</td>
                                                <td>{evt.user_id[..8.min(evt.user_id.len())].to_string()}</td>
                                                <td class="detail-cell" title={evt.detail_json.clone()}>{detail_short}</td>
                                            </tr>
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
