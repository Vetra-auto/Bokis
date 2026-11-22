//! Loggen över vad agenten gjorde under en körning, en JSON-rad per ärende.
//!
//! [`RecordingBackend`] skickar allt vidare till ett annat lager och skriver
//! varje förslag, matchning, fråga och flagga till loggen när lagret har tagit
//! emot det. Loggen öppnas för tillägg, så flera körningar kan samlas i samma
//! fil. Id:n räknas om från början i varje körning; ordningen i filen avgör
//! vilket förslag som är senast.

use std::fs::File;
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use bok_mcp::backend::*;

use crate::EvalError;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "typ")]
pub enum Record {
    #[serde(rename = "verifikation")]
    Voucher {
        id: String,
        #[serde(rename = "tid")]
        at: String,
        #[serde(rename = "forslag")]
        proposal: Box<Proposal>,
        #[serde(rename = "harkomst")]
        provenance: Provenance,
    },
    #[serde(rename = "matchning")]
    Match {
        id: String,
        #[serde(rename = "tid")]
        at: String,
        #[serde(rename = "bankrad")]
        bank_row: String,
        #[serde(rename = "verifikation")]
        voucher: String,
        #[serde(rename = "harkomst")]
        provenance: Provenance,
    },
    #[serde(rename = "fraga")]
    Question {
        id: String,
        #[serde(rename = "tid")]
        at: String,
        #[serde(rename = "underlag")]
        document: Option<String>,
        #[serde(rename = "fraga")]
        question: String,
        #[serde(rename = "harkomst")]
        provenance: Provenance,
    },
    #[serde(rename = "flagga")]
    Flag {
        id: String,
        #[serde(rename = "tid")]
        at: String,
        #[serde(rename = "underlag")]
        document: String,
        #[serde(rename = "orsak")]
        reason: String,
        #[serde(rename = "kommentar")]
        comment: String,
        #[serde(rename = "harkomst")]
        provenance: Provenance,
    },
}

/// Läser en logg. Tomma rader hoppas över.
pub fn parse_records(text: &str) -> Result<Vec<Record>, EvalError> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(i, l)| {
            serde_json::from_str(l).map_err(|e| EvalError::Records {
                line: i + 1,
                message: e.to_string(),
            })
        })
        .collect()
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Ett lager som loggar allt agenten lägger.
pub struct RecordingBackend<B> {
    inner: B,
    log: File,
}

impl<B: AgentBackend> RecordingBackend<B> {
    /// Öppnar loggen för tillägg och skapar den om den saknas.
    pub fn open(inner: B, log: &Path) -> Result<Self, EvalError> {
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .map_err(|e| EvalError::Io(log.to_path_buf(), e))?;
        Ok(Self { inner, log })
    }

    pub fn inner(&self) -> &B {
        &self.inner
    }

    fn write(&mut self, record: &Record) -> Result<(), BackendError> {
        let mut line =
            serde_json::to_string(record).map_err(|e| BackendError::Internal(e.to_string()))?;
        line.push('\n');
        self.log
            .write_all(line.as_bytes())
            .and_then(|()| self.log.flush())
            .map_err(|e| {
                BackendError::Internal(format!("kunde inte skriva till utvärderingsloggen: {e}"))
            })
    }
}

impl<B: AgentBackend> AgentBackend for RecordingBackend<B> {
    fn list_documents(
        &self,
        only_open: bool,
        max: usize,
    ) -> Result<Vec<DocumentInfo>, BackendError> {
        self.inner.list_documents(only_open, max)
    }

    fn get_document(&self, id: &str) -> Result<Document, BackendError> {
        self.inner.get_document(id)
    }

    fn accounts(&self) -> Result<Vec<Account>, BackendError> {
        self.inner.accounts()
    }

    fn rules(&self) -> Result<Vec<Rule>, BackendError> {
        self.inner.rules()
    }

    fn search_counterparty(
        &self,
        query: &str,
        max: usize,
    ) -> Result<Vec<PastPosting>, BackendError> {
        self.inner.search_counterparty(query, max)
    }

    fn unmatched_bank_rows(&self, max: usize) -> Result<Vec<BankRow>, BackendError> {
        self.inner.unmatched_bank_rows(max)
    }

    fn propose_voucher(
        &mut self,
        proposal: Proposal,
        provenance: Provenance,
    ) -> Result<Receipt, BackendError> {
        let receipt = self
            .inner
            .propose_voucher(proposal.clone(), provenance.clone())?;
        self.write(&Record::Voucher {
            id: receipt.id.clone(),
            at: now(),
            proposal: Box::new(proposal),
            provenance,
        })?;
        Ok(receipt)
    }

    fn propose_match(
        &mut self,
        bank_row_id: &str,
        voucher: &str,
        provenance: Provenance,
    ) -> Result<Receipt, BackendError> {
        let receipt = self
            .inner
            .propose_match(bank_row_id, voucher, provenance.clone())?;
        self.write(&Record::Match {
            id: receipt.id.clone(),
            at: now(),
            bank_row: bank_row_id.into(),
            voucher: voucher.into(),
            provenance,
        })?;
        Ok(receipt)
    }

    fn ask_question(
        &mut self,
        document_id: Option<&str>,
        question: &str,
        provenance: Provenance,
    ) -> Result<Receipt, BackendError> {
        let receipt = self
            .inner
            .ask_question(document_id, question, provenance.clone())?;
        self.write(&Record::Question {
            id: receipt.id.clone(),
            at: now(),
            document: document_id.map(str::to_string),
            question: question.into(),
            provenance,
        })?;
        Ok(receipt)
    }

    fn flag_document(
        &mut self,
        document_id: &str,
        reason: &str,
        comment: &str,
        provenance: Provenance,
    ) -> Result<Receipt, BackendError> {
        let receipt = self
            .inner
            .flag_document(document_id, reason, comment, provenance.clone())?;
        self.write(&Record::Flag {
            id: receipt.id.clone(),
            at: now(),
            document: document_id.into(),
            reason: reason.into(),
            comment: comment.into(),
            provenance,
        })?;
        Ok(receipt)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(crate) mod tests {
    use super::*;
    use bok_mcp::memory::MemoryBackend;

    pub(crate) fn provenance(uncertainty: f64) -> Provenance {
        Provenance {
            model: "testmodell".into(),
            provider: None,
            region: None,
            client: None,
            rationale: "test".into(),
            uncertainty,
            arguments_sha256: "0".repeat(64),
        }
    }

    pub(crate) fn voucher(
        id: &str,
        document: &str,
        lines: Vec<ProposalLine>,
        uncertainty: f64,
    ) -> Record {
        Record::Voucher {
            id: id.into(),
            at: "2026-11-14T02:00:00Z".into(),
            proposal: Box::new(Proposal {
                transaction_date: "2026-05-12".into(),
                description: "Test".into(),
                counterparty: None,
                lines,
                document_ids: vec![document.into()],
                bank_row_ids: vec![],
                rule_ids: vec![],
                corrects: None,
            }),
            provenance: provenance(uncertainty),
        }
    }

    #[test]
    fn records_round_trip_with_swedish_tags() {
        let r = voucher(
            "F1",
            "booking-001",
            vec![ProposalLine {
                account: 6530,
                debit_ore: 100,
                credit_ore: 0,
                text: None,
            }],
            0.2,
        );
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.starts_with("{\"typ\":\"verifikation\",\"id\":\"F1\",\"tid\":"));
        assert!(json.contains("\"underlag_ids\":[\"booking-001\"]"));
        let back = parse_records(&format!("{json}\n\n{json}\n")).unwrap();
        assert_eq!(back, vec![r.clone(), r]);
        assert!(matches!(
            parse_records("{\"typ\":\"okand\"}"),
            Err(EvalError::Records { line: 1, .. })
        ));
    }

    #[test]
    fn appends_accepted_cases_and_skips_refused_ones() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("forslag.jsonl");
        std::fs::write(&log, "").unwrap();
        let mut backend = RecordingBackend::open(MemoryBackend::demo(), &log).unwrap();
        let Record::Voucher { proposal, .. } = voucher("x", "INK-1", vec![], 0.2) else {
            unreachable!()
        };
        backend
            .propose_voucher(*proposal.clone(), provenance(0.2))
            .unwrap();
        let mut missing = proposal;
        missing.document_ids = vec!["INK-404".into()];
        assert!(backend.propose_voucher(*missing, provenance(0.2)).is_err());
        backend
            .ask_question(Some("INK-2"), "Vilka deltog?", provenance(0.8))
            .unwrap();
        backend
            .flag_document("INK-3", "dubblett", "Samma som INK-1", provenance(0.1))
            .unwrap();
        drop(backend);

        let mut again = RecordingBackend::open(MemoryBackend::demo(), &log).unwrap();
        again.propose_match("b1", "A7", provenance(0.3)).unwrap();
        assert_eq!(again.inner().cases.len(), 1);

        let records = parse_records(&std::fs::read_to_string(&log).unwrap()).unwrap();
        let kinds: Vec<&str> = records
            .iter()
            .map(|r| match r {
                Record::Voucher { .. } => "verifikation",
                Record::Match { .. } => "matchning",
                Record::Question { .. } => "fraga",
                Record::Flag { .. } => "flagga",
            })
            .collect();
        assert_eq!(kinds, vec!["verifikation", "fraga", "flagga", "matchning"]);
    }
}
