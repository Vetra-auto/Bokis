//! Ett lager i minnet, för tester och för att prova servern med en riktig
//! MCP-klient (`bok-mcp --demo`) innan den kopplas till Bokis databas.

use base64::Engine;

use crate::backend::*;

/// Ett förslag eller annat ärende som agenten har lagt.
#[derive(Debug, Clone)]
pub enum Case {
    Voucher(Proposal, Provenance),
    Match {
        bank_row: String,
        voucher: String,
        provenance: Provenance,
    },
    Question {
        document: Option<String>,
        question: String,
        provenance: Provenance,
    },
    Flag {
        document: String,
        reason: String,
        comment: String,
        provenance: Provenance,
    },
}

/// Lager i minnet.
#[derive(Debug, Default, Clone)]
pub struct MemoryBackend {
    pub documents: Vec<Document>,
    pub account_list: Vec<Account>,
    pub rule_list: Vec<Rule>,
    pub history: Vec<(String, PastPosting)>,
    pub bank_rows: Vec<BankRow>,
    pub cases: Vec<(String, Case)>,
}

const PNG_1X1: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=";

fn account(number: u32, name: &str, kind: &str) -> Account {
    Account {
        number,
        name: name.into(),
        kind: kind.into(),
        active: true,
    }
}

fn info(id: &str, file_name: &str, media_type: &str, data: &[u8]) -> DocumentInfo {
    DocumentInfo {
        id: id.into(),
        sha256: crate::sha256_hex(data),
        file_name: file_name.into(),
        media_type: media_type.into(),
        size: data.len() as u64,
        received_at: "2026-03-16T08:12:00Z".into(),
        channel: "mapp".into(),
        sender: None,
        status: "oppen".into(),
    }
}

fn line(account: u32, debit_ore: i64, credit_ore: i64) -> ProposalLine {
    ProposalLine {
        account,
        debit_ore,
        credit_ore,
        text: None,
    }
}

impl MemoryBackend {
    /// Exempeldata: tre underlag, en liten kontoplan, tre regler, en tidigare
    /// kontering och en omatchad bankrad.
    pub fn demo() -> Self {
        let text =
            "Exempel Hosting AB\norg.nr 556000-0000\nFaktura 2026-311, fakturadatum 2026-03-14\n\
                    Molntjänst mars 400,00 kr\nMoms 25 % 100,00 kr\nAtt betala 500,00 kr\n"
                .as_bytes()
                .to_vec();
        let png = base64::engine::general_purpose::STANDARD
            .decode(PNG_1X1)
            .unwrap_or_default();
        let pdf = b"%PDF-1.4\n% Exempel: Telia mars\n%%EOF\n".to_vec();
        let documents = vec![
            Document {
                info: info("INK-1", "faktura-2026-311.txt", "text/plain", &text),
                data: text,
            },
            Document {
                info: info("INK-2", "kvitto-lunch.png", "image/png", &png),
                data: png,
            },
            Document {
                info: info("INK-3", "telia-mars.pdf", "application/pdf", &pdf),
                data: pdf,
            },
        ];
        let account_list = vec![
            account(1930, "Företagskonto", "tillgang"),
            account(2440, "Leverantörsskulder", "skuld"),
            account(2641, "Debiterad ingående moms", "skuld"),
            account(5420, "Programvaror", "kostnad"),
            account(6071, "Representation, avdragsgill", "kostnad"),
            account(6212, "Mobiltelefon", "kostnad"),
            account(6570, "Bankkostnader", "kostnad"),
        ];
        let rule = |id: &str, text: &str| Rule {
            id: id.into(),
            version: 1,
            text: text.into(),
            valid_from: Some("2026-01-01".into()),
            valid_to: None,
        };
        let rule_list = vec![
            rule("R1", "Programvara och molntjänster: 5420, ingående moms 25 % på 2641, leverantörsskuld 2440."),
            rule("R2", "Mobil och internet: 6212, ingående moms 25 % på 2641."),
            rule("R3", "Representation: syfte och deltagare måste anges av granskaren; fråga om de saknas i underlaget."),
        ];
        let history = vec![(
            "exempel hosting ab 556000-0000".to_string(),
            PastPosting {
                voucher: "A7".into(),
                date: "2026-02-14".into(),
                description: "Molntjänst, månadsabonnemang februari".into(),
                lines: vec![
                    line(5420, 40_000, 0),
                    line(2641, 10_000, 0),
                    line(2440, 0, 50_000),
                ],
            },
        )];
        let bank_rows = vec![BankRow {
            post_id: "b1".into(),
            account: "SE4550000000058398257466".into(),
            booking_date: "2026-03-28".into(),
            amount_ore: -50_000,
            text: "Exempel Hosting".into(),
            reference: Some("2026311".into()),
            counterparty: Some("Exempel Hosting AB".into()),
        }];
        Self {
            documents,
            account_list,
            rule_list,
            history,
            bank_rows,
            cases: Vec::new(),
        }
    }

    /// Antal sparade verifikationsförslag.
    pub fn proposal_count(&self) -> usize {
        self.cases
            .iter()
            .filter(|(_, c)| matches!(c, Case::Voucher(..)))
            .count()
    }

    /// Ett sparat verifikationsförslag.
    pub fn proposal(&self, id: &str) -> Option<(&Proposal, &Provenance)> {
        self.cases.iter().find_map(|(i, c)| match c {
            Case::Voucher(p, prov) if i == id => Some((p, prov)),
            _ => None,
        })
    }

    fn next_id(&self, prefix: &str) -> String {
        format!("{prefix}{}", self.cases.len() + 1)
    }

    fn has_document(&self, id: &str) -> bool {
        self.documents.iter().any(|d| d.info.id == id)
    }
}

impl AgentBackend for MemoryBackend {
    fn list_documents(
        &self,
        only_open: bool,
        max: usize,
    ) -> Result<Vec<DocumentInfo>, BackendError> {
        Ok(self
            .documents
            .iter()
            .map(|d| d.info.clone())
            .filter(|i| !only_open || i.status == "oppen")
            .take(max)
            .collect())
    }

    fn get_document(&self, id: &str) -> Result<Document, BackendError> {
        self.documents
            .iter()
            .find(|d| d.info.id == id)
            .cloned()
            .ok_or_else(|| BackendError::NotFound(format!("underlaget {id}")))
    }

    fn accounts(&self) -> Result<Vec<Account>, BackendError> {
        Ok(self.account_list.clone())
    }

    fn rules(&self) -> Result<Vec<Rule>, BackendError> {
        Ok(self.rule_list.clone())
    }

    fn search_counterparty(
        &self,
        query: &str,
        max: usize,
    ) -> Result<Vec<PastPosting>, BackendError> {
        let q = query.to_lowercase();
        Ok(self
            .history
            .iter()
            .filter(|(key, _)| key.contains(&q))
            .map(|(_, p)| p.clone())
            .take(max)
            .collect())
    }

    fn unmatched_bank_rows(&self, max: usize) -> Result<Vec<BankRow>, BackendError> {
        Ok(self.bank_rows.iter().take(max).cloned().collect())
    }

    fn propose_voucher(
        &mut self,
        proposal: Proposal,
        provenance: Provenance,
    ) -> Result<Receipt, BackendError> {
        if let Some(id) = proposal
            .document_ids
            .iter()
            .find(|id| !self.has_document(id))
        {
            return Err(BackendError::Invalid(format!("underlaget {id} finns inte")));
        }
        let id = self.next_id("F");
        for d in &mut self.documents {
            if proposal.document_ids.contains(&d.info.id) {
                d.info.status = "foreslagen".into();
            }
        }
        self.cases
            .push((id.clone(), Case::Voucher(proposal, provenance)));
        Ok(Receipt {
            id,
            warnings: vec![],
        })
    }

    fn propose_match(
        &mut self,
        bank_row_id: &str,
        voucher: &str,
        provenance: Provenance,
    ) -> Result<Receipt, BackendError> {
        if !self.bank_rows.iter().any(|b| b.post_id == bank_row_id) {
            return Err(BackendError::NotFound(format!("bankraden {bank_row_id}")));
        }
        let id = self.next_id("M");
        self.cases.push((
            id.clone(),
            Case::Match {
                bank_row: bank_row_id.into(),
                voucher: voucher.into(),
                provenance,
            },
        ));
        Ok(Receipt {
            id,
            warnings: vec![],
        })
    }

    fn ask_question(
        &mut self,
        document_id: Option<&str>,
        question: &str,
        provenance: Provenance,
    ) -> Result<Receipt, BackendError> {
        let id = self.next_id("Q");
        self.cases.push((
            id.clone(),
            Case::Question {
                document: document_id.map(str::to_string),
                question: question.into(),
                provenance,
            },
        ));
        Ok(Receipt {
            id,
            warnings: vec![],
        })
    }

    fn flag_document(
        &mut self,
        document_id: &str,
        reason: &str,
        comment: &str,
        provenance: Provenance,
    ) -> Result<Receipt, BackendError> {
        if !self.has_document(document_id) {
            return Err(BackendError::NotFound(format!("underlaget {document_id}")));
        }
        let id = self.next_id("G");
        self.cases.push((
            id.clone(),
            Case::Flag {
                document: document_id.into(),
                reason: reason.into(),
                comment: comment.into(),
                provenance,
            },
        ));
        Ok(Receipt {
            id,
            warnings: vec![],
        })
    }
}
