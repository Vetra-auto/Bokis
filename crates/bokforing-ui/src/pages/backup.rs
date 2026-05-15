use leptos::prelude::*;
use serde::Deserialize;
use wasm_bindgen::prelude::*;
use crate::api;
use crate::state::AppContext;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI_INTERNALS__"], js_name = invoke, catch)]
    async fn tauri_invoke_raw(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackupJobDto {
    id: String,
    started_at: String,
    completed_at: Option<String>,
    status: String,
    backup_path: Option<String>,
    size_bytes: Option<i64>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackupVerifyResultDto {
    is_valid: bool,
    details: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RestoreResultDto {
    message: String,
}

#[component]
pub fn BackupPage() -> impl IntoView {
    let _ctx = expect_context::<AppContext>();
    let refresh = RwSignal::new(0u32);
    let creating = RwSignal::new(false);
    let backup_error = RwSignal::new(Option::<String>::None);
    let backup_success = RwSignal::new(Option::<String>::None);
    let verify_result = RwSignal::new(Option::<BackupVerifyResultDto>::None);
    let restore_confirm = RwSignal::new(Option::<String>::None);
    let restore_result = RwSignal::new(Option::<String>::None);

    let backups = leptos::prelude::LocalResource::new(move || {
        let _ = refresh.get();
        async move {
            api::invoke_no_args::<Vec<BackupJobDto>>("list_backups").await.unwrap_or_default()
        }
    });

    let create = move |_| {
        creating.set(true);
        backup_error.set(None);
        backup_success.set(None);
        verify_result.set(None);
        leptos::task::spawn_local(async move {
            match api::invoke_no_args::<BackupJobDto>("create_backup").await {
                Ok(job) => {
                    let path = job.backup_path.unwrap_or_default();
                    backup_success.set(Some(format!("Backup skapad: {}", path)));
                    refresh.update(|c| *c += 1);
                }
                Err(e) => backup_error.set(Some(e)),
            }
            creating.set(false);
        });
    };

    let do_verify = move |job_id: String| {
        backup_error.set(None);
        backup_success.set(None);
        verify_result.set(None);
        restore_result.set(None);
        leptos::task::spawn_local(async move {
            let args = serde_json::json!({"backupJobId": job_id});
            match api::invoke::<_, BackupVerifyResultDto>("verify_backup", &args).await {
                Ok(result) => verify_result.set(Some(result)),
                Err(e) => backup_error.set(Some(e)),
            }
        });
    };

    let do_restore = move |job_id: String| {
        backup_error.set(None);
        backup_success.set(None);
        verify_result.set(None);
        restore_result.set(None);
        restore_confirm.set(None);
        leptos::task::spawn_local(async move {
            let args = serde_json::json!({"backupJobId": job_id});
            match api::invoke::<_, RestoreResultDto>("restore_backup", &args).await {
                Ok(result) => restore_result.set(Some(result.message)),
                Err(e) => backup_error.set(Some(e)),
            }
        });
    };

    let file_restore_confirm = RwSignal::new(Option::<String>::None);

    let pick_file = move |_| {
        backup_error.set(None);
        backup_success.set(None);
        restore_result.set(None);
        file_restore_confirm.set(None);
        leptos::task::spawn_local(async move {
            let args = serde_json::json!({
                "options": {
                    "multiple": false,
                    "filters": [{
                        "name": "SQLite-databas",
                        "extensions": ["db", "sqlite", "sqlite3"]
                    }]
                }
            });
            let args_js = serde_wasm_bindgen::to_value(&args).unwrap();
            match tauri_invoke_raw("plugin:dialog|open", args_js).await {
                Ok(val) => {
                    if let Some(path) = val.as_string() {
                        file_restore_confirm.set(Some(path));
                    }
                }
                Err(e) => {
                    let msg: String = serde_wasm_bindgen::from_value(e)
                        .unwrap_or_else(|_| "Kunde inte öppna filväljare".into());
                    backup_error.set(Some(msg));
                }
            }
        });
    };

    let do_file_restore = move |path: String| {
        backup_error.set(None);
        file_restore_confirm.set(None);
        leptos::task::spawn_local(async move {
            let args = serde_json::json!({"filePath": path});
            match api::invoke::<_, RestoreResultDto>("restore_from_file", &args).await {
                Ok(result) => restore_result.set(Some(result.message)),
                Err(e) => backup_error.set(Some(e)),
            }
        });
    };

    view! {
        <div class="page">
            <h1>"Backup"</h1>

            <section class="settings-section">
                <h2>"Skapa backup"</h2>
                <p class="section-desc">"Skapar en fullständig kopia av databasen. Backupen sparas i appens datamapp."</p>

                {move || backup_error.get().map(|msg| view! {
                    <div class="error-banner">{msg}</div>
                })}

                {move || backup_success.get().map(|msg| view! {
                    <div class="success-banner">{msg}</div>
                })}

                {move || verify_result.get().map(|r| {
                    if r.is_valid {
                        view! { <div class="success-banner">{r.details}</div> }.into_any()
                    } else {
                        view! { <div class="error-banner">{r.details}</div> }.into_any()
                    }
                })}

                {move || restore_result.get().map(|msg| view! {
                    <div class="success-banner">
                        <strong>"⚠ "{msg}</strong>
                    </div>
                })}

                {move || restore_confirm.get().map(|job_id| {
                    let jid = job_id.clone();
                    let do_restore = do_restore.clone();
                    view! {
                        <div class="warning-banner">
                            <p><strong>"Varning:"</strong>" All nuvarande data ersätts med backupen. Åtgärden kan inte ångras."</p>
                            <div class="confirm-actions">
                                <button class="btn btn-danger"
                                    on:click=move |_| do_restore(jid.clone())
                                >"Ja, återställ"</button>
                                <button class="btn btn-secondary"
                                    on:click=move |_| restore_confirm.set(None)
                                >"Avbryt"</button>
                            </div>
                        </div>
                    }
                })}

                <button
                    class="btn btn-primary"
                    on:click=create
                    disabled=move || creating.get()
                >
                    {move || if creating.get() { "Skapar backup..." } else { "Skapa backup nu" }}
                </button>
            </section>

            <section class="settings-section">
                <h2>"Läs in backup"</h2>
                <p class="section-desc">"Välj en backup-fil (.db) för att återställa databasen. Nuvarande data ersätts."</p>

                {move || file_restore_confirm.get().map(|path| {
                    let p = path.clone();
                    let do_file_restore = do_file_restore.clone();
                    view! {
                        <div class="warning-banner">
                            <p><strong>"Varning:"</strong>" All nuvarande data ersätts med filen:"</p>
                            <p class="path-cell">{path.clone()}</p>
                            <div class="confirm-actions">
                                <button class="btn btn-danger"
                                    on:click=move |_| do_file_restore(p.clone())
                                >"Ja, läs in"</button>
                                <button class="btn btn-secondary"
                                    on:click=move |_| file_restore_confirm.set(None)
                                >"Avbryt"</button>
                            </div>
                        </div>
                    }
                })}

                <button class="btn btn-secondary" on:click=pick_file>
                    "Välj backup-fil..."
                </button>
            </section>

            <section class="settings-section">
                <h2>"Backup-historik"</h2>

                <Suspense fallback=|| view! { <p>"Laddar..."</p> }>
                    {move || backups.get().map(|jobs| {
                        if jobs.is_empty() {
                            return view! { <p>"Inga backuper har skapats ännu."</p> }.into_any();
                        }
                        let jobs_vec: Vec<_> = jobs.iter().cloned().collect();
                        view! {
                            <table class="data-table">
                                <thead>
                                    <tr>
                                        <th>"Datum"</th>
                                        <th>"Status"</th>
                                        <th>"Sökväg"</th>
                                        <th>"Storlek"</th>
                                        <th>"Åtgärd"</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {jobs_vec.into_iter().map(|job| {
                                        let size_str = job.size_bytes
                                            .map(|b| format_bytes(b))
                                            .unwrap_or_else(|| "\u{2014}".to_string());
                                        let path_str = job.backup_path.clone().unwrap_or_else(|| "\u{2014}".to_string());
                                        let is_completed = job.status == "completed";
                                        let job_id = job.id.clone();
                                        let job_id2 = job.id.clone();
                                        let do_verify = do_verify.clone();
                                        view! {
                                            <tr>
                                                <td>{job.started_at.clone()}</td>
                                                <td>
                                                    <span class={format!("status-badge status-{}", match job.status.as_str() {
                                                        "completed" => "open",
                                                        "failed" => "closed",
                                                        _ => "review",
                                                    })}>{job.status.clone()}</span>
                                                </td>
                                                <td class="path-cell">{path_str}</td>
                                                <td>{size_str}</td>
                                                <td>
                                                    {if is_completed {
                                                        let jid = job_id.clone();
                                                        let jid2 = job_id2.clone();
                                                        Some(view! {
                                                            <button class="btn btn-sm"
                                                                on:click=move |_| do_verify(jid.clone())
                                                            >"Verifiera"</button>
                                                            " "
                                                            <button class="btn btn-sm btn-warning"
                                                                on:click=move |_| restore_confirm.set(Some(jid2.clone()))
                                                            >"Återställ"</button>
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
                        }.into_any()
                    })}
                </Suspense>
            </section>
        </div>
    }
}

fn format_bytes(bytes: i64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}
