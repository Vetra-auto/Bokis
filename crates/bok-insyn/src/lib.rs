//! Insyn i Bokis för revisorer och Skatteverket.
//!
//! Bokföringen visas som den står i databasen, utan möjlighet att ändra
//! något: översikt med kontrollens resultat, grundbok, verifikationer med
//! underlag, signatur och historik, huvudbok, saldobalans, årsbokslut med
//! resultat- och balansräkning, moms, bankavstämning, inventarieregistret,
//! utkastet till inkomstdeklaration 2, underlag, behandlingshistorik och
//! systemdokumentation.
//!
//! Samma sidor används på två sätt:
//! - live på 127.0.0.1 med en engångslänk ([`server`]), och
//! - som fristående filer ([`export`]) som öppnas i valfri webbläsare utan
//!   Bokis och utan nät, till exempel från ett USB-minne.
//!
//! Sidorna har inget JavaScript och inga externa resurser. Alla länkar är
//! relativa, så exporten fungerar från en lokal katalog.

use std::cell::OnceCell;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use chrono::NaiveDate;

use bok_archive::Archive;
use bok_store::bok_core::VoucherRef;
use bok_store::{AttachmentRow, Company, EventRow, FiscalYear, Store, VerifyReport};

mod assets_page;
mod declaration_page;
pub mod html;
pub mod links;
mod markdown;
mod pages;
pub mod server;
pub mod sysdoc;
pub mod sysdoc_archive;

#[derive(Debug, thiserror::Error)]
pub enum InsynError {
    #[error(transparent)]
    Store(#[from] bok_store::StoreError),
    #[error(transparent)]
    Archive(#[from] bok_archive::ArchiveError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("kunde inte skriva sidan: {0}")]
    Fmt(#[from] std::fmt::Error),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, InsynError>;

/// Live på 127.0.0.1 eller som fristående filer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Live,
    Export,
}

/// En sida eller fil i insynen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    pub content_type: &'static str,
    pub body: Vec<u8>,
    /// Filen ska laddas ner, inte visas.
    pub download: bool,
}

impl File {
    fn html(body: String) -> Self {
        Self {
            content_type: "text/html; charset=utf-8",
            body: body.into_bytes(),
            download: false,
        }
    }
}

/// En fil som tas fram utanför insynen, till exempel SIE-filen för ett år,
/// och följer med i exporten och på exportsidan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtraFile {
    /// Relativ sökväg, till exempel `sie/bokis-2026.se`.
    pub path: String,
    pub body: Vec<u8>,
    /// Vad filen är, för exportsidan.
    pub description: String,
}

/// Uppslag som flera sidor behöver.
#[derive(Debug, Default)]
pub(crate) struct Index {
    /// Verifikationerna som bygger på varje underlag, per SHA-256.
    pub used_by: BTreeMap<String, Vec<(i64, VoucherRef)>>,
    /// År och nummer per verifikation.
    pub vouchers: HashMap<i64, (i64, VoucherRef)>,
    /// Verifikationen som ett godkänt förslag blev.
    pub by_proposal: HashMap<i64, i64>,
    /// Händelserna per objekt, till exempel `verifikation:12`, som index i historiken.
    pub events_by_object: HashMap<String, Vec<usize>>,
    /// Bankraderna som stämts av mot varje verifikation.
    pub bank_by_voucher: HashMap<i64, Vec<i64>>,
}

/// Bokföringen sedd genom insynen.
pub struct Insyn<'a> {
    pub(crate) store: &'a Store,
    pub(crate) archive: &'a Archive,
    pub(crate) db: &'a Path,
    pub(crate) program: &'a str,
    pub(crate) generated: String,
    pub(crate) mode: Mode,
    pub(crate) company: Company,
    pub(crate) years: Vec<FiscalYear>,
    /// Alla räkenskapsår i databasen, även när insynen gäller ett år.
    pub(crate) all_years: Vec<FiscalYear>,
    pub(crate) extra: Vec<ExtraFile>,
    events: OnceCell<Vec<EventRow>>,
    attachments: OnceCell<Vec<AttachmentRow>>,
    index: OnceCell<Index>,
    verify: OnceCell<VerifyReport>,
    /// När kontrollen gjordes, om den gjordes före insynen.
    checked_at: Option<String>,
}

pub(crate) const CSS: &str = include_str!("../assets/insyn.css");

/// Katalogen för ett räkenskapsår: `ar-2026` eller `ar-2025-26`.
pub(crate) fn year_dir(year: &FiscalYear) -> String {
    format!("ar-{}", year.label().replace('/', "-"))
}

/// Filändelse för ett underlag efter dess innehåll. Allt som inte är bild,
/// PDF eller ren text får `.bin`, så att inget underlag kan öppnas som en
/// webbsida, varken live eller ur en export.
pub(crate) fn extension(media_type: &str) -> &'static str {
    match media_type {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "application/pdf" => "pdf",
        "text/plain" | "text/csv" | "application/xml" | "text/xml" | "application/json"
        | "message/rfc822" => "txt",
        _ => "bin",
    }
}

fn content_type_for(ext: &str) -> (&'static str, bool) {
    match ext {
        "png" => ("image/png", false),
        "jpg" => ("image/jpeg", false),
        "gif" => ("image/gif", false),
        "webp" => ("image/webp", false),
        "pdf" => ("application/pdf", false),
        "txt" => ("text/plain; charset=utf-8", false),
        _ => ("application/octet-stream", true),
    }
}

/// Godtar bara enkla relativa sökvägar för extrafiler.
fn safe_extra_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && path.split('/').all(|seg| {
            !seg.is_empty()
                && seg != "."
                && seg != ".."
                && seg
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        })
}

/// Hela kontrollen som `bok verify` gör: databasen och underlagen i arkivet.
pub fn verify_all(store: &Store, archive: &Archive) -> Result<VerifyReport> {
    let mut report = store.verify()?;
    let mut files = bok_store::Check {
        name: "Underlagen i arkivet är oförändrade",
        ..Default::default()
    };
    for a in store.attachments()? {
        files.checked += 1;
        match archive.read(&a.sha256) {
            Ok(_) => {}
            Err(bok_archive::ArchiveError::Missing(_)) => files
                .problems
                .push(format!("underlag {}: filen saknas i arkivet", a.id)),
            Err(bok_archive::ArchiveError::Changed { .. }) => files
                .problems
                .push(format!("underlag {}: filen har ändrats", a.id)),
            Err(e) => files.problems.push(format!("underlag {}: {e}", a.id)),
        }
    }
    report.checks.push(files);
    Ok(report)
}

impl<'a> Insyn<'a> {
    pub fn new(
        store: &'a Store,
        archive: &'a Archive,
        db: &'a Path,
        program: &'a str,
        generated: impl Into<String>,
        mode: Mode,
    ) -> Result<Self> {
        let years = store.fiscal_years()?;
        Ok(Self {
            store,
            archive,
            db,
            program,
            generated: generated.into(),
            mode,
            company: store.company()?,
            all_years: years.clone(),
            years,
            extra: Vec::new(),
            events: OnceCell::new(),
            attachments: OnceCell::new(),
            index: OnceCell::new(),
            verify: OnceCell::new(),
            checked_at: None,
        })
    }

    /// En kontroll som redan är gjord, så att den inte görs om. Den läser
    /// varje underlag i arkivet och tar tid när de är många.
    pub fn with_verify(self, report: VerifyReport, checked_at: impl Into<String>) -> Self {
        let _ = self.verify.set(report);
        Self {
            checked_at: Some(checked_at.into()),
            ..self
        }
    }

    /// Kontrollen, om den gjordes för den här insynen.
    pub fn take_verify(&mut self) -> Option<VerifyReport> {
        self.verify.take()
    }

    /// När kontrollen gjordes.
    pub(crate) fn checked_at(&self) -> &str {
        self.checked_at.as_deref().unwrap_or(&self.generated)
    }

    /// Bara ett räkenskapsår: årets sidor och underlag. Historiken och
    /// systemdokumentationen gäller alltid hela bokföringen, eftersom
    /// hashkedjan bara kan kontrolleras i sin helhet.
    pub fn only_year(mut self, fiscal_year_id: i64) -> Result<Self> {
        self.years.retain(|y| y.id == fiscal_year_id);
        if self.years.is_empty() {
            return Err(InsynError::Other(format!(
                "räkenskapsåret {fiscal_year_id} finns inte"
            )));
        }
        Ok(self)
    }

    /// Filer som tas fram utanför insynen, till exempel SIE.
    pub fn with_files(mut self, files: Vec<ExtraFile>) -> Result<Self> {
        for f in &files {
            if !safe_extra_path(&f.path) {
                return Err(InsynError::Other(format!("ogiltig sökväg {}", f.path)));
            }
        }
        self.extra = files;
        Ok(self)
    }

    pub(crate) fn year_by_dir(&self, dir: &str) -> Option<&FiscalYear> {
        self.years.iter().find(|y| year_dir(y) == dir)
    }

    /// Ett år i insynen, eller `None` om det inte är med.
    pub(crate) fn year_by_id(&self, id: i64) -> Option<&FiscalYear> {
        self.years.iter().find(|y| y.id == id)
    }

    /// Året som omfattar i dag, annars det senaste.
    pub(crate) fn current_year(&self) -> Option<&FiscalYear> {
        let today = self.store.today();
        self.years
            .iter()
            .find(|y| y.contains(today))
            .or_else(|| self.years.last())
    }

    /// Året före, om det finns i databasen.
    pub(crate) fn previous_year(&self, year: &FiscalYear) -> Option<&FiscalYear> {
        let day_before = year.start.pred_opt()?;
        self.all_years.iter().find(|y| y.end == day_before)
    }

    /// Året efter, om det finns i databasen.
    pub(crate) fn next_year(&self, year: &FiscalYear) -> Option<&FiscalYear> {
        let day_after = year.end.succ_opt()?;
        self.all_years.iter().find(|y| y.start == day_after)
    }

    pub(crate) fn events(&self) -> Result<&[EventRow]> {
        if let Some(e) = self.events.get() {
            return Ok(e);
        }
        let events = self.store.events(None)?;
        Ok(self.events.get_or_init(|| events))
    }

    /// Alla mottagna underlag i databasen.
    pub(crate) fn all_attachments(&self) -> Result<&[AttachmentRow]> {
        if let Some(a) = self.attachments.get() {
            return Ok(a);
        }
        let rows = self.store.attachments()?;
        Ok(self.attachments.get_or_init(|| rows))
    }

    /// Index över verifikationer, underlag och händelser, byggt en gång.
    pub(crate) fn index(&self) -> Result<&Index> {
        if let Some(i) = self.index.get() {
            return Ok(i);
        }
        let mut index = Index::default();
        for y in &self.all_years {
            for v in self.store.journal(y.id)? {
                index.vouchers.insert(v.id, (y.id, v.reference));
                index.by_proposal.insert(v.proposal_id, v.id);
                for sha in v.attachments {
                    index
                        .used_by
                        .entry(sha)
                        .or_default()
                        .push((y.id, v.reference));
                }
            }
        }
        for r in self.store.bank_rows(false)? {
            if let Some(v) = r.reconciliation.and_then(|rec| rec.voucher_id) {
                index.bank_by_voucher.entry(v).or_default().push(r.id);
            }
        }
        for (n, e) in self.events()?.iter().enumerate() {
            if let Some(object) = &e.object {
                index
                    .events_by_object
                    .entry(object.clone())
                    .or_default()
                    .push(n);
            }
        }
        Ok(self.index.get_or_init(|| index))
    }

    /// Underlagen i insynen: alla, eller för ett enskilt år de som årets
    /// verifikationer bygger på och de som togs emot under året.
    pub(crate) fn attachments(&self) -> Result<Vec<&AttachmentRow>> {
        let all = self.all_attachments()?;
        if self.years.len() == self.all_years.len() {
            return Ok(all.iter().collect());
        }
        let used_by = &self.index()?.used_by;
        Ok(all
            .iter()
            .filter(|a| {
                let used = used_by
                    .get(&a.sha256)
                    .is_some_and(|v| v.iter().any(|(y, _)| self.year_by_id(*y).is_some()));
                let received = a
                    .received_at
                    .get(..10)
                    .and_then(|d| d.parse::<NaiveDate>().ok());
                used || received.is_some_and(|d| self.years.iter().any(|y| y.contains(d)))
            })
            .collect())
    }

    /// Kontrollen av hela bokföringen, gjord en gång per insyn.
    pub(crate) fn verify(&self) -> Result<&VerifyReport> {
        if let Some(r) = self.verify.get() {
            return Ok(r);
        }
        let report = verify_all(self.store, self.archive)?;
        Ok(self.verify.get_or_init(|| report))
    }

    /// Kontonamnen som de stod vid `at`, med dagens namn för konton som
    /// lagts till senare.
    pub(crate) fn account_names(&self, at: NaiveDate) -> Result<HashMap<u16, String>> {
        let mut names: HashMap<u16, String> = self
            .store
            .chart(self.store.today().max(at))?
            .into_iter()
            .map(|a| (a.number.value(), a.name))
            .collect();
        for a in self.store.chart(at)? {
            names.insert(a.number.value(), a.name);
        }
        Ok(names)
    }

    /// Länken till en verifikation från en sida, eller `None` om året inte
    /// är med i insynen.
    pub(crate) fn voucher_href(
        &self,
        up: &str,
        fiscal_year_id: i64,
        reference: VoucherRef,
    ) -> Option<String> {
        self.year_by_id(fiscal_year_id)
            .map(|y| format!("{up}{}/v-{reference}.html", year_dir(y)))
    }

    /// Verifikationsnumret, med året om det är ett annat än `own_year`.
    pub(crate) fn voucher_label(
        &self,
        fiscal_year_id: i64,
        reference: VoucherRef,
        own_year: Option<i64>,
    ) -> String {
        if own_year == Some(fiscal_year_id) {
            return reference.to_string();
        }
        match self.all_years.iter().find(|y| y.id == fiscal_year_id) {
            Some(y) => format!("{reference} ({})", y.label()),
            None => reference.to_string(),
        }
    }

    /// Sidan eller filen på en relativ sökväg, till exempel
    /// `ar-2026/grundbok.html`. `None` om den inte finns.
    pub fn get(&self, path: &str) -> Result<Option<File>> {
        let path = path.trim_start_matches('/');
        if let Some(f) = self.extra.iter().find(|f| f.path == path) {
            return Ok(Some(File {
                content_type: "application/octet-stream",
                body: f.body.clone(),
                download: true,
            }));
        }
        let parts: Vec<&str> = path.split('/').collect();
        let file = match parts.as_slice() {
            [""] | ["index.html"] => File::html(pages::overview(self)?),
            ["insyn.css"] => File {
                content_type: "text/css; charset=utf-8",
                body: CSS.as_bytes().to_vec(),
                download: false,
            },
            ["underlag.html"] => File::html(pages::attachments(self)?),
            ["historik.html"] => File::html(pages::history(self)?),
            ["systemdokumentation.html"] => File::html(pages::system_doc(self)?),
            ["export.html"] => File::html(pages::export_page(self)?),
            ["underlag", name] => match self.attachment_file(name)? {
                Some(f) => f,
                None => return Ok(None),
            },
            [dir, name] => {
                let Some(year) = self.year_by_dir(dir) else {
                    return Ok(None);
                };
                match *name {
                    "grundbok.html" => File::html(pages::journal(self, year)?),
                    "grundbok.csv" => File {
                        content_type: "text/csv; charset=utf-8",
                        body: pages::journal_csv(self, year)?.into_bytes(),
                        download: true,
                    },
                    "huvudbok.html" => File::html(pages::ledger(self, year)?),
                    "saldobalans.html" => File::html(pages::trial_balance(self, year)?),
                    "arsbokslut.html" => File::html(pages::annual_accounts(self, year)?),
                    "moms.html" => File::html(pages::vat(self, year)?),
                    "bank.html" => File::html(pages::bank(self, year)?),
                    "inventarier.html" => File::html(assets_page::assets(self, year)?),
                    "deklaration.html" => File::html(declaration_page::declaration(self, year)?),
                    other => {
                        let Some(reference) = other
                            .strip_prefix("v-")
                            .and_then(|r| r.strip_suffix(".html"))
                        else {
                            return Ok(None);
                        };
                        let Ok(vref) = reference.parse::<VoucherRef>() else {
                            return Ok(None);
                        };
                        // Bara den kanoniska formen, så att varje verifikation har en adress.
                        if vref.to_string() != reference {
                            return Ok(None);
                        }
                        match self.store.voucher_by_ref(year.id, vref) {
                            Ok(v) => File::html(pages::voucher(self, year, &v)?),
                            Err(bok_store::StoreError::Missing { .. }) => return Ok(None),
                            Err(e) => return Err(e.into()),
                        }
                    }
                }
            }
            _ => return Ok(None),
        };
        Ok(Some(file))
    }

    /// Ett underlag som fil, `underlag/<sha256>.<ändelse>`. Innehållet läses
    /// ur arkivet och kontrolleras mot hashen.
    fn attachment_file(&self, name: &str) -> Result<Option<File>> {
        let Some((sha, ext)) = name.split_once('.') else {
            return Ok(None);
        };
        if bok_archive::validate_hash(sha).is_err() {
            return Ok(None);
        }
        if !self.attachments()?.iter().any(|a| a.sha256 == sha) {
            return Ok(None);
        }
        let (path, media, data) = self.attachment(sha)?;
        if path != format!("underlag/{sha}.{ext}") {
            return Ok(None);
        }
        let (content_type, download) = content_type_for(extension(&media));
        Ok(Some(File {
            content_type,
            body: data,
            download,
        }))
    }

    /// Ett underlag: sökvägen relativt sajtens rot, medietypen och innehållet.
    pub(crate) fn attachment(&self, sha256: &str) -> Result<(String, String, Vec<u8>)> {
        let data = self.archive.read(sha256)?;
        let name = self
            .all_attachments()?
            .iter()
            .find(|a| a.sha256 == sha256)
            .and_then(|a| a.original_name.clone())
            .unwrap_or_default();
        let media = bok_archive::media::detect(&data, &name);
        Ok((
            format!("underlag/{sha256}.{}", extension(&media)),
            media,
            data,
        ))
    }

    /// Alla sidor och filer, för en export.
    pub fn paths(&self) -> Result<Vec<String>> {
        let mut out: Vec<String> = [
            "index.html",
            "insyn.css",
            "underlag.html",
            "historik.html",
            "systemdokumentation.html",
            "export.html",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        for y in &self.years {
            let dir = year_dir(y);
            for page in [
                "grundbok.html",
                "grundbok.csv",
                "huvudbok.html",
                "saldobalans.html",
                "arsbokslut.html",
                "moms.html",
                "bank.html",
                "inventarier.html",
                "deklaration.html",
            ] {
                out.push(format!("{dir}/{page}"));
            }
            for v in self.store.journal(y.id)? {
                out.push(format!("{dir}/v-{}.html", v.reference));
            }
        }
        for a in self.attachments()? {
            let (path, _, _) = self.attachment(&a.sha256)?;
            out.push(path);
        }
        for f in &self.extra {
            out.push(f.path.clone());
        }
        Ok(out)
    }
}

/// Resultatet av en export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportReport {
    pub dir: std::path::PathBuf,
    pub files: usize,
    pub manifest_sha256: String,
}

/// Skriver insynen som fristående filer i `dir`, som inte får finnas, med
/// `manifest.sha256` i samma form som `sha256sum`.
pub fn export(insyn: &Insyn, dir: &Path) -> Result<ExportReport> {
    if dir.exists() {
        return Err(InsynError::Other(format!(
            "{} finns redan; en export skrivs aldrig över",
            dir.display()
        )));
    }
    std::fs::create_dir_all(dir)?;
    let mut manifest = String::new();
    let mut files = 0;
    for path in insyn.paths()? {
        let Some(file) = insyn.get(&path)? else {
            return Err(InsynError::Other(format!(
                "sidan {path} kunde inte tas fram"
            )));
        };
        let target = dir.join(&path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, &file.body)?;
        manifest.push_str(&format!(
            "{}  {path}\n",
            bok_store::bok_core::canonical::sha256_hex(&file.body)
        ));
        files += 1;
    }
    std::fs::write(dir.join("manifest.sha256"), &manifest)?;
    Ok(ExportReport {
        dir: dir.to_path_buf(),
        files,
        manifest_sha256: bok_store::bok_core::canonical::sha256_hex(manifest.as_bytes()),
    })
}
