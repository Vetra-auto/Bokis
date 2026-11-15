//! Sidorna som människan ser: startsidan där telefonen hämtar sin länk,
//! godkännandet av en anslutning och uppladdningen från telefonen. Samma
//! papper, bläck och violetta stämpel
//! som insynen, i ett smalt kort som passar en telefon. Inget JavaScript,
//! och allt som kommer utifrån escapas.

use crate::http::Response;
use crate::oauth::{redirect_host, Authorize};

pub(crate) const CSS: &str = r#":root {
  --papper: #FBFCF8;
  --linje: #B9D3C1;
  --linje-svag: #DCEADF;
  --blck: #1F2A44;
  --rod: #B3261E;
  --stampel: #5B3FA6;
  --grafit: #5E6672;
  --serif: Charter, "Bitstream Charter", "Sitka Text", "Iowan Old Style", Cambria, Georgia, serif;
  --mono: "DejaVu Sans Mono", Menlo, Consolas, monospace;
}
html { background: var(--papper); color: var(--blck); font: 17px/1.55 var(--serif); -webkit-text-size-adjust: 100%; }
body { margin: 0; }
.kort { max-width: 34rem; margin: 0 auto; padding: 2rem 1.25rem 3rem; }
.avsandare { color: var(--grafit); margin: 0 0 .4rem; font-size: .95rem; }
h1 { font-size: 1.75rem; line-height: 1.2; margin: 0 0 .8rem; font-weight: 600; }
h2 { font-size: 1.15rem; margin: 2.2rem 0 .6rem; font-weight: 600; }
p { margin: 0 0 1rem; }
.tyst { color: var(--grafit); font-size: .95rem; }
code { font-family: var(--mono); font-size: .85em; overflow-wrap: anywhere; }
a { color: var(--blck); text-decoration-color: var(--linje); text-underline-offset: .18em; }
a:focus-visible, button:focus-visible, input:focus-visible { outline: 2px solid var(--stampel); outline-offset: 2px; }
form { margin: 1.5rem 0; }
label { display: block; font-weight: 600; margin-bottom: .3rem; }
.hjalp { color: var(--grafit); font-size: .92rem; margin: 0 0 .6rem; }
input[type=text] { box-sizing: border-box; width: 100%; font: 1.5rem/1.2 var(--mono); letter-spacing: .12em; padding: .55rem .7rem; border: 1px solid var(--linje); border-radius: 6px; background: #fff; color: var(--blck); text-transform: uppercase; }
input[type=file] { box-sizing: border-box; width: 100%; font: inherit; padding: .9rem; border: 1px dashed var(--linje); border-radius: 8px; background: #fff; }
input[type=file]::file-selector-button { font: inherit; margin-right: .8rem; padding: .45rem .8rem; border: 1px solid var(--linje); border-radius: 6px; background: var(--papper); color: var(--blck); }
button { display: block; width: 100%; margin-top: 1.1rem; padding: .85rem 1rem; font: 600 1.05rem/1.2 var(--serif); color: #fff; background: var(--stampel); border: 0; border-radius: 8px; cursor: pointer; }
.fel { color: var(--rod); border-left: 3px solid var(--rod); padding: .2rem 0 .2rem .8rem; margin: .8rem 0 0; }
.kvitto { border: 2px solid var(--stampel); border-radius: 6px; padding: .9rem 1.1rem; margin: 1.2rem 0; }
.kvitto p { margin: 0 0 .4rem; }
.kvitto p:last-child { margin: 0; }
.kvitto strong { color: var(--stampel); }
ul.senast { list-style: none; padding: 0; margin: 0; border-top: 1px solid var(--linje-svag); }
ul.senast li { display: flex; justify-content: space-between; gap: 1rem; padding: .5rem 0; border-bottom: 1px solid var(--linje-svag); }
ul.senast .namn { overflow-wrap: anywhere; }
ul.senast .status { color: var(--grafit); white-space: nowrap; text-align: right; }
ul.senast .status.bokford { color: var(--stampel); }
ul.ratt { padding-left: 1.2rem; margin: 0 0 1rem; }
ul.ratt li { margin: .2rem 0; }
@media (prefers-reduced-motion: reduce) { * { scroll-behavior: auto; } }
"#;

/// Escapar text för HTML. Styrtecken och riktningsmarkörer visas som `\u{…}`.
pub(crate) fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            '\n' | '\t' => out.push(c),
            c if c.is_control()
                || matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}') =>
            {
                out.push_str(&format!("\\u{{{:x}}}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

/// En hel sida. `form_target` är det ursprung ett formulär får skicka
/// vidare till efter inskickningen, utöver sidans eget.
pub(crate) fn page(status: u16, title: &str, body: &str, form_target: Option<&str>) -> Response {
    let html = format!(
        "<!doctype html>\n<html lang=\"sv\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{}</title>\n\
         <link rel=\"stylesheet\" href=\"/stil.css\">\n</head>\n<body>\n<main class=\"kort\">\n{body}</main>\n</body>\n</html>\n",
        esc(title)
    );
    let csp = format!(
        "default-src 'none'; style-src 'self'; img-src 'self'; form-action 'self'{}; base-uri 'none'; frame-ancestors 'none'",
        form_target.map(|t| format!(" {t}")).unwrap_or_default()
    );
    Response::new(status, "text/html; charset=utf-8", html)
        .with("Content-Security-Policy", csp)
        .with("X-Frame-Options", "DENY")
        // Inte no-referrer: då skickar webbläsare `Origin: null` på formulär,
        // och ursprungskontrollen nekar godkännandet och uppladdningen.
        // same-origin skickar ingen adress till andra webbplatser.
        .replace("Referrer-Policy", "same-origin")
}

pub(crate) fn message(status: u16, title: &str, text: &str) -> Response {
    page(
        status,
        title,
        &format!("<h1>{}</h1>\n<p>{}</p>\n", esc(title), esc(text)),
        None,
    )
}

/// Svaret när en uppladdningslänk är okänd eller återkallad.
pub(crate) fn link_gone() -> Response {
    message(
        404,
        "Länken gäller inte",
        "Länken är okänd eller återkallad. Ta fram en ny med bok-fjarr uppladdning \
         på datorn med bokföringen.",
    )
}

/// Svaret på fel kod, ingen kod och en kod som har gått ut. Det är detsamma
/// i alla tre fallen, så att sidan inte avslöjar om en kod väntar.
pub(crate) const WRONG_LINK_CODE: &str = "Koden stämmer inte eller gäller inte längre. \
     Kontrollera den och försök igen, eller ta fram en ny med bok-fjarr uppladdning.";

/// Startsidan, där telefonen hämtar sin uppladdningslänk med en kod. Vem som
/// helst kan öppna den, så den säger inget om vems bokföringen är.
pub(crate) fn start(origin: &str, problem: Option<&str>) -> Response {
    let status = if problem.is_some() { 400 } else { 200 };
    let problem = problem
        .map(|p| format!("<p class=\"fel\" role=\"alert\">{}</p>\n", esc(p)))
        .unwrap_or_default();
    let body = format!(
        "<p class=\"avsandare\">Bokis</p>\n\
         <h1>Hämta uppladdningslänken</h1>\n\
         <p>Telefonen får en egen länk för att skicka kvitton och fakturor till bokföringen. Ta fram \
         en kod på datorn med bokföringen, till exempel med <code>bok-fjarr uppladdning telefon</code>, \
         och skriv den här.</p>\n\
         <form method=\"post\" action=\"/\">\n\
         <label for=\"kod\">Kod</label>\n\
         <p class=\"hjalp\" id=\"kod-hjalp\">Koden gäller i 15 minuter och en gång.</p>\n\
         <input type=\"text\" id=\"kod\" name=\"kod\" required autocomplete=\"one-time-code\" \
         autocapitalize=\"characters\" spellcheck=\"false\" aria-describedby=\"kod-hjalp\" \
         placeholder=\"XXXX-XXXX\" maxlength=\"20\">\n{problem}\
         <button type=\"submit\">Hämta länken</button>\n</form>\n\
         <p class=\"tyst\">Ska en agent ansluta? Lägg till <code>{mcp}</code> som anpassad anslutning \
         i Claude, ChatGPT eller Claude Code.</p>\n",
        mcp = esc(&format!("{origin}/mcp")),
    );
    page(status, "Hämta uppladdningslänken", &body, None)
}

/// Ursprunget i en omdirigering, för sidans CSP.
fn origin_of(uri: &str) -> String {
    match uri.split_once("://") {
        Some((scheme, rest)) => format!(
            "{scheme}://{}",
            rest.split(['/', '?']).next().unwrap_or(rest)
        ),
        None => String::new(),
    }
}

/// Godkännandet av en ny anslutning.
pub(crate) fn consent(
    company: &str,
    a: &Authorize,
    params: &[(String, String)],
    origin: &str,
    problem: Option<&str>,
) -> Response {
    let name = esc(&a.client.name);
    let mut hidden = String::new();
    for key in [
        "response_type",
        "client_id",
        "redirect_uri",
        "code_challenge",
        "code_challenge_method",
        "state",
        "scope",
        "resource",
    ] {
        if let Some((_, v)) = params.iter().find(|(k, _)| k == key) {
            hidden.push_str(&format!(
                "<input type=\"hidden\" name=\"{key}\" value=\"{}\">\n",
                esc(v)
            ));
        }
    }
    let mut cancel = format!("{}?error=access_denied", a.redirect_uri);
    if a.redirect_uri.contains('?') {
        cancel = cancel.replacen("?error", "&error", 1);
    }
    if let Some(s) = &a.state {
        cancel.push_str(&format!("&state={}", crate::http::encode(s)));
    }
    cancel.push_str(&format!("&iss={}", crate::http::encode(origin)));
    let problem = problem
        .map(|p| format!("<p class=\"fel\" role=\"alert\">{}</p>\n", esc(p)))
        .unwrap_or_default();
    let body = format!(
        "<p class=\"avsandare\">Bokis för {company}</p>\n\
         <h1>Ansluta {name} till bokföringen?</h1>\n\
         <p>{name} får använda agentens verktyg: läsa underlag, kontoplan, regler och tidigare \
         konteringar, och lägga förslag, frågor och flaggor. Den kan inte bokföra. Varje förslag \
         granskas och signeras av dig.</p>\n\
         <p class=\"tyst\">Efter godkännandet skickas du tillbaka till {host}.</p>\n\
         <form method=\"post\" action=\"/oauth/authorize\">\n{hidden}\
         <label for=\"kod\">Kopplingskod</label>\n\
         <p class=\"hjalp\" id=\"kod-hjalp\">Kör <code>bok-fjarr kod</code> på datorn med bokföringen. \
         Koden gäller i 15 minuter och en gång.</p>\n\
         <input type=\"text\" id=\"kod\" name=\"kod\" required autocomplete=\"one-time-code\" \
         autocapitalize=\"characters\" spellcheck=\"false\" aria-describedby=\"kod-hjalp\" \
         placeholder=\"XXXX-XXXX\" maxlength=\"20\">\n{problem}\
         <button type=\"submit\">Godkänn anslutningen</button>\n</form>\n\
         <p><a href=\"{cancel}\">Avbryt</a></p>\n",
        company = esc(company),
        host = esc(redirect_host(&a.redirect_uri)),
        cancel = esc(&cancel),
    );
    page(
        if problem.is_empty() { 200 } else { 400 },
        &format!("Ansluta {}", a.client.name),
        &body,
        Some(&origin_of(&a.redirect_uri)),
    )
}

/// En rad i listan över senast skickade underlag.
pub(crate) struct Recent {
    pub id: i64,
    pub name: String,
    pub status: String,
    pub booked: bool,
}

/// Utfallet av en uppladdning, som det står i adressen efter omdirigeringen.
pub(crate) struct Outcome {
    pub received: Vec<(i64, String, usize)>,
    pub existing: Vec<(i64, String)>,
    pub refused: usize,
}

/// Uppladdningssidan. `fresh` är en länk som inte har använts än: då ber
/// sidan telefonen spara den.
pub(crate) fn upload(
    company: &str,
    link_name: &str,
    fresh: bool,
    outcome: &Outcome,
    recent: &[Recent],
) -> Response {
    let link = esc(&format!("\u{201d}{link_name}\u{201d}"));
    let mut result = String::new();
    if fresh {
        result.push_str(&format!(
            "<div class=\"kvitto\" role=\"status\">\n<p><strong>Spara den här sidan.</strong> Lägg den på \
             hemskärmen eller som bokmärke, så kommer du hit direkt nästa gång. Länken {link} är personlig: \
             den som har den kan skicka underlag till bokföringen.</p>\n</div>\n"
        ));
    }
    if !outcome.received.is_empty() || !outcome.existing.is_empty() || outcome.refused > 0 {
        result.push_str("<div class=\"kvitto\" role=\"status\">\n");
        for (id, name, size) in &outcome.received {
            result.push_str(&format!(
                "<p><strong>Mottaget:</strong> underlag {id}, {} ({}).</p>\n",
                esc(name),
                size_text(*size)
            ));
        }
        for (id, name) in &outcome.existing {
            result.push_str(&format!(
                "<p>{} fanns redan som underlag {id}.</p>\n",
                esc(name)
            ));
        }
        if outcome.refused > 0 {
            result.push_str(&format!(
                "<p class=\"fel\">{} togs inte emot. Skicka bilder eller PDF på högst 25 MB.</p>\n",
                if outcome.refused == 1 {
                    "En fil".to_string()
                } else {
                    format!("{} filer", outcome.refused)
                }
            ));
        }
        let ids: Vec<String> = outcome
            .received
            .iter()
            .map(|(id, _, _)| id.to_string())
            .collect();
        if !ids.is_empty() {
            result.push_str(&format!(
                "<p>Be agenten: <q>Bokför underlag {}</q>.</p>\n",
                ids.join(" och ")
            ));
        }
        result.push_str("</div>\n");
    }
    let mut list = String::new();
    if !recent.is_empty() {
        list.push_str("<h2>Senast skickade</h2>\n<ul class=\"senast\">\n");
        for r in recent {
            list.push_str(&format!(
                "<li><span class=\"namn\">{}<br><span class=\"tyst\">underlag {}</span></span><span class=\"status{}\">{}</span></li>\n",
                esc(&r.name),
                r.id,
                if r.booked { " bokford" } else { "" },
                esc(&r.status)
            ));
        }
        list.push_str("</ul>\n");
    }
    // En ny länk har redan uppmaningen överst.
    let footer = if fresh {
        String::new()
    } else {
        format!("<p class=\"tyst\">Länken {link} är personlig. Spara den på hemskärmen och dela den inte.</p>\n")
    };
    let body = format!(
        "<p class=\"avsandare\">Bokis för {company}</p>\n\
         <h1>Skicka underlag</h1>\n\
         <p>Fotografera kvittot eller välj en bild eller PDF. Filen sparas oförändrad i bokföringen, \
         och sedan ber du agenten bokföra den.</p>\n{result}\
         <form method=\"post\" enctype=\"multipart/form-data\">\n\
         <label for=\"fil\">Kvitto, faktura eller annat underlag</label>\n\
         <input type=\"file\" id=\"fil\" name=\"fil\" accept=\"image/*,application/pdf\" multiple required>\n\
         <button type=\"submit\">Skicka till Bokis</button>\n</form>\n{list}{footer}",
        company = esc(company),
    );
    page(200, "Skicka underlag", &body, None)
}

/// En storlek som `2,1 MB` eller `240 kB`.
pub(crate) fn size_text(bytes: usize) -> String {
    const MB: usize = 1024 * 1024;
    if bytes < 1024 {
        format!("{bytes} byte")
    } else if bytes < MB {
        format!("{} kB", bytes.div_ceil(1024))
    } else {
        format!("{},{} MB", bytes / MB, (bytes % MB) * 10 / MB)
    }
}
