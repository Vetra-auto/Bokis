//! Det MCP-servern behöver av lagret, och datatyperna som går över gränsen.
//!
//! [`AgentBackend`] innehåller bara läsning och förslag. Det finns ingen metod
//! för att bokföra, godkänna, stänga perioder eller ändra regler, så en agent
//! kan inte nå sådana operationer genom den här servern ens om den försöker.
//!
//! Fältnamnen i JSON är svenska eftersom det är dem agenten ser.

use serde::{Deserialize, Serialize};

/// Fel från lagret.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BackendError {
    #[error("{0} finns inte")]
    NotFound(String),
    #[error("ogiltigt: {0}")]
    Invalid(String),
    #[error("internt fel: {0}")]
    Internal(String),
}

/// Ett mottaget underlag i inkorgen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentInfo {
    pub id: String,
    pub sha256: String,
    #[serde(rename = "filnamn")]
    pub file_name: String,
    #[serde(rename = "filtyp")]
    pub media_type: String,
    #[serde(rename = "storlek")]
    pub size: u64,
    /// Serverns mottagningstid, ISO 8601.
    #[serde(rename = "mottagen")]
    pub received_at: String,
    #[serde(rename = "kanal")]
    pub channel: String,
    #[serde(rename = "avsandare")]
    pub sender: Option<String>,
    /// "oppen", "foreslagen", "bokford", "ej_rakenskapsinformation", "dubblett".
    pub status: String,
}

/// Ett underlag med innehåll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub info: DocumentInfo,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    #[serde(rename = "nummer")]
    pub number: u32,
    #[serde(rename = "namn")]
    pub name: String,
    /// "tillgang", "skuld", "eget_kapital", "intakt", "kostnad".
    #[serde(rename = "typ")]
    pub kind: String,
    #[serde(rename = "aktiv")]
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub version: u32,
    pub text: String,
    #[serde(rename = "giltig_fran")]
    pub valid_from: Option<String>,
    #[serde(rename = "giltig_till")]
    pub valid_to: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PastPosting {
    #[serde(rename = "verifikation")]
    pub voucher: String,
    #[serde(rename = "datum")]
    pub date: String,
    #[serde(rename = "beskrivning")]
    pub description: String,
    #[serde(rename = "rader")]
    pub lines: Vec<ProposalLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankRow {
    pub post_id: String,
    #[serde(rename = "konto")]
    pub account: String,
    #[serde(rename = "bokforingsdag")]
    pub booking_date: String,
    #[serde(rename = "belopp_ore")]
    pub amount_ore: i64,
    pub text: String,
    #[serde(rename = "referens")]
    pub reference: Option<String>,
    #[serde(rename = "motpart")]
    pub counterparty: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counterparty {
    #[serde(rename = "namn")]
    pub name: String,
    #[serde(rename = "orgnr", default)]
    pub org_number: Option<String>,
    #[serde(rename = "momsnr", default)]
    pub vat_number: Option<String>,
    #[serde(rename = "land", default)]
    pub country: Option<String>,
}

/// En konteringsrad i ett förslag. Exakt ett av debet och kredit ska vara satt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalLine {
    #[serde(rename = "konto")]
    pub account: u32,
    #[serde(rename = "debet_ore", default)]
    pub debit_ore: i64,
    #[serde(rename = "kredit_ore", default)]
    pub credit_ore: i64,
    #[serde(default)]
    pub text: Option<String>,
}

/// Agentens förslag till verifikation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    #[serde(rename = "affarshandelse_datum")]
    pub transaction_date: String,
    #[serde(rename = "beskrivning")]
    pub description: String,
    #[serde(rename = "motpart", default)]
    pub counterparty: Option<Counterparty>,
    #[serde(rename = "rader")]
    pub lines: Vec<ProposalLine>,
    #[serde(rename = "underlag_ids", default)]
    pub document_ids: Vec<String>,
    /// Bankrader förslaget gäller, om något.
    #[serde(rename = "bankrad_ids", default)]
    pub bank_row_ids: Vec<String>,
    /// Regler i `hamta_regler` som förslaget bygger på.
    #[serde(rename = "regel_ids", default)]
    pub rule_ids: Vec<String>,
    /// Verifikation som förslaget rättar, om det är en rättelse.
    #[serde(rename = "rattar", default)]
    pub corrects: Option<String>,
}

/// Härkomst för ett förslag: vem som föreslog och på vilket underlag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    /// Modellens id, som agenten själv anger.
    #[serde(rename = "modell")]
    pub model: String,
    #[serde(rename = "leverantor")]
    pub provider: Option<String>,
    pub region: Option<String>,
    /// MCP-klientens namn och version från `initialize`.
    #[serde(rename = "klient")]
    pub client: Option<String>,
    /// Agentens motivering i klartext.
    #[serde(rename = "motivering")]
    pub rationale: String,
    /// 0 = helt säker, 1 = helt osäker.
    #[serde(rename = "osakerhet")]
    pub uncertainty: f64,
    /// SHA-256 av argumenten exakt som de kom in, för spårbarhet.
    #[serde(rename = "argument_sha256")]
    pub arguments_sha256: String,
}

/// Svar när ett förslag har tagits emot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub id: String,
    #[serde(rename = "varningar")]
    pub warnings: Vec<String>,
}

/// Ett inventarium i registret, som agenten ser det.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetInfo {
    #[serde(rename = "nummer")]
    pub id: i64,
    #[serde(rename = "beskrivning")]
    pub description: String,
    #[serde(rename = "anskaffad")]
    pub acquired: String,
    #[serde(rename = "anskaffningsvarde_ore")]
    pub cost_ore: i64,
    #[serde(rename = "nyttjandeperiod_manader")]
    pub life_months: u32,
    #[serde(rename = "avskrivning_fran")]
    pub depreciation_start: String,
    /// Tillgång, ackumulerade avskrivningar och kostnad.
    #[serde(rename = "konton")]
    pub accounts: [u16; 3],
    /// Ackumulerad avskrivning enligt plan vid räkenskapsårets slut.
    #[serde(rename = "ackumulerad_avskrivning_ore")]
    pub accumulated_ore: i64,
    #[serde(rename = "bokfort_varde_ore")]
    pub book_value_ore: i64,
    /// "i_bruk", "forsaljning", "utrangering" eller "felregistrering".
    pub status: String,
}

/// Det MCP-servern får göra mot lagret. Inget här bokför.
pub trait AgentBackend {
    fn list_documents(
        &self,
        only_open: bool,
        max: usize,
    ) -> Result<Vec<DocumentInfo>, BackendError>;
    fn get_document(&self, id: &str) -> Result<Document, BackendError>;
    fn accounts(&self) -> Result<Vec<Account>, BackendError>;
    fn rules(&self) -> Result<Vec<Rule>, BackendError>;
    fn search_counterparty(
        &self,
        query: &str,
        max: usize,
    ) -> Result<Vec<PastPosting>, BackendError>;
    fn unmatched_bank_rows(&self, max: usize) -> Result<Vec<BankRow>, BackendError>;
    fn propose_voucher(
        &mut self,
        proposal: Proposal,
        provenance: Provenance,
    ) -> Result<Receipt, BackendError>;
    fn propose_match(
        &mut self,
        bank_row_id: &str,
        voucher: &str,
        provenance: Provenance,
    ) -> Result<Receipt, BackendError>;
    fn ask_question(
        &mut self,
        document_id: Option<&str>,
        question: &str,
        provenance: Provenance,
    ) -> Result<Receipt, BackendError>;
    fn flag_document(
        &mut self,
        document_id: &str,
        reason: &str,
        comment: &str,
        provenance: Provenance,
    ) -> Result<Receipt, BackendError>;
    /// Inventarieregistret med värden vid räkenskapsårets slut. Demoläget
    /// har inget register.
    fn assets(&self) -> Result<Vec<AssetInfo>, BackendError> {
        Ok(Vec::new())
    }
    /// Föreslår årets avskrivningar enligt plan för räkenskapsåret `year`
    /// ("2026"), eller året som omfattar i dag.
    fn propose_depreciation(
        &mut self,
        _year: Option<&str>,
        _provenance: Provenance,
    ) -> Result<Receipt, BackendError> {
        Err(BackendError::Invalid(
            "inventarieregistret finns inte i demoläget".into(),
        ))
    }
}
