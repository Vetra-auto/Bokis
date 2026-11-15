//! Poängsättning. Agenten svarar med en hel kontering, inte med ett konto och
//! en momskod som i benchens eget format. Därför härleds båda ur raderna:
//!
//! - **Konto**: det konto som bär störst belopp när avräknings- och
//!   momskonton räknats bort. Lika belopp avgörs av radordningen.
//! - **Momsbehandling**: vilka momskonton och underlagskonton som används och,
//!   för svensk moms, hur stor den avdragna momsen är i förhållande till
//!   transaktionens belopp.
//!
//! Kända begränsningar: beloppstaket vid representation kontrolleras inte,
//! bara att momsen är delvis avdragen och att ett representationskonto
//! används. Omvänd betalningsskyldighet inom Sverige räknas som byggtjänst,
//! eftersom det är det enda fallet i uppgifterna.

use serde::Serialize;

use bok_mcp::backend::ProposalLine;

use crate::record::Record;
use crate::tasks::Task;

/// Konton som bara avräknar eller redovisar och inte säger vad affärshändelsen
/// var: kundfordringar, kassa och bank, leverantörsskulder, moms,
/// motkontot för beräknad moms och öresutjämning.
pub fn is_settlement(account: u32) -> bool {
    matches!(account, 1510..=1519 | 1900..=1999 | 2440..=2449 | 2600..=2699 | 3740 | 4598)
}

fn is_representation(account: u32) -> bool {
    matches!(account, 6070..=6079 | 7630..=7639)
}

/// Konton för varor. Programvaror (5420) räknas inte hit, eftersom de oftast
/// köps som tjänst.
fn is_goods(account: u32) -> bool {
    matches!(account, 1200..=1299 | 1400..=1499 | 4000..=4399 | 5400..=5419 | 5460..=5469)
}

/// Konton med belopp som inte är avräkning, i den ordning de först förekommer,
/// med nettobelopp (debet minus kredit).
pub fn substantive(lines: &[ProposalLine]) -> Vec<(u32, i64)> {
    let mut out: Vec<(u32, i64)> = Vec::new();
    for l in lines.iter().filter(|l| !is_settlement(l.account)) {
        let net = l.debit_ore - l.credit_ore;
        match out.iter_mut().find(|(a, _)| *a == l.account) {
            Some((_, sum)) => *sum += net,
            None => out.push((l.account, net)),
        }
    }
    out.retain(|(_, net)| *net != 0);
    out
}

/// Kontot som bär störst belopp. Vid lika belopp vinner det som kom först.
pub fn primary_account(lines: &[ProposalLine]) -> Option<u32> {
    let mut best: Option<(u32, i64)> = None;
    for (account, net) in substantive(lines) {
        if best.is_none_or(|(_, b)| net.abs() > b) {
            best = Some((account, net.abs()));
        }
    }
    best.map(|(a, _)| a)
}

/// Momsbehandling med benchens namn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VatTreatment {
    #[serde(rename = "domestic_25")]
    Domestic25,
    #[serde(rename = "domestic_12")]
    Domestic12,
    #[serde(rename = "domestic_6")]
    Domestic6,
    #[serde(rename = "domestic_25_half_deduction")]
    Domestic25HalfDeduction,
    RepresentationCapped,
    NoVat,
    ReverseChargeServices,
    ReverseChargeGoods,
    ReverseChargeConstruction,
    Import,
    /// Momskontona passar ingen behandling.
    #[serde(rename = "oklar")]
    Unclear,
}

impl VatTreatment {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Domestic25 => "domestic_25",
            Self::Domestic12 => "domestic_12",
            Self::Domestic6 => "domestic_6",
            Self::Domestic25HalfDeduction => "domestic_25_half_deduction",
            Self::RepresentationCapped => "representation_capped",
            Self::NoVat => "no_vat",
            Self::ReverseChargeServices => "reverse_charge_services",
            Self::ReverseChargeGoods => "reverse_charge_goods",
            Self::ReverseChargeConstruction => "reverse_charge_construction",
            Self::Import => "import",
            Self::Unclear => "oklar",
        }
    }
}

/// Härleder momsbehandlingen ur konteringen. `gross_ore` är transaktionens
/// belopp utan tecken.
pub fn classify_vat(lines: &[ProposalLine], gross_ore: i64) -> VatTreatment {
    let used = |pred: fn(u32) -> bool| {
        lines
            .iter()
            .any(|l| pred(l.account) && (l.debit_ore != 0 || l.credit_ore != 0))
    };
    if used(|a| matches!(a, 2615 | 2625 | 2635 | 4545..=4547 | 4085..=4087)) {
        return VatTreatment::Import;
    }
    if used(|a| matches!(a, 2617 | 2627 | 2637 | 2647 | 4415..=4417 | 4425..=4427 | 4065..=4067)) {
        return VatTreatment::ReverseChargeConstruction;
    }
    if used(|a| matches!(a, 2614 | 2624 | 2634 | 2645 | 4510..=4518 | 4530..=4538 | 4070..=4078)) {
        let goods = used(|a| matches!(a, 4510..=4518 | 4070..=4078))
            || (!used(|a| matches!(a, 4530..=4538))
                && primary_account(lines).is_some_and(is_goods));
        return if goods {
            VatTreatment::ReverseChargeGoods
        } else {
            VatTreatment::ReverseChargeServices
        };
    }
    let input_vat: i64 = lines
        .iter()
        .filter(|l| matches!(l.account, 2640..=2642 | 2646 | 2649))
        .map(|l| l.debit_ore - l.credit_ore)
        .sum::<i64>()
        .abs();
    if !used(|a| (2600..=2699).contains(&a)) {
        return VatTreatment::NoVat;
    }
    if input_vat == 0
        || gross_ore <= 0
        || used(|a| (2600..=2639).contains(&a) || (2650..=2699).contains(&a))
    {
        return VatTreatment::Unclear;
    }
    let representation = substantive(lines)
        .iter()
        .any(|(a, _)| is_representation(*a));
    // Avdragen moms som andel av bruttobeloppet: 25/125, 12/112, 6/106 och
    // halva 25 % = 12,5/125. Marginalen är 0,2 % av beloppet, minst 2 öre.
    let tolerance = (gross_ore / 500).max(2);
    let candidates = [
        (VatTreatment::Domestic25, 25, 125),
        (VatTreatment::Domestic12, 12, 112),
        (VatTreatment::Domestic6, 6, 106),
        (VatTreatment::Domestic25HalfDeduction, 1, 10),
    ];
    for (treatment, num, den) in candidates {
        if representation && treatment == VatTreatment::Domestic25HalfDeduction {
            continue;
        }
        let expected = (gross_ore * num + den / 2) / den;
        if (input_vat - expected).abs() <= tolerance {
            return treatment;
        }
    }
    if representation {
        VatTreatment::RepresentationCapped
    } else {
        VatTreatment::Unclear
    }
}

/// Hur uppgiften besvarades.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Status {
    #[serde(rename = "besvarad")]
    Answered,
    /// Agenten ställde en fråga men lade inget förslag.
    #[serde(rename = "bara_fraga")]
    QuestionOnly,
    /// Agenten flaggade underlaget men lade inget förslag.
    #[serde(rename = "bara_flagga")]
    FlagOnly,
    #[serde(rename = "saknas")]
    Missing,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TaskResult {
    pub id: String,
    #[serde(rename = "svarighetsgrad")]
    pub difficulty: String,
    pub status: Status,
    #[serde(rename = "forslag_id")]
    pub proposal_id: Option<String>,
    /// Antal förslag på uppgiften. Det senaste räknas.
    #[serde(rename = "antal_forslag")]
    pub proposals: usize,
    #[serde(rename = "konto")]
    pub account: Option<u32>,
    #[serde(rename = "ovriga_konton")]
    pub other_accounts: Vec<u32>,
    #[serde(rename = "moms")]
    pub vat: Option<VatTreatment>,
    #[serde(rename = "facit_konto")]
    pub gold_account: String,
    #[serde(rename = "facit_godtar")]
    pub gold_acceptable: Vec<String>,
    #[serde(rename = "facit_moms")]
    pub gold_vat: String,
    #[serde(rename = "konto_ratt")]
    pub account_correct: bool,
    #[serde(rename = "exakt_konto")]
    pub exact_account: bool,
    #[serde(rename = "moms_ratt")]
    pub vat_correct: bool,
    #[serde(rename = "godkand")]
    pub pass: bool,
    #[serde(rename = "osakerhet")]
    pub uncertainty: Option<f64>,
    #[serde(rename = "modell")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DifficultyRow {
    #[serde(rename = "svarighetsgrad")]
    pub difficulty: String,
    #[serde(rename = "uppgifter")]
    pub tasks: usize,
    #[serde(rename = "godkanda")]
    pub passed: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Summary {
    #[serde(rename = "uppgifter")]
    pub tasks: usize,
    #[serde(rename = "besvarade")]
    pub answered: usize,
    #[serde(rename = "godkanda")]
    pub passed: usize,
    #[serde(rename = "konto_ratt")]
    pub account_correct: usize,
    #[serde(rename = "exakt_konto")]
    pub exact_account: usize,
    #[serde(rename = "moms_ratt")]
    pub vat_correct: usize,
    #[serde(rename = "per_svarighetsgrad")]
    pub by_difficulty: Vec<DifficultyRow>,
    /// Medelosäkerhet för godkända respektive underkända svar.
    #[serde(rename = "osakerhet_godkanda")]
    pub mean_uncertainty_passed: Option<f64>,
    #[serde(rename = "osakerhet_underkanda")]
    pub mean_uncertainty_failed: Option<f64>,
    /// Förslag i loggen som inte hänvisar till någon av uppgifterna.
    #[serde(rename = "forslag_utan_uppgift")]
    pub unmatched_proposals: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    #[serde(rename = "sammanfattning")]
    pub summary: Summary,
    #[serde(rename = "uppgifter")]
    pub results: Vec<TaskResult>,
}

fn mean(values: impl Iterator<Item = f64>) -> Option<f64> {
    let (sum, n) = values.fold((0.0, 0usize), |(s, n), v| (s + v, n + 1));
    (n > 0).then(|| sum / n as f64)
}

/// Poängsätter uppgifterna mot loggen. Det senaste förslaget för en uppgift
/// räknas, så en ny körning kan läggas till i samma logg.
pub fn score(tasks: &[Task], records: &[Record]) -> Report {
    let mut results = Vec::with_capacity(tasks.len());
    for task in tasks {
        let refers = |ids: &[String]| ids.iter().any(|i| i == &task.id);
        let vouchers: Vec<&Record> = records
            .iter()
            .filter(
                |r| matches!(r, Record::Voucher { proposal, .. } if refers(&proposal.document_ids)),
            )
            .collect();
        let asked = records
            .iter()
            .any(|r| matches!(r, Record::Question { document: Some(d), .. } if d == &task.id));
        let flagged = records
            .iter()
            .any(|r| matches!(r, Record::Flag { document, .. } if document == &task.id));
        let gold = &task.gold;
        let mut result = TaskResult {
            id: task.id.clone(),
            difficulty: task.difficulty.clone(),
            status: if !vouchers.is_empty() {
                Status::Answered
            } else if asked {
                Status::QuestionOnly
            } else if flagged {
                Status::FlagOnly
            } else {
                Status::Missing
            },
            proposal_id: None,
            proposals: vouchers.len(),
            account: None,
            other_accounts: Vec::new(),
            vat: None,
            gold_account: gold.account.clone(),
            gold_acceptable: gold.acceptable_accounts.clone(),
            gold_vat: gold.vat_treatment.clone(),
            account_correct: false,
            exact_account: false,
            vat_correct: false,
            pass: false,
            uncertainty: None,
            model: None,
        };
        if let Some(Record::Voucher {
            id,
            proposal,
            provenance,
            ..
        }) = vouchers.last()
        {
            let account = primary_account(&proposal.lines);
            let vat = classify_vat(&proposal.lines, task.amount_ore().abs());
            result.proposal_id = Some(id.clone());
            result.other_accounts = substantive(&proposal.lines)
                .into_iter()
                .map(|(a, _)| a)
                .filter(|a| Some(*a) != account)
                .collect();
            result.account_correct = account.is_some_and(|a| gold.accepts(a));
            result.exact_account = account.is_some_and(|a| gold.account == a.to_string());
            result.vat_correct = vat.as_str() == gold.vat_treatment;
            result.pass = result.account_correct && result.vat_correct;
            result.account = account;
            result.vat = Some(vat);
            result.uncertainty = Some(provenance.uncertainty);
            result.model = Some(provenance.model.clone());
        }
        results.push(result);
    }

    let unmatched_proposals = records
        .iter()
        .filter(|r| match r {
            Record::Voucher { proposal, .. } => !proposal
                .document_ids
                .iter()
                .any(|d| tasks.iter().any(|t| &t.id == d)),
            _ => false,
        })
        .count();
    let mut by_difficulty: Vec<DifficultyRow> = Vec::new();
    for r in &results {
        let row = match by_difficulty
            .iter_mut()
            .find(|d| d.difficulty == r.difficulty)
        {
            Some(row) => row,
            None => {
                by_difficulty.push(DifficultyRow {
                    difficulty: r.difficulty.clone(),
                    tasks: 0,
                    passed: 0,
                });
                let last = by_difficulty.len() - 1;
                &mut by_difficulty[last]
            }
        };
        row.tasks += 1;
        row.passed += usize::from(r.pass);
    }
    by_difficulty.sort_by(|a, b| a.difficulty.cmp(&b.difficulty));
    let summary = Summary {
        tasks: results.len(),
        answered: results
            .iter()
            .filter(|r| r.status == Status::Answered)
            .count(),
        passed: results.iter().filter(|r| r.pass).count(),
        account_correct: results.iter().filter(|r| r.account_correct).count(),
        exact_account: results.iter().filter(|r| r.exact_account).count(),
        vat_correct: results.iter().filter(|r| r.vat_correct).count(),
        by_difficulty,
        mean_uncertainty_passed: mean(
            results
                .iter()
                .filter(|r| r.pass)
                .filter_map(|r| r.uncertainty),
        ),
        mean_uncertainty_failed: mean(
            results
                .iter()
                .filter(|r| !r.pass && r.status == Status::Answered)
                .filter_map(|r| r.uncertainty),
        ),
        unmatched_proposals,
    };
    Report { summary, results }
}

fn percent(part: usize, whole: usize) -> String {
    if whole == 0 {
        return "–".into();
    }
    format!("{:.0} %", part as f64 * 100.0 / whole as f64)
}

/// Rapporten som text.
pub fn render_text(report: &Report) -> String {
    let s = &report.summary;
    let mut out = format!(
        "Uppgifter: {}  besvarade: {}\n\
         Godkända (konto och moms rätt): {} ({})\n\
         Konto rätt: {} ({}), exakt facitkonto: {}\n\
         Moms rätt: {} ({})\n",
        s.tasks,
        s.answered,
        s.passed,
        percent(s.passed, s.tasks),
        s.account_correct,
        percent(s.account_correct, s.tasks),
        s.exact_account,
        s.vat_correct,
        percent(s.vat_correct, s.tasks),
    );
    for d in &s.by_difficulty {
        out.push_str(&format!(
            "  {}: {} av {} ({})\n",
            d.difficulty,
            d.passed,
            d.tasks,
            percent(d.passed, d.tasks)
        ));
    }
    let fmt = |u: Option<f64>| u.map_or("–".to_string(), |u| format!("{:.0} %", u * 100.0));
    out.push_str(&format!(
        "Agentens osäkerhet i snitt: godkända {}, underkända {}\n",
        fmt(s.mean_uncertainty_passed),
        fmt(s.mean_uncertainty_failed)
    ));
    if s.unmatched_proposals > 0 {
        out.push_str(&format!(
            "Förslag utan uppgift i loggen: {}\n",
            s.unmatched_proposals
        ));
    }
    let failed: Vec<&TaskResult> = report.results.iter().filter(|r| !r.pass).collect();
    if !failed.is_empty() {
        out.push_str("\nEj godkända\n");
        for r in failed {
            let got = match r.status {
                Status::Answered => format!(
                    "{} {}",
                    r.account.map_or("–".to_string(), |a| a.to_string()),
                    r.vat.map_or("–", VatTreatment::as_str)
                ),
                Status::QuestionOnly => "bara en fråga".into(),
                Status::FlagOnly => "bara en flagga".into(),
                Status::Missing => "inget förslag".into(),
            };
            let accepted = if r.gold_acceptable.is_empty() {
                r.gold_account.clone()
            } else {
                format!(
                    "{} (eller {})",
                    r.gold_account,
                    r.gold_acceptable.join(", ")
                )
            };
            out.push_str(&format!(
                "  {}: svar {got}; facit {accepted} {}\n",
                r.id, r.gold_vat
            ));
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::record::tests::{provenance, voucher};
    use crate::tasks::tests::SAMPLE;

    fn l(account: u32, debit: i64, credit: i64) -> ProposalLine {
        ProposalLine {
            account,
            debit_ore: debit,
            credit_ore: credit,
            text: None,
        }
    }

    #[test]
    fn primary_account_skips_settlement_and_breaks_ties_by_order() {
        let lines = [
            l(6530, 500_000, 0),
            l(2641, 125_000, 0),
            l(1930, 0, 625_000),
        ];
        assert_eq!(primary_account(&lines), Some(6530));
        let reverse = [
            l(4535, 135_200, 0),
            l(4598, 0, 135_200),
            l(5420, 135_200, 0),
            l(1930, 0, 135_200),
        ];
        assert_eq!(primary_account(&reverse), Some(4535));
        let representation = [
            l(6071, 60_000, 0),
            l(6072, 20_400, 0),
            l(2641, 9_200, 0),
            l(1930, 0, 89_600),
        ];
        assert_eq!(primary_account(&representation), Some(6071));
        assert_eq!(primary_account(&[l(2440, 100, 0), l(1930, 0, 100)]), None);
    }

    #[test]
    fn domestic_rates() {
        let v = |vat: i64, gross: i64| {
            classify_vat(
                &[l(6530, gross - vat, 0), l(2641, vat, 0), l(1930, 0, gross)],
                gross,
            )
        };
        assert_eq!(v(125_000, 625_000), VatTreatment::Domestic25);
        assert_eq!(v(10_714, 100_000), VatTreatment::Domestic12);
        assert_eq!(v(5_660, 100_000), VatTreatment::Domestic6);
        assert_eq!(v(56_250, 562_500), VatTreatment::Domestic25HalfDeduction);
        assert_eq!(v(10_000, 100_000), VatTreatment::Domestic25HalfDeduction);
        assert_eq!(v(10_714, 100_000), VatTreatment::Domestic12);
        assert_eq!(v(3_000, 100_000), VatTreatment::Unclear);
    }

    #[test]
    fn credit_note_is_domestic_25() {
        let lines = [l(1930, 312_500, 0), l(5410, 0, 250_000), l(2641, 0, 62_500)];
        assert_eq!(classify_vat(&lines, 312_500), VatTreatment::Domestic25);
        assert_eq!(primary_account(&lines), Some(5410));
    }

    #[test]
    fn representation_with_partial_deduction() {
        let capped = [
            l(6071, 60_000, 0),
            l(6072, 20_400, 0),
            l(2641, 9_200, 0),
            l(1930, 0, 89_600),
        ];
        assert_eq!(
            classify_vat(&capped, 89_600),
            VatTreatment::RepresentationCapped
        );
        let full_12 = [l(6071, 80_000, 0), l(2641, 9_600, 0), l(1930, 0, 89_600)];
        assert_eq!(classify_vat(&full_12, 89_600), VatTreatment::Domestic12);
    }

    #[test]
    fn foreign_and_domestic_reverse_charge_and_import() {
        let services = [
            l(5420, 135_200, 0),
            l(1930, 0, 135_200),
            l(2645, 33_800, 0),
            l(2614, 0, 33_800),
        ];
        assert_eq!(
            classify_vat(&services, 135_200),
            VatTreatment::ReverseChargeServices
        );
        let goods = [
            l(4010, 2_315_000, 0),
            l(1930, 0, 2_315_000),
            l(2645, 578_750, 0),
            l(2614, 0, 578_750),
        ];
        assert_eq!(
            classify_vat(&goods, 2_315_000),
            VatTreatment::ReverseChargeGoods
        );
        let goods_marker = [
            l(4515, 100, 0),
            l(4598, 0, 100),
            l(6540, 100, 0),
            l(2645, 25, 0),
            l(2614, 0, 25),
            l(1930, 0, 100),
        ];
        assert_eq!(
            classify_vat(&goods_marker, 100),
            VatTreatment::ReverseChargeGoods
        );
        let construction = [
            l(4610, 6_250_000, 0),
            l(1930, 0, 6_250_000),
            l(2647, 1_562_500, 0),
            l(2614, 0, 1_562_500),
        ];
        assert_eq!(
            classify_vat(&construction, 6_250_000),
            VatTreatment::ReverseChargeConstruction
        );
        let import = [
            l(4010, 3_240_000, 0),
            l(1930, 0, 3_240_000),
            l(2645, 810_000, 0),
            l(2615, 0, 810_000),
        ];
        assert_eq!(classify_vat(&import, 3_240_000), VatTreatment::Import);
    }

    #[test]
    fn no_vat_and_unclear() {
        assert_eq!(
            classify_vat(&[l(2013, 1_500_000, 0), l(1930, 0, 1_500_000)], 1_500_000),
            VatTreatment::NoVat
        );
        let sale = [l(1930, 125, 0), l(3001, 0, 100), l(2611, 0, 25)];
        assert_eq!(classify_vat(&sale, 125), VatTreatment::Unclear);
        assert_eq!(
            VatTreatment::Domestic25HalfDeduction.as_str(),
            "domestic_25_half_deduction"
        );
        assert_eq!(
            serde_json::to_value(VatTreatment::Unclear).unwrap(),
            "oklar"
        );
        // JSON-utdata ska använda samma namn som facit och as_str.
        for t in [
            VatTreatment::Domestic25,
            VatTreatment::Domestic12,
            VatTreatment::Domestic6,
            VatTreatment::NoVat,
        ] {
            assert_eq!(serde_json::to_value(t).unwrap(), t.as_str());
        }
        assert_eq!(
            serde_json::to_value(VatTreatment::Domestic25HalfDeduction).unwrap(),
            "domestic_25_half_deduction"
        );
    }

    #[test]
    fn scores_latest_proposal_and_counts_statuses() {
        let tasks: Vec<Task> = serde_json::from_str(SAMPLE).unwrap();
        let tasks = &tasks[..2];
        let wrong = voucher(
            "F1",
            "booking-001",
            vec![
                l(6550, 500_000, 0),
                l(2641, 125_000, 0),
                l(1930, 0, 625_000),
            ],
            0.6,
        );
        let right = voucher(
            "F2",
            "booking-001",
            vec![
                l(6530, 500_000, 0),
                l(2641, 125_000, 0),
                l(1930, 0, 625_000),
            ],
            0.1,
        );
        let stray = voucher(
            "F3",
            "booking-999",
            vec![l(6530, 100, 0), l(1930, 0, 100)],
            0.5,
        );
        let question = Record::Question {
            id: "Q4".into(),
            at: "2026-11-14T02:00:00Z".into(),
            document: Some("booking-101".into()),
            question: "Har ni ett momsregistreringsnummer i EU?".into(),
            provenance: provenance(0.9),
        };
        let report = score(tasks, &[wrong, right, stray, question]);
        let s = &report.summary;
        assert_eq!(
            (s.tasks, s.answered, s.passed, s.unmatched_proposals),
            (2, 1, 1, 1)
        );
        let first = &report.results[0];
        assert_eq!(
            (first.proposals, first.account, first.vat),
            (2, Some(6530), Some(VatTreatment::Domestic25))
        );
        assert!(first.pass && first.exact_account);
        assert_eq!(first.proposal_id.as_deref(), Some("F2"));
        assert_eq!(report.results[1].status, Status::QuestionOnly);
        assert_eq!(s.mean_uncertainty_passed, Some(0.1));
        assert_eq!(s.mean_uncertainty_failed, None);
        let text = render_text(&report);
        assert!(text.contains("Godkända (konto och moms rätt): 1 (50 %)"));
        assert!(text.contains("booking-101: svar bara en fråga; facit 5420 (eller 6540, 4535) reverse_charge_services"));
        assert!(text.contains("Förslag utan uppgift i loggen: 1"));
    }
}
