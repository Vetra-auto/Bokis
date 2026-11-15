//! Kontroller av agentens förslag innan de sparas.
//!
//! Fel stoppar förslaget. Varningar sparas med förslaget och visas för
//! granskaren. Samma kontroller körs igen i lagret när förslaget bokförs.

use chrono::NaiveDate;

use crate::backend::{Account, Proposal};

/// Beskrivningar som säger för lite om vad affärshändelsen avser (BFL 5 kap. 7 §).
const TOO_GENERIC: &[&str] = &[
    "kvitto",
    "faktura",
    "köp",
    "betalning",
    "utgift",
    "inköp",
    "diverse",
    "övrigt",
];

/// Resultat av en kontroll.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Check {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl Check {
    pub fn passed(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Kontrollerar ett förslag mot kontoplanen.
pub fn check(proposal: &Proposal, accounts: &[Account]) -> Check {
    let mut c = Check::default();

    if NaiveDate::parse_from_str(&proposal.transaction_date, "%Y-%m-%d").is_err() {
        c.errors.push(format!(
            "affarshandelse_datum {:?} är inte ett datum på formen ÅÅÅÅ-MM-DD",
            proposal.transaction_date
        ));
    }

    let description = proposal.description.trim();
    if description.is_empty() {
        c.errors
            .push("beskrivning saknas; den ska säga vad affärshändelsen avser".into());
    } else if TOO_GENERIC.contains(&description.to_lowercase().as_str()) {
        c.warnings.push(format!(
            "beskrivningen {description:?} säger för lite om vad affärshändelsen avser"
        ));
    }

    if proposal.lines.len() < 2 {
        c.errors
            .push("en verifikation behöver minst två konteringsrader".into());
    }

    let mut debit = 0i64;
    let mut credit = 0i64;
    for (i, line) in proposal.lines.iter().enumerate() {
        let nr = i + 1;
        match (line.debit_ore, line.credit_ore) {
            (d, 0) if d > 0 => debit = debit.saturating_add(d),
            (0, k) if k > 0 => credit = credit.saturating_add(k),
            (d, k) if d < 0 || k < 0 => c.errors.push(format!(
                "rad {nr}: belopp får inte vara negativa; använd debet eller kredit"
            )),
            (0, 0) => c.errors.push(format!("rad {nr}: belopp saknas")),
            _ => c.errors.push(format!(
                "rad {nr}: ange antingen debet eller kredit, inte båda"
            )),
        }
        match accounts.iter().find(|a| a.number == line.account) {
            None => c.errors.push(format!(
                "rad {nr}: konto {} finns inte i kontoplanen",
                line.account
            )),
            Some(a) if !a.active => c
                .errors
                .push(format!("rad {nr}: konto {} är inte aktivt", line.account)),
            Some(_) => {}
        }
    }
    if debit != credit {
        c.errors.push(format!(
            "debet ({debit} öre) och kredit ({credit} öre) balanserar inte"
        ));
    }

    if proposal.document_ids.is_empty()
        && proposal.bank_row_ids.is_empty()
        && proposal.corrects.is_none()
    {
        c.warnings
            .push("förslaget har inget underlag; en verifikation ska ange sitt underlag".into());
    }
    // Bokföringslagen 5 kap. 7 § kräver motpart i varje verifikation. Den får
    // utelämnas bara när det är förenat med svårighet (5 kap. 8 §), i praktiken
    // vid anonym kontantförsäljning (BFNAR 2013:2 punkt 5.7). Därför en varning
    // också för förslag som bara bygger på en bankrad.
    if proposal.counterparty.is_none() {
        c.warnings.push(
            "motpart saknas; bokföringslagen (5 kap. 7 §) kräver motpart utom vid anonym \
             kontantförsäljning. Ange namn och organisationsnummer om underlaget visar dem, \
             annars ställ en fråga"
                .into(),
        );
    }
    if let Some(cp) = &proposal.counterparty {
        if cp.name.trim().is_empty() {
            c.errors.push("motpartens namn är tomt".into());
        }
    }
    c
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::backend::{Counterparty, ProposalLine};

    fn accounts() -> Vec<Account> {
        [
            (1930, "Företagskonto", true),
            (2440, "Leverantörsskulder", true),
            (2641, "Ingående moms", true),
            (5420, "Programvaror", true),
            (5410, "Förbrukningsinventarier", false),
        ]
        .iter()
        .map(|(n, name, active)| Account {
            number: *n,
            name: (*name).into(),
            kind: "x".into(),
            active: *active,
        })
        .collect()
    }

    fn line(account: u32, debit: i64, credit: i64) -> ProposalLine {
        ProposalLine {
            account,
            debit_ore: debit,
            credit_ore: credit,
            text: None,
        }
    }

    fn valid() -> Proposal {
        Proposal {
            transaction_date: "2026-03-14".into(),
            description: "Molntjänst, månadsabonnemang mars".into(),
            counterparty: Some(Counterparty {
                name: "Exempel Hosting AB".into(),
                org_number: Some("556000-0000".into()),
                vat_number: None,
                country: Some("SE".into()),
            }),
            lines: vec![
                line(5420, 40_000, 0),
                line(2641, 10_000, 0),
                line(2440, 0, 50_000),
            ],
            document_ids: vec!["INK-1".into()],
            bank_row_ids: vec![],
            rule_ids: vec![],
            corrects: None,
        }
    }

    #[test]
    fn valid_proposal_passes_without_warnings() {
        let c = check(&valid(), &accounts());
        assert!(c.passed(), "{:?}", c.errors);
        assert!(c.warnings.is_empty(), "{:?}", c.warnings);
    }

    #[test]
    fn imbalance_is_refused() {
        let mut p = valid();
        p.lines[2].credit_ore = 49_999;
        let c = check(&p, &accounts());
        assert!(!c.passed());
        assert!(c.errors.iter().any(|e| e.contains("balanserar inte")));
    }

    #[test]
    fn unknown_and_inactive_accounts_are_refused() {
        let mut p = valid();
        p.lines[0].account = 9999;
        p.lines[1].account = 5410;
        let c = check(&p, &accounts());
        assert!(c.errors.iter().any(|e| e.contains("9999 finns inte")));
        assert!(c.errors.iter().any(|e| e.contains("5410 är inte aktivt")));
    }

    #[test]
    fn negative_and_double_amounts_are_refused() {
        let mut p = valid();
        p.lines[0] = line(5420, -40_000, 0);
        p.lines[1] = line(2641, 10_000, 10_000);
        let c = check(&p, &accounts());
        assert!(c.errors.iter().any(|e| e.contains("negativa")));
        assert!(c.errors.iter().any(|e| e.contains("inte båda")));
    }

    #[test]
    fn generic_description_and_missing_document_give_warnings() {
        let mut p = valid();
        p.description = "Kvitto".into();
        p.document_ids.clear();
        let c = check(&p, &accounts());
        assert!(c.passed());
        assert_eq!(c.warnings.len(), 2, "{:?}", c.warnings);
    }

    #[test]
    fn a_missing_counterparty_is_warned_about_even_without_a_document() {
        let mut p = valid();
        p.counterparty = None;
        p.document_ids.clear();
        p.bank_row_ids = vec!["rad".into()];
        let c = check(&p, &accounts());
        assert!(c.passed());
        assert!(
            c.warnings.iter().any(|w| w.contains("5 kap. 7 §")),
            "{:?}",
            c.warnings
        );
    }

    #[test]
    fn invalid_date_is_refused() {
        let mut p = valid();
        p.transaction_date = "14/3/2026".into();
        assert!(!check(&p, &accounts()).passed());
    }
}
