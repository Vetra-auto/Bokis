//! Gemensam testmiljö: en ny databas med en mjukvarunyckel, ett
//! räkenskapsår 2026 och en liten kontoplan.

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use tempfile::TempDir;

use bok_store::bok_core::signature::SoftwareSigner;
use bok_store::bok_core::voucher::Line;
use bok_store::bok_core::{AccountNumber, Money, Series};
use bok_store::{
    AccountChange, Actor, FiscalYear, NewBook, NewProposal, Posted, Provenance, Store,
};

pub struct Book {
    pub store: Store,
    pub dir: TempDir,
    pub path: PathBuf,
    pub signer: SoftwareSigner,
    pub human: Actor,
    pub clock: Arc<Mutex<DateTime<Utc>>>,
    pub year: FiscalYear,
}

pub fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

pub fn acc(n: i64) -> AccountNumber {
    AccountNumber::new(n).unwrap()
}

pub fn kr(ore: i64) -> Money {
    Money::from_ore(ore)
}

pub fn chart() -> Vec<AccountChange> {
    [
        (1930, "Företagskonto"),
        (2081, "Aktiekapital"),
        (2091, "Balanserad vinst eller förlust"),
        (2440, "Leverantörsskulder"),
        (2641, "Debiterad ingående moms"),
        (3010, "Försäljning"),
        (5410, "Förbrukningsinventarier"),
        (6570, "Bankkostnader"),
    ]
    .into_iter()
    .map(|(n, name)| AccountChange {
        number: acc(n),
        name: name.into(),
        active: true,
        sru: None,
        vat_code: None,
    })
    .collect()
}

/// Klockan går en sekund per avläsning och börjar 2026-03-16 kl. 12 UTC.
pub fn book() -> Book {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bokis.db");
    let signer = SoftwareSigner::from_seed([42; 32]);
    let clock = Arc::new(Mutex::new(
        Utc.with_ymd_and_hms(2026, 3, 16, 12, 0, 0).unwrap(),
    ));
    let c = clock.clone();
    let mut store = Store::create_with_clock(
        &path,
        "bok-store-test",
        NewBook {
            company_name: "Testbolaget AB",
            org_number: "559999-9991",
            key_name: "granskare",
            public_key: &signer.public_key_openssh(),
            signer: &signer,
            security_level: bok_store::bok_core::security::SecurityLevel::Software,
        },
        Box::new(move || {
            let mut t = c.lock().unwrap();
            *t += Duration::seconds(1);
            *t
        }),
    )
    .unwrap();
    let human = Actor::human("granskare");
    let year = store
        .add_fiscal_year(&human, d(2026, 1, 1), d(2026, 12, 31))
        .unwrap();
    store
        .change_chart(&human, d(2025, 1, 1), &chart(), &signer)
        .unwrap();
    Book {
        store,
        dir,
        path,
        signer,
        human,
        clock,
        year,
    }
}

impl Book {
    /// Ställer klockan på ett datum (kl. 12 UTC). Klockan går aldrig bakåt,
    /// lika lite som i en riktig bokföring: är det redan det datumet står den
    /// kvar.
    pub fn set_today(&self, date: NaiveDate) {
        let mut clock = self.clock.lock().unwrap();
        assert!(
            date >= clock.date_naive(),
            "klockan kan inte ställas bakåt till {date}"
        );
        let noon = Utc.from_utc_datetime(&date.and_hms_opt(12, 0, 0).unwrap());
        if noon > *clock {
            *clock = noon;
        }
    }

    pub fn proposal(
        &self,
        series: Series,
        date: NaiveDate,
        description: &str,
        lines: Vec<Line>,
    ) -> NewProposal {
        NewProposal {
            series,
            date,
            description: description.into(),
            counterparty: None,
            lines,
            attachments: vec![],
            idempotency_key: None,
            provenance: Provenance::default(),
        }
    }

    /// Ett förslag om ett köp: debet `debit_account`, kredit 1930.
    pub fn purchase(
        &self,
        date: NaiveDate,
        description: &str,
        debit_account: i64,
        ore: i64,
    ) -> NewProposal {
        self.proposal(
            Series::A,
            date,
            description,
            vec![
                Line::debit(acc(debit_account), kr(ore)),
                Line::credit(acc(1930), kr(ore)),
            ],
        )
    }

    pub fn propose(&mut self, p: &NewProposal) -> i64 {
        self.store.create_proposal(&self.human, p).unwrap()
    }

    pub fn approve(&mut self, proposal_id: i64) -> Posted {
        self.store
            .approve(&self.human, proposal_id, &self.signer)
            .unwrap()
    }

    pub fn post(&mut self, p: &NewProposal) -> Posted {
        let id = self.propose(p);
        self.approve(id)
    }

    pub fn post_purchase(&mut self, date: NaiveDate, description: &str, ore: i64) -> Posted {
        let p = self.purchase(date, description, 5410, ore);
        self.post(&p)
    }

    /// En egen anslutning förbi programmet, som ett annat verktyg skulle ha.
    pub fn raw(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(&self.path).unwrap()
    }

    pub fn period_id(&self, label: &str) -> i64 {
        self.store.period_by_label(label).unwrap().id
    }

    pub fn event_count(&self) -> usize {
        self.store.events(None).unwrap().len()
    }
}
