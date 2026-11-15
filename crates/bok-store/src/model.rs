//! Typerna som lagringen tar emot och lämnar ut.

use chrono::NaiveDate;

use bok_core::voucher::{Counterparty, Line};
use bok_core::{AccountNumber, Money, PeriodStatus, Series, VoucherRef};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Company {
    pub name: String,
    pub org_number: String,
}

/// Ett beslut om säkerhetsnivån, i behandlingshistoriken som händelsen
/// `sakerhetsniva.beslutad`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityDecision {
    pub level: bok_core::security::SecurityLevel,
    /// Nivån som gällde innan; `None` för bokföringens första beslut.
    pub previous: Option<bok_core::security::SecurityLevel>,
    /// Tiden i lagringsformat (UTC).
    pub decided: String,
    /// Händelsen i behandlingshistoriken.
    pub event: i64,
    /// Den som beslutade.
    pub actor: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRow {
    pub id: i64,
    pub name: String,
    pub public_key: String,
    pub fingerprint: String,
    pub algorithm: String,
    pub hardware: bool,
    pub registered_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FiscalYear {
    pub id: i64,
    pub start: NaiveDate,
    pub end: NaiveDate,
}

impl FiscalYear {
    pub fn contains(&self, date: NaiveDate) -> bool {
        self.start <= date && date <= self.end
    }

    /// "2026" för ett kalenderår, annars "2025/26".
    pub fn label(&self) -> String {
        use chrono::Datelike;
        if self.start.year() == self.end.year() {
            self.start.year().to_string()
        } else {
            format!("{}/{:02}", self.start.year(), self.end.year() % 100)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Period {
    pub id: i64,
    pub fiscal_year_id: i64,
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub status: PeriodStatus,
}

impl Period {
    /// "2026-03"
    pub fn label(&self) -> String {
        self.start.format("%Y-%m").to_string()
    }

    pub fn contains(&self, date: NaiveDate) -> bool {
        self.start <= date && date <= self.end
    }
}

/// Ett konto som det ser ut i kontoplanen vid ett visst datum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountDef {
    pub number: AccountNumber,
    pub name: String,
    pub active: bool,
    pub sru: Option<String>,
    pub vat_code: Option<String>,
    pub valid_from: NaiveDate,
}

/// En rad i en signerad ändring av kontoplanen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountChange {
    pub number: AccountNumber,
    pub name: String,
    pub active: bool,
    pub sru: Option<String>,
    pub vat_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CounterpartyRow {
    pub id: i64,
    pub counterparty: Counterparty,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Channel {
    Manual,
    Folder,
    Email,
    Bank,
}

impl Channel {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Channel::Manual => "manuell",
            Channel::Folder => "mapp",
            Channel::Email => "mejl",
            Channel::Bank => "bank",
        }
    }
}

/// Ett underlag som tas emot. Filen själv sparas i arkivet av anroparen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAttachment {
    pub sha256: String,
    pub size: i64,
    pub channel: Channel,
    pub sender: Option<String>,
    pub original_name: Option<String>,
    pub file_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentRow {
    pub id: i64,
    pub sha256: String,
    pub size: i64,
    pub received_at: String,
    pub channel: String,
    pub sender: Option<String>,
    pub original_name: Option<String>,
    pub file_type: Option<String>,
    /// Senaste statushändelsen, till exempel "bokford".
    pub status: String,
    pub status_text: Option<String>,
}

/// Motparten i ett förslag: en post i registret eller en uppgift direkt i
/// förslaget. Vid bokföring kopieras uppgifterna in på verifikationen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProposalCounterparty {
    Register(i64),
    Inline(Counterparty),
}

/// Härkomsten för ett förslag från en agent.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Provenance {
    pub questions: Option<String>,
    pub flags: Option<String>,
    pub confidence: Option<f64>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub region: Option<String>,
    pub prompt_hash: Option<String>,
    pub rule_version: Option<String>,
    pub raw_response: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewProposal {
    pub series: Series,
    pub date: NaiveDate,
    pub description: String,
    pub counterparty: Option<ProposalCounterparty>,
    pub lines: Vec<Line>,
    /// Id för underlag som verifikationen bygger på.
    pub attachments: Vec<i64>,
    pub idempotency_key: Option<String>,
    pub provenance: Provenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Approved,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub outcome: Outcome,
    pub actor: String,
    pub time: String,
    pub reason: Option<String>,
    pub changes: Option<String>,
    pub voucher_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProposalRow {
    pub id: i64,
    pub series: Series,
    pub date: NaiveDate,
    pub description: String,
    pub counterparty: Option<ProposalCounterparty>,
    pub lines: Vec<Line>,
    pub attachments: Vec<i64>,
    pub corrects: Option<i64>,
    pub correction_reason: Option<String>,
    pub source: String,
    pub actor: String,
    pub created_at: String,
    /// Härkomsten: modell, leverantör, region, säkerhet, frågor, flaggor och
    /// agentens råa svar. Tom för förslag som en människa har lagt.
    pub provenance: Provenance,
    pub decision: Option<Decision>,
}

/// En version av en regel (kontering, moms, kurs) i tabellen `regler`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleRow {
    pub name: String,
    pub version: i64,
    pub valid_from: NaiveDate,
    /// Sista giltighetsdag, inklusive. `None` betyder att regeln gäller tills vidare.
    pub valid_to: Option<NaiveDate>,
    /// Regelns innehåll som JSON.
    pub content: String,
}

/// Resultatet av en bokföring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Posted {
    pub voucher_id: i64,
    pub fiscal_year_id: i64,
    pub reference: VoucherRef,
    pub period: String,
    pub date: NaiveDate,
    pub canonical_hash: String,
}

/// En verifikation med alla vyer behöver: rader, motpart och kopplingar
/// till rättelser åt båda hållen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoucherView {
    pub id: i64,
    pub fiscal_year_id: i64,
    pub reference: VoucherRef,
    pub period_id: i64,
    pub date: NaiveDate,
    pub compiled_on: NaiveDate,
    pub registered_at: String,
    pub description: String,
    pub counterparty: Option<Counterparty>,
    pub proposal_id: i64,
    pub lines: Vec<Line>,
    pub canonical_hash: String,
    pub signature: String,
    pub key_name: String,
    pub key_fingerprint: String,
    pub chain_hash: String,
    /// Rättelser som pekar på den här verifikationen ("rättad genom R4").
    pub corrected_by: Vec<CorrectionLink>,
    /// Originalet, om den här verifikationen är en rättelse.
    pub corrects: Option<CorrectionLink>,
    /// SHA-256 för underlagen som länkades vid bokföringen.
    pub attachments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorrectionLink {
    pub voucher_id: i64,
    pub fiscal_year_id: i64,
    pub reference: VoucherRef,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerEntry {
    pub voucher_id: i64,
    pub reference: VoucherRef,
    pub date: NaiveDate,
    pub registered_at: String,
    pub text: String,
    pub debit: Money,
    pub credit: Money,
    pub balance: Money,
    /// "rättad genom R4" om verifikationen har rättats.
    pub corrected_by: Vec<CorrectionLink>,
}

/// Ett konto i huvudboken: ingående balans, posterna i registreringsordning
/// och utgående balans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerAccount {
    pub account: AccountNumber,
    pub name: String,
    pub opening: Money,
    pub entries: Vec<LedgerEntry>,
    pub closing: Money,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrialBalanceRow {
    pub account: AccountNumber,
    pub name: String,
    pub opening: Money,
    pub debit: Money,
    pub credit: Money,
    pub closing: Money,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRow {
    pub seq: i64,
    pub time: String,
    pub actor_kind: String,
    pub actor: String,
    pub role: String,
    pub action: String,
    pub object: Option<String>,
    pub old_values: Option<String>,
    pub new_values: Option<String>,
    pub program_version: String,
    pub signed: bool,
    pub hash: String,
}

/// Vad en periodövergång lämnade efter sig.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub period: String,
    pub from: PeriodStatus,
    pub to: PeriodStatus,
    /// Obeslutade förslag daterade i perioden. De kan inte bokföras där när
    /// perioden är stängd.
    pub pending_proposals: usize,
}

/// En rad ur ett kontoutdrag som ska läsas in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewBankRow {
    /// Stabilt id från kontoutdraget, 64 hexadecimala tecken.
    pub post_id: String,
    /// Bankens kontonummer eller IBAN.
    pub bank_account: String,
    pub booking_date: NaiveDate,
    pub value_date: Option<NaiveDate>,
    /// Negativt är pengar ut från kontot.
    pub amount: Money,
    pub currency: String,
    pub text: String,
    pub reference: Option<String>,
    pub counterparty: Option<String>,
    pub balance: Option<Money>,
}

/// Resultatet av en inläsning: nya rader och rader som redan fanns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BankImport {
    pub new: usize,
    pub existing: usize,
}

/// En inläst bankrad med dess avstämning, om den finns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankRowRecord {
    pub id: i64,
    pub attachment_id: i64,
    pub post_id: String,
    pub bank_account: String,
    /// Bankens bokföringskonto, till exempel 1930.
    pub account: AccountNumber,
    pub booking_date: NaiveDate,
    pub value_date: Option<NaiveDate>,
    pub amount: Money,
    pub currency: String,
    pub text: String,
    pub reference: Option<String>,
    pub counterparty: Option<String>,
    pub balance: Option<Money>,
    pub imported_at: String,
    pub reconciliation: Option<Reconciliation>,
}

/// Hur en bankrad stämdes av, och av vem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reconciliation {
    pub voucher_id: Option<i64>,
    pub explanation: Option<String>,
    pub actor: String,
    pub time: String,
}

/// Vad en bankrad stäms av mot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchTarget {
    /// En bokförd verifikation (id).
    Voucher(i64),
    /// En skriftlig förklaring, till exempel en överföring som redan är bokförd på annat sätt.
    Explanation(String),
}

/// Vad ett matchningsförslag pekar på.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuggestedMatch {
    Voucher(i64),
    /// Ett verifikationsförslag som väntar på beslut.
    Proposal(i64),
    Explanation(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewMatchSuggestion {
    pub target: SuggestedMatch,
    pub confidence: Option<f64>,
    pub model: Option<String>,
    pub rationale: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchSuggestion {
    pub id: i64,
    pub bank_row_id: i64,
    pub target: SuggestedMatch,
    /// "manniska", "agent" eller "system".
    pub source: String,
    pub actor: String,
    pub confidence: Option<f64>,
    pub model: Option<String>,
    pub rationale: Option<String>,
    pub created_at: String,
}

/// En bokförd verifikation som kan motsvara en bankrad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchCandidate {
    pub voucher_id: i64,
    pub fiscal_year_id: i64,
    pub reference: VoucherRef,
    pub date: NaiveDate,
    pub description: String,
    pub counterparty: Option<String>,
    /// Verifikationens datum minus bankradens bokföringsdag.
    pub days_apart: i64,
    /// Högre är bättre: nära i tid, referens eller motpart som känns igen.
    pub score: i64,
}

/// Orsakerna en flagga kan ha.
pub const FLAG_REASONS: [&str; 4] = [
    "dubblett",
    "ej_rakenskapsinformation",
    "behover_komplettering",
    "olasligt",
];

/// Vad ett ärende är.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaseKind {
    /// En fråga, med eller utan underlag.
    Question,
    /// En flagga på ett underlag, med en av [`FLAG_REASONS`].
    Flag { reason: String },
}

/// Ett ärende som ska läggas.
#[derive(Debug, Clone, PartialEq)]
pub struct NewCase {
    pub kind: CaseKind,
    pub attachment_id: Option<i64>,
    pub text: String,
    pub model: Option<String>,
    pub confidence: Option<f64>,
}

/// Ett ärende med dess svar, om det finns.
#[derive(Debug, Clone, PartialEq)]
pub struct CaseRow {
    pub id: i64,
    pub kind: CaseKind,
    pub attachment_id: Option<i64>,
    pub text: String,
    /// "manniska", "agent" eller "system".
    pub source: String,
    pub actor: String,
    pub model: Option<String>,
    pub confidence: Option<f64>,
    pub created_at: String,
    pub answer: Option<CaseAnswer>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseAnswer {
    pub text: String,
    /// Underlagets nya status, om svaret ändrade den.
    pub status: Option<String>,
    pub actor: String,
    pub time: String,
}

/// Granskarens beslut om ett flaggat underlag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentVerdict {
    NotAccountingInformation,
    Duplicate,
}

impl AttachmentVerdict {
    pub fn as_db_str(self) -> &'static str {
        match self {
            AttachmentVerdict::NotAccountingInformation => "ej_rakenskapsinformation",
            AttachmentVerdict::Duplicate => "dubblett",
        }
    }
}

/// En avstämning av ett konto mot ett externt saldo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountReconciliation {
    pub account: AccountNumber,
    pub date: NaiveDate,
    /// Bokfört saldo, debet positivt.
    pub booked: Money,
    /// Saldot enligt den externa källan, med samma tecken.
    pub external: Money,
    /// Bokfört minus externt. Noll betyder att kontot stämmer.
    pub difference: Money,
}

/// Händelsen där en människa godtar att klockan står före den senaste
/// händelsen, för att det är den händelsen som fick fel tid. Det är den enda
/// händelse som får en tid före den föregående.
pub const CLOCK_ACCEPTED: &str = "klocka.godtagen";

/// Händelserna som `Store::record_remote_access` skriver: en agentanslutning
/// som godkänns eller återkallas, och en uppladdningslänk som skapas eller
/// återkallas.
pub const REMOTE_ACCESS_ACTIONS: [&str; 4] = [
    "fjarr.anslutning.godkand",
    "fjarr.anslutning.aterkallad",
    "fjarr.lank.skapad",
    "fjarr.lank.aterkallad",
];
