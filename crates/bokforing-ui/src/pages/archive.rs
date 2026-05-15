use leptos::prelude::*;
use serde::Deserialize;
use crate::api;
use crate::state::AppContext;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArchiveManifestDto {
    #[allow(dead_code)]
    id: String,
    fiscal_year_id: String,
    created_at: String,
    archive_path: String,
    #[allow(dead_code)]
    content_hash: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SystemDocDto {
    #[allow(dead_code)]
    id: String,
    generated_at: String,
    content_hash: String,
}

#[component]
pub fn ArchivePage() -> impl IntoView {
    let ctx = expect_context::<AppContext>();
    let refresh = RwSignal::new(0u32);
    let archiving = RwSignal::new(false);
    let generating_doc = RwSignal::new(false);
    let error_msg = RwSignal::new(Option::<String>::None);
    let success_msg = RwSignal::new(Option::<String>::None);

    let archives = leptos::prelude::LocalResource::new(move || {
        let _ = refresh.get();
        async move {
            api::invoke_no_args::<Vec<ArchiveManifestDto>>("list_archives").await.unwrap_or_default()
        }
    });

    let system_docs = leptos::prelude::LocalResource::new(move || {
        let _ = refresh.get();
        async move {
            api::invoke_no_args::<Vec<SystemDocDto>>("list_system_docs").await.unwrap_or_default()
        }
    });

    let do_archive = move |_| {
        let fy = match ctx.active_fiscal_year.get_untracked() {
            Some(fy) => fy,
            None => {
                error_msg.set(Some("Inget räkenskapsår valt".into()));
                return;
            }
        };
        archiving.set(true);
        error_msg.set(None);
        success_msg.set(None);
        leptos::task::spawn_local(async move {
            let args = serde_json::json!({"fiscalYearId": fy.id});
            match api::invoke::<_, ArchiveManifestDto>("create_archive", &args).await {
                Ok(a) => {
                    success_msg.set(Some(format!("Arkiv skapat: {}", a.archive_path)));
                    refresh.update(|c| *c += 1);
                }
                Err(e) => error_msg.set(Some(e)),
            }
            archiving.set(false);
        });
    };

    let do_generate_doc = move |_| {
        generating_doc.set(true);
        error_msg.set(None);
        success_msg.set(None);
        leptos::task::spawn_local(async move {
            match api::invoke_no_args::<SystemDocDto>("generate_system_doc").await {
                Ok(doc) => {
                    success_msg.set(Some(format!("Systemdokumentation genererad (hash: {}...)", &doc.content_hash[..16])));
                    refresh.update(|c| *c += 1);
                }
                Err(e) => error_msg.set(Some(e)),
            }
            generating_doc.set(false);
        });
    };

    view! {
        <div class="page">
            <h1>"Arkiv & Dokumentation"</h1>

            {move || error_msg.get().map(|msg| view! {
                <div class="error-banner">{msg}</div>
            })}

            {move || success_msg.get().map(|msg| view! {
                <div class="success-banner">{msg}</div>
            })}

            <section class="settings-section">
                <h2>"Årsarkivering"</h2>
                <p class="section-desc">"Skapar ett arkiv av ett stängt räkenskapsår. Arkivet innehåller en komplett databaskopia och bevaras i 7 år enligt BFL 7 kap. 2§."</p>

                {move || ctx.active_fiscal_year.get().map(|fy| view! {
                    <p>"Aktivt räkenskapsår: " <strong>{fy.name}</strong></p>
                })}

                <button
                    class="btn btn-primary"
                    style="margin-top: 0.5rem"
                    on:click=do_archive
                    disabled=move || archiving.get()
                >
                    {move || if archiving.get() { "Arkiverar..." } else { "Arkivera räkenskapsår" }}
                </button>

                <Suspense fallback=|| view! { <p>"Laddar..."</p> }>
                    {move || archives.get().map(|list| {
                        if list.is_empty() {
                            return view! { <p class="text-muted" style="margin-top: 1rem">"Inga arkiv skapade."</p> }.into_any();
                        }
                        view! {
                            <table class="data-table" style="margin-top: 1rem">
                                <thead>
                                    <tr>
                                        <th>"Räkenskapsår"</th>
                                        <th>"Datum"</th>
                                        <th>"Sökväg"</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {list.iter().map(|a| view! {
                                        <tr>
                                            <td>{a.fiscal_year_id[..8].to_string()}"..."</td>
                                            <td>{a.created_at.clone()}</td>
                                            <td class="path-cell">{a.archive_path.clone()}</td>
                                        </tr>
                                    }).collect_view()}
                                </tbody>
                            </table>
                        }.into_any()
                    })}
                </Suspense>
            </section>

            <section class="settings-section">
                <h2>"Systemdokumentation"</h2>
                <p class="section-desc">"Genererar en snapshot av systemets konfiguration enligt BFNAR 2013:2. Dokumentationen innehåller företagsinfo, kontoplan, användare och arkiveringsstatus."</p>

                <button
                    class="btn btn-primary"
                    on:click=do_generate_doc
                    disabled=move || generating_doc.get()
                >
                    {move || if generating_doc.get() { "Genererar..." } else { "Generera systemdokumentation" }}
                </button>

                <Suspense fallback=|| view! { <p>"Laddar..."</p> }>
                    {move || system_docs.get().map(|docs| {
                        if docs.is_empty() {
                            return view! { <p class="text-muted" style="margin-top: 1rem">"Ingen systemdokumentation genererad."</p> }.into_any();
                        }
                        view! {
                            <table class="data-table" style="margin-top: 1rem">
                                <thead>
                                    <tr>
                                        <th>"Datum"</th>
                                        <th>"Innehålls-hash"</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {docs.iter().map(|d| view! {
                                        <tr>
                                            <td>{d.generated_at.clone()}</td>
                                            <td class="path-cell">{d.content_hash[..32].to_string()}"..."</td>
                                        </tr>
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
