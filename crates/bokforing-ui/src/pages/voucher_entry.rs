use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use crate::api;

#[derive(Clone, Deserialize)]
struct AccountDto {
    id: String,
    account_number: i64,
    name: String,
}

#[derive(Clone, Deserialize)]
#[allow(dead_code)]
struct VoucherDto {
    id: String,
    voucher_series: String,
    voucher_number: i64,
    voucher_date: String,
    description: String,
    status: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateLineReq {
    account_id: String,
    debit_amount: i64,
    credit_amount: i64,
    description: Option<String>,
}

#[derive(Clone)]
struct LineDraft {
    account_id: String,
    account_display: String,
    debit: String,
    credit: String,
    description: String,
}

fn parse_amount(s: &str) -> i64 {
    let s = s.trim().replace(' ', "").replace(',', ".");
    if s.is_empty() {
        return 0;
    }
    if let Ok(f) = s.parse::<f64>() {
        (f * 100.0).round() as i64
    } else {
        0
    }
}

fn format_ore(ore: i64) -> String {
    if ore == 0 {
        return String::new();
    }
    let kr = ore / 100;
    let rest = (ore % 100).abs();
    format!("{},{:02}", kr, rest)
}

#[component]
pub fn VoucherEntryPage() -> impl IntoView {
    let voucher_date = RwSignal::new(String::new());
    let description = RwSignal::new(String::new());
    let lines = RwSignal::new(vec![
        LineDraft { account_id: String::new(), account_display: String::new(), debit: String::new(), credit: String::new(), description: String::new() },
        LineDraft { account_id: String::new(), account_display: String::new(), debit: String::new(), credit: String::new(), description: String::new() },
    ]);
    let error_msg = RwSignal::new(Option::<String>::None);
    let success_msg = RwSignal::new(Option::<String>::None);
    let loading = RwSignal::new(false);

    let accounts = leptos::prelude::LocalResource::new(move || async move {
        api::invoke_no_args::<Vec<AccountDto>>("list_accounts").await.unwrap_or_default()
    });

    let total_debit = move || {
        lines.get().iter().map(|l| parse_amount(&l.debit)).sum::<i64>()
    };
    let total_credit = move || {
        lines.get().iter().map(|l| parse_amount(&l.credit)).sum::<i64>()
    };
    let is_balanced = move || total_debit() == total_credit() && total_debit() > 0;

    let add_line = move |_| {
        lines.update(|ls| {
            ls.push(LineDraft {
                account_id: String::new(),
                account_display: String::new(),
                debit: String::new(),
                credit: String::new(),
                description: String::new(),
            });
        });
    };

    let remove_line = move |idx: usize| {
        lines.update(|ls| {
            if ls.len() > 2 {
                ls.remove(idx);
            }
        });
    };

    let on_submit = move |ev: web_sys::SubmitEvent| {
        ev.prevent_default();
        leptos::task::spawn_local(async move {
            loading.set(true);
            error_msg.set(None);
            success_msg.set(None);

            let ls: Vec<CreateLineReq> = lines.get_untracked().iter().filter_map(|l| {
                if l.account_id.is_empty() { return None; }
                let d = parse_amount(&l.debit);
                let c = parse_amount(&l.credit);
                if d == 0 && c == 0 { return None; }
                Some(CreateLineReq {
                    account_id: l.account_id.clone(),
                    debit_amount: d,
                    credit_amount: c,
                    description: if l.description.is_empty() { None } else { Some(l.description.clone()) },
                })
            }).collect();

            let args = serde_json::json!({
                "request": {
                    "voucherDate": voucher_date.get_untracked(),
                    "description": description.get_untracked(),
                    "voucherSeries": "A",
                    "counterpartyId": null,
                    "lines": ls
                }
            });

            match api::invoke::<_, VoucherDto>("create_voucher", &args).await {
                Ok(v) => {
                    let post_args = serde_json::json!({"voucherId": v.id});
                    match api::invoke::<_, VoucherDto>("post_voucher", &post_args).await {
                        Ok(posted) => {
                            success_msg.set(Some(format!(
                                "Verifikation {}{} bokförd!",
                                posted.voucher_series, posted.voucher_number
                            )));
                            voucher_date.set(String::new());
                            description.set(String::new());
                            lines.set(vec![
                                LineDraft { account_id: String::new(), account_display: String::new(), debit: String::new(), credit: String::new(), description: String::new() },
                                LineDraft { account_id: String::new(), account_display: String::new(), debit: String::new(), credit: String::new(), description: String::new() },
                            ]);
                        }
                        Err(e) => error_msg.set(Some(format!("Skapad men ej bokförd: {}", e))),
                    }
                }
                Err(e) => error_msg.set(Some(e)),
            }
            loading.set(false);
        });
    };

    view! {
        <div class="page">
            <h1>"Ny verifikation"</h1>

            {move || error_msg.get().map(|msg| view! {
                <div class="error-banner">{msg}</div>
            })}
            {move || success_msg.get().map(|msg| view! {
                <div class="success-banner">{msg}</div>
            })}

            <form on:submit=on_submit>
                <div class="form-row">
                    <div class="form-group">
                        <label>"Datum"</label>
                        <input
                            type="date"
                            prop:value=move || voucher_date.get()
                            on:change=move |ev| voucher_date.set(event_target_value(&ev))
                            required=true
                        />
                    </div>
                    <div class="form-group" style="flex: 2">
                        <label>"Beskrivning"</label>
                        <input
                            type="text"
                            prop:value=move || description.get()
                            on:input=move |ev| description.set(event_target_value(&ev))
                            placeholder="T.ex. Köp kontorsmaterial"
                            required=true
                        />
                    </div>
                </div>

                <table class="voucher-lines-table">
                    <thead>
                        <tr>
                            <th>"Konto"</th>
                            <th>"Debet"</th>
                            <th>"Kredit"</th>
                            <th>"Text"</th>
                            <th></th>
                        </tr>
                    </thead>
                    <tbody>
                        <For
                            each=move || {
                                let ls = lines.get();
                                (0..ls.len()).collect::<Vec<_>>()
                            }
                            key=|idx| *idx
                            children=move |idx| {
                                let update_line = move |field: &str, value: String| {
                                    lines.update(|ls| {
                                        if let Some(l) = ls.get_mut(idx) {
                                            match field {
                                                "account" => {
                                                    l.account_id = value.clone();
                                                    l.account_display = value;
                                                },
                                                "debit" => l.debit = value,
                                                "credit" => l.credit = value,
                                                "description" => l.description = value,
                                                _ => {}
                                            }
                                        }
                                    });
                                };

                                view! {
                                    <tr>
                                        <td>
                                            <select
                                                on:change={
                                                    let update_line = update_line.clone();
                                                    move |ev| {
                                                        update_line("account", event_target_value(&ev));
                                                    }
                                                }
                                            >
                                                <option value="">"— Välj konto —"</option>
                                                <Suspense fallback=|| ()>
                                                    {move || accounts.get().map(|accs| {
                                                        accs.iter().map(|a| {
                                                            let val = a.id.clone();
                                                            let label = format!("{} {}", a.account_number, a.name);
                                                            view! { <option value=val>{label}</option> }
                                                        }).collect_view()
                                                    })}
                                                </Suspense>
                                            </select>
                                        </td>
                                        <td>
                                            <input
                                                type="text"
                                                class="amount-input"
                                                placeholder="0,00"
                                                on:input={
                                                    let update_line = update_line.clone();
                                                    move |ev| update_line("debit", event_target_value(&ev))
                                                }
                                            />
                                        </td>
                                        <td>
                                            <input
                                                type="text"
                                                class="amount-input"
                                                placeholder="0,00"
                                                on:input={
                                                    let update_line = update_line.clone();
                                                    move |ev| update_line("credit", event_target_value(&ev))
                                                }
                                            />
                                        </td>
                                        <td>
                                            <input
                                                type="text"
                                                on:input={
                                                    let update_line = update_line.clone();
                                                    move |ev| update_line("description", event_target_value(&ev))
                                                }
                                            />
                                        </td>
                                        <td>
                                            <button
                                                type="button"
                                                class="btn-icon"
                                                on:click=move |_| remove_line(idx)
                                            >"×"</button>
                                        </td>
                                    </tr>
                                }
                            }
                        />
                    </tbody>
                    <tfoot>
                        <tr class="totals-row">
                            <td>"Summa"</td>
                            <td class="amount">{move || format_ore(total_debit())}</td>
                            <td class="amount">{move || format_ore(total_credit())}</td>
                            <td colspan="2">
                                {move || if is_balanced() {
                                    view! { <span class="balance-ok">"Balanserar ✓"</span> }.into_any()
                                } else {
                                    let diff = total_debit() - total_credit();
                                    view! { <span class="balance-err">"Differens: " {format_ore(diff)}</span> }.into_any()
                                }}
                            </td>
                        </tr>
                    </tfoot>
                </table>

                <div class="form-actions">
                    <button type="button" class="btn btn-secondary" on:click=add_line>
                        "+ Lägg till rad"
                    </button>
                    <button
                        type="submit"
                        class="btn btn-primary"
                        disabled=move || !is_balanced() || loading.get()
                    >
                        {move || if loading.get() { "Bokför..." } else { "Bokför" }}
                    </button>
                </div>
            </form>
        </div>
    }
}
