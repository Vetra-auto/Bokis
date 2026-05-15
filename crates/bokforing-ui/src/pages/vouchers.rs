use leptos::prelude::*;
use serde::Deserialize;
use crate::api;
use crate::state::AppContext;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VoucherDto {
    id: String,
    voucher_series: String,
    voucher_number: i64,
    voucher_date: String,
    description: String,
    status: String,
    #[allow(dead_code)]
    created_at: String,
    posted_at: Option<String>,
}

fn format_ore(ore: i64) -> String {
    if ore == 0 { return "0,00".to_string(); }
    let kr = ore / 100;
    let rest = (ore % 100).abs();
    format!("{},{:02}", kr, rest)
}

fn format_bytes_short(bytes: i64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VoucherLineDetailDto {
    account_id: String,
    debit_amount: i64,
    credit_amount: i64,
    description: Option<String>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CorrectionLinkDto {
    #[allow(dead_code)]
    id: String,
    #[allow(dead_code)]
    original_voucher_id: String,
    reversal_voucher_id: String,
    #[allow(dead_code)]
    replacement_voucher_id: Option<String>,
    reason: String,
    created_at: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VoucherWithLinesDto {
    #[allow(dead_code)]
    id: String,
    voucher_series: String,
    voucher_number: i64,
    voucher_date: String,
    description: String,
    status: String,
    #[allow(dead_code)]
    created_at: String,
    #[allow(dead_code)]
    posted_at: Option<String>,
    lines: Vec<VoucherLineDetailDto>,
    corrections: Vec<CorrectionLinkDto>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct CorrectionResultDto {
    correction_link_id: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AttachmentDto {
    #[allow(dead_code)]
    id: String,
    #[allow(dead_code)]
    voucher_id: String,
    filename: String,
    #[allow(dead_code)]
    mime_type: String,
    file_size: i64,
    uploaded_at: String,
}

#[component]
pub fn VouchersPage() -> impl IntoView {
    let ctx = expect_context::<AppContext>();
    let refresh = RwSignal::new(0u32);
    let selected_voucher = RwSignal::new(Option::<String>::None);
    let correction_mode = RwSignal::new(false);
    let correction_reason = RwSignal::new(String::new());
    let correction_date = RwSignal::new(String::new());
    let correction_error = RwSignal::new(Option::<String>::None);
    let correction_success = RwSignal::new(Option::<String>::None);
    let correcting = RwSignal::new(false);

    let vouchers = leptos::prelude::LocalResource::new(move || {
        let _ = refresh.get();
        let fy = ctx.active_fiscal_year.get();
        async move {
            match fy {
                Some(fy) => {
                    let args = serde_json::json!({"fiscalYearId": fy.id});
                    api::invoke::<_, Vec<VoucherDto>>("list_vouchers", &args).await.unwrap_or_default()
                }
                None => vec![],
            }
        }
    });

    let voucher_detail = leptos::prelude::LocalResource::new(move || {
        let vid = selected_voucher.get();
        async move {
            match vid {
                Some(id) => {
                    let args = serde_json::json!({"voucherId": id});
                    api::invoke::<_, VoucherWithLinesDto>("get_voucher_details", &args).await.ok()
                }
                None => None,
            }
        }
    });

    let attachments = leptos::prelude::LocalResource::new(move || {
        let vid = selected_voucher.get();
        let _ = refresh.get();
        async move {
            match vid {
                Some(id) => {
                    let args = serde_json::json!({"voucherId": id});
                    api::invoke::<_, Vec<AttachmentDto>>("list_attachments", &args).await.unwrap_or_default()
                }
                None => vec![],
            }
        }
    });

    let attach_path = RwSignal::new(String::new());
    let attach_error = RwSignal::new(Option::<String>::None);

    let start_correction = move |voucher_id: String| {
        selected_voucher.set(Some(voucher_id));
        correction_mode.set(true);
        correction_reason.set(String::new());
        correction_date.set(String::new());
        correction_error.set(None);
        correction_success.set(None);
    };

    let do_correct = move |ev: web_sys::SubmitEvent| {
        ev.prevent_default();
        let vid = match selected_voucher.get_untracked() {
            Some(v) => v,
            None => return,
        };
        let reason = correction_reason.get_untracked();
        let date = correction_date.get_untracked();

        if reason.is_empty() || date.is_empty() {
            correction_error.set(Some("Fyll i alla fält".into()));
            return;
        }

        correcting.set(true);
        correction_error.set(None);
        leptos::task::spawn_local(async move {
            let args = serde_json::json!({
                "request": {
                    "voucherId": vid,
                    "reason": reason,
                    "correctionDate": date
                }
            });
            match api::invoke::<_, serde_json::Value>("correct_voucher", &args).await {
                Ok(_) => {
                    correction_success.set(Some("Rättelse genomförd! En reverseringsverifikation har skapats.".into()));
                    correction_mode.set(false);
                    refresh.update(|c| *c += 1);
                }
                Err(e) => correction_error.set(Some(e)),
            }
            correcting.set(false);
        });
    };

    let close_detail = move |_| {
        selected_voucher.set(None);
        correction_mode.set(false);
        correction_success.set(None);
    };

    view! {
        <div class="page">
            <h1>"Verifikationer"</h1>

            {move || correction_success.get().map(|msg| view! {
                <div class="success-banner">{msg}</div>
            })}

            // Detail/correction panel
            {move || selected_voucher.get().map(|_| {
                view! {
                    <div class="detail-panel">
                        <div class="detail-header">
                            <h2>"Verifikationsdetaljer"</h2>
                            <button class="btn btn-sm btn-secondary" on:click=close_detail>"Stäng"</button>
                        </div>

                        <Suspense fallback=|| view! { <p>"Laddar..."</p> }>
                            {move || voucher_detail.get().map(|opt| {
                                match opt.as_ref() {
                                    Some(v) => {
                                        let total_debit: i64 = v.lines.iter().map(|l| l.debit_amount).sum();
                                        let total_credit: i64 = v.lines.iter().map(|l| l.credit_amount).sum();
                                        let lines = v.lines.clone();
                                        let corrections = v.corrections.clone();
                                        let is_posted = v.status == "posted";
                                        let has_corrections = !corrections.is_empty();
                                        view! {
                                            <div>
                                                <p><strong>{format!("{}{}", v.voucher_series, v.voucher_number)}</strong>
                                                   " — " {v.voucher_date.clone()} " — " {v.description.clone()}
                                                   " "
                                                   <span class={format!("status-badge status-{}", if v.status == "posted" { "open" } else { "review" })}>{v.status.clone()}</span>
                                                </p>

                                                <table class="data-table" style="margin: 1rem 0">
                                                    <thead>
                                                        <tr>
                                                            <th>"Konto"</th>
                                                            <th class="amount">"Debet"</th>
                                                            <th class="amount">"Kredit"</th>
                                                            <th>"Beskrivning"</th>
                                                        </tr>
                                                    </thead>
                                                    <tbody>
                                                        {lines.iter().map(|l| view! {
                                                            <tr>
                                                                <td>{l.account_id.clone()}</td>
                                                                <td class="amount">{format_ore(l.debit_amount)}</td>
                                                                <td class="amount">{format_ore(l.credit_amount)}</td>
                                                                <td>{l.description.clone().unwrap_or_default()}</td>
                                                            </tr>
                                                        }).collect_view()}
                                                        <tr class="total-row">
                                                            <td><strong>"Summa"</strong></td>
                                                            <td class="amount"><strong>{format_ore(total_debit)}</strong></td>
                                                            <td class="amount"><strong>{format_ore(total_credit)}</strong></td>
                                                            <td></td>
                                                        </tr>
                                                    </tbody>
                                                </table>

                                                {if has_corrections {
                                                    view! {
                                                        <div class="corrections-info">
                                                            <h3>"Rättelser"</h3>
                                                            {corrections.iter().map(|c| view! {
                                                                <p>"Reversering: " {c.reversal_voucher_id[..8].to_string()} "... — "
                                                                   {c.reason.clone()} " (" {c.created_at.clone()} ")"</p>
                                                            }).collect_view()}
                                                        </div>
                                                    }.into_any()
                                                } else {
                                                    view! { <div></div> }.into_any()
                                                }}

                                                {if is_posted && !has_corrections {
                                                    let vid = v.id.clone();
                                                    Some(view! {
                                                        <button class="btn btn-primary" style="margin-top: 0.5rem"
                                                            on:click=move |_| start_correction(vid.clone())
                                                        >"Rätta denna verifikation"</button>
                                                    })
                                                } else {
                                                    None
                                                }}
                                            </div>
                                        }.into_any()
                                    }
                                    None => view! { <p>"Kunde inte hämta detaljer"</p> }.into_any(),
                                }
                            })}
                        </Suspense>

                        // Attachments section
                        <div class="attachments-section" style="margin-top: 1rem; padding-top: 1rem; border-top: 1px solid var(--border, #dee2e6)">
                            <h3>"Bilagor"</h3>

                            {move || attach_error.get().map(|msg| view! {
                                <div class="error-banner">{msg}</div>
                            })}

                            <Suspense fallback=|| view! { <p>"Laddar..."</p> }>
                                {move || attachments.get().map(|atts| {
                                    if atts.is_empty() {
                                        return view! { <p class="text-muted">"Inga bilagor"</p> }.into_any();
                                    }
                                    view! {
                                        <ul class="attachment-list">
                                            {atts.iter().map(|a| view! {
                                                <li>{a.filename.clone()} " (" {format_bytes_short(a.file_size)} ", " {a.uploaded_at.clone()} ")"</li>
                                            }).collect_view()}
                                        </ul>
                                    }.into_any()
                                })}
                            </Suspense>

                            <form class="inline-form" style="margin-top: 0.5rem" on:submit=move |ev: web_sys::SubmitEvent| {
                                ev.prevent_default();
                                let path = attach_path.get_untracked();
                                let vid = selected_voucher.get_untracked();
                                if path.is_empty() || vid.is_none() { return; }
                                attach_error.set(None);
                                let vid = vid.unwrap();
                                leptos::task::spawn_local(async move {
                                    let args = serde_json::json!({"voucherId": vid, "filePath": path});
                                    match api::invoke::<_, serde_json::Value>("upload_attachment", &args).await {
                                        Ok(_) => {
                                            attach_path.set(String::new());
                                            refresh.update(|c| *c += 1);
                                        }
                                        Err(e) => attach_error.set(Some(e)),
                                    }
                                });
                            }>
                                <div class="form-row">
                                    <div class="form-group" style="flex: 1">
                                        <input
                                            type="text"
                                            placeholder="Sökväg till fil..."
                                            prop:value=move || attach_path.get()
                                            on:input=move |ev| attach_path.set(event_target_value(&ev))
                                        />
                                    </div>
                                    <div class="form-group form-group-btn">
                                        <button type="submit" class="btn btn-sm">"Ladda upp"</button>
                                    </div>
                                </div>
                            </form>
                        </div>

                        // Correction form
                        {move || if correction_mode.get() {
                            view! {
                                <div class="correction-form">
                                    <h3>"Rätta verifikation"</h3>
                                    <p class="section-desc">"En reverseringsverifikation skapas automatiskt med omvända belopp."</p>

                                    {move || correction_error.get().map(|msg| view! {
                                        <div class="error-banner">{msg}</div>
                                    })}

                                    <form on:submit=do_correct>
                                        <div class="form-group">
                                            <label>"Anledning till rättelse"</label>
                                            <input
                                                type="text"
                                                placeholder="t.ex. Fel belopp, fel konto..."
                                                prop:value=move || correction_reason.get()
                                                on:input=move |ev| correction_reason.set(event_target_value(&ev))
                                                required=true
                                            />
                                        </div>
                                        <div class="form-group">
                                            <label>"Datum för rättelse"</label>
                                            <input
                                                type="date"
                                                prop:value=move || correction_date.get()
                                                on:change=move |ev| correction_date.set(event_target_value(&ev))
                                                required=true
                                            />
                                        </div>
                                        <div class="form-actions">
                                            <button type="submit" class="btn btn-primary"
                                                disabled=move || correcting.get()
                                            >
                                                {move || if correcting.get() { "Rättar..." } else { "Genomför rättelse" }}
                                            </button>
                                            <button type="button" class="btn btn-secondary"
                                                on:click=move |_| correction_mode.set(false)
                                            >"Avbryt"</button>
                                        </div>
                                    </form>
                                </div>
                            }.into_any()
                        } else {
                            view! { <div></div> }.into_any()
                        }}
                    </div>
                }
            })}

            // Voucher list
            <Suspense fallback=|| view! { <p>"Laddar..."</p> }>
                {move || vouchers.get().map(|vs| {
                    if vs.is_empty() {
                        return view! { <p>"Inga verifikationer"</p> }.into_any();
                    }
                    view! {
                        <table class="data-table">
                            <thead>
                                <tr>
                                    <th>"Ver"</th>
                                    <th>"Datum"</th>
                                    <th>"Beskrivning"</th>
                                    <th>"Status"</th>
                                    <th>"Bokförd"</th>
                                    <th>"Åtgärd"</th>
                                </tr>
                            </thead>
                            <tbody>
                                {vs.iter().map(|v| {
                                    let vid = v.id.clone();
                                    let vid2 = v.id.clone();
                                    view! {
                                        <tr>
                                            <td>{format!("{}{}", v.voucher_series, v.voucher_number)}</td>
                                            <td>{v.voucher_date.clone()}</td>
                                            <td>{v.description.clone()}</td>
                                            <td>
                                                <span class={format!("status-badge status-{}", match v.status.as_str() {
                                                    "posted" => "open",
                                                    "draft" => "review",
                                                    _ => "",
                                                })}>{v.status.clone()}</span>
                                            </td>
                                            <td>{v.posted_at.clone().unwrap_or_default()}</td>
                                            <td>
                                                <button class="btn btn-sm"
                                                    on:click=move |_| selected_voucher.set(Some(vid.clone()))
                                                >"Visa"</button>
                                                {if v.status == "posted" {
                                                    Some(view! {
                                                        <button class="btn btn-sm btn-secondary" style="margin-left: 0.25rem"
                                                            on:click=move |_| start_correction(vid2.clone())
                                                        >"Rätta"</button>
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
        </div>
    }
}
