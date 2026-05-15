use leptos::prelude::*;
use serde::Deserialize;
use crate::api;
use crate::state::AppContext;

#[derive(Clone, Deserialize)]
struct JournalEntryDto {
    voucher_series: String,
    voucher_number: i64,
    voucher_date: String,
    voucher_description: String,
    line_number: i64,
    account_number: i64,
    account_name: String,
    debit_amount: i64,
    credit_amount: i64,
}

fn format_ore(ore: i64) -> String {
    if ore == 0 { return String::new(); }
    let kr = ore / 100;
    let rest = (ore % 100).abs();
    format!("{},{:02}", kr, rest)
}

#[component]
pub fn JournalPage() -> impl IntoView {
    let ctx = expect_context::<AppContext>();

    let entries = leptos::prelude::LocalResource::new(move || {
        let fy = ctx.active_fiscal_year.get();
        async move {
            match fy {
                Some(fy) => {
                    let args = serde_json::json!({
                        "fiscalYearId": fy.id,
                        "fromDate": serde_json::Value::Null,
                        "toDate": serde_json::Value::Null
                    });
                    api::invoke::<_, Vec<JournalEntryDto>>("get_journal", &args).await.unwrap_or_default()
                }
                None => vec![],
            }
        }
    });

    view! {
        <div class="page">
            <h1>"Grundbok"</h1>
            <p class="page-description">"Alla bokförda verifikationer i registreringsordning"</p>

            <Suspense fallback=|| view! { <p>"Laddar..."</p> }>
                {move || entries.get().map(|rows| {
                    if rows.is_empty() {
                        return view! { <p class="empty">"Inga bokförda verifikationer"</p> }.into_any();
                    }
                    view! {
                        <table class="data-table">
                            <thead>
                                <tr>
                                    <th>"Ver"</th>
                                    <th>"Datum"</th>
                                    <th>"Konto"</th>
                                    <th>"Beskrivning"</th>
                                    <th class="amount">"Debet"</th>
                                    <th class="amount">"Kredit"</th>
                                </tr>
                            </thead>
                            <tbody>
                                {rows.iter().map(|e| {
                                    let is_first_line = e.line_number == 1;
                                    view! {
                                        <tr class:voucher-first-line=is_first_line>
                                            <td>{if is_first_line { format!("{}{}", e.voucher_series, e.voucher_number) } else { String::new() }}</td>
                                            <td>{if is_first_line { e.voucher_date.clone() } else { String::new() }}</td>
                                            <td>{format!("{} {}", e.account_number, e.account_name)}</td>
                                            <td>{if is_first_line { e.voucher_description.clone() } else { String::new() }}</td>
                                            <td class="amount">{format_ore(e.debit_amount)}</td>
                                            <td class="amount">{format_ore(e.credit_amount)}</td>
                                        </tr>
                                    }
                                }).collect_view()}
                            </tbody>
                        </table>
                    }.into_any()
                })}
            </Suspense>
        </div>
    }
}
