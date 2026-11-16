//! Bokslutsförslagen mot en riktig databas: avskrivningar, avyttring,
//! periodisering och skatt blir förslag i serie B och bokförs först när
//! granskaren godkänner dem.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use chrono::{NaiveDate, TimeZone, Utc};

use bok_bokslut::accrual::{Accrual, AccrualKind};
use bok_bokslut::{depreciation, disposal, tax};
use bok_core::signature::SoftwareSigner;
use bok_core::voucher::Line;
use bok_core::{AccountNumber, Money, Series};
use bok_inventarier::{AssetAccounts, Disposal, DisposalAccounts, DisposalKind, NewAsset};
use bok_store::{AccountChange, Actor, FiscalYear, NewBook, NewProposal, Provenance, Store};

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn acc(n: i64) -> AccountNumber {
    AccountNumber::new(n).unwrap()
}

fn kr(k: i64) -> Money {
    Money::from_ore(k * 100)
}

struct Book {
    store: Store,
    _dir: tempfile::TempDir,
    signer: SoftwareSigner,
    human: Actor,
    year: FiscalYear,
}

fn book() -> Book {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bokis.db");
    let signer = SoftwareSigner::from_seed([7; 32]);
    let clock = std::sync::Mutex::new(Utc.with_ymd_and_hms(2027, 1, 15, 12, 0, 0).unwrap());
    let mut store = Store::create_with_clock(
        &path,
        "bok-bokslut-test",
        NewBook {
            company_name: "Provbolaget AB",
            org_number: "559000-0005",
            key_name: "granskare",
            public_key: &signer.public_key_openssh(),
            signer: &signer,
            security_level: bok_core::security::SecurityLevel::Software,
        },
        Box::new(move || {
            let mut t = clock.lock().unwrap();
            *t += chrono::Duration::seconds(1);
            *t
        }),
    )
    .unwrap();
    let human = Actor::human("granskare");
    let year = store
        .add_fiscal_year(&human, d(2026, 4, 27), d(2026, 12, 31))
        .unwrap();
    store
        .add_fiscal_year(&human, d(2027, 1, 1), d(2027, 12, 31))
        .unwrap();
    let chart: Vec<AccountChange> = [
        (1220, "Inventarier och verktyg"),
        (1229, "Ackumulerade avskrivningar på inventarier"),
        (1790, "Övriga förutbetalda kostnader och upplupna intäkter"),
        (1930, "Företagskonto"),
        (2081, "Aktiekapital"),
        (2512, "Beräknad inkomstskatt"),
        (2611, "Utgående moms på försäljning inom Sverige, 25 %"),
        (2990, "Övriga upplupna kostnader och förutbetalda intäkter"),
        (3001, "Försäljning inom Sverige, 25 % moms"),
        (3973, "Vinst vid avyttring av maskiner och inventarier"),
        (6072, "Representation, ej avdragsgill"),
        (6310, "Företagsförsäkringar"),
        (7832, "Avskrivningar på inventarier och verktyg"),
        (7973, "Förlust vid avyttring av maskiner och inventarier"),
        (8910, "Skatt som belastar årets resultat"),
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
        .change_chart(&human, d(2026, 4, 27), &chart, &signer)
        .unwrap();
    Book {
        store,
        _dir: dir,
        signer,
        human,
        year,
    }
}

impl Book {
    fn post(&mut self, date: NaiveDate, text: &str, lines: Vec<Line>) {
        let id = self
            .store
            .create_proposal(
                &self.human,
                &NewProposal {
                    series: Series::A,
                    date,
                    description: text.into(),
                    counterparty: None,
                    lines,
                    attachments: vec![],
                    idempotency_key: None,
                    provenance: Provenance::default(),
                },
            )
            .unwrap();
        self.approve(id);
    }

    fn approve(&mut self, id: i64) {
        self.store.approve(&self.human, id, &self.signer).unwrap();
    }

    fn balance(&self, n: i64) -> Money {
        self.store.account_balance(acc(n), self.year.end).unwrap()
    }

    fn laptop(&mut self) -> i64 {
        self.post(
            d(2026, 5, 12),
            "Dator",
            vec![
                Line::debit(acc(1220), kr(36_000)),
                Line::credit(acc(1930), kr(36_000)),
            ],
        );
        self.store
            .register_asset(
                &self.human,
                &NewAsset {
                    description: "Bärbar dator".into(),
                    acquired: d(2026, 5, 12),
                    cost: kr(36_000),
                    life_months: 36,
                    depreciation_start: d(2026, 5, 1),
                    residual: Money::ZERO,
                    accounts: AssetAccounts {
                        asset: acc(1220),
                        accumulated: acc(1229),
                        expense: acc(7832),
                    },
                },
                None,
            )
            .unwrap()
    }
}

#[test]
fn depreciation_is_a_proposal_until_approved_and_only_once() {
    let mut b = book();
    b.laptop();
    let year = b.year;
    let (proposed, rows) =
        depreciation::propose_depreciation(&mut b.store, &b.human, &year, Provenance::default())
            .unwrap();
    assert_eq!(rows[0].amount, kr(8_000));
    assert!(proposed.notes.is_empty(), "{:?}", proposed.notes);
    assert_eq!(b.store.proposal(proposed.id).unwrap().series, Series::B);
    // Inget är bokfört förrän granskaren godkänner.
    assert_eq!(b.balance(1229), Money::ZERO);
    // Samma förslag igen ger samma id.
    let (again, _) =
        depreciation::propose_depreciation(&mut b.store, &b.human, &year, Provenance::default())
            .unwrap();
    assert_eq!(again.id, proposed.id);
    b.approve(proposed.id);
    assert_eq!(b.balance(1229), kr(-8_000));
    assert_eq!(b.balance(7832), kr(8_000));
    let checks = depreciation::reconcile(&b.store, &year).unwrap();
    assert!(
        checks.iter().all(|c| c.difference().is_zero()),
        "{checks:?}"
    );
    // Bokförda: inget nytt förslag.
    assert!(
        depreciation::propose_depreciation(&mut b.store, &b.human, &year, Provenance::default())
            .is_err()
    );
}

#[test]
fn an_agent_may_propose_depreciation_but_not_approve_it() {
    let mut b = book();
    b.laptop();
    let year = b.year;
    let agent = Actor::agent("agent");
    let (p, _) =
        depreciation::propose_depreciation(&mut b.store, &agent, &year, Provenance::default())
            .unwrap();
    assert!(b.store.approve(&agent, p.id, &b.signer).is_err());
}

#[test]
fn selling_an_asset() {
    let mut b = book();
    let id = b.laptop();
    let sold = Disposal {
        date: d(2026, 11, 20),
        kind: DisposalKind::Sold { price: kr(30_000) },
        note: Some("Såld till Exempel AB".into()),
    };
    let accounts = DisposalAccounts {
        payment: acc(1930),
        output_vat: acc(2611),
        gain: acc(3973),
        loss: acc(7973),
    };
    let (proposed, voucher) = disposal::dispose(
        &mut b.store,
        &b.human,
        id,
        &sold,
        kr(7_500),
        &accounts,
        Provenance::default(),
    )
    .unwrap()
    .unwrap();
    // Maj–oktober: 6 000 avskrivet, bokfört värde 30 000, ingen vinst.
    assert_eq!(voucher.depreciation_this_year, kr(6_000));
    assert_eq!(voucher.result, Money::ZERO);
    b.approve(proposed.id);
    assert_eq!(b.balance(1220), Money::ZERO);
    assert_eq!(b.balance(1229), Money::ZERO);
    assert_eq!(b.balance(1930), kr(-36_000 + 37_500));
    assert!(b.store.asset(id).unwrap().disposal.is_some());
    // Ett sålt inventarium skrivs inte av vid årets slut.
    let year = b.year;
    assert!(
        depreciation::propose_depreciation(&mut b.store, &b.human, &year, Provenance::default())
            .is_err()
    );
}

#[test]
fn accrual_with_reversal_next_year() {
    let mut b = book();
    b.post(
        d(2026, 11, 1),
        "Försäkring",
        vec![
            Line::debit(acc(6310), kr(12_000)),
            Line::credit(acc(1930), kr(12_000)),
        ],
    );
    let year = b.year;
    let accrual = Accrual {
        kind: AccrualKind::PrepaidExpense,
        account: acc(6310),
        balance_account: acc(1790),
        amount: kr(12_000),
        from: d(2026, 11, 1),
        to: d(2027, 10, 31),
        text: "Företagsförsäkring".into(),
    };
    let (p, plan) = bok_bokslut::accrual::propose(
        &mut b.store,
        &b.human,
        &year,
        &accrual,
        true,
        false,
        Provenance::default(),
    )
    .unwrap();
    assert_eq!(plan.amount, Money::from_ore(999_452));
    assert_eq!(p.notes.len(), 2, "{:?}", p.notes);
    b.approve(p.id);
    assert_eq!(b.balance(1790), Money::from_ore(999_452));
    let reversal = b.store.proposals(true).unwrap();
    assert_eq!(reversal.len(), 1);
    assert_eq!(reversal[0].date, d(2027, 1, 1));

    // Under gränsen bara med --anda.
    let mut small = accrual.clone();
    small.amount = kr(6_000);
    assert!(
        bok_bokslut::accrual::propose(
            &mut b.store,
            &b.human,
            &year,
            &small,
            false,
            false,
            Provenance::default()
        )
        .is_err()
    );
}

#[test]
fn tax_on_the_surplus_with_non_deductible_costs_added_back() {
    let mut b = book();
    b.post(
        d(2026, 6, 1),
        "Aktiekapital",
        vec![
            Line::debit(acc(1930), kr(25_000)),
            Line::credit(acc(2081), kr(25_000)),
        ],
    );
    b.post(
        d(2026, 9, 1),
        "Försäljning",
        vec![
            Line::debit(acc(1930), kr(125_000)),
            Line::credit(acc(3001), kr(100_000)),
            Line::credit(acc(2611), kr(25_000)),
        ],
    );
    b.post(
        d(2026, 10, 1),
        "Kundmiddag",
        vec![
            Line::debit(acc(6072), kr(2_005)),
            Line::credit(acc(1930), kr(2_005)),
        ],
    );
    let year = b.year;
    let outcome = tax::propose_tax(
        &mut b.store,
        &b.human,
        &year,
        vec![],
        acc(8910),
        acc(2512),
        Provenance::default(),
    )
    .unwrap();
    // Resultat 97 995 + ej avdragsgill representation 2 005 = 100 000.
    assert_eq!(outcome.declaration.tax.taxable, 100_000);
    assert_eq!(outcome.declaration.tax.tax_ore, 2_060_000);
    let p = outcome.proposed.unwrap();
    b.approve(p.id);
    assert_eq!(b.balance(8910), kr(20_600));
    assert_eq!(b.balance(2512), kr(-20_600));
    // När skatten är bokförd blir den samma och inget nytt förslag läggs.
    let again = tax::propose_tax(
        &mut b.store,
        &b.human,
        &year,
        vec![],
        acc(8910),
        acc(2512),
        Provenance::default(),
    )
    .unwrap();
    assert!(again.proposed.is_none());
    assert_eq!(again.declaration.tax.tax_ore, 2_060_000);
    assert!(again.declaration.is_ok());
}
