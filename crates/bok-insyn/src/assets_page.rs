//! Sidan med inventarieregistret: varje inventarium med anskaffning,
//! nyttjandeperiod, årets avskrivning och bokfört värde enligt plan, och
//! registret avstämt mot huvudboken vid årets slut.

use std::fmt::Write as _;

use bok_store::bok_core::Money;
use bok_store::bok_inventarier::Plan;
use bok_store::FiscalYear;

use crate::html::{amount, esc, Frame, Tab};
use crate::{Insyn, Result};

pub(crate) fn assets(i: &Insyn, y: &FiscalYear) -> Result<String> {
    let assets = i.store.assets()?;
    let mut b = String::new();
    writeln!(b, "<h1>Inventarier {}</h1>", esc(&y.label()))?;
    b.push_str(
        "<p class=\"ingress\">Inventarieregistret med avskrivning enligt plan: lika stort belopp \
         varje månad från den månad inventariet togs i bruk. Årets avskrivningar bokförs som \
         bokslutsverifikation i serie B.</p>\n",
    );
    let held: Vec<_> = assets
        .iter()
        .filter(|a| a.details.acquired <= y.end)
        .collect();
    if held.is_empty() {
        b.push_str("<p class=\"notis\">Registret har inga inventarier för året.</p>\n");
    } else {
        b.push_str(
            "<div class=\"rull\"><table class=\"bok\">\n<thead><tr><th scope=\"col\">Nr</th>\
             <th scope=\"col\">Beskrivning</th><th scope=\"col\">Anskaffad</th>\
             <th scope=\"col\" class=\"num\">Anskaffningsvärde</th><th scope=\"col\" class=\"num\">Månader</th>\
             <th scope=\"col\">Konton</th><th scope=\"col\" class=\"num\">Årets avskrivning</th>\
             <th scope=\"col\" class=\"num\">Bokfört värde vid årets slut</th><th scope=\"col\">Status</th>\
             </tr></thead>\n<tbody>\n",
        );
        for a in held {
            let plan = Plan::of(&a.details);
            let disposed = a.disposal.as_ref().filter(|d| d.date <= y.end);
            let (this_year, value) = if a.is_void() || disposed.is_some() {
                (Money::ZERO, Money::ZERO)
            } else {
                let acc = plan.accumulated_through(y.end);
                (
                    plan.between(y.start, y.end),
                    Money::from_ore(a.details.cost.ore() - acc.ore()),
                )
            };
            let status = match &a.disposal {
                None => "i bruk".to_string(),
                Some(d) => format!("{} {}", d.kind.as_str(), d.date),
            };
            let acc = a.details.accounts;
            writeln!(
                b,
                "<tr><td>{}</td><td>{}</td><td>{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td>\
                 <td>{} / {} / {}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td>{}</td></tr>",
                a.id,
                esc(&a.details.description),
                a.details.acquired,
                amount(a.details.cost),
                a.details.life_months,
                acc.asset,
                acc.accumulated,
                acc.expense,
                amount(this_year),
                amount(value),
                esc(&status)
            )?;
        }
        b.push_str("</tbody></table></div>\n");
    }
    let checks = bok_bokslut::depreciation::reconcile(i.store, y)
        .map_err(|e| crate::InsynError::Other(e.to_string()))?;
    if !checks.is_empty() {
        writeln!(b, "<h2>Avstämning mot huvudboken per {}</h2>", y.end)?;
        b.push_str(
            "<div class=\"rull\"><table class=\"bok\">\n<thead><tr><th scope=\"col\">Konto</th>\
             <th scope=\"col\" class=\"num\">Enligt registret</th><th scope=\"col\" class=\"num\">Bokfört</th>\
             <th scope=\"col\" class=\"num\">Skillnad</th></tr></thead>\n<tbody>\n",
        );
        for c in &checks {
            writeln!(
                b,
                "<tr><td>{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
                c.account,
                amount(c.register),
                amount(c.ledger),
                amount(c.difference())
            )?;
        }
        b.push_str("</tbody></table></div>\n");
        if checks.iter().any(|c| !c.difference().is_zero()) {
            b.push_str(
                "<p class=\"notis varning\">Registret och huvudboken skiljer sig. Det kan vara årets \
                 avskrivningar som inte är bokförda än, eller en anskaffning som saknas i registret.</p>\n",
            );
        }
    }
    Ok(i.page(
        &Frame::year(
            format!("Inventarier {}", y.label()),
            Tab::Assets,
            y,
            "inventarier.html",
        ),
        &b,
    ))
}
