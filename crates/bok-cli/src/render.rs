//! Utskrifter för terminalen.

use anyhow::Result;

use bok_store::bok_core::Money;
use bok_store::bok_core::voucher::{Counterparty, Line};
use bok_store::{
    CorrectionLink, EventRow, FiscalYear, LedgerAccount, Outcome, ProposalCounterparty,
    ProposalRow, Store, TrialBalanceRow, VerifyReport, VoucherView, display_timestamp,
};

/// Text från agenten eller utifrån (motiveringar, banktexter, filnamn,
/// modellnamn), säker att skriva i en terminal. Styrtecken kan flytta
/// markören, skriva över en rad eller färga om skärmen, och tecken som vänder
/// skrivriktningen kan ändra hur en rad läses. De visas som synliga koder, och
/// radbrytningar blir mellanslag.
pub fn safe(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\n' | '\t' => out.push(' '),
            c if c.is_control()
                || matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}') =>
            {
                out.push_str(&format!("\\u{{{:x}}}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

/// `1 234,56` med mellanslag som tusentalsavgränsare.
pub fn amount(m: Money) -> String {
    let (negative, kronor, ore) = m.split();
    let digits = kronor.to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(' ');
        }
        grouped.push(c);
    }
    format!("{}{grouped},{ore:02}", if negative { "-" } else { "" })
}

fn side(m: Money) -> String {
    if m.is_zero() {
        String::new()
    } else {
        amount(m)
    }
}

pub fn counterparty(c: &Counterparty) -> String {
    let ids: Vec<&str> = [c.org_number.as_deref(), c.vat_number.as_deref()]
        .into_iter()
        .flatten()
        .collect();
    let mut text = if ids.is_empty() {
        c.name.clone()
    } else {
        format!("{} ({})", c.name, ids.join(", "))
    };
    if let Some(a) = &c.address {
        text.push_str(", ");
        text.push_str(a);
    }
    safe(&text)
}

/// "R1", eller "R1 (2027)" om rättelsen ligger i ett annat räkenskapsår.
pub fn link(store: &Store, l: &CorrectionLink, own_year: i64) -> Result<String> {
    Ok(if l.fiscal_year_id == own_year {
        l.reference.to_string()
    } else {
        format!(
            "{} ({})",
            l.reference,
            store.fiscal_year(l.fiscal_year_id)?.label()
        )
    })
}

pub fn lines(store: &Store, lines: &[Line], at: chrono::NaiveDate, indent: &str) -> Result<()> {
    let names = store.chart(at)?;
    for l in lines {
        let name = names
            .iter()
            .find(|a| a.number == l.account)
            .map(|a| a.name.as_str())
            .unwrap_or("");
        let text = l
            .text
            .as_deref()
            .map(|t| format!("  {}", safe(t)))
            .unwrap_or_default();
        println!(
            "{indent}{} {:<28} {:>14} {:>14}{text}",
            l.account,
            truncate(name, 28),
            side(l.debit),
            side(l.credit)
        );
    }
    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max - 1).chain(['…']).collect()
    }
}

pub fn voucher(store: &Store, v: &VoucherView, verbose: bool) -> Result<()> {
    println!(
        "{:<6} {}  {}    registrerad {}",
        v.reference.to_string(),
        v.date,
        v.description,
        display_timestamp(&v.registered_at)
    );
    if let Some(c) = &v.counterparty {
        println!("       Motpart: {}", counterparty(c));
    }
    lines(store, &v.lines, v.date, "       ")?;
    for l in &v.corrected_by {
        println!(
            "       Rättad genom {} ({})",
            link(store, l, v.fiscal_year_id)?,
            l.reason
        );
    }
    if let Some(l) = &v.corrects {
        println!(
            "       Rättar {} ({})",
            link(store, l, v.fiscal_year_id)?,
            l.reason
        );
    }
    if verbose {
        println!(
            "       Sammanställd {}, förslag {}",
            v.compiled_on, v.proposal_id
        );
        for a in &v.attachments {
            println!("       Underlag {a}");
        }
        println!("       Signerad av {} ({})", v.key_name, v.key_fingerprint);
        println!("       Kanonisk hash {}", v.canonical_hash);
        println!("       Kedjehash     {}", v.chain_hash);
    }
    Ok(())
}

pub fn proposal(store: &Store, p: &ProposalRow) -> Result<()> {
    let status = match &p.decision {
        None => "väntar på beslut".to_string(),
        Some(d) if d.outcome == Outcome::Approved => match d.voucher_id {
            Some(id) => format!("bokförd som {}", store.voucher(id)?.reference),
            None => "godkänd".into(),
        },
        Some(d) => format!("avvisad: {}", safe(d.reason.as_deref().unwrap_or(""))),
    };
    println!(
        "Förslag {}  serie {}  {}  {}  [{status}]",
        p.id,
        p.series,
        p.date,
        safe(&p.description)
    );
    println!(
        "       Från {} ({}), {}",
        safe(&p.actor),
        p.source,
        display_timestamp(&p.created_at)
    );
    match &p.counterparty {
        Some(ProposalCounterparty::Inline(c)) => println!("       Motpart: {}", counterparty(c)),
        Some(ProposalCounterparty::Register(id)) => {
            let row = store.counterparties()?.into_iter().find(|r| r.id == *id);
            let text = row
                .map(|r| counterparty(&r.counterparty))
                .unwrap_or_else(|| format!("motpart {id}"));
            println!("       Motpart: {text} (ur registret, kopieras vid bokföring)");
        }
        None => {}
    }
    if let Some(original) = p.corrects {
        println!(
            "       Rättar {}: {}",
            store.voucher(original)?.reference,
            safe(p.correction_reason.as_deref().unwrap_or(""))
        );
    }
    if !p.attachments.is_empty() {
        let ids: Vec<String> = p.attachments.iter().map(|i| i.to_string()).collect();
        println!("       Underlag: {}", ids.join(", "));
    }
    lines(store, &p.lines, p.date, "       ")?;
    Ok(())
}

pub fn ledger(year: &FiscalYear, accounts: &[LedgerAccount]) {
    for a in accounts {
        println!("{} {}", a.account, a.name);
        println!(
            "       {:<46} {:>14} {:>14} {:>14}",
            "Ingående balans",
            "",
            "",
            amount(a.opening)
        );
        for e in &a.entries {
            let note = if e.corrected_by.is_empty() {
                String::new()
            } else {
                let refs: Vec<String> = e
                    .corrected_by
                    .iter()
                    .map(|l| {
                        if l.fiscal_year_id == year.id {
                            l.reference.to_string()
                        } else {
                            format!("{} (annat år)", l.reference)
                        }
                    })
                    .collect();
                format!("  [rättad genom {}]", refs.join(", "))
            };
            println!(
                "       {:<6} {} {:<34} {:>14} {:>14} {:>14}{note}",
                e.reference.to_string(),
                e.date,
                truncate(&e.text, 34),
                side(e.debit),
                side(e.credit),
                amount(e.balance)
            );
        }
        println!(
            "       {:<46} {:>14} {:>14} {:>14}",
            "Utgående balans",
            "",
            "",
            amount(a.closing)
        );
        println!();
    }
}

pub fn trial_balance(rows: &[TrialBalanceRow]) {
    println!(
        "{:<4} {:<30} {:>14} {:>14} {:>14} {:>14}",
        "Kto", "Namn", "Ingående", "Debet", "Kredit", "Utgående"
    );
    let mut totals = [Money::ZERO; 4];
    for r in rows {
        println!(
            "{} {:<30} {:>14} {:>14} {:>14} {:>14}",
            r.account,
            truncate(&r.name, 30),
            amount(r.opening),
            amount(r.debit),
            amount(r.credit),
            amount(r.closing)
        );
        for (t, v) in totals
            .iter_mut()
            .zip([r.opening, r.debit, r.credit, r.closing])
        {
            *t = Money::from_ore(t.ore() + v.ore());
        }
    }
    println!(
        "{:<35} {:>14} {:>14} {:>14} {:>14}",
        "Summa",
        amount(totals[0]),
        amount(totals[1]),
        amount(totals[2]),
        amount(totals[3])
    );
}

pub fn events(events: &[EventRow]) {
    for e in events {
        let signed = if e.signed { " [signerad]" } else { "" };
        println!(
            "{:>5}  {}  {} ({}, {})  {} {}{signed}",
            e.seq,
            display_timestamp(&e.time),
            e.actor,
            e.actor_kind,
            e.role,
            e.action,
            e.object.as_deref().unwrap_or("")
        );
        if let Some(old) = &e.old_values {
            println!("         före:  {old}");
        }
        if let Some(new) = &e.new_values {
            println!("         efter: {new}");
        }
    }
}

pub fn report(report: &VerifyReport) {
    for c in &report.checks {
        let mark = if c.problems.is_empty() { "OK " } else { "FEL" };
        println!("{mark}  {} ({} kontrollerade)", c.name, c.checked);
        for p in &c.problems {
            println!("       {p}");
        }
        for w in &c.warnings {
            println!("       obs: {w}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_text_cannot_steer_the_terminal() {
        let tricky = "underlag 12\x1b[2K\runderlag 13\u{202E}31\nny rad";
        let shown = safe(tricky);
        assert!(!shown.chars().any(|c| c.is_control()), "{shown:?}");
        assert_eq!(
            shown,
            "underlag 12\\u{1b}[2K\\u{d}underlag 13\\u{202e}31 ny rad"
        );
        assert_eq!(safe("Café Öst, 5420"), "Café Öst, 5420");
    }

    #[test]
    fn amounts_are_grouped_and_signed() {
        assert_eq!(amount(Money::from_ore(123456789)), "1 234 567,89");
        assert_eq!(amount(Money::from_ore(-50)), "-0,50");
        assert_eq!(amount(Money::from_ore(100000)), "1 000,00");
        assert_eq!(amount(Money::from_ore(0)), "0,00");
    }
}
