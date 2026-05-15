use leptos::prelude::*;
use crate::api;
use crate::state::AppContext;

#[component]
pub fn ExportPage() -> impl IntoView {
    let ctx = expect_context::<AppContext>();
    let exporting = RwSignal::new(false);
    let export_error = RwSignal::new(Option::<String>::None);
    let export_success = RwSignal::new(Option::<String>::None);
    let sie4_path = RwSignal::new(String::new());
    let csv_path = RwSignal::new(String::new());
    let json_path = RwSignal::new(String::new());
    let pdf_path = RwSignal::new(String::new());
    let pdf_report_type = RwSignal::new("journal".to_string());

    let do_export = move |format: &'static str, path_signal: RwSignal<String>| {
        let path = path_signal.get_untracked();
        if path.is_empty() {
            export_error.set(Some("Ange en sökväg för filen".into()));
            return;
        }
        exporting.set(true);
        export_error.set(None);
        export_success.set(None);

        leptos::task::spawn_local(async move {
            let fy = match ctx.active_fiscal_year.get_untracked() {
                Some(fy) => fy,
                None => {
                    export_error.set(Some("Inget räkenskapsår valt".into()));
                    exporting.set(false);
                    return;
                }
            };

            let args = serde_json::json!({
                "fiscalYearId": fy.id,
                "outputPath": path
            });

            let cmd = match format {
                "sie4" => "export_sie4",
                "csv" => "export_csv",
                "json" => "export_json",
                _ => "export_sie4",
            };

            match api::invoke::<_, String>(cmd, &args).await {
                Ok(result_path) => {
                    let label = match format {
                        "csv" => "CSV",
                        "json" => "JSON",
                        _ => "SIE4",
                    };
                    export_success.set(Some(format!("{}-fil exporterad till: {}", label, result_path)));
                }
                Err(e) => export_error.set(Some(e)),
            }
            exporting.set(false);
        });
    };

    let do_export_sie4 = move |ev: web_sys::SubmitEvent| {
        ev.prevent_default();
        do_export("sie4", sie4_path);
    };

    let do_export_csv = move |ev: web_sys::SubmitEvent| {
        ev.prevent_default();
        do_export("csv", csv_path);
    };

    let do_export_json = move |ev: web_sys::SubmitEvent| {
        ev.prevent_default();
        do_export("json", json_path);
    };

    let do_export_pdf = move |ev: web_sys::SubmitEvent| {
        ev.prevent_default();
        let path = pdf_path.get_untracked();
        if path.is_empty() {
            export_error.set(Some("Ange en sökväg för filen".into()));
            return;
        }
        let rtype = pdf_report_type.get_untracked();
        exporting.set(true);
        export_error.set(None);
        export_success.set(None);

        leptos::task::spawn_local(async move {
            let fy = match ctx.active_fiscal_year.get_untracked() {
                Some(fy) => fy,
                None => {
                    export_error.set(Some("Inget räkenskapsår valt".into()));
                    exporting.set(false);
                    return;
                }
            };

            let args = serde_json::json!({
                "fiscalYearId": fy.id,
                "reportType": rtype,
                "outputPath": path
            });

            match api::invoke::<_, String>("export_pdf", &args).await {
                Ok(result_path) => {
                    export_success.set(Some(format!("PDF exporterad till: {}", result_path)));
                }
                Err(e) => export_error.set(Some(e)),
            }
            exporting.set(false);
        });
    };

    view! {
        <div class="page">
            <h1>"Export"</h1>

            {move || export_error.get().map(|msg| view! {
                <div class="error-banner">{msg}</div>
            })}

            {move || export_success.get().map(|msg| view! {
                <div class="success-banner">{msg}</div>
            })}

            {move || ctx.active_fiscal_year.get().map(|fy| view! {
                <p style="margin-bottom: 1.5rem">"Räkenskapsår: " <strong>{fy.name}</strong> " (" {fy.start_date} " — " {fy.end_date} ")"</p>
            })}

            <section class="settings-section">
                <h2>"SIE4-export"</h2>
                <p class="section-desc">"Exportera bokföringsdata i SIE4-format (svensk standard). Filen kan importeras i andra bokföringsprogram."</p>

                <form class="inline-form" on:submit=do_export_sie4>
                    <div class="form-row">
                        <div class="form-group" style="flex: 1">
                            <label>"Sökväg"</label>
                            <input
                                type="text"
                                placeholder="t.ex. /home/user/export.se"
                                prop:value=move || sie4_path.get()
                                on:input=move |ev| sie4_path.set(event_target_value(&ev))
                                required=true
                            />
                        </div>
                        <div class="form-group form-group-btn">
                            <button type="submit" class="btn btn-primary"
                                disabled=move || exporting.get()
                            >
                                {move || if exporting.get() { "Exporterar..." } else { "Exportera SIE4" }}
                            </button>
                        </div>
                    </div>
                </form>
            </section>

            <section class="settings-section">
                <h2>"CSV-export"</h2>
                <p class="section-desc">"Exportera grundboken som semikolonseparerad CSV-fil. Kan öppnas i Excel eller LibreOffice."</p>

                <form class="inline-form" on:submit=do_export_csv>
                    <div class="form-row">
                        <div class="form-group" style="flex: 1">
                            <label>"Sökväg"</label>
                            <input
                                type="text"
                                placeholder="t.ex. /home/user/grundbok.csv"
                                prop:value=move || csv_path.get()
                                on:input=move |ev| csv_path.set(event_target_value(&ev))
                                required=true
                            />
                        </div>
                        <div class="form-group form-group-btn">
                            <button type="submit" class="btn btn-primary"
                                disabled=move || exporting.get()
                            >
                                {move || if exporting.get() { "Exporterar..." } else { "Exportera CSV" }}
                            </button>
                        </div>
                    </div>
                </form>
            </section>

            <section class="settings-section">
                <h2>"JSON-export"</h2>
                <p class="section-desc">"Komplett export med verifikationer, konton och revisionslogg i JSON-format. Lämplig för arkivering."</p>

                <form class="inline-form" on:submit=do_export_json>
                    <div class="form-row">
                        <div class="form-group" style="flex: 1">
                            <label>"Sökväg"</label>
                            <input
                                type="text"
                                placeholder="t.ex. /home/user/bokforing_2026.json"
                                prop:value=move || json_path.get()
                                on:input=move |ev| json_path.set(event_target_value(&ev))
                                required=true
                            />
                        </div>
                        <div class="form-group form-group-btn">
                            <button type="submit" class="btn btn-primary"
                                disabled=move || exporting.get()
                            >
                                {move || if exporting.get() { "Exporterar..." } else { "Exportera JSON" }}
                            </button>
                        </div>
                    </div>
                </form>
            </section>

            <section class="settings-section">
                <h2>"PDF-export"</h2>
                <p class="section-desc">"Generera en PDF-rapport av grundbok eller balansrapport. Lämplig för utskrift och arkivering."</p>

                <form class="inline-form" on:submit=do_export_pdf>
                    <div class="form-row">
                        <div class="form-group">
                            <label>"Rapporttyp"</label>
                            <select
                                prop:value=move || pdf_report_type.get()
                                on:change=move |ev| pdf_report_type.set(event_target_value(&ev))
                            >
                                <option value="journal">"Grundbok"</option>
                                <option value="trial_balance">"Balansrapport"</option>
                            </select>
                        </div>
                        <div class="form-group" style="flex: 1">
                            <label>"Sökväg"</label>
                            <input
                                type="text"
                                placeholder="t.ex. /home/user/grundbok.pdf"
                                prop:value=move || pdf_path.get()
                                on:input=move |ev| pdf_path.set(event_target_value(&ev))
                                required=true
                            />
                        </div>
                        <div class="form-group form-group-btn">
                            <button type="submit" class="btn btn-primary"
                                disabled=move || exporting.get()
                            >
                                {move || if exporting.get() { "Exporterar..." } else { "Exportera PDF" }}
                            </button>
                        </div>
                    </div>
                </form>
            </section>
        </div>
    }
}
