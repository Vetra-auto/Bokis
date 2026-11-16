//! Sidan med utkastet till inkomstdeklaration 2: INK2R, INK2S och INK2 som
//! de räknas ur bokföringen, med kontrollerna, bolagsskatten och vilka konton
//! varje fält består av. Samma beräkning som `bok deklaration ink2`.

use std::fmt::Write as _;

use bok_deklaration::summary::{grouped, ore_text};
use bok_deklaration::{Declaration, FormKind};
use bok_store::FiscalYear;

use crate::html::{esc, Frame, Tab};
use crate::{Insyn, Result};

fn title(form: FormKind) -> &'static str {
    match form {
        FormKind::Ink2 => "INK2 – huvudblankett",
        FormKind::Ink2R => "INK2R – räkenskapsschema",
        FormKind::Ink2S => "INK2S – skattemässiga justeringar",
    }
}

fn body(b: &mut String, d: &Declaration) -> std::fmt::Result {
    writeln!(
        b,
        "<p>Blanketterna {}, {} och {} för räkenskapsåret {} – {}.</p>",
        esc(&d.catalog.form_name(FormKind::Ink2)),
        esc(&d.catalog.form_name(FormKind::Ink2R)),
        esc(&d.catalog.form_name(FormKind::Ink2S)),
        d.year_start,
        d.year_end
    )?;
    b.push_str("<h2>Kontroller</h2>\n<ul>\n");
    for c in &d.checks {
        writeln!(
            b,
            "<li>{}{}{}</li>",
            if c.ok { "" } else { "<strong>Fel:</strong> " },
            esc(&c.name),
            if c.detail.is_empty() {
                String::new()
            } else {
                format!(" – {}", esc(&c.detail))
            }
        )?;
    }
    b.push_str("</ul>\n");
    for w in &d.warnings {
        writeln!(b, "<p class=\"notis varning\">{}</p>", esc(w))?;
    }
    for form in [FormKind::Ink2R, FormKind::Ink2S, FormKind::Ink2] {
        writeln!(b, "<h2>{}</h2>", title(form))?;
        b.push_str(
            "<div class=\"rull\"><table class=\"bok\">\n<thead><tr><th scope=\"col\">Fält</th>\
             <th scope=\"col\">Kod</th><th scope=\"col\">Text och konton</th>\
             <th scope=\"col\" class=\"num\">Belopp, kr</th></tr></thead>\n<tbody>\n",
        );
        for (f, k, v) in d.amounts(form) {
            let parts = v
                .parts
                .iter()
                .map(|p| format!("{}: {}", esc(&p.label), ore_text(p.ore)))
                .collect::<Vec<_>>()
                .join("<br>");
            writeln!(
                b,
                "<tr><td>{}</td><td>{}</td><td>{}<details><summary>Består av</summary>{}</details></td>\
                 <td class=\"num\">{}</td></tr>",
                esc(&f.number),
                esc(&f.code),
                esc(&f.text),
                parts,
                grouped(k)
            )?;
        }
        for ((kind, code), text) in &d.texts {
            if *kind == form {
                writeln!(
                    b,
                    "<tr><td></td><td>{}</td><td>{}</td><td class=\"num\">{}</td></tr>",
                    esc(code),
                    esc(d.catalog.field(form, code).map_or("", |f| f.text.as_str())),
                    esc(text)
                )?;
            }
        }
        b.push_str("</tbody></table></div>\n");
    }
    b.push_str("<h2>Bolagsskatt</h2>\n");
    writeln!(
        b,
        "<p>Överskott {} kr, beskattningsbar inkomst {} kr (avrundad nedåt till helt tiotal kronor, \
         IL 1 kap. 7 §), skatt 20,6 % (IL 65 kap. 10 §): <strong>{} kr</strong>.</p>",
        grouped(d.tax.surplus),
        grouped(d.tax.taxable),
        ore_text(d.tax.tax_ore)
    )
}

pub(crate) fn declaration(i: &Insyn, y: &FiscalYear) -> Result<String> {
    let mut b = String::new();
    writeln!(b, "<h1>Inkomstdeklaration 2 {}</h1>", esc(&y.label()))?;
    b.push_str(
        "<p class=\"ingress\">Utkast räknat ur bokföringen med fältförteckningen från Skatteverket \
         och BAS kopplingstabell. Uppgifter som inte syns i bokföringen, till exempel underskott från \
         tidigare år, anges när filerna skrivs med <code>bok deklaration ink2</code>; de finns inte \
         med här.</p>\n",
    );
    let computed = bok_bokslut::input::declaration_input(i.store, y, Vec::new())
        .map_err(|e| e.to_string())
        .and_then(|input| Declaration::compute_bas(&input).map_err(|e| e.to_string()));
    match computed {
        Ok(d) => body(&mut b, &d)?,
        Err(e) => writeln!(
            b,
            "<p class=\"notis varning\">Utkastet går inte att räkna fram: {}</p>",
            esc(&e)
        )?,
    }
    Ok(i.page(
        &Frame::year(
            format!("Inkomstdeklaration 2 {}", y.label()),
            Tab::Declaration,
            y,
            "deklaration.html",
        ),
        &b,
    ))
}
