//! HTML för granskningsvyn. Allt dynamiskt innehåll escapas.

use crate::backend::{BankRowView, CaseView, ProposalDetail, ProposalSummary};

/// Escapar text för HTML-innehåll och attributvärden.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Formaterar öre som kronor med tusentalsavgränsare och decimalkomma.
/// Minustecknet behålls även för belopp under en krona.
pub fn format_ore(ore: i64) -> String {
    let negative = ore < 0;
    let abs = ore.unsigned_abs();
    let kronor = (abs / 100).to_string();
    let mut grouped = String::new();
    for (i, c) in kronor.chars().enumerate() {
        if i > 0 && (kronor.len() - i).is_multiple_of(3) {
            grouped.push('\u{a0}');
        }
        grouped.push(c);
    }
    format!(
        "{}{},{:02}",
        if negative { "-" } else { "" },
        grouped,
        abs % 100
    )
}

/// Osäkerheten i procent, eller ett streck när den saknas.
fn format_uncertainty(u: Option<f64>) -> String {
    u.map_or_else(|| "–".to_string(), |u| format!("{:.0}&nbsp;%", u * 100.0))
}

const STYLE: &str = "body{font-family:system-ui,sans-serif;margin:24px;max-width:1200px;color:#1d1d1f}\
table{border-collapse:collapse;width:100%}th,td{text-align:left;padding:6px 8px;border-bottom:1px solid #ddd}\
td.num,th.num{text-align:right;font-variant-numeric:tabular-nums}.warn{background:#fff4e5;padding:8px 12px;border-left:4px solid #e8a33d}\
.grid{display:grid;grid-template-columns:1fr 1fr;gap:24px}.doc img,.doc iframe,.doc pre{width:100%;min-height:480px;border:1px solid #ddd}\
.doc pre{white-space:pre-wrap;padding:8px;min-height:0}button{padding:8px 14px;font-size:15px}\
form{margin:12px 0}textarea{width:100%;min-height:60px}.meta td:first-child{color:#666;width:40%}\
nav{margin-bottom:16px}nav a{margin-right:16px}.inline{display:inline;margin:0 8px 0 0}.sug{color:#555;margin:4px 0}\
.case{border-bottom:1px solid #ddd;padding:8px 0}";

const NAV: &str = "<nav><a href=\"/\">Förslag</a><a href=\"/bank\">Bank</a><a href=\"/arenden\">Ärenden</a></nav>";

fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"sv\"><head><meta charset=\"utf-8\"><title>{}</title><style>{STYLE}</style></head><body>{NAV}{body}</body></html>",
        escape(title)
    )
}

/// Bankrader som inte är avstämda, med en knapp per möjlig verifikation och
/// ett fält för en förklaring.
pub fn bank_page(rows: &[BankRowView], csrf: &str) -> String {
    let mut body = String::from("<h1>Bankrader att stämma av</h1>");
    if rows.is_empty() {
        body.push_str("<p>Alla bankrader är avstämda.</p>");
        return page("Bankrader", &body);
    }
    body.push_str("<p>En period kan inte stängas förrän varje bankrad i den är avstämd.</p>");
    body.push_str("<table><tr><th>Datum</th><th class=\"num\">Belopp</th><th>Text</th><th>Möjliga verifikationer</th><th>Förklaring</th></tr>");
    let csrf = escape(csrf);
    for r in rows {
        let id = escape(&r.id);
        let mut options = String::new();
        if r.candidates.is_empty() {
            options.push_str("<p class=\"sug\">Ingen bokförd verifikation med samma belopp.</p>");
        }
        for c in &r.candidates {
            options.push_str(&format!(
                "<form class=\"inline\" method=\"post\" action=\"/bank/{id}/stam-av\">\
                 <input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\
                 <input type=\"hidden\" name=\"verifikation\" value=\"{}\">\
                 <button type=\"submit\">Stäm av mot {}</button></form> {} {}<br>",
                escape(&c.voucher_id),
                escape(&c.reference),
                escape(&c.date),
                escape(&c.description)
            ));
        }
        for sug in &r.suggestions {
            options.push_str(&format!("<p class=\"sug\">{}</p>", escape(sug)));
        }
        body.push_str(&format!(
            "<tr><td>{}</td><td class=\"num\">{}</td><td>{}{}</td><td>{options}</td>\
             <td><form method=\"post\" action=\"/bank/{id}/stam-av\">\
             <input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\
             <input name=\"forklaring\" required placeholder=\"Förklaring\"> \
             <button type=\"submit\">Förklara</button></form></td></tr>",
            escape(&r.date),
            format_ore(r.amount_ore),
            escape(&r.text),
            r.reference
                .as_deref()
                .map(|x| format!("<br>ref {}", escape(x)))
                .unwrap_or_default()
        ));
    }
    body.push_str("</table>");
    page("Bankrader", &body)
}

/// Agentens frågor och flaggor, med svarsformulär. En flagga kan också
/// ändra underlagets status.
pub fn cases_page(cases: &[CaseView], csrf: &str) -> String {
    let mut body = String::from("<h1>Ärenden</h1>");
    if cases.is_empty() {
        body.push_str("<p>Inga frågor eller flaggor väntar på svar.</p>");
        return page("Ärenden", &body);
    }
    let csrf = escape(csrf);
    for c in cases {
        let id = escape(&c.id);
        let on = c
            .document_id
            .as_deref()
            .map(|d| format!(" om <a href=\"/underlag/{0}\">underlag {0}</a>", escape(d)))
            .unwrap_or_default();
        let mark = if c.is_flag {
            "<select name=\"markera\"><option value=\"\">Ändra inte underlaget</option>\
             <option value=\"dubblett\">Markera underlaget som dubblett</option>\
             <option value=\"ej_rakenskapsinformation\">Markera som ej räkenskapsinformation</option></select> "
        } else {
            ""
        };
        body.push_str(&format!(
            "<div class=\"case\"><p><strong>{}</strong>{on} från {}, {}</p><p>{}</p>\
             <form method=\"post\" action=\"/arenden/{id}/svara\">\
             <input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\
             <textarea name=\"svar\" required></textarea>{mark}<button type=\"submit\">Svara</button></form></div>",
            escape(&c.kind),
            escape(&c.actor),
            escape(&c.created_at),
            escape(&c.text)
        ));
    }
    page("Ärenden", &body)
}

/// Listan över förslag som väntar på granskning.
pub fn list_page(proposals: &[ProposalSummary], notice: Option<&str>) -> String {
    let mut body = String::from("<h1>Förslag att granska</h1>");
    if let Some(n) = notice {
        body.push_str(&format!("<p class=\"warn\">{}</p>", escape(n)));
    }
    if proposals.is_empty() {
        body.push_str("<p>Inga förslag väntar.</p>");
        return page("Förslag att granska", &body);
    }
    body.push_str("<table><tr><th>Datum</th><th>Beskrivning</th><th>Motpart</th><th class=\"num\">Belopp</th><th class=\"num\">Osäkerhet</th><th class=\"num\">Varningar</th><th></th></tr>");
    for p in proposals {
        body.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td><a href=\"/forslag/{}\">Granska</a></td></tr>",
            escape(&p.transaction_date),
            escape(&p.description),
            escape(p.counterparty.as_deref().unwrap_or("–")),
            format_ore(p.amount_ore),
            format_uncertainty(p.uncertainty),
            p.warnings,
            escape(&p.id),
        ));
    }
    body.push_str("</table>");
    page("Förslag att granska", &body)
}

fn document_html(id: &str, media_type: &str, file_name: &str) -> String {
    let src = format!("/underlag/{}", escape(id));
    let label = format!("<p>{} ({})</p>", escape(file_name), escape(media_type));
    match media_type {
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" => {
            format!("{label}<img src=\"{src}\" alt=\"{}\">", escape(file_name))
        }
        "application/pdf" => format!(
            "{label}<iframe src=\"{src}\" title=\"{}\"></iframe>",
            escape(file_name)
        ),
        _ => format!("{label}<p><a href=\"{src}\">Öppna underlaget som text</a></p>"),
    }
}

/// Ett förslag bredvid sina underlag, med formulär för beslut.
pub fn detail_page(p: &ProposalDetail, csrf: &str, inline_texts: &[(String, String)]) -> String {
    let s = &p.summary;
    let mut body = format!(
        "<p><a href=\"/\">← Alla förslag</a></p><h1>{}</h1>",
        escape(&s.description)
    );
    if !p.warnings.is_empty() {
        body.push_str("<div class=\"warn\"><strong>Varningar</strong><ul>");
        for w in &p.warnings {
            body.push_str(&format!("<li>{}</li>", escape(w)));
        }
        body.push_str("</ul></div>");
    }
    body.push_str("<div class=\"grid\"><div>");
    let row = |k: &str, v: &str| format!("<tr><td>{}</td><td>{}</td></tr>", escape(k), escape(v));
    body.push_str("<table class=\"meta\">");
    body.push_str(&row("Förslag", &s.id));
    body.push_str(&row("Affärshändelsedatum", &s.transaction_date));
    body.push_str(&row(
        "Motpart",
        s.counterparty.as_deref().unwrap_or("saknas"),
    ));
    body.push_str(&row(
        "Organisationsnummer",
        p.counterparty_org_number.as_deref().unwrap_or("saknas"),
    ));
    if let Some(a) = &p.counterparty_address {
        body.push_str(&row("Adress", a));
    }
    if let Some(c) = &p.corrects {
        body.push_str(&row("Rättar", c));
    }
    body.push_str(&row("Modell", &p.model));
    body.push_str(&row(
        "Leverantör och region",
        &format!(
            "{} / {}",
            p.provider.as_deref().unwrap_or("okänd"),
            p.region.as_deref().unwrap_or("okänd")
        ),
    ));
    body.push_str(&row("Klient", p.client.as_deref().unwrap_or("okänd")));
    body.push_str(&row(
        "Osäkerhet",
        &format_uncertainty(s.uncertainty).replace("&nbsp;", " "),
    ));
    body.push_str(&row(
        "Regler",
        &if p.rule_ids.is_empty() {
            "inga".to_string()
        } else {
            p.rule_ids.join(", ")
        },
    ));
    body.push_str(&row("Föreslaget", &s.created_at));
    body.push_str("</table>");

    body.push_str("<h2>Kontering</h2><table><tr><th>Konto</th><th>Namn</th><th class=\"num\">Debet</th><th class=\"num\">Kredit</th><th>Text</th></tr>");
    let (mut debit, mut credit) = (0i64, 0i64);
    for l in &p.lines {
        debit += l.debit_ore;
        credit += l.credit_ore;
        let cell = |v: i64| if v == 0 { String::new() } else { format_ore(v) };
        body.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td>{}</td></tr>",
            l.account,
            escape(&l.account_name),
            cell(l.debit_ore),
            cell(l.credit_ore),
            escape(l.text.as_deref().unwrap_or("")),
        ));
    }
    body.push_str(&format!(
        "<tr><th></th><th>Summa</th><th class=\"num\">{}</th><th class=\"num\">{}</th><th></th></tr></table>",
        format_ore(debit),
        format_ore(credit)
    ));
    body.push_str(&format!(
        "<h2>Agentens motivering</h2><p>{}</p>",
        escape(&p.rationale)
    ));

    let id = escape(&s.id);
    let csrf = escape(csrf);
    body.push_str(&format!(
        "<h2>Beslut</h2>\
         <form method=\"post\" action=\"/forslag/{id}/godkann\"><input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\
         <button type=\"submit\">Godkänn och signera med nyckeln</button></form>\
         <form method=\"post\" action=\"/forslag/{id}/avvisa\"><input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\
         <label>Orsak till avvisning<textarea name=\"orsak\" required></textarea></label>\
         <button type=\"submit\">Avvisa</button></form>"
    ));
    body.push_str("</div><div class=\"doc\"><h2>Underlag</h2>");
    if p.documents.is_empty() {
        body.push_str("<p class=\"warn\">Förslaget har inget underlag.</p>");
    }
    for d in &p.documents {
        match inline_texts.iter().find(|(i, _)| i == &d.id) {
            Some((_, text)) => body.push_str(&format!(
                "<p>{} ({})</p><pre>{}</pre>",
                escape(&d.file_name),
                escape(&d.media_type),
                escape(text)
            )),
            None => body.push_str(&document_html(&d.id, &d.media_type, &d.file_name)),
        }
    }
    body.push_str("</div></div>");
    page(&format!("Granska {}", s.id), &body)
}

/// Resultatsida efter ett beslut eller ett fel.
pub fn message_page(title: &str, message: &str) -> String {
    page(
        title,
        &format!(
            "<h1>{}</h1><p>{}</p><p><a href=\"/\">Till förslagen</a></p>",
            escape(title),
            escape(message)
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_keep_their_sign() {
        assert_eq!(format_ore(-40), "-0,40");
        assert_eq!(format_ore(40), "0,40");
        assert_eq!(format_ore(0), "0,00");
        assert_eq!(format_ore(123_456), "1\u{a0}234,56");
        assert_eq!(format_ore(-123_456_789), "-1\u{a0}234\u{a0}567,89");
        assert_eq!(format_ore(100), "1,00");
    }

    #[test]
    fn escaping() {
        assert_eq!(
            escape("<script>alert('x')</script> & \""),
            "&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt; &amp; &quot;"
        );
    }
}
