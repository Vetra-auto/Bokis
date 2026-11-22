//! Det granskningsvyn behöver av lagret.
//!
//! [`ReviewBackend::approve`] är det enda som bokför. Implementationen mot
//! `bok-store` signerar verifikationens kanoniska hash med granskarens
//! hårdvarunyckel, så ett godkännande kräver att någon fysiskt rör nyckeln.

/// Fel från lagret.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReviewError {
    #[error("{0} finns inte")]
    NotFound(String),
    #[error("{0}")]
    Refused(String),
    #[error("internt fel: {0}")]
    Internal(String),
}

/// En rad i listan över förslag.
#[derive(Debug, Clone, PartialEq)]
pub struct ProposalSummary {
    pub id: String,
    pub created_at: String,
    pub transaction_date: String,
    pub description: String,
    pub counterparty: Option<String>,
    /// Summa debet i öre.
    pub amount_ore: i64,
    /// Agentens osäkerhet, 0–1. Saknas för förslag som en människa har lagt.
    pub uncertainty: Option<f64>,
    pub warnings: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposalLine {
    pub account: u32,
    pub account_name: String,
    pub debit_ore: i64,
    pub credit_ore: i64,
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentRef {
    pub id: String,
    pub file_name: String,
    pub media_type: String,
}

/// Allt granskaren behöver se för att besluta.
#[derive(Debug, Clone, PartialEq)]
pub struct ProposalDetail {
    pub summary: ProposalSummary,
    pub counterparty_org_number: Option<String>,
    pub lines: Vec<ProposalLine>,
    pub documents: Vec<DocumentRef>,
    pub warnings: Vec<String>,
    pub rationale: String,
    pub model: String,
    pub provider: Option<String>,
    pub region: Option<String>,
    pub client: Option<String>,
    pub rule_ids: Vec<String>,
    /// Verifikation som förslaget rättar, om det är en rättelse.
    pub corrects: Option<String>,
}

/// En bokförd verifikation som kan motsvara en bankrad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateView {
    pub voucher_id: String,
    /// Till exempel "A7 (2026)".
    pub reference: String,
    pub date: String,
    pub description: String,
}

/// En bankrad som inte är avstämd, med möjliga verifikationer och förslag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankRowView {
    pub id: String,
    pub date: String,
    pub amount_ore: i64,
    pub text: String,
    pub reference: Option<String>,
    pub candidates: Vec<CandidateView>,
    /// Agentens och andras matchningsförslag, som text.
    pub suggestions: Vec<String>,
}

/// En fråga eller flagga som väntar på svar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseView {
    pub id: String,
    /// "Fråga" eller "Flagga (dubblett)".
    pub kind: String,
    pub is_flag: bool,
    pub document_id: Option<String>,
    pub text: String,
    pub actor: String,
    pub created_at: String,
}

/// Det granskningsvyn får göra mot lagret.
pub trait ReviewBackend {
    fn open_proposals(&self) -> Result<Vec<ProposalSummary>, ReviewError>;
    fn proposal(&self, id: &str) -> Result<ProposalDetail, ReviewError>;
    /// Underlagets filtyp och innehåll.
    fn document(&self, id: &str) -> Result<(String, Vec<u8>), ReviewError>;
    /// Bokför förslaget och returnerar verifikationens nummer, till exempel `A12`.
    fn approve(&mut self, id: &str) -> Result<String, ReviewError>;
    fn reject(&mut self, id: &str, reason: &str) -> Result<(), ReviewError>;

    /// Bankrader som inte är avstämda.
    fn bank_rows(&self) -> Result<Vec<BankRowView>, ReviewError> {
        Ok(Vec::new())
    }
    /// Stämmer av en bankrad mot en verifikation (id) eller med en förklaring.
    fn reconcile(
        &mut self,
        _row_id: &str,
        _voucher_id: Option<&str>,
        _explanation: Option<&str>,
    ) -> Result<(), ReviewError> {
        Err(ReviewError::Refused(
            "bankavstämning finns inte i det här läget".into(),
        ))
    }
    /// Frågor och flaggor som väntar på svar.
    fn cases(&self) -> Result<Vec<CaseView>, ReviewError> {
        Ok(Vec::new())
    }
    /// Besvarar ett ärende. `mark` är "dubblett" eller "ej_rakenskapsinformation" för en flagga.
    fn answer_case(
        &mut self,
        _id: &str,
        _answer: &str,
        _mark: Option<&str>,
    ) -> Result<(), ReviewError> {
        Err(ReviewError::Refused(
            "ärenden finns inte i det här läget".into(),
        ))
    }
}

/// Ett lager i minnet för tester och `bok-review --demo`.
#[derive(Debug, Default, Clone)]
pub struct MemoryReviewBackend {
    pub proposals: Vec<ProposalDetail>,
    pub documents: Vec<(String, String, Vec<u8>)>,
    pub approved: Vec<(String, String)>,
    pub rejected: Vec<(String, String)>,
}

impl MemoryReviewBackend {
    /// Två förslag: en faktura med full härkomst, och ett kvitto med varningar.
    pub fn demo() -> Self {
        let line = |account: u32, name: &str, d: i64, c: i64| ProposalLine {
            account,
            account_name: name.into(),
            debit_ore: d,
            credit_ore: c,
            text: None,
        };
        let invoice =
            "Exempel Hosting AB\norg.nr 556000-0001\nFaktura 2026-311, fakturadatum 2026-03-14\n\
                       Molntjänst mars 400,00 kr\nMoms 25 % 100,00 kr\nAtt betala 500,00 kr\n";
        let svg_attempt = "<svg xmlns=\"http://www.w3.org/2000/svg\" onload=\"alert(1)\"></svg>";
        Self {
            proposals: vec![
                ProposalDetail {
                    summary: ProposalSummary {
                        id: "F1".into(),
                        created_at: "2026-03-16T08:15:02Z".into(),
                        transaction_date: "2026-03-14".into(),
                        description: "Molntjänst, månadsabonnemang mars".into(),
                        counterparty: Some("Exempel Hosting AB".into()),
                        amount_ore: 50_000,
                        uncertainty: Some(0.1),
                        warnings: 0,
                    },
                    counterparty_org_number: Some("556000-0001".into()),
                    lines: vec![
                        line(5420, "Programvaror", 40_000, 0),
                        line(2641, "Debiterad ingående moms", 10_000, 0),
                        line(2440, "Leverantörsskulder", 0, 50_000),
                    ],
                    documents: vec![DocumentRef {
                        id: "INK-1".into(),
                        file_name: "faktura-2026-311.txt".into(),
                        media_type: "text/plain".into(),
                    }],
                    warnings: vec![],
                    rationale: "Faktura från Exempel Hosting för molntjänst; regel R1 och samma kontering som A7 i februari.".into(),
                    model: "claude-opus-5-5".into(),
                    provider: Some("anthropic".into()),
                    region: Some("eu".into()),
                    client: Some("claude-code".into()),
                    rule_ids: vec!["R1".into()],
                    corrects: None,
                },
                ProposalDetail {
                    summary: ProposalSummary {
                        id: "F2".into(),
                        created_at: "2026-03-16T08:16:40Z".into(),
                        transaction_date: "2026-03-15".into(),
                        description: "Lunch med kund <okänd>".into(),
                        counterparty: Some("Café Öst".into()),
                        amount_ore: 34_000,
                        uncertainty: Some(0.6),
                        warnings: 2,
                    },
                    counterparty_org_number: None,
                    lines: vec![
                        line(6071, "Representation, avdragsgill", 30_357, 0),
                        line(2641, "Debiterad ingående moms", 3_643, 0),
                        line(1930, "Företagskonto", 0, 34_000),
                    ],
                    documents: vec![DocumentRef {
                        id: "INK-9".into(),
                        file_name: "bild.svg".into(),
                        media_type: "image/svg+xml".into(),
                    }],
                    warnings: vec![
                        "deltagare och syfte saknas i underlaget".into(),
                        "motpartens organisationsnummer saknas".into(),
                    ],
                    rationale: "Kvitto från restaurang. Syfte och deltagare framgår inte; regel R3 kräver dem.".into(),
                    model: "qwen3-coder".into(),
                    provider: Some("lokal".into()),
                    region: Some("lokal".into()),
                    client: Some("qwen-code".into()),
                    rule_ids: vec!["R3".into()],
                    corrects: None,
                },
            ],
            documents: vec![
                ("INK-1".into(), "text/plain".into(), invoice.as_bytes().to_vec()),
                ("INK-9".into(), "image/svg+xml".into(), svg_attempt.as_bytes().to_vec()),
            ],
            approved: vec![],
            rejected: vec![],
        }
    }

    fn is_open(&self, id: &str) -> bool {
        !self.approved.iter().any(|(i, _)| i == id) && !self.rejected.iter().any(|(i, _)| i == id)
    }
}

impl ReviewBackend for MemoryReviewBackend {
    fn open_proposals(&self) -> Result<Vec<ProposalSummary>, ReviewError> {
        Ok(self
            .proposals
            .iter()
            .filter(|p| self.is_open(&p.summary.id))
            .map(|p| p.summary.clone())
            .collect())
    }

    fn proposal(&self, id: &str) -> Result<ProposalDetail, ReviewError> {
        self.proposals
            .iter()
            .find(|p| p.summary.id == id)
            .cloned()
            .ok_or_else(|| ReviewError::NotFound(format!("förslaget {id}")))
    }

    fn document(&self, id: &str) -> Result<(String, Vec<u8>), ReviewError> {
        self.documents
            .iter()
            .find(|(i, _, _)| i == id)
            .map(|(_, t, d)| (t.clone(), d.clone()))
            .ok_or_else(|| ReviewError::NotFound(format!("underlaget {id}")))
    }

    fn approve(&mut self, id: &str) -> Result<String, ReviewError> {
        self.proposal(id)?;
        if !self.is_open(id) {
            return Err(ReviewError::Refused(format!(
                "förslaget {id} är redan avgjort"
            )));
        }
        let number = format!("A{}", self.approved.len() + 1);
        self.approved.push((id.into(), number.clone()));
        Ok(number)
    }

    fn reject(&mut self, id: &str, reason: &str) -> Result<(), ReviewError> {
        self.proposal(id)?;
        if !self.is_open(id) {
            return Err(ReviewError::Refused(format!(
                "förslaget {id} är redan avgjort"
            )));
        }
        self.rejected.push((id.into(), reason.into()));
        Ok(())
    }
}
