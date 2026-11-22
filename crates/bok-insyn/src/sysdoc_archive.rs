//! Avsnittet Arkivplan i systemdokumentationen.
//!
//! Texten kommer från den arkivplan företaget själv har lagt in (`bok
//! arkivplan satt`), inte från programmet. Till den lägger Bokis det som går
//! att räkna ut ur bokföringen: hur länge varje räkenskapsår ska bevaras och
//! när `bok kopia skapa` senast skrev till en plats i planen.

use std::fmt::Write as _;

use bok_store::bok_arkivplan::{observations, retained_through};
use bok_store::{EventRow, Store};

use crate::Result;

/// En text som cell i en Markdown-tabell: utan radbrytningar och lodstreck.
fn cell(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.is_empty() {
        "–".into()
    } else {
        flat.replace('|', "/")
    }
}

/// En text som eget stycke: radbrytningar blir mellanslag.
fn paragraph(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Den senaste kopian i historiken som skrevs under `path`, som tid.
fn latest_copy<'a>(events: &'a [EventRow], path: &str) -> Option<&'a str> {
    let prefix = path.trim_end_matches('/');
    events
        .iter()
        .rev()
        .filter(|e| e.action == "export.skapad")
        .find(|e| {
            e.new_values
                .as_deref()
                .and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok())
                .is_some_and(|v| {
                    v["format"] == "kopia"
                        && v["fil"]
                            .as_str()
                            .is_some_and(|f| f == prefix || f.starts_with(&format!("{prefix}/")))
                })
        })
        .map(|e| e.time.as_str())
}

/// Avsnittet som Markdown, med rubriken `## Arkivplan`.
pub fn section(store: &Store, events: &[EventRow]) -> Result<String> {
    let mut d = String::new();
    let w = &mut d;
    writeln!(w, "## Arkivplan")?;
    writeln!(w)?;
    let Some(record) = store.archive_plan()? else {
        writeln!(
            w,
            "Ingen arkivplan är inlagd. BFNAR 2013:2 punkt 8.3 och 9.2 d: arkivplanen ska visa vad \
             som arkiverats och var det förvaras, och ingå i systemdokumentationen. Lägg in den med \
             `bok arkivplan satt <fil>` (se `docs/arkivplan-exempel.json`)."
        )?;
        return Ok(d);
    };
    let plan = &record.plan;
    writeln!(
        w,
        "Planen lades in {} av {} (händelse {} i behandlingshistoriken). Ansvarig: {}.",
        record.time,
        record.actor,
        record.event_seq,
        paragraph(&plan.responsible)
    )?;
    writeln!(w)?;
    writeln!(
        w,
        "| Vad | Slag | Plats | Land | Hur ofta | Sparas | Kontroll | Åtskild | Senaste kopia |"
    )?;
    writeln!(w, "| --- | --- | --- | --- | --- | --- | --- | --- | --- |")?;
    for p in &plan.parts {
        let latest = p
            .path
            .as_deref()
            .map(|path| latest_copy(events, path).unwrap_or("ingen i historiken"))
            .unwrap_or("–");
        let country = match &p.notified {
            Some(n) => format!("{} (anmäld {})", p.country, n),
            None => p.country.clone(),
        };
        writeln!(
            w,
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            cell(&p.what),
            p.kind.title(),
            cell(&p.place),
            cell(&country),
            cell(p.frequency.as_deref().unwrap_or("")),
            cell(p.kept.as_deref().unwrap_or("")),
            cell(p.check.as_deref().unwrap_or("")),
            if p.separate { "ja" } else { "nej" },
            cell(latest)
        )?;
    }
    if let Some(r) = &plan.readable {
        writeln!(w)?;
        writeln!(w, "Läsbar form: {}", paragraph(r))?;
    }
    if let Some(n) = &plan.note {
        writeln!(w)?;
        writeln!(w, "{}", paragraph(n))?;
    }
    let years = store.fiscal_years()?;
    if !years.is_empty() {
        writeln!(w)?;
        writeln!(
            w,
            "Bevarandetid (BFL 7 kap. 2 §: till och med det sjunde året efter det kalenderår då \
             räkenskapsåret avslutades):"
        )?;
        writeln!(w)?;
        for y in &years {
            writeln!(
                w,
                "- Räkenskapsåret {} ({} – {}) bevaras till och med {}.",
                y.label(),
                y.start,
                y.end,
                retained_through(y.end)
            )?;
        }
    }
    let found = observations(plan);
    writeln!(w)?;
    if found.is_empty() {
        writeln!(
            w,
            "Planen täcker original, åtskild säkerhetskopia, kontroll och läsbar form."
        )?;
    } else {
        writeln!(w, "Att se över i planen:")?;
        writeln!(w)?;
        for o in found {
            writeln!(w, "- {} ({}).", o.text, o.source)?;
        }
    }
    Ok(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_stay_on_one_line() {
        assert_eq!(cell("a |\n b"), "a / b");
        assert_eq!(cell("  "), "–");
    }
}
