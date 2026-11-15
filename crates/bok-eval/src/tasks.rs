//! Uppgifterna i Accounted Benchs bokföringssvit och hur de visas för agenten.
//!
//! Uppgifterna läses från en lokal kopia av benchen och kopieras inte in här.
//! De är licensierade CC BY 4.0, © 2025–2026 Jakob Wennberg.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::EvalError;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Company {
    pub entity_type: String,
    pub vat_registered: bool,
    pub accounting_method: String,
    pub business: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Transaction {
    pub date: String,
    /// Kronor. Negativt är pengar ut från företagskontot.
    pub amount: f64,
    pub currency: String,
    pub counterpart: String,
    pub description: String,
    /// Text från kvitto eller faktura, om det finns något.
    #[serde(default)]
    pub underlag: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Input {
    pub company: Company,
    pub transaction: Transaction,
}

/// Facit. Visas aldrig för agenten.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Gold {
    pub account: String,
    #[serde(default)]
    pub acceptable_accounts: Vec<String>,
    pub vat_treatment: String,
}

impl Gold {
    /// Om kontot godtas av facit.
    pub fn accepts(&self, account: u32) -> bool {
        let a = account.to_string();
        self.account == a || self.acceptable_accounts.contains(&a)
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Task {
    pub id: String,
    #[serde(default)]
    pub difficulty: String,
    /// Nya uppgifter som ännu inte ingår i den frysta mätningen.
    #[serde(default)]
    pub staged: bool,
    #[serde(default)]
    pub probe: String,
    pub input: Input,
    pub gold: Gold,
}

impl Task {
    /// Transaktionens belopp i öre, med tecken.
    pub fn amount_ore(&self) -> i64 {
        (self.input.transaction.amount * 100.0).round() as i64
    }
}

/// Läser uppgifter från filer eller kataloger. En katalog läses som alla
/// `.json`-filer i den. Varje fil innehåller en lista med uppgifter.
/// Uppgifter med `staged` tas bara med om `include_staged` är satt.
/// Resultatet är sorterat på id, och ett id som förekommer två gånger är ett fel.
pub fn load_tasks(sources: &[PathBuf], include_staged: bool) -> Result<Vec<Task>, EvalError> {
    let mut files = Vec::new();
    for source in sources {
        if source.is_dir() {
            let entries =
                std::fs::read_dir(source).map_err(|e| EvalError::Io(source.clone(), e))?;
            let mut found: Vec<PathBuf> = entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "json"))
                .collect();
            found.sort();
            files.extend(found);
        } else {
            files.push(source.clone());
        }
    }
    if files.is_empty() {
        return Err(EvalError::Tasks("inga uppgiftsfiler hittades".into()));
    }
    let mut tasks: Vec<Task> = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).map_err(|e| EvalError::Io(file.clone(), e))?;
        let parsed: Vec<Task> = serde_json::from_str(&text)
            .map_err(|e| EvalError::Tasks(format!("{}: {e}", file.display())))?;
        tasks.extend(parsed.into_iter().filter(|t| include_staged || !t.staged));
    }
    tasks.sort_by(|a, b| a.id.cmp(&b.id));
    if let Some(w) = tasks.windows(2).find(|w| w[0].id == w[1].id) {
        return Err(EvalError::Tasks(format!(
            "uppgiften {} förekommer två gånger",
            w[0].id
        )));
    }
    if tasks.is_empty() {
        return Err(EvalError::Tasks("filerna innehåller inga uppgifter".into()));
    }
    Ok(tasks)
}

/// Benchens egen kontoplan, om den ligger bredvid uppgifterna.
pub fn bench_chart_path(sources: &[PathBuf]) -> Option<PathBuf> {
    sources.iter().find_map(|s| {
        let dir: &Path = if s.is_dir() { s } else { s.parent()? };
        let p = dir.join("context-accounts.txt");
        p.is_file().then_some(p)
    })
}

/// Formaterar öre som kronor: `-1 352,00`.
pub fn format_kronor(ore: i64) -> String {
    let abs = ore.unsigned_abs();
    let kronor = (abs / 100).to_string();
    let mut grouped = String::new();
    for (i, c) in kronor.chars().enumerate() {
        if i > 0 && (kronor.len() - i).is_multiple_of(3) {
            grouped.push(' ');
        }
        grouped.push(c);
    }
    format!(
        "{}{grouped},{:02}",
        if ore < 0 { "-" } else { "" },
        abs % 100
    )
}

/// Underlaget som agenten får läsa för en uppgift: företagets förhållanden,
/// banktransaktionen och, om det finns, texten från kvittot eller fakturan.
/// Facit och benchens kommentarer tas aldrig med.
pub fn render_document(task: &Task) -> String {
    let c = &task.input.company;
    let t = &task.input.transaction;
    let ore = task.amount_ore();
    let direction = if ore < 0 {
        "utbetalning från företagskontot"
    } else {
        "inbetalning till företagskontot"
    };
    let entity = match c.entity_type.as_str() {
        "aktiebolag" => "aktiebolag",
        "enskild_firma" => "enskild firma",
        other => other,
    };
    let method = match c.accounting_method.as_str() {
        "invoice" => "faktureringsmetoden",
        "cash" => "kontantmetoden",
        other => other,
    };
    let mut text = format!(
        "Företaget\n\
         Verksamhet: {}\n\
         Bolagsform: {entity}\n\
         Momsregistrerat: {}\n\
         Bokföringsmetod: {method}\n\
         \n\
         Banktransaktion på företagskontot 1930\n\
         Datum: {}\n\
         Belopp: {} {} ({direction})\n\
         Motpart: {}\n\
         Text: {}\n",
        c.business,
        if c.vat_registered { "ja" } else { "nej" },
        t.date,
        format_kronor(ore),
        t.currency,
        t.counterpart,
        t.description,
    );
    match &t.underlag {
        Some(u) if !u.trim().is_empty() => {
            text.push_str(&format!("\nText från kvitto eller faktura\n{u}\n"))
        }
        _ => text.push_str("\nInget kvitto eller faktura finns, bara banktransaktionen.\n"),
    }
    text
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const SAMPLE: &str = r#"[
      {"id": "booking-001", "suite": "booking", "difficulty": "core", "probe": "p",
       "input": {"company": {"entity_type": "aktiebolag", "vat_registered": true, "accounting_method": "invoice", "business": "IT-konsultbolag"},
                 "transaction": {"date": "2026-05-12", "amount": -6250.0, "currency": "SEK", "counterpart": "Exempel Redovisning AB",
                                 "description": "Faktura 1001", "underlag": "Redovisningstjänster oktober. Belopp 5 000,00. Moms 25 % 1 250,00."}},
       "gold": {"account": "6530", "vat_treatment": "domestic_25"}, "canary": "x"},
      {"id": "booking-101", "suite": "booking", "difficulty": "hard",
       "input": {"company": {"entity_type": "aktiebolag", "vat_registered": true, "accounting_method": "invoice", "business": "IT-konsultbolag"},
                 "transaction": {"date": "2026-06-02", "amount": -1352.0, "currency": "SEK", "counterpart": "EXEMPEL MOLN LTD", "description": "EXEMPELMOLN*KONTOR"}},
       "gold": {"account": "5420", "acceptable_accounts": ["6540", "4535"], "vat_treatment": "reverse_charge_services"}},
      {"id": "booking-216", "suite": "booking", "difficulty": "hard", "staged": true, "inflow": true,
       "input": {"company": {"entity_type": "aktiebolag", "vat_registered": true, "accounting_method": "invoice", "business": "Byrå"},
                 "transaction": {"date": "2026-05-20", "amount": 3125.0, "currency": "SEK", "counterpart": "Exempel Datorer AB",
                                 "description": "Kreditfaktura 1234", "underlag": "Retur 2 500,00 kr, moms 625,00 kr."}},
       "gold": {"account": "5410", "acceptable_accounts": ["5400", "5420"], "vat_treatment": "domestic_25"}}
    ]"#;

    fn write_sample(dir: &Path) -> PathBuf {
        let p = dir.join("core.json");
        std::fs::write(&p, SAMPLE).unwrap();
        p
    }

    #[test]
    fn loads_and_filters_staged() {
        let dir = tempfile::tempdir().unwrap();
        let file = write_sample(dir.path());
        let frozen = load_tasks(std::slice::from_ref(&file), false).unwrap();
        assert_eq!(
            frozen.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            vec!["booking-001", "booking-101"]
        );
        let all = load_tasks(&[dir.path().to_path_buf()], true).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[2].amount_ore(), 312_500);
        assert!(all[1].gold.accepts(4535));
        assert!(!all[1].gold.accepts(6530));
    }

    #[test]
    fn duplicate_ids_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let file = write_sample(dir.path());
        let copy = dir.path().join("copy.json");
        std::fs::copy(&file, &copy).unwrap();
        assert!(matches!(
            load_tasks(&[file, copy], false),
            Err(EvalError::Tasks(_))
        ));
    }

    #[test]
    fn finds_bench_chart_next_to_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let file = write_sample(dir.path());
        assert_eq!(bench_chart_path(std::slice::from_ref(&file)), None);
        std::fs::write(
            dir.path().join("context-accounts.txt"),
            "1930 Företagskonto\n",
        )
        .unwrap();
        assert_eq!(
            bench_chart_path(&[file]),
            Some(dir.path().join("context-accounts.txt"))
        );
    }

    #[test]
    fn document_shows_input_but_never_gold() {
        let tasks: Vec<Task> = serde_json::from_str(SAMPLE).unwrap();
        let doc = render_document(&tasks[0]);
        assert!(doc.contains("Belopp: -6 250,00 SEK (utbetalning från företagskontot)"));
        assert!(doc.contains("Bokföringsmetod: faktureringsmetoden"));
        assert!(doc.contains("Moms 25 % 1 250,00"));
        assert!(!doc.contains("6530"));
        assert!(!doc.contains("domestic"));
        let bank_only = render_document(&tasks[1]);
        assert!(bank_only.contains("Inget kvitto eller faktura finns"));
        assert!(!bank_only.contains("5420"));
        let inflow = render_document(&tasks[2]);
        assert!(inflow.contains("3 125,00 SEK (inbetalning till företagskontot)"));
    }

    #[test]
    fn kronor_format() {
        assert_eq!(format_kronor(-135_200), "-1 352,00");
        assert_eq!(format_kronor(5), "0,05");
        assert_eq!(format_kronor(-123_456_789), "-1 234 567,89");
    }
}
