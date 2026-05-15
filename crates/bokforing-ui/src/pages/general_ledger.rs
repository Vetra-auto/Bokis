use leptos::prelude::*;
use serde::Deserialize;
use crate::api;
use crate::state::AppContext;

#[derive(Clone, Deserialize)]
struct GeneralLedgerEntryDto {
    account_number: i64,
    account_name: String,
    voucher_date: String,
    voucher_series: String,
    voucher_number: i64,
    description: String,
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
pub fn GeneralLedgerPage() -> impl IntoView {
    let ctx = expect_context::<AppContext>();
    let selected_account = RwSignal::new(Option::<i64>::None);

    let entries = leptos::prelude::LocalResource::new(move || {
        let fy = ctx.active_fiscal_year.get();
        let acc = selected_account.get();
        async move {
            match fy {
                Some(fy) => {
                    let args = serde_json::json!({
                        "fiscalYearId": fy.id,
                        "accountNumber": acc
                    });
                    api::invoke::<_, Vec<GeneralLedgerEntryDto>>("get_general_ledger", &args).await.unwrap_or_default()
                }
                None => vec![],
            }
        }
    });

    view! {
        <div class="page">
            <h1>"Huvudbok"</h1>
            <p class="page-description">"Poster per konto i systematisk ordning"</p>

            <Suspense fallback=|| view! { <p>"Laddar..."</p> }>
                {move || entries.get().map(|rows| {
                    if rows.is_empty() {
                        return view! { <p class="empty">"Inga poster"</p> }.into_any();
                    }

                    let mut current_account: Option<i64> = None;
                    let mut running_balance: i64 = 0;

                    let rendered_rows: Vec<_> = rows.iter().map(|e| {
                        let show_header = current_account != Some(e.account_number);
                        if show_header {
                            current_account = Some(e.account_number);
                            running_balance = 0;
                        }
                        running_balance += e.debit_amount - e.credit_amount;
                        let bal = running_balance;

                        view! {
                            <>
                                {if show_header {
                                    Some(view! {
                                        <tr class="account-header">
                                            <td colspan="6">
                                                <strong>{format!("{} {}", e.account_number, e.account_name)}</strong>
                                            </td>
                                        </tr>
                                    })
                                } else { None }}
                                <tr>
                                    <td>{e.voucher_date.clone()}</td>
                                    <td>{format!("{}{}", e.voucher_series, e.voucher_number)}</td>
                                    <td>{e.description.clone()}</td>
                                    <td class="amount">{format_ore(e.debit_amount)}</td>
                                    <td class="amount">{format_ore(e.credit_amount)}</td>
                                    <td class="amount">{format_ore(bal)}</td>
                                </tr>
                            </>
                        }
                    }).collect();

                    view! {
                        <table class="data-table">
                            <thead>
                                <tr>
                                    <th>"Datum"</th>
                                    <th>"Ver"</th>
                                    <th>"Beskrivning"</th>
                                    <th class="amount">"Debet"</th>
                                    <th class="amount">"Kredit"</th>
                                    <th class="amount">"Saldo"</th>
                                </tr>
                            </thead>
                            <tbody>
                                {rendered_rows}
                            </tbody>
                        </table>
                    }.into_any()
                })}
            </Suspense>
        </div>
    }
}
