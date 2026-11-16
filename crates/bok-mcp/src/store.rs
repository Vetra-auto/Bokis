//! [`AgentBackend`] mot Bokis databas (`bok-store`) och underlagsarkivet.
//!
//! Agenten skrivs in i historiken som en aktör av typen agent, med modellens
//! namn. Förslag läggs med `Store::create_proposal`, som kontrollerar
//! kontoplan, period och balans och aldrig delar ut något nummer. Härkomsten
//! sparas i förslaget: modell, leverantör, region och säkerhet i egna
//! kolumner, hela anropet (med motivering och klient) som JSON i `ratt_svar`,
//! och argumentens SHA-256 som idempotensnyckel, så att ett upprepat anrop
//! inte ger två förslag.
//!
//! Inget här kan bokföra: lagret kräver en människa och en registrerad nyckel.
//!
//! Bankrader och matchning: agenten ser raderna som inte är avstämda och kan
//! föreslå vad de motsvarar, men avstämningen görs av granskaren.
//!
//! Frågor och flaggor blir ärenden som granskaren besvarar. En agent ändrar
//! aldrig själv ett underlags status, eftersom den då skulle kunna gömma
//! underlag; det gör bara granskarens svar på en flagga.

use std::collections::HashSet;
use std::path::Path;

use bok_archive::{media, Archive, ArchiveError};
use bok_store::bok_core::voucher::{Counterparty as CoreCounterparty, Line};
use bok_store::bok_core::{AccountNumber, Money, Series, VoucherRef};
use bok_store::{
    Actor, AttachmentRow, CaseKind, NewCase, NewMatchSuggestion, NewProposal, ProposalCounterparty,
    Provenance as StoreProvenance, RuleRow, Store, StoreError, SuggestedMatch,
};
use chrono::NaiveDate;

use crate::backend::*;

/// Lagret och arkivet som agenten arbetar mot.
pub struct StoreBackend {
    store: Store,
    archive: Archive,
}

/// Arkivet ligger bredvid databasen, i katalogen `underlag`, som för `bok`.
pub fn archive_beside(db: &Path) -> std::path::PathBuf {
    db.parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
        .join("underlag")
}

fn store_err(e: StoreError) -> BackendError {
    match e {
        StoreError::Missing { .. } => BackendError::NotFound(e.to_string()),
        StoreError::Sqlite(_)
        | StoreError::Io(_)
        | StoreError::Json(_)
        | StoreError::Signature(_)
        | StoreError::AlreadyExists(_)
        | StoreError::NotFound(_)
        | StoreError::NotABokisDatabase(_)
        | StoreError::NewerSchema { .. } => BackendError::Internal(e.to_string()),
        other => BackendError::Invalid(other.to_string()),
    }
}

fn archive_err(id: i64, e: ArchiveError) -> BackendError {
    match e {
        ArchiveError::Missing(_) => BackendError::Internal(format!(
            "underlag {id} finns i bokföringen men filen saknas i arkivet"
        )),
        ArchiveError::Changed { .. } => BackendError::Internal(format!(
            "filen för underlag {id} har ändrats i arkivet och lämnas inte ut"
        )),
        other => BackendError::Internal(other.to_string()),
    }
}

/// Modellnamnet som agenten själv anger blir aktörens namn i historiken och
/// visas för granskaren. Styrtecken och tecken som vänder skrivriktningen tas
/// bort, och namnet kortas till 100 tecken.
fn model_label(model: &str) -> String {
    let cleaned: String = model
        .chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(
                    c,
                    '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
                )
        })
        .take(100)
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        "okänd modell".to_string()
    } else {
        cleaned.to_string()
    }
}

fn parse_document_id(id: &str) -> Result<i64, BackendError> {
    id.trim()
        .parse()
        .map_err(|_| BackendError::NotFound(format!("underlaget {id}")))
}

fn kind(number: u16) -> &'static str {
    match number {
        1000..=1999 => "tillgang",
        2000..=2099 => "eget_kapital",
        2100..=2999 => "skuld",
        3000..=3999 | 8000..=8399 => "intakt",
        _ => "kostnad",
    }
}

/// En regel som agenten ser den. Innehållet lagras som JSON; är det en sträng
/// eller ett objekt med fältet `text` visas texten, annars hela JSON-värdet.
fn rule_from_row(r: RuleRow) -> Rule {
    let text = match serde_json::from_str::<serde_json::Value>(&r.content) {
        Ok(serde_json::Value::String(t)) => t,
        Ok(serde_json::Value::Object(o)) => match o.get("text") {
            Some(serde_json::Value::String(t)) => t.clone(),
            _ => r.content.clone(),
        },
        _ => r.content.clone(),
    };
    Rule {
        id: r.name,
        version: u32::try_from(r.version).unwrap_or(0),
        text,
        valid_from: Some(r.valid_from.to_string()),
        valid_to: r.valid_to.map(|d| d.to_string()),
    }
}

fn to_proposal_line(l: &Line) -> ProposalLine {
    ProposalLine {
        account: u32::from(l.account.value()),
        debit_ore: l.debit.ore(),
        credit_ore: l.credit.ore(),
        text: l.text.clone(),
    }
}

impl StoreBackend {
    pub fn new(store: Store, archive: Archive) -> Self {
        Self { store, archive }
    }

    /// Öppnar databasen och arkivet bredvid den. Skapar aldrig en databas.
    pub fn open(db: &Path, program_version: &str) -> Result<Self, BackendError> {
        let store = Store::open(db, program_version).map_err(store_err)?;
        let archive =
            Archive::open(archive_beside(db)).map_err(|e| BackendError::Internal(e.to_string()))?;
        Ok(Self::new(store, archive))
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Underlag som ett väntande förslag bygger på.
    fn proposed_attachments(&self) -> Result<HashSet<i64>, BackendError> {
        Ok(self
            .store
            .proposals(true)
            .map_err(store_err)?
            .iter()
            .flat_map(|p| p.attachments.iter().copied())
            .collect())
    }

    fn info(a: &AttachmentRow, proposed: &HashSet<i64>, data: Option<&[u8]>) -> DocumentInfo {
        let status = match a.status.as_str() {
            "mottagen" if proposed.contains(&a.id) => "foreslagen",
            "mottagen" => "oppen",
            other => other,
        };
        DocumentInfo {
            id: a.id.to_string(),
            sha256: a.sha256.clone(),
            file_name: a
                .original_name
                .clone()
                .unwrap_or_else(|| format!("underlag-{}", a.id)),
            media_type: match data {
                Some(d) => media::detect(d, a.original_name.as_deref().unwrap_or("")),
                None => a
                    .file_type
                    .as_deref()
                    .or(a.original_name.as_deref())
                    .and_then(media::from_name)
                    .unwrap_or_else(|| "application/octet-stream".into()),
            },
            size: u64::try_from(a.size).unwrap_or(0),
            received_at: a.received_at.clone(),
            channel: a.channel.clone(),
            sender: a.sender.clone(),
            status: status.to_string(),
        }
    }

    fn attachment(&self, id: i64) -> Result<AttachmentRow, BackendError> {
        self.store
            .attachments()
            .map_err(store_err)?
            .into_iter()
            .find(|a| a.id == id)
            .ok_or_else(|| BackendError::NotFound(format!("underlaget {id}")))
    }

    /// En bokförd verifikation som "A7" eller "A7 (2026)". Utan år gäller
    /// räkenskapsåret som omfattar `default_date`.
    fn find_voucher(
        &self,
        text: &str,
        default_date: NaiveDate,
    ) -> Result<bok_store::VoucherView, BackendError> {
        let text = text.trim();
        let (reference, label) = match text.split_once(' ') {
            Some((r, y)) => (
                r,
                Some(y.trim().trim_start_matches('(').trim_end_matches(')')),
            ),
            None => (text, None),
        };
        let vref: VoucherRef =
            reference
                .parse()
                .map_err(|e: bok_store::bok_core::series::SeriesError| {
                    BackendError::Invalid(e.to_string())
                })?;
        let year = match label {
            Some(label) => self
                .store
                .fiscal_years()
                .map_err(store_err)?
                .into_iter()
                .find(|y| y.label() == label)
                .ok_or_else(|| {
                    BackendError::Invalid(format!("räkenskapsåret {label} finns inte"))
                })?,
            None => self
                .store
                .fiscal_year_for_date(default_date)
                .map_err(store_err)?
                .ok_or_else(|| {
                    BackendError::Invalid(format!("inget räkenskapsår omfattar {default_date}"))
                })?,
        };
        self.store.voucher_by_ref(year.id, vref).map_err(store_err)
    }

    fn lines(proposal: &Proposal) -> Result<Vec<Line>, BackendError> {
        proposal
            .lines
            .iter()
            .map(|l| {
                let account = AccountNumber::new(i64::from(l.account))
                    .map_err(|e| BackendError::Invalid(e.0))?;
                Ok(Line {
                    account,
                    debit: Money::from_ore(l.debit_ore),
                    credit: Money::from_ore(l.credit_ore),
                    vat_code: None,
                    text: l.text.clone(),
                })
            })
            .collect()
    }
}

impl AgentBackend for StoreBackend {
    fn list_documents(
        &self,
        only_open: bool,
        max: usize,
    ) -> Result<Vec<DocumentInfo>, BackendError> {
        let proposed = self.proposed_attachments()?;
        Ok(self
            .store
            .attachments()
            .map_err(store_err)?
            .iter()
            .map(|a| Self::info(a, &proposed, None))
            .filter(|i| !only_open || i.status == "oppen")
            .take(max)
            .collect())
    }

    fn get_document(&self, id: &str) -> Result<Document, BackendError> {
        let id = parse_document_id(id)?;
        let row = self.attachment(id)?;
        let data = self
            .archive
            .read(&row.sha256)
            .map_err(|e| archive_err(id, e))?;
        let proposed = self.proposed_attachments()?;
        Ok(Document {
            info: Self::info(&row, &proposed, Some(&data)),
            data,
        })
    }

    fn accounts(&self) -> Result<Vec<Account>, BackendError> {
        Ok(self
            .store
            .chart(self.store.today())
            .map_err(store_err)?
            .into_iter()
            .map(|a| Account {
                number: u32::from(a.number.value()),
                name: a.name,
                kind: kind(a.number.value()).into(),
                active: a.active,
            })
            .collect())
    }

    /// Reglerna som gäller i dag.
    fn rules(&self) -> Result<Vec<Rule>, BackendError> {
        Ok(self
            .store
            .rules(self.store.today())
            .map_err(store_err)?
            .into_iter()
            .map(rule_from_row)
            .collect())
    }

    /// Tidigare bokförda verifikationer där motparten eller beskrivningen
    /// matchar, nyast först. Rättade verifikationer och rättelser tas inte med,
    /// eftersom de inte är förebilder.
    fn search_counterparty(
        &self,
        query: &str,
        max: usize,
    ) -> Result<Vec<PastPosting>, BackendError> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Err(BackendError::Invalid("sökningen är tom".into()));
        }
        let mut out = Vec::new();
        let mut years = self.store.fiscal_years().map_err(store_err)?;
        years.reverse();
        for year in years {
            for v in self.store.journal(year.id).map_err(store_err)?.iter().rev() {
                if v.reference.series == Series::R || !v.corrected_by.is_empty() {
                    continue;
                }
                let hit = v.description.to_lowercase().contains(&q)
                    || v.counterparty.as_ref().is_some_and(|c| {
                        c.name.to_lowercase().contains(&q)
                            || c.org_number.as_deref().is_some_and(|o| o.contains(&q))
                    });
                if hit {
                    out.push(PastPosting {
                        voucher: format!("{} ({})", v.reference, year.label()),
                        date: v.date.to_string(),
                        description: v.description.clone(),
                        lines: v.lines.iter().map(to_proposal_line).collect(),
                    });
                    if out.len() >= max {
                        return Ok(out);
                    }
                }
            }
        }
        Ok(out)
    }

    /// Bankrader som inte är avstämda. `post_id` är radens nummer i
    /// bokföringen, det som `foresla_matchning` och `bankrad_ids` tar emot.
    fn unmatched_bank_rows(&self, max: usize) -> Result<Vec<BankRow>, BackendError> {
        Ok(self
            .store
            .bank_rows(true)
            .map_err(store_err)?
            .into_iter()
            .take(max)
            .map(|r| BankRow {
                post_id: r.id.to_string(),
                account: r.bank_account,
                booking_date: r.booking_date.to_string(),
                amount_ore: r.amount.ore(),
                text: r.text,
                reference: r.reference,
                counterparty: r.counterparty,
            })
            .collect())
    }

    fn propose_voucher(
        &mut self,
        proposal: Proposal,
        provenance: Provenance,
    ) -> Result<Receipt, BackendError> {
        let date =
            NaiveDate::parse_from_str(&proposal.transaction_date, "%Y-%m-%d").map_err(|_| {
                BackendError::Invalid("affarshandelse_datum ska vara ÅÅÅÅ-MM-DD".into())
            })?;
        let lines = Self::lines(&proposal)?;
        let actor = Actor::agent(model_label(&provenance.model));
        let mut warnings = Vec::new();
        let id = match &proposal.corrects {
            Some(reference) => {
                let original = self.find_voucher(reference, date)?;
                warnings.push(
                    "rättelsen sparades utan härkomst: lagret tar inte emot härkomst för rättelser än".to_string(),
                );
                if !proposal.bank_row_ids.is_empty() {
                    warnings.push("bankraderna kopplas inte till en rättelse".to_string());
                }
                self.store
                    .propose_correction(&actor, original.id, &proposal.description, &lines)
                    .map_err(store_err)?
            }
            None => {
                let attachments = proposal
                    .document_ids
                    .iter()
                    .map(|d| parse_document_id(d))
                    .collect::<Result<Vec<_>, _>>()?;
                let raw = serde_json::json!({ "forslag": &proposal, "harkomst": &provenance })
                    .to_string();
                let new = NewProposal {
                    series: Series::A,
                    date,
                    description: proposal.description.clone(),
                    counterparty: proposal.counterparty.as_ref().map(|c| {
                        ProposalCounterparty::Inline(CoreCounterparty {
                            name: c.name.clone(),
                            org_number: c.org_number.clone(),
                            vat_number: c.vat_number.clone(),
                        })
                    }),
                    lines,
                    attachments,
                    idempotency_key: Some(provenance.arguments_sha256.clone()),
                    provenance: StoreProvenance {
                        questions: None,
                        flags: None,
                        confidence: Some((1.0 - provenance.uncertainty).clamp(0.0, 1.0)),
                        model: Some(model_label(&provenance.model)),
                        provider: provenance.provider.clone(),
                        region: provenance.region.clone(),
                        prompt_hash: None,
                        rule_version: (!proposal.rule_ids.is_empty())
                            .then(|| proposal.rule_ids.join(",")),
                        raw_response: Some(raw),
                    },
                };
                let id = self
                    .store
                    .create_proposal(&actor, &new)
                    .map_err(store_err)?;
                // Bankraderna förslaget gäller blir matchningsförslag som pekar
                // på verifikationsförslaget. Granskaren stämmer av efter bokföringen.
                for row in &proposal.bank_row_ids {
                    let linked = row
                        .trim()
                        .parse::<i64>()
                        .map_err(|_| format!("bankraden {row} finns inte"))
                        .and_then(|row_id| {
                            self.store
                                .suggest_match(
                                    &actor,
                                    row_id,
                                    &NewMatchSuggestion {
                                        target: SuggestedMatch::Proposal(id),
                                        confidence: new.provenance.confidence,
                                        model: Some(model_label(&provenance.model)),
                                        rationale: Some(provenance.rationale.clone()),
                                    },
                                )
                                .map_err(|e| e.to_string())
                        });
                    if let Err(e) = linked {
                        warnings.push(format!("bankrad {row} kopplades inte: {e}"));
                    }
                }
                id
            }
        };
        Ok(Receipt {
            id: id.to_string(),
            warnings,
        })
    }

    /// Föreslår att en bankrad motsvarar en bokförd verifikation, "A7" eller
    /// "A7 (2026)". Utan år används räkenskapsåret för bankradens dag.
    /// Förslaget avstämmer ingenting; det gör granskaren.
    fn propose_match(
        &mut self,
        bank_row_id: &str,
        voucher: &str,
        provenance: Provenance,
    ) -> Result<Receipt, BackendError> {
        let row_id: i64 = bank_row_id
            .trim()
            .parse()
            .map_err(|_| BackendError::NotFound(format!("bankraden {bank_row_id}")))?;
        let row = self.store.bank_row(row_id).map_err(store_err)?;
        let v = self.find_voucher(voucher, row.booking_date)?;
        let id = self
            .store
            .suggest_match(
                &Actor::agent(model_label(&provenance.model)),
                row_id,
                &NewMatchSuggestion {
                    target: SuggestedMatch::Voucher(v.id),
                    confidence: Some((1.0 - provenance.uncertainty).clamp(0.0, 1.0)),
                    model: Some(model_label(&provenance.model)),
                    rationale: Some(provenance.rationale.clone()),
                },
            )
            .map_err(store_err)?;
        Ok(Receipt {
            id: format!("M{id}"),
            warnings: vec![],
        })
    }

    /// En fråga till granskaren, som ett ärende. Den ändrar ingenting.
    fn ask_question(
        &mut self,
        document_id: Option<&str>,
        question: &str,
        provenance: Provenance,
    ) -> Result<Receipt, BackendError> {
        let attachment_id = document_id.map(parse_document_id).transpose()?;
        let id = self
            .store
            .open_case(
                &Actor::agent(model_label(&provenance.model)),
                &NewCase {
                    kind: CaseKind::Question,
                    attachment_id,
                    text: question.to_string(),
                    model: Some(model_label(&provenance.model)),
                    confidence: Some((1.0 - provenance.uncertainty).clamp(0.0, 1.0)),
                },
            )
            .map_err(store_err)?;
        Ok(Receipt {
            id: id.to_string(),
            warnings: vec![],
        })
    }

    /// En flagga på ett underlag, som ett ärende. Underlagets status ändras
    /// först om granskaren beslutar det.
    fn flag_document(
        &mut self,
        document_id: &str,
        reason: &str,
        comment: &str,
        provenance: Provenance,
    ) -> Result<Receipt, BackendError> {
        let attachment_id = parse_document_id(document_id)?;
        let text = [comment, provenance.rationale.as_str(), reason]
            .into_iter()
            .find(|t| !t.trim().is_empty())
            .unwrap_or(reason)
            .to_string();
        let id = self
            .store
            .open_case(
                &Actor::agent(model_label(&provenance.model)),
                &NewCase {
                    kind: CaseKind::Flag {
                        reason: reason.to_string(),
                    },
                    attachment_id: Some(attachment_id),
                    text,
                    model: Some(model_label(&provenance.model)),
                    confidence: Some((1.0 - provenance.uncertainty).clamp(0.0, 1.0)),
                },
            )
            .map_err(store_err)?;
        Ok(Receipt {
            id: id.to_string(),
            warnings: vec![],
        })
    }

    /// Registret med värden enligt plan vid slutet av räkenskapsåret som
    /// omfattar i dag (eller det senaste året).
    fn assets(&self) -> Result<Vec<AssetInfo>, BackendError> {
        use bok_store::bok_inventarier::Plan;
        let years = self.store.fiscal_years().map_err(store_err)?;
        let today = self.store.today();
        let end = years
            .iter()
            .find(|y| y.contains(today))
            .or(years.last())
            .map_or(today, |y| y.end);
        let assets = self.store.assets().map_err(store_err)?;
        Ok(assets
            .into_iter()
            .map(|a| {
                let accumulated = if a.is_void() {
                    0
                } else {
                    Plan::of(&a.details).accumulated_through(end).ore()
                };
                let acc = a.details.accounts;
                AssetInfo {
                    id: a.id,
                    status: a
                        .disposal
                        .as_ref()
                        .map_or("i_bruk".to_string(), |d| d.kind.as_str().to_string()),
                    description: a.details.description,
                    acquired: a.details.acquired.to_string(),
                    cost_ore: a.details.cost.ore(),
                    life_months: a.details.life_months,
                    depreciation_start: a.details.depreciation_start.to_string(),
                    accounts: [
                        acc.asset.value(),
                        acc.accumulated.value(),
                        acc.expense.value(),
                    ],
                    accumulated_ore: accumulated,
                    book_value_ore: a.details.cost.ore() - accumulated,
                }
            })
            .collect())
    }

    /// Årets avskrivningar enligt plan som bokslutsförslag. Beloppen räknas
    /// av Bokis ur registret, inte av agenten.
    fn propose_depreciation(
        &mut self,
        year: Option<&str>,
        provenance: Provenance,
    ) -> Result<Receipt, BackendError> {
        let years = self.store.fiscal_years().map_err(store_err)?;
        let today = self.store.today();
        let y = match year.map(str::trim) {
            Some(label) => years
                .iter()
                .find(|y| y.label() == label)
                .copied()
                .ok_or_else(|| BackendError::NotFound(format!("räkenskapsåret {label}")))?,
            None => years
                .iter()
                .find(|y| y.contains(today))
                .or(years.last())
                .copied()
                .ok_or_else(|| BackendError::NotFound("något räkenskapsår".into()))?,
        };
        let actor = Actor::agent(model_label(&provenance.model));
        let raw = serde_json::json!({ "ar": y.label(), "harkomst": &provenance }).to_string();
        let store_provenance = StoreProvenance {
            confidence: Some((1.0 - provenance.uncertainty).clamp(0.0, 1.0)),
            model: Some(model_label(&provenance.model)),
            provider: provenance.provider.clone(),
            region: provenance.region.clone(),
            raw_response: Some(raw),
            ..StoreProvenance::default()
        };
        let (proposed, _) = bok_bokslut::depreciation::propose_depreciation(
            &mut self.store,
            &actor,
            &y,
            store_provenance,
        )
        .map_err(|e| match e {
            bok_bokslut::ClosingError::Store(e) => store_err(e),
            other => BackendError::Invalid(other.to_string()),
        })?;
        Ok(Receipt {
            id: proposed.id.to_string(),
            warnings: proposed.notes,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use bok_store::bok_core::signature::SoftwareSigner;
    use bok_store::{AccountChange, Channel, NewAttachment, NewBook};

    struct Fixture {
        _dir: tempfile::TempDir,
        backend: StoreBackend,
        signer: SoftwareSigner,
    }

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("bokis.db");
        let signer = SoftwareSigner::from_seed([9; 32]);
        let mut store = Store::create(
            &db,
            "bok-mcp-test",
            NewBook {
                company_name: "Testbolaget AB",
                org_number: "559999-9999",
                key_name: "granskare",
                public_key: &signer.public_key_openssh(),
                signer: &signer,
                security_level: bok_store::bok_core::security::SecurityLevel::Software,
            },
        )
        .unwrap();
        let human = Actor::human("granskare");
        let today = store.today();
        let year = chrono::Datelike::year(&today);
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
            number: AccountNumber::new(n).unwrap(),
            name: name.into(),
            active: true,
            sru: None,
            vat_code: None,
        })
        .collect();
        store
            .change_chart(&human, d(year - 1, 1, 1), &chart, &signer)
            .unwrap();
        let archive = Archive::open(archive_beside(&db)).unwrap();
        Fixture {
            _dir: dir,
            backend: StoreBackend::new(store, archive),
            signer,
        }
    }

    fn receive(f: &mut Fixture, data: &[u8], name: &str) -> String {
        let stored = f.backend.archive.put_bytes(data).unwrap();
        let id = f
            .backend
            .store
            .register_attachment(
                &Actor::system("test"),
                &NewAttachment {
                    sha256: stored.sha256,
                    size: data.len() as i64,
                    channel: Channel::Folder,
                    sender: None,
                    original_name: Some(name.into()),
                    file_type: Path::new(name)
                        .extension()
                        .map(|e| e.to_string_lossy().to_string()),
                },
            )
            .unwrap();
        id.to_string()
    }

    fn provenance(sha: &str) -> Provenance {
        Provenance {
            model: "testmodell".into(),
            provider: Some("lokal".into()),
            region: Some("SE".into()),
            client: Some("test 1".into()),
            rationale: "Programvara enligt fakturan.".into(),
            uncertainty: 0.2,
            arguments_sha256: sha.into(),
        }
    }

    fn proposal(fixture: &Fixture, document: &str) -> Proposal {
        let date = fixture.backend.store.today().to_string();
        let line = |account, debit_ore, credit_ore| ProposalLine {
            account,
            debit_ore,
            credit_ore,
            text: None,
        };
        Proposal {
            transaction_date: date,
            description: "Molntjänst, Exempel Hosting AB".into(),
            counterparty: Some(Counterparty {
                name: "Exempel Hosting AB".into(),
                org_number: Some("556000-0000".into()),
                vat_number: None,
                country: None,
            }),
            lines: vec![
                line(5420, 40_000, 0),
                line(2641, 10_000, 0),
                line(2440, 0, 50_000),
            ],
            document_ids: vec![document.into()],
            bank_row_ids: vec![],
            rule_ids: vec!["R1".into()],
            corrects: None,
        }
    }

    #[test]
    fn documents_accounts_and_media_types() {
        let mut f = fixture();
        let pdf = receive(&mut f, b"%PDF-1.7 faktura", "faktura.pdf");
        let unknown = receive(&mut f, b"\x89PNG\r\n\x1a\nbild", "skanning");
        let listed = f.backend.list_documents(true, 10).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].media_type, "application/pdf");
        // Utan ändelse säger listan inget om typen; underlaget själv avgör.
        assert_eq!(listed[1].media_type, "application/octet-stream");
        assert_eq!(listed[0].status, "oppen");
        let doc = f.backend.get_document(&unknown).unwrap();
        assert_eq!(doc.info.media_type, "image/png");
        assert_eq!(doc.data, b"\x89PNG\r\n\x1a\nbild".to_vec());
        assert!(matches!(
            f.backend.get_document("99"),
            Err(BackendError::NotFound(_))
        ));
        assert!(matches!(
            f.backend.get_document("x"),
            Err(BackendError::NotFound(_))
        ));
        let accounts = f.backend.accounts().unwrap();
        assert!(accounts
            .iter()
            .any(|a| a.number == 2641 && a.kind == "skuld" && a.active));
        let _ = pdf;
    }

    #[test]
    fn proposal_is_stored_once_with_provenance_and_marks_the_document() {
        let mut f = fixture();
        let doc = receive(&mut f, b"%PDF-1.7 faktura", "faktura.pdf");
        let p = proposal(&f, &doc);
        let first = f
            .backend
            .propose_voucher(p.clone(), provenance(&"a".repeat(64)))
            .unwrap();
        let again = f
            .backend
            .propose_voucher(p, provenance(&"a".repeat(64)))
            .unwrap();
        assert_eq!(first.id, again.id, "samma anrop ger samma förslag");
        let pending = f.backend.store.proposals(true).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].source, "agent");
        assert_eq!(pending[0].actor, "testmodell");
        assert_eq!(
            f.backend.list_documents(false, 10).unwrap()[0].status,
            "foreslagen"
        );
        assert!(f.backend.list_documents(true, 10).unwrap().is_empty());
    }

    #[test]
    fn invalid_proposals_are_refused_by_the_store() {
        let mut f = fixture();
        let doc = receive(&mut f, b"kvitto", "kvitto.txt");
        let mut p = proposal(&f, &doc);
        p.lines[0].account = 6071;
        assert!(matches!(
            f.backend.propose_voucher(p, provenance(&"b".repeat(64))),
            Err(BackendError::Invalid(_))
        ));
        let mut p = proposal(&f, &doc);
        p.transaction_date = "1999-01-01".into();
        assert!(matches!(
            f.backend.propose_voucher(p, provenance(&"c".repeat(64))),
            Err(BackendError::Invalid(_))
        ));
        let mut p = proposal(&f, &doc);
        p.document_ids = vec!["4711".into()];
        assert!(matches!(
            f.backend.propose_voucher(p, provenance(&"d".repeat(64))),
            Err(BackendError::NotFound(_))
        ));
        assert!(f.backend.store.proposals(true).unwrap().is_empty());
    }

    #[test]
    fn posted_vouchers_are_found_by_counterparty() {
        let mut f = fixture();
        let doc = receive(&mut f, b"faktura", "faktura.txt");
        let receipt = f
            .backend
            .propose_voucher(proposal(&f, &doc), provenance(&"e".repeat(64)))
            .unwrap();
        let id: i64 = receipt.id.parse().unwrap();
        let posted = f
            .backend
            .store
            .approve(&Actor::human("granskare"), id, &f.signer)
            .unwrap();
        let found = f.backend.search_counterparty("556000-0000", 5).unwrap();
        assert_eq!(found.len(), 1);
        assert!(found[0].voucher.starts_with(&posted.reference.to_string()));
        assert_eq!(found[0].lines[0].account, 5420);
        assert!(f.backend.search_counterparty("hosting", 5).unwrap().len() == 1);
        assert!(f
            .backend
            .search_counterparty("okänd", 5)
            .unwrap()
            .is_empty());
        assert_eq!(
            f.backend.list_documents(false, 10).unwrap()[0].status,
            "bokford"
        );
    }

    #[test]
    fn agent_cannot_post_even_through_the_store() {
        let mut f = fixture();
        let doc = receive(&mut f, b"faktura", "faktura.txt");
        let receipt = f
            .backend
            .propose_voucher(proposal(&f, &doc), provenance(&"f".repeat(64)))
            .unwrap();
        let id: i64 = receipt.id.parse().unwrap();
        let err = f
            .backend
            .store
            .approve(&Actor::agent("testmodell"), id, &f.signer)
            .unwrap_err();
        assert!(matches!(err, StoreError::NotAllowed(_)), "{err}");
    }

    #[test]
    fn rules_are_shown_as_text() {
        let row = |content: &str| RuleRow {
            name: "moms-hosting".into(),
            version: 2,
            valid_from: d(2026, 1, 1),
            valid_to: Some(d(2026, 12, 31)),
            content: content.into(),
        };
        let r = rule_from_row(row(
            r#"{"text": "Molntjänster: 5420 och 2641.", "konto": 5420}"#,
        ));
        assert_eq!(
            (r.id.as_str(), r.version, r.text.as_str()),
            ("moms-hosting", 2, "Molntjänster: 5420 och 2641.")
        );
        assert_eq!(r.valid_to.as_deref(), Some("2026-12-31"));
        assert_eq!(rule_from_row(row(r#""Fri text""#)).text, "Fri text");
        assert_eq!(
            rule_from_row(row(r#"{"konto": 5420}"#)).text,
            r#"{"konto": 5420}"#
        );
        assert!(fixture().backend.rules().unwrap().is_empty());
    }

    #[test]
    fn corrections_can_name_the_fiscal_year() {
        let mut f = fixture();
        let doc = receive(&mut f, b"faktura", "faktura.txt");
        let receipt = f
            .backend
            .propose_voucher(proposal(&f, &doc), provenance(&"8".repeat(64)))
            .unwrap();
        let posted = f
            .backend
            .store
            .approve(
                &Actor::human("granskare"),
                receipt.id.parse().unwrap(),
                &f.signer,
            )
            .unwrap();
        let year = f
            .backend
            .store
            .fiscal_year(posted.fiscal_year_id)
            .unwrap()
            .label();
        let mut fix = proposal(&f, &doc);
        fix.description = "Fel konto, ska vara 6540".into();
        fix.corrects = Some(format!("{} ({year})", posted.reference));
        let ok = f
            .backend
            .propose_voucher(fix.clone(), provenance(&"9".repeat(64)))
            .unwrap();
        let p = f.backend.store.proposal(ok.id.parse().unwrap()).unwrap();
        assert_eq!(p.corrects, Some(posted.voucher_id));
        fix.corrects = Some(format!("{} (1999)", posted.reference));
        assert!(matches!(
            f.backend.propose_voucher(fix, provenance(&"a".repeat(64))),
            Err(BackendError::Invalid(_))
        ));
    }

    #[test]
    fn model_names_are_cleaned_before_they_become_actors() {
        assert_eq!(model_label("claude-opus-5-5"), "claude-opus-5-5");
        assert_eq!(model_label("x\x1b[2K\rundrag\u{202E}"), "x[2Kundrag");
        assert_eq!(model_label(" \n "), "okänd modell");
        assert_eq!(model_label(&"m".repeat(500)).len(), 100);
        let mut f = fixture();
        let doc = receive(&mut f, b"faktura", "faktura.txt");
        let mut prov = provenance(&"7".repeat(64));
        prov.model = "evil\r\x1b[1Ainjected".into();
        f.backend.propose_voucher(proposal(&f, &doc), prov).unwrap();
        let actor = &f.backend.store.proposals(true).unwrap()[0].actor;
        assert_eq!(actor, "evil[1Ainjected");
    }

    #[test]
    fn questions_and_flags_become_cases_that_change_nothing() {
        let mut f = fixture();
        let doc = receive(&mut f, b"%PDF-1.7 lunch", "lunch.pdf");
        let q = f
            .backend
            .ask_question(Some(&doc), "Vilka deltog i lunchen?", provenance("0"))
            .unwrap();
        let flag = f
            .backend
            .flag_document(&doc, "dubblett", "", provenance("0"))
            .unwrap();
        assert_ne!(q.id, flag.id);
        let cases = f.backend.store.cases(true).unwrap();
        assert_eq!(cases.len(), 2);
        assert_eq!(
            cases[1].text, "Programvara enligt fakturan.",
            "motiveringen när kommentar saknas"
        );
        assert_eq!(
            f.backend.list_documents(true, 10).unwrap().len(),
            1,
            "underlaget är fortfarande öppet"
        );
        assert!(f
            .backend
            .ask_question(None, "Allmän fråga?", provenance("0"))
            .is_ok());
        assert!(matches!(
            f.backend
                .flag_document("999", "dubblett", "", provenance("0")),
            Err(BackendError::NotFound(_))
        ));
        assert!(matches!(
            f.backend.flag_document(&doc, "spam", "", provenance("0")),
            Err(BackendError::Invalid(_))
        ));
        assert!(matches!(
            f.backend.propose_match("b1", "A1", provenance("0")),
            Err(BackendError::NotFound(_))
        ));
    }

    #[test]
    fn bank_rows_are_listed_and_suggestions_reconcile_nothing() {
        use bok_store::{MatchTarget, NewBankRow};
        let mut f = fixture();
        let statement = receive(&mut f, b"<camt.053 utdrag/>", "utdrag.xml");
        let today = f.backend.store.today();
        let row = NewBankRow {
            post_id: "cd".repeat(32),
            bank_account: "SE4550000000058398257466".into(),
            booking_date: today,
            value_date: None,
            amount: Money::from_ore(-50_000),
            currency: "SEK".into(),
            text: "Exempel Hosting".into(),
            reference: Some("2026311".into()),
            counterparty: Some("Exempel Hosting AB".into()),
            balance: None,
        };
        f.backend
            .store
            .import_bank_rows(
                &Actor::human("granskare"),
                statement.parse().unwrap(),
                AccountNumber::new(1930).unwrap(),
                &[row],
            )
            .unwrap();
        let rows = f.backend.unmatched_bank_rows(10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].amount_ore, rows[0].text.as_str()),
            (-50_000, "Exempel Hosting")
        );
        let row_id = rows[0].post_id.clone();

        // Ett verifikationsförslag för bankraden blir ett matchningsförslag.
        let invoice = receive(&mut f, b"faktura", "faktura.txt");
        let mut p = proposal(&f, &invoice);
        p.lines = vec![
            ProposalLine {
                account: 5420,
                debit_ore: 40_000,
                credit_ore: 0,
                text: None,
            },
            ProposalLine {
                account: 2641,
                debit_ore: 10_000,
                credit_ore: 0,
                text: None,
            },
            ProposalLine {
                account: 1930,
                debit_ore: 0,
                credit_ore: 50_000,
                text: None,
            },
        ];
        p.bank_row_ids = vec![row_id.clone(), "999".into()];
        let receipt = f
            .backend
            .propose_voucher(p, provenance(&"9".repeat(64)))
            .unwrap();
        assert_eq!(receipt.warnings.len(), 1, "{:?}", receipt.warnings);
        assert!(receipt.warnings[0].contains("bankrad 999"));
        let id: i64 = receipt.id.parse().unwrap();
        let suggestions = f
            .backend
            .store
            .match_suggestions(Some(row_id.parse().unwrap()))
            .unwrap();
        assert_eq!(suggestions[0].target, SuggestedMatch::Proposal(id));

        // Efter bokföringen kan agenten föreslå verifikationen, men inte stämma av.
        let posted = f
            .backend
            .store
            .approve(&Actor::human("granskare"), id, &f.signer)
            .unwrap();
        let m = f
            .backend
            .propose_match(&row_id, &posted.reference.to_string(), provenance("0"))
            .unwrap();
        assert!(m.id.starts_with('M'));
        assert_eq!(f.backend.store.match_suggestions(None).unwrap().len(), 2);
        assert_eq!(
            f.backend.unmatched_bank_rows(10).unwrap().len(),
            1,
            "inget är avstämt"
        );
        assert!(matches!(
            f.backend.propose_match(&row_id, "A9", provenance("0")),
            Err(BackendError::NotFound(_))
        ));

        // Granskaren stämmer av; raden försvinner ur agentens lista.
        f.backend
            .store
            .reconcile_bank_row(
                &Actor::human("granskare"),
                row_id.parse().unwrap(),
                &MatchTarget::Voucher(posted.voucher_id),
            )
            .unwrap();
        assert!(f.backend.unmatched_bank_rows(10).unwrap().is_empty());
    }

    #[test]
    fn assets_are_listed_and_depreciation_is_only_proposed() {
        use bok_store::bok_inventarier::{AssetAccounts, NewAsset};
        let mut f = fixture();
        let human = Actor::human("granskare");
        let today = f.backend.store.today();
        let year = chrono::Datelike::year(&today);
        let chart: Vec<AccountChange> = [
            (1220, "Inventarier"),
            (1229, "Ack. avskrivningar"),
            (7832, "Avskrivningar"),
        ]
        .into_iter()
        .map(|(n, name)| AccountChange {
            number: AccountNumber::new(n).unwrap(),
            name: name.into(),
            active: true,
            sru: None,
            vat_code: None,
        })
        .collect();
        f.backend
            .store
            .change_chart(&human, d(year - 1, 1, 1), &chart, &f.signer)
            .unwrap();
        f.backend
            .store
            .register_asset(
                &human,
                &NewAsset {
                    description: "Server".into(),
                    acquired: d(year, 1, 10),
                    cost: Money::from_ore(6_000_000),
                    life_months: 60,
                    depreciation_start: d(year, 1, 1),
                    residual: Money::ZERO,
                    accounts: AssetAccounts {
                        asset: AccountNumber::new(1220).unwrap(),
                        accumulated: AccountNumber::new(1229).unwrap(),
                        expense: AccountNumber::new(7832).unwrap(),
                    },
                },
                None,
            )
            .unwrap();
        let assets = f.backend.assets().unwrap();
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].accumulated_ore, 1_200_000);
        assert_eq!(assets[0].status, "i_bruk");

        let receipt = f
            .backend
            .propose_depreciation(None, provenance("avskrivning"))
            .unwrap();
        let id: i64 = receipt.id.parse().unwrap();
        let p = f.backend.store.proposal(id).unwrap();
        assert_eq!(p.series, Series::B);
        assert_eq!(p.source, "agent");
        assert!(p.decision.is_none());
        // Samma förslag igen ger samma id, och agenten kan inte bokföra det.
        let again = f
            .backend
            .propose_depreciation(Some(&year.to_string()), provenance("igen"))
            .unwrap();
        assert_eq!(again.id, receipt.id);
        let agent = Actor::agent("agent");
        assert!(f.backend.store.approve(&agent, id, &f.signer).is_err());
        assert!(f
            .backend
            .propose_depreciation(Some("1999"), provenance("fel år"))
            .is_err());
    }
}
