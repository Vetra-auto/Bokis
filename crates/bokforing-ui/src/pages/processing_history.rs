use leptos::prelude::*;
use serde::Deserialize;
use crate::api;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProcessingHistoryDto {
    #[allow(dead_code)]
    id: String,
    event_type: String,
    entity_type: String,
    entity_id: String,
    user_id: String,
    occurred_at: String,
    description: String,
    #[allow(dead_code)]
    metadata_json: Option<String>,
}

#[component]
pub fn ProcessingHistoryPage() -> impl IntoView {
    let events = leptos::prelude::LocalResource::new(move || {
        async move {
            let args = serde_json::json!({"limit": 200, "offset": 0});
            api::invoke::<_, Vec<ProcessingHistoryDto>>("list_processing_history", &args)
                .await
                .unwrap_or_default()
        }
    });

    view! {
        <div class="page">
            <h1>"Behandlingshistorik"</h1>
            <p class="section-desc">"Visar en logg över alla behandlingsåtgärder som utförts på bokföringsmaterial — bokföring, rättelser, periodstängningar m.m."</p>

            <Suspense fallback=|| view! { <p>"Laddar..."</p> }>
                {move || events.get().map(|list| {
                    if list.is_empty() {
                        return view! { <p class="text-muted">"Ingen behandlingshistorik registrerad."</p> }.into_any();
                    }
                    view! {
                        <table class="data-table">
                            <thead>
                                <tr>
                                    <th>"Tidpunkt"</th>
                                    <th>"Händelsetyp"</th>
                                    <th>"Entitetstyp"</th>
                                    <th>"Entitet"</th>
                                    <th>"Beskrivning"</th>
                                    <th>"Användare"</th>
                                </tr>
                            </thead>
                            <tbody>
                                {list.iter().map(|e| view! {
                                    <tr>
                                        <td>{e.occurred_at.clone()}</td>
                                        <td><span class="badge">{e.event_type.clone()}</span></td>
                                        <td>{e.entity_type.clone()}</td>
                                        <td class="path-cell">{e.entity_id[..8.min(e.entity_id.len())].to_string()}"..."</td>
                                        <td>{e.description.clone()}</td>
                                        <td>{e.user_id[..8.min(e.user_id.len())].to_string()}"..."</td>
                                    </tr>
                                }).collect_view()}
                            </tbody>
                        </table>
                    }.into_any()
                })}
            </Suspense>
        </div>
    }
}
