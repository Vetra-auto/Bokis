use leptos::prelude::*;
use serde::Deserialize;
use crate::api;
use crate::state::AppContext;

#[derive(Clone, Deserialize)]
#[allow(dead_code)]
struct PeriodDto {
    id: String,
    year_month: String,
    start_date: String,
    end_date: String,
    status: String,
    closed_at: Option<String>,
    locked_at: Option<String>,
}

#[component]
pub fn PeriodsPage() -> impl IntoView {
    let ctx = expect_context::<AppContext>();
    let refresh_counter = RwSignal::new(0u32);

    let periods = leptos::prelude::LocalResource::new(move || {
        let fy = ctx.active_fiscal_year.get();
        let _ = refresh_counter.get();
        async move {
            match fy {
                Some(fy) => {
                    let args = serde_json::json!({"fiscalYearId": fy.id});
                    api::invoke::<_, Vec<PeriodDto>>("list_periods", &args).await.unwrap_or_default()
                }
                None => vec![],
            }
        }
    });

    let transition = move |period_id: String, new_status: String| {
        leptos::task::spawn_local(async move {
            let args = serde_json::json!({
                "periodId": period_id,
                "newStatus": new_status
            });
            match api::invoke::<_, PeriodDto>("transition_period", &args).await {
                Ok(_) => refresh_counter.update(|c| *c += 1),
                Err(e) => web_sys::console::error_1(&e.into()),
            }
        });
    };

    view! {
        <div class="page">
            <h1>"Periodhantering"</h1>

            <Suspense fallback=|| view! { <p>"Laddar..."</p> }>
                {move || periods.get().map(|ps| {
                    let rows: Vec<_> = ps.iter().map(|p| {
                        let status_class = match p.status.as_str() {
                            "open" => "status-open",
                            "under_review" => "status-review",
                            "closed" => "status-closed",
                            "locked" => "status-locked",
                            _ => "",
                        };
                        let status_label = match p.status.as_str() {
                            "open" => "Öppen".to_string(),
                            "under_review" => "Under avstämning".to_string(),
                            "closed" => "Stängd".to_string(),
                            "locked" => "Låst".to_string(),
                            _ => p.status.clone(),
                        };

                        let pid = p.id.clone();
                        let pid2 = p.id.clone();
                        let pid3 = p.id.clone();
                        let status = p.status.clone();
                        let year_month = p.year_month.clone();
                        let start_date = p.start_date.clone();
                        let end_date = p.end_date.clone();
                        let transition = transition.clone();

                        view! {
                            <tr>
                                <td><strong>{year_month}</strong></td>
                                <td>{start_date}</td>
                                <td>{end_date}</td>
                                <td><span class={format!("status-badge {}", status_class)}>{status_label}</span></td>
                                <td>
                                    {match status.as_str() {
                                        "open" => view! {
                                            <button class="btn btn-sm" on:click=move |_| {
                                                transition(pid.clone(), "under_review".to_string());
                                            }>"Påbörja avstämning"</button>
                                        }.into_any(),
                                        "under_review" => view! {
                                            <button class="btn btn-sm" on:click=move |_| {
                                                transition(pid2.clone(), "closed".to_string());
                                            }>"Stäng period"</button>
                                        }.into_any(),
                                        "closed" => view! {
                                            <button class="btn btn-sm btn-danger" on:click=move |_| {
                                                transition(pid3.clone(), "locked".to_string());
                                            }>"Lås permanent"</button>
                                        }.into_any(),
                                        _ => view! { <span></span> }.into_any(),
                                    }}
                                </td>
                            </tr>
                        }
                    }).collect();

                    view! {
                        <table class="data-table">
                            <thead>
                                <tr>
                                    <th>"Period"</th>
                                    <th>"Startdatum"</th>
                                    <th>"Slutdatum"</th>
                                    <th>"Status"</th>
                                    <th>"Åtgärd"</th>
                                </tr>
                            </thead>
                            <tbody>
                                {rows}
                            </tbody>
                        </table>
                    }
                })}
            </Suspense>
        </div>
    }
}
