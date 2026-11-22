//! [`ReviewBackend`] mot Bokis databas (`bok-store`) och underlagsarkivet.
//!
//! Ett godkännande anropar `Store::approve` med granskaren som människa och
//! en signerare. I skarp drift är signeraren `ssh-keygen` med granskarens
//! hårdvarunyckel, så varje bokföring kräver en beröring av nyckeln. Lagret
//! tar numret ur serien i samma transaktion och kontrollerar att signaturen
//! kommer från en registrerad nyckel.
//!
//! Varningarna på sidan räknas fram här ur förslaget och dess härkomst:
//! saknat underlag, saknad motpart, hög osäkerhet och agentens frågor och
//! flaggor.

use std::collections::HashMap;
use std::path::Path;

use bok_archive::{media, Archive};
use bok_store::bok_core::signature::Signer;
use bok_store::{
    Actor, AttachmentVerdict, CaseKind, MatchTarget, ProposalCounterparty, ProposalRow, Store,
    StoreError, SuggestedMatch,
};

use crate::backend::*;

/// Lagret, arkivet, granskaren och nyckeln som signerar.
pub struct StoreReviewBackend {
    store: Store,
    archive: Archive,
    reviewer: Actor,
    signer: Box<dyn Signer>,
}

fn store_err(e: StoreError) -> ReviewError {
    match e {
        StoreError::Missing { .. } => ReviewError::NotFound(e.to_string()),
        StoreError::Sqlite(_)
        | StoreError::Io(_)
        | StoreError::Json(_)
        | StoreError::AlreadyExists(_)
        | StoreError::NotFound(_)
        | StoreError::NotABokisDatabase(_)
        | StoreError::NewerSchema { .. } => ReviewError::Internal(e.to_string()),
        other => ReviewError::Refused(other.to_string()),
    }
}

fn parse_id(id: &str, what: &str) -> Result<i64, ReviewError> {
    id.trim()
        .parse()
        .map_err(|_| ReviewError::NotFound(format!("{what} {id}")))
}

/// Motivering och klient ur det sparade anropet, om förslaget kom från MCP-servern.
fn from_raw(raw: Option<&str>) -> (String, Option<String>) {
    let Some(v) = raw.and_then(|r| serde_json::from_str::<serde_json::Value>(r).ok()) else {
        return (String::new(), None);
    };
    let h = &v["harkomst"];
    (
        h["motivering"].as_str().unwrap_or_default().to_string(),
        h["klient"].as_str().map(str::to_string),
    )
}

/// Motparten som granskningsvyn visar den.
#[derive(Debug, Default)]
struct Party {
    name: Option<String>,
    org_number: Option<String>,
    address: Option<String>,
}

impl StoreReviewBackend {
    pub fn new(store: Store, archive: Archive, reviewer: &str, signer: Box<dyn Signer>) -> Self {
        Self {
            store,
            archive,
            reviewer: Actor::human(reviewer),
            signer,
        }
    }

    /// Öppnar databasen och arkivet bredvid den (`underlag`). Skapar aldrig en databas.
    pub fn open(
        db: &Path,
        program_version: &str,
        reviewer: &str,
        signer: Box<dyn Signer>,
    ) -> Result<Self, ReviewError> {
        let store = Store::open(db, program_version).map_err(store_err)?;
        let dir = db
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default()
            .join("underlag");
        let archive = Archive::open(dir).map_err(|e| ReviewError::Internal(e.to_string()))?;
        Ok(Self::new(store, archive, reviewer, signer))
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    fn counterparty(&self, p: &ProposalRow) -> Result<Party, ReviewError> {
        Ok(match &p.counterparty {
            None => Party::default(),
            Some(ProposalCounterparty::Inline(c)) => Party {
                name: Some(c.name.clone()),
                org_number: c.org_number.clone(),
                address: c.address.clone(),
            },
            Some(ProposalCounterparty::Register(id)) => {
                let all = self.store.counterparties().map_err(store_err)?;
                match all.into_iter().find(|c| c.id == *id) {
                    Some(c) => Party {
                        name: Some(c.counterparty.name),
                        org_number: c.counterparty.org_number,
                        address: c.counterparty.address,
                    },
                    None => Party {
                        name: Some(format!("motpart {id}")),
                        ..Party::default()
                    },
                }
            }
        })
    }

    fn warnings(p: &ProposalRow, counterparty: Option<&str>) -> Vec<String> {
        let mut w = Vec::new();
        if p.attachments.is_empty() && p.corrects.is_none() {
            w.push("förslaget har inget underlag".to_string());
        }
        if counterparty.is_none() {
            w.push("motpart saknas".to_string());
        }
        if let Some(c) = p.provenance.confidence.filter(|c| *c <= 0.5) {
            w.push(format!(
                "agenten anger hög osäkerhet ({:.0} %)",
                (1.0 - c) * 100.0
            ));
        }
        if let Some(q) = p
            .provenance
            .questions
            .as_deref()
            .filter(|q| !q.trim().is_empty())
        {
            w.push(format!("agentens fråga: {q}"));
        }
        if let Some(f) = p
            .provenance
            .flags
            .as_deref()
            .filter(|f| !f.trim().is_empty())
        {
            w.push(format!("agentens flagga: {f}"));
        }
        w
    }

    fn summary(
        &self,
        p: &ProposalRow,
    ) -> Result<(ProposalSummary, Party, Vec<String>), ReviewError> {
        let party = self.counterparty(p)?;
        let counterparty = party.name.clone();
        let warnings = Self::warnings(p, counterparty.as_deref());
        let summary = ProposalSummary {
            id: p.id.to_string(),
            created_at: p.created_at.clone(),
            transaction_date: p.date.to_string(),
            description: p.description.clone(),
            counterparty,
            amount_ore: p.lines.iter().map(|l| l.debit.ore()).sum(),
            uncertainty: p.provenance.confidence.map(|c| (1.0 - c).clamp(0.0, 1.0)),
            warnings: warnings.len(),
        };
        Ok((summary, party, warnings))
    }

    fn file_name(row: &bok_store::AttachmentRow) -> String {
        row.original_name
            .clone()
            .unwrap_or_else(|| format!("underlag-{}", row.id))
    }
}

impl ReviewBackend for StoreReviewBackend {
    fn open_proposals(&self) -> Result<Vec<ProposalSummary>, ReviewError> {
        self.store
            .proposals(true)
            .map_err(store_err)?
            .iter()
            .map(|p| self.summary(p).map(|(s, _, _)| s))
            .collect()
    }

    fn proposal(&self, id: &str) -> Result<ProposalDetail, ReviewError> {
        let p = self
            .store
            .proposal(parse_id(id, "förslaget")?)
            .map_err(store_err)?;
        let (summary, party, warnings) = self.summary(&p)?;
        let names: HashMap<u16, String> = self
            .store
            .chart(p.date)
            .map_err(store_err)?
            .into_iter()
            .map(|a| (a.number.value(), a.name))
            .collect();
        let lines = p
            .lines
            .iter()
            .map(|l| ProposalLine {
                account: u32::from(l.account.value()),
                account_name: names
                    .get(&l.account.value())
                    .cloned()
                    .unwrap_or_else(|| "(saknas i kontoplanen)".to_string()),
                debit_ore: l.debit.ore(),
                credit_ore: l.credit.ore(),
                text: l.text.clone(),
            })
            .collect();
        let attachments = self.store.attachments().map_err(store_err)?;
        let mut documents = Vec::new();
        for a in &p.attachments {
            let Some(row) = attachments.iter().find(|r| r.id == *a) else {
                continue;
            };
            let name = Self::file_name(row);
            // Typen avgörs av innehållet, som vid visningen.
            let media_type = match self.archive.read(&row.sha256) {
                Ok(data) => media::detect(&data, &name),
                Err(_) => "application/octet-stream".to_string(),
            };
            documents.push(DocumentRef {
                id: row.id.to_string(),
                file_name: name,
                media_type,
            });
        }
        let corrects = match p.corrects {
            None => None,
            Some(voucher_id) => {
                let v = self.store.voucher(voucher_id).map_err(store_err)?;
                let year = self
                    .store
                    .fiscal_year(v.fiscal_year_id)
                    .map_err(store_err)?;
                Some(format!("{} ({})", v.reference, year.label()))
            }
        };
        let (rationale, client) = from_raw(p.provenance.raw_response.as_deref());
        let model = match &p.provenance.model {
            Some(m) => m.clone(),
            None => format!("ingen, lagt av {} ({})", p.actor, p.source),
        };
        Ok(ProposalDetail {
            summary,
            counterparty_org_number: party.org_number,
            counterparty_address: party.address,
            lines,
            documents,
            warnings,
            rationale: if rationale.is_empty() {
                p.correction_reason.clone().unwrap_or_default()
            } else {
                rationale
            },
            model,
            provider: p.provenance.provider.clone(),
            region: p.provenance.region.clone(),
            client,
            rule_ids: p
                .provenance
                .rule_version
                .as_deref()
                .map(|r| {
                    r.split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
            corrects,
        })
    }

    fn document(&self, id: &str) -> Result<(String, Vec<u8>), ReviewError> {
        let id = parse_id(id, "underlaget")?;
        let row = self
            .store
            .attachments()
            .map_err(store_err)?
            .into_iter()
            .find(|r| r.id == id)
            .ok_or_else(|| ReviewError::NotFound(format!("underlaget {id}")))?;
        let data = self
            .archive
            .read(&row.sha256)
            .map_err(|e| ReviewError::Internal(e.to_string()))?;
        Ok((media::detect(&data, &Self::file_name(&row)), data))
    }

    fn approve(&mut self, id: &str) -> Result<String, ReviewError> {
        let id = parse_id(id, "förslaget")?;
        let posted = self
            .store
            .approve(&self.reviewer, id, self.signer.as_ref())
            .map_err(store_err)?;
        let year = self
            .store
            .fiscal_year(posted.fiscal_year_id)
            .map_err(store_err)?;
        Ok(format!("{} ({})", posted.reference, year.label()))
    }

    fn reject(&mut self, id: &str, reason: &str) -> Result<(), ReviewError> {
        let id = parse_id(id, "förslaget")?;
        self.store
            .reject(&self.reviewer, id, reason)
            .map_err(store_err)
    }

    fn bank_rows(&self) -> Result<Vec<BankRowView>, ReviewError> {
        let mut out = Vec::new();
        for r in self.store.bank_rows(true).map_err(store_err)? {
            let mut candidates = Vec::new();
            for c in self
                .store
                .match_candidates(r.id, 14)
                .map_err(store_err)?
                .into_iter()
                .take(3)
            {
                let year = self
                    .store
                    .fiscal_year(c.fiscal_year_id)
                    .map_err(store_err)?;
                candidates.push(CandidateView {
                    voucher_id: c.voucher_id.to_string(),
                    reference: format!("{} ({})", c.reference, year.label()),
                    date: c.date.to_string(),
                    description: c.description,
                });
            }
            let mut suggestions = Vec::new();
            for sug in self
                .store
                .match_suggestions(Some(r.id))
                .map_err(store_err)?
            {
                let target = match &sug.target {
                    SuggestedMatch::Voucher(v) => {
                        format!(
                            "verifikation {}",
                            self.store.voucher(*v).map_err(store_err)?.reference
                        )
                    }
                    SuggestedMatch::Proposal(p) => {
                        match self
                            .store
                            .proposal(*p)
                            .map_err(store_err)?
                            .decision
                            .and_then(|d| d.voucher_id)
                        {
                            Some(v) => format!(
                                "förslag {p}, bokfört som {}",
                                self.store.voucher(v).map_err(store_err)?.reference
                            ),
                            None => format!("förslag {p}, inte bokfört än"),
                        }
                    }
                    SuggestedMatch::Explanation(t) => format!("förklaringen \"{t}\""),
                };
                suggestions.push(format!(
                    "{} föreslår {target}{}",
                    sug.actor,
                    sug.rationale
                        .as_deref()
                        .map(|r| format!(": {r}"))
                        .unwrap_or_default()
                ));
            }
            out.push(BankRowView {
                id: r.id.to_string(),
                date: r.booking_date.to_string(),
                amount_ore: r.amount.ore(),
                text: r.text,
                reference: r.reference,
                candidates,
                suggestions,
            });
        }
        Ok(out)
    }

    fn reconcile(
        &mut self,
        row_id: &str,
        voucher_id: Option<&str>,
        explanation: Option<&str>,
    ) -> Result<(), ReviewError> {
        let row = parse_id(row_id, "bankraden")?;
        let target = match (voucher_id, explanation) {
            (Some(v), None) => MatchTarget::Voucher(parse_id(v, "verifikationen")?),
            (None, Some(e)) => MatchTarget::Explanation(e.to_string()),
            _ => {
                return Err(ReviewError::Refused(
                    "ange antingen en verifikation eller en förklaring".into(),
                ))
            }
        };
        self.store
            .reconcile_bank_row(&self.reviewer, row, &target)
            .map_err(store_err)
    }

    fn cases(&self) -> Result<Vec<CaseView>, ReviewError> {
        Ok(self
            .store
            .cases(true)
            .map_err(store_err)?
            .into_iter()
            .map(|c| {
                let (kind, is_flag) = match &c.kind {
                    CaseKind::Question => ("Fråga".to_string(), false),
                    CaseKind::Flag { reason } => (format!("Flagga ({reason})"), true),
                };
                CaseView {
                    id: c.id.to_string(),
                    kind,
                    is_flag,
                    document_id: c.attachment_id.map(|a| a.to_string()),
                    text: c.text,
                    actor: c.actor,
                    created_at: c.created_at,
                }
            })
            .collect())
    }

    fn answer_case(
        &mut self,
        id: &str,
        answer: &str,
        mark: Option<&str>,
    ) -> Result<(), ReviewError> {
        let id = parse_id(id, "ärendet")?;
        let verdict = match mark {
            None => None,
            Some("dubblett") => Some(AttachmentVerdict::Duplicate),
            Some("ej_rakenskapsinformation") => Some(AttachmentVerdict::NotAccountingInformation),
            Some(other) => return Err(ReviewError::Refused(format!("okänt val {other}"))),
        };
        self.store
            .answer_case(&self.reviewer, id, answer, verdict)
            .map_err(store_err)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use bok_store::bok_core::signature::SoftwareSigner;
    use bok_store::bok_core::voucher::{Counterparty, Line};
    use bok_store::bok_core::{AccountNumber, Money, Series};
    use bok_store::{AccountChange, Channel, NewAttachment, NewBook, NewProposal, Provenance};
    use chrono::{Datelike, NaiveDate};

    const SEED: [u8; 32] = [5; 32];

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn acc(n: i64) -> AccountNumber {
        AccountNumber::new(n).unwrap()
    }

    /// En bokföring med ett väntande agentförslag på ett underlag.
    fn fixture() -> (tempfile::TempDir, StoreReviewBackend, String, String) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("bokis.db");
        let signer = SoftwareSigner::from_seed(SEED);
        let mut store = Store::create(
            &db,
            "bok-review-test",
            NewBook {
                company_name: "Testbolaget AB",
                org_number: "559999-9991",
                key_name: "granskare",
                public_key: &signer.public_key_openssh(),
                signer: &signer,
                security_level: bok_store::bok_core::security::SecurityLevel::Software,
            },
        )
        .unwrap();
        let human = Actor::human("granskare");
        let year = store.today().year();
        store
            .add_fiscal_year(&human, d(year, 1, 1), d(year, 12, 31))
            .unwrap();
        let chart: Vec<AccountChange> = [
            (1930, "Företagskonto"),
            (2440, "Leverantörsskulder"),
            (2641, "Debiterad ingående moms"),
            (5420, "Programvaror"),
        ]
        .into_iter()
        .map(|(n, name)| AccountChange {
            number: acc(n),
            name: name.into(),
            active: true,
            sru: None,
            vat_code: None,
        })
        .collect();
        store
            .change_chart(&human, d(year - 1, 1, 1), &chart, &signer)
            .unwrap();
        let archive = Archive::open(dir.path().join("underlag")).unwrap();
        let stored = archive.put_bytes(b"%PDF-1.7 faktura 2026-311").unwrap();
        let attachment = store
            .register_attachment(
                &Actor::system("inkorg"),
                &NewAttachment {
                    sha256: stored.sha256,
                    size: 25,
                    channel: Channel::Folder,
                    sender: None,
                    original_name: Some("faktura.jpg".into()),
                    file_type: Some("jpg".into()),
                },
            )
            .unwrap();
        let raw = serde_json::json!({"harkomst": {"motivering": "Molntjänst enligt regel R1.", "klient": "claude-code 2.1"}});
        let proposal = store
            .create_proposal(
                &Actor::agent("claude-opus-5-5"),
                &NewProposal {
                    series: Series::A,
                    date: store.today(),
                    description: "Molntjänst, Exempel Hosting AB".into(),
                    counterparty: Some(ProposalCounterparty::Inline(Counterparty {
                        name: "Exempel Hosting AB".into(),
                        org_number: Some("556000-0001".into()),
                        vat_number: None,
                        address: None,
                    })),
                    lines: vec![
                        Line::debit(acc(5420), Money::from_ore(40_000)),
                        Line::debit(acc(2641), Money::from_ore(10_000)),
                        Line::credit(acc(2440), Money::from_ore(50_000)),
                    ],
                    attachments: vec![attachment],
                    idempotency_key: None,
                    provenance: Provenance {
                        confidence: Some(0.9),
                        model: Some("claude-opus-5-5".into()),
                        provider: Some("anthropic".into()),
                        region: Some("eu".into()),
                        rule_version: Some("R1".into()),
                        raw_response: Some(raw.to_string()),
                        ..Provenance::default()
                    },
                },
            )
            .unwrap();
        let backend = StoreReviewBackend::new(
            store,
            archive,
            "granskare",
            Box::new(SoftwareSigner::from_seed(SEED)),
        );
        (dir, backend, proposal.to_string(), attachment.to_string())
    }

    #[test]
    fn detail_shows_provenance_names_and_content_type() {
        let (_dir, backend, id, attachment) = fixture();
        let open = backend.open_proposals().unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].amount_ore, 50_000);
        assert!((open[0].uncertainty.unwrap() - 0.1).abs() < 1e-9);
        let p = backend.proposal(&id).unwrap();
        assert_eq!(p.model, "claude-opus-5-5");
        assert_eq!(p.rationale, "Molntjänst enligt regel R1.");
        assert_eq!(p.client.as_deref(), Some("claude-code 2.1"));
        assert_eq!(p.rule_ids, vec!["R1".to_string()]);
        assert_eq!(p.counterparty_org_number.as_deref(), Some("556000-0001"));
        assert_eq!(p.lines[0].account_name, "Programvaror");
        assert!(p.warnings.is_empty(), "{:?}", p.warnings);
        // Filen heter .jpg men är en PDF; innehållet avgör.
        assert_eq!(p.documents[0].media_type, "application/pdf");
        let (media_type, data) = backend.document(&attachment).unwrap();
        assert_eq!(media_type, "application/pdf");
        assert!(data.starts_with(b"%PDF"));
        assert!(matches!(
            backend.document("999"),
            Err(ReviewError::NotFound(_))
        ));
        assert!(matches!(
            backend.proposal("x"),
            Err(ReviewError::NotFound(_))
        ));
    }

    #[test]
    fn approval_posts_with_a_number_and_signature() {
        let (_dir, mut backend, id, _) = fixture();
        let number = backend.approve(&id).unwrap();
        assert!(number.starts_with("A1 ("), "{number}");
        assert!(backend.open_proposals().unwrap().is_empty());
        let report = backend.store().verify().unwrap();
        assert!(
            report.is_ok(),
            "{:?}",
            report.problems().collect::<Vec<_>>()
        );
        assert!(matches!(backend.approve(&id), Err(ReviewError::Refused(_))));
        assert!(matches!(
            backend.reject(&id, "för sent"),
            Err(ReviewError::Refused(_))
        ));
    }

    #[test]
    fn unregistered_key_cannot_post() {
        let (dir, backend, id, _) = fixture();
        let StoreReviewBackend { store, archive, .. } = backend;
        let mut wrong = StoreReviewBackend::new(
            store,
            archive,
            "granskare",
            Box::new(SoftwareSigner::from_seed([6; 32])),
        );
        assert!(matches!(wrong.approve(&id), Err(ReviewError::Refused(_))));
        assert_eq!(
            wrong.open_proposals().unwrap().len(),
            1,
            "inget nummer förbrukat, förslaget väntar kvar"
        );
        drop(dir);
    }

    #[test]
    fn bank_rows_and_cases_through_the_review_backend() {
        use bok_store::{CaseKind, NewBankRow, NewCase};
        let (_dir, mut backend, id, attachment) = fixture();
        let posted = backend.approve(&id).unwrap();
        assert!(posted.starts_with("A1"));
        // A1 krediterar 2440; en betalning mot 1930 behövs för en kandidat.
        let chart = backend.store().chart(backend.store().today()).unwrap();
        assert!(chart.iter().any(|a| a.number.value() == 1930));
        let row = NewBankRow {
            post_id: "ef".repeat(32),
            bank_account: "SE4550000000058398257466".into(),
            booking_date: backend.store().today(),
            value_date: None,
            amount: Money::from_ore(-50_000),
            currency: "SEK".into(),
            text: "Exempel Hosting <betalning>".into(),
            reference: Some("2026311".into()),
            counterparty: None,
            balance: None,
        };
        let statement = attachment.parse::<i64>().unwrap();
        backend
            .store
            .import_bank_rows(&Actor::human("granskare"), statement, acc(1930), &[row])
            .unwrap();
        let rows = backend.bank_rows().unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].candidates.is_empty(), "A1 rör inte 1930");
        assert!(matches!(
            backend.reconcile(&rows[0].id, None, None),
            Err(ReviewError::Refused(_))
        ));
        backend
            .reconcile(&rows[0].id, None, Some("Betalt från privat konto, utlägg"))
            .unwrap();
        assert!(backend.bank_rows().unwrap().is_empty());

        let case = backend
            .store
            .open_case(
                &Actor::agent("modell"),
                &NewCase {
                    kind: CaseKind::Flag {
                        reason: "dubblett".into(),
                    },
                    attachment_id: Some(statement),
                    text: "Samma som förra månaden".into(),
                    model: None,
                    confidence: None,
                },
            )
            .unwrap();
        let cases = backend.cases().unwrap();
        assert_eq!((cases.len(), cases[0].is_flag), (1, true));
        assert!(matches!(
            backend.answer_case(&case.to_string(), "Nej", Some("spam")),
            Err(ReviewError::Refused(_))
        ));
        // Underlaget är bokfört (A1), så det kan inte markeras som dubblett.
        assert!(matches!(
            backend.answer_case(&case.to_string(), "Ja", Some("dubblett")),
            Err(ReviewError::Refused(_))
        ));
        backend
            .answer_case(&case.to_string(), "Nej, en ny faktura", None)
            .unwrap();
        assert!(backend.cases().unwrap().is_empty());
    }

    #[test]
    fn rejection_needs_a_reason() {
        let (_dir, mut backend, id, _) = fixture();
        assert!(matches!(
            backend.reject(&id, "  "),
            Err(ReviewError::Refused(_))
        ));
        backend.reject(&id, "fel konto, ska vara 6540").unwrap();
        assert!(backend.open_proposals().unwrap().is_empty());
    }

    #[test]
    fn warnings_from_provenance() {
        let (_dir, backend, id, _) = fixture();
        let mut p = backend.store().proposal(id.parse().unwrap()).unwrap();
        p.provenance.confidence = Some(0.3);
        p.provenance.questions = Some("Vilka deltog?".into());
        p.attachments.clear();
        let w = StoreReviewBackend::warnings(&p, None);
        assert_eq!(w.len(), 4, "{w:?}");
        assert!(w.iter().any(|x| x.contains("70 %")));
    }

    #[test]
    fn raw_response_parsing() {
        assert_eq!(from_raw(None), (String::new(), None));
        assert_eq!(from_raw(Some("inte json")), (String::new(), None));
        let (r, c) = from_raw(Some(r#"{"harkomst": {"motivering": "M", "klient": "K"}}"#));
        assert_eq!((r.as_str(), c.as_deref()), ("M", Some("K")));
    }
}
