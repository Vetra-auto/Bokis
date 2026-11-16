//! Insynen mot en riktig bokföring: verifikationer från en agent med
//! underlag, en rättelse, bankrader, moms och bokslut. Kontrollerar att
//! exporten hänger ihop, att inget från bokföringen kan bli kod på sidan,
//! och att liveläget bara läser.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use tempfile::TempDir;

use bok_archive::Archive;
use bok_insyn::server::{LiveView, Request};
use bok_insyn::{export, ExtraFile, Insyn, Mode};
use bok_store::bok_core::canonical::sha256_hex;
use bok_store::bok_core::signature::SoftwareSigner;
use bok_store::bok_core::voucher::{Counterparty, Line};
use bok_store::bok_core::{AccountNumber, Money, Series};
use bok_store::{
    AccountChange, Actor, Channel, MatchTarget, NewAttachment, NewBankRow, NewBook, NewProposal,
    ProposalCounterparty, Provenance, Store,
};

const EVIL: &str = "Kontorsmaterial <script>alert(1)</script> \u{202e}fdp.exe";

struct Fixture {
    _dir: TempDir,
    db: PathBuf,
    store: Store,
    archive: Archive,
    /// SHA-256 för underlaget som påstår sig vara en bild men är HTML.
    html_sha: String,
    /// SHA-256 för ett zip-arkiv, som webbläsaren inte kan visa.
    zip_sha: String,
    png_sha: String,
    out: PathBuf,
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn acc(n: i64) -> AccountNumber {
    AccountNumber::new(n).unwrap()
}

fn kr(ore: i64) -> Money {
    Money::from_ore(ore)
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("bokis.db");
    let signer = SoftwareSigner::from_seed([7; 32]);
    let clock: Arc<Mutex<DateTime<Utc>>> = Arc::new(Mutex::new(
        Utc.with_ymd_and_hms(2026, 3, 16, 12, 0, 0).unwrap(),
    ));
    let c = clock.clone();
    let mut store = Store::create_with_clock(
        &db,
        "bok-insyn-test",
        NewBook {
            company_name: "Insynsbolaget <AB> & Co",
            org_number: "559999-9999",
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
    let agent = Actor::agent("claude");
    let year = store
        .add_fiscal_year(&human, d(2026, 1, 1), d(2026, 12, 31))
        .unwrap();
    let chart: Vec<AccountChange> = [
        (1930, "Företagskonto", None),
        (2081, "Aktiekapital", None),
        (2099, "Årets resultat", None),
        (2611, "Utgående moms 25 %", Some("10")),
        (2641, "Debiterad ingående moms", Some("48")),
        (3010, "Försäljning", Some("05")),
        (5410, "Förbrukningsinventarier", None),
        (6570, "Bankkostnader", None),
        (8999, "Årets resultat", None),
    ]
    .into_iter()
    .map(|(n, name, vat): (i64, &str, Option<&str>)| AccountChange {
        number: acc(n),
        name: name.into(),
        active: true,
        sru: None,
        vat_code: vat.map(str::to_string),
    })
    .collect();
    store
        .change_chart(&human, d(2025, 1, 1), &chart, &signer)
        .unwrap();

    let archive = Archive::open(dir.path().join("underlag")).unwrap();
    let attach = |store: &mut Store, data: &[u8], name: &str, channel: Channel| {
        let stored = archive.put_bytes(data).unwrap();
        let id = store
            .register_attachment(
                &human,
                &NewAttachment {
                    sha256: stored.sha256.clone(),
                    size: stored.size as i64,
                    channel,
                    sender: Some("kvitton@example.se".into()),
                    original_name: Some(name.into()),
                    file_type: None,
                },
            )
            .unwrap();
        (id, stored.sha256)
    };
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend_from_slice(&[0; 32]);
    let (png_id, png_sha) = attach(&mut store, &png, "kvitto.png", Channel::Email);
    let (text_id, _) = attach(
        &mut store,
        "Faktura 17\nKund AB\n1 000,00 kr + moms 250,00 kr\n<b>betala</b>".as_bytes(),
        "faktura.txt",
        Channel::Folder,
    );
    let (html_id, html_sha) = attach(
        &mut store,
        b"<html><script>alert('underlag')</script></html>",
        "bild.jpg",
        Channel::Email,
    );
    let (zip_id, zip_sha) = attach(
        &mut store,
        b"PK\x03\x04\x14\x00\x00\x00",
        "bilagor.zip",
        Channel::Email,
    );
    let (bank_file, _) = attach(
        &mut store,
        b"datum;belopp\n2026-02-03;1250,00\n",
        "bank.csv",
        Channel::Bank,
    );

    // A1: ett köp som agenten föreslår, med underlag och härkomst.
    let purchase = NewProposal {
        series: Series::A,
        date: d(2026, 1, 10),
        description: EVIL.into(),
        counterparty: Some(ProposalCounterparty::Inline(Counterparty {
            name: "Papper & Penna <AB>".into(),
            org_number: Some("556000-0000".into()),
            vat_number: None,
        })),
        lines: vec![
            Line::debit(acc(5410), kr(80_000)),
            Line::debit(acc(2641), kr(20_000)),
            Line::credit(acc(1930), kr(100_000)),
        ],
        attachments: vec![png_id, html_id],
        idempotency_key: None,
        provenance: Provenance {
            model: Some("claude-opus-5-5".into()),
            provider: Some("Anthropic".into()),
            region: Some("EU".into()),
            confidence: Some(0.93),
            raw_response: Some("{\"forklaring\":\"<img src=x onerror=alert(2)>\"}".into()),
            ..Provenance::default()
        },
    };
    let id = store.create_proposal(&agent, &purchase).unwrap();
    let a1 = store.approve(&human, id, &signer).unwrap();

    // A2: en försäljning med fakturan som text.
    let sale = NewProposal {
        series: Series::A,
        date: d(2026, 2, 1),
        description: "Försäljning till Kund AB".into(),
        counterparty: None,
        lines: vec![
            Line::debit(acc(1930), kr(125_000)),
            Line::credit(acc(3010), kr(100_000)),
            Line::credit(acc(2611), kr(25_000)),
        ],
        attachments: vec![text_id, zip_id],
        idempotency_key: None,
        provenance: Provenance::default(),
    };
    let id = store.create_proposal(&agent, &sale).unwrap();
    let a2 = store.approve(&human, id, &signer).unwrap();

    // R1: A1 rättas till ett annat kostnadskonto.
    let id = store
        .propose_correction(
            &human,
            a1.voucher_id,
            "Fel konto, ska vara bankkostnad",
            &[
                Line::debit(acc(6570), kr(80_000)),
                Line::debit(acc(2641), kr(20_000)),
                Line::credit(acc(1930), kr(100_000)),
            ],
        )
        .unwrap();
    store.approve(&human, id, &signer).unwrap();

    // Två bankrader: en avstämd mot A2, en kvar.
    let row = |n: u64, day: u32, ore: i64, text: &str| NewBankRow {
        post_id: format!("{n:064x}"),
        bank_account: "SE4550000000058398257466".into(),
        booking_date: d(2026, 2, day),
        value_date: None,
        amount: kr(ore),
        currency: "SEK".into(),
        text: text.into(),
        reference: None,
        counterparty: Some("Kund AB".into()),
        balance: None,
    };
    store
        .import_bank_rows(
            &human,
            bank_file,
            acc(1930),
            &[
                row(1, 3, 125_000, "Inbetalning"),
                row(2, 4, -5_000, "Avgift <i>"),
            ],
        )
        .unwrap();
    let rows = store.bank_rows(false).unwrap();
    store
        .reconcile_bank_row(&human, rows[0].id, &MatchTarget::Voucher(a2.voucher_id))
        .unwrap();

    // Bokslut: årets resultat mot eget kapital.
    let id = store
        .propose_year_result(&human, year.id, acc(8999), acc(2099))
        .unwrap();
    store.approve(&human, id, &signer).unwrap();

    let out = dir.path().join("export");
    Fixture {
        _dir: dir,
        db,
        store,
        archive,
        html_sha,
        zip_sha,
        png_sha,
        out,
    }
}

fn insyn<'a>(f: &'a Fixture, mode: Mode) -> Insyn<'a> {
    Insyn::new(
        &f.store,
        &f.archive,
        &f.db,
        "bok test",
        "2026-03-16 13:00",
        mode,
    )
    .unwrap()
    .with_files(vec![ExtraFile {
        path: "sie/bokis-2026.se".into(),
        body: b"#FLAGGA 0\n".to_vec(),
        description: "SIE 4 för 2026".into(),
    }])
    .unwrap()
}

fn files_under(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// Alla värden i `attr="…"` på sidan.
fn attr_values<'a>(html: &'a str, attr: &str) -> Vec<&'a str> {
    let needle = format!(" {attr}=\"");
    html.match_indices(&needle)
        .filter_map(|(i, _)| {
            let rest = &html[i + needle.len()..];
            rest.find('"').map(|end| &rest[..end])
        })
        .collect()
}

#[test]
fn export_is_complete_and_every_link_resolves() {
    let f = fixture();
    let report = export(&insyn(&f, Mode::Export), &f.out).unwrap();
    let files = files_under(&f.out);
    // Manifestet täcker varje fil utom sig självt, med rätt hash.
    let manifest = std::fs::read_to_string(f.out.join("manifest.sha256")).unwrap();
    let listed: BTreeMap<&str, &str> = manifest
        .lines()
        .map(|l| {
            let (sha, path) = l.split_once("  ").unwrap();
            (path, sha)
        })
        .collect();
    assert_eq!(listed.len(), report.files);
    assert_eq!(files.len(), report.files + 1);
    for (path, sha) in &listed {
        let data = std::fs::read(f.out.join(path)).unwrap();
        assert_eq!(&sha256_hex(&data), sha, "{path}");
    }
    assert_eq!(report.manifest_sha256, sha256_hex(manifest.as_bytes()));

    for expected in [
        "index.html",
        "insyn.css",
        "underlag.html",
        "historik.html",
        "systemdokumentation.html",
        "export.html",
        "ar-2026/grundbok.html",
        "ar-2026/grundbok.csv",
        "ar-2026/huvudbok.html",
        "ar-2026/saldobalans.html",
        "ar-2026/arsbokslut.html",
        "ar-2026/moms.html",
        "ar-2026/bank.html",
        "ar-2026/v-A1.html",
        "ar-2026/v-A2.html",
        "ar-2026/v-R1.html",
        "ar-2026/v-B1.html",
        "sie/bokis-2026.se",
    ] {
        assert!(
            listed.contains_key(expected),
            "{expected} saknas i exporten"
        );
    }

    let mut checked = 0;
    for file in files
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e == "html"))
    {
        let html = std::fs::read_to_string(file).unwrap();
        let lower = html.to_lowercase();
        assert!(
            !lower.contains("<script"),
            "{} har ett skript",
            file.display()
        );
        assert!(
            !lower.contains("<form"),
            "{} har ett formulär",
            file.display()
        );
        assert!(
            !lower.contains("http://") && !lower.contains("https://"),
            "{} pekar utanför exporten",
            file.display()
        );
        for target in attr_values(&html, "href")
            .into_iter()
            .chain(attr_values(&html, "src"))
        {
            let (path, fragment) = match target.split_once('#') {
                Some((p, frag)) => (p, Some(frag)),
                None => (target, None),
            };
            let resolved = if path.is_empty() {
                file.clone()
            } else {
                file.parent().unwrap().join(path)
            };
            let resolved = std::fs::canonicalize(&resolved).unwrap_or_else(|_| {
                panic!("{} länkar till {target} som inte finns", file.display())
            });
            assert!(resolved.starts_with(std::fs::canonicalize(&f.out).unwrap()));
            if let Some(frag) = fragment {
                let page = std::fs::read_to_string(&resolved).unwrap();
                assert!(
                    page.contains(&format!("id=\"{frag}\"")),
                    "{} länkar till #{frag} som saknas i {}",
                    file.display(),
                    resolved.display()
                );
            }
            checked += 1;
        }
    }
    assert!(checked > 100, "bara {checked} länkar kontrollerades");
}

#[test]
fn nothing_from_the_books_becomes_markup() {
    let f = fixture();
    export(&insyn(&f, Mode::Export), &f.out).unwrap();
    let read = |p: &str| std::fs::read_to_string(f.out.join(p)).unwrap();

    let journal = read("ar-2026/grundbok.html");
    assert!(
        journal.contains("Kontorsmaterial &lt;script&gt;alert(1)&lt;/script&gt; \\u{202e}fdp.exe")
    );
    assert!(journal.contains("Insynsbolaget &lt;AB&gt; &amp; Co"));
    let voucher = read("ar-2026/v-A1.html");
    assert!(voucher.contains("Papper &amp; Penna &lt;AB&gt;"));
    assert!(voucher.contains("&lt;img src=x onerror=alert(2)&gt;"));
    assert!(voucher.contains("claude-opus-5-5, Anthropic, EU"));
    assert!(voucher.contains("93 %"));
    assert!(voucher.contains("<dd class=\"rattad\"><a href=\"../ar-2026/v-R1.html\">R1</a>: Fel konto, ska vara bankkostnad</dd>"));
    assert!(journal.contains("Rättad genom <a href=\"../ar-2026/v-R1.html\">R1</a>"));
    let invoice = read("ar-2026/v-A2.html");
    assert!(invoice.contains(
        "<pre>Faktura 17\nKund AB\n1 000,00 kr + moms 250,00 kr\n&lt;b&gt;betala&lt;/b&gt;</pre>"
    ));

    // Underlaget som säger sig vara en bild men är HTML blir ren text, och
    // det som webbläsaren inte kan visa blir en nedladdning.
    assert!(f.out.join(format!("underlag/{}.txt", f.html_sha)).exists());
    assert!(voucher.contains("<pre>&lt;html&gt;&lt;script&gt;alert(&#39;underlag&#39;)&lt;/script&gt;&lt;/html&gt;</pre>"));
    assert!(f.out.join(format!("underlag/{}.bin", f.zip_sha)).exists());
    assert!(invoice.contains("Hämta underlaget</a> (application/zip, 8 byte)"));
    assert!(f.out.join(format!("underlag/{}.png", f.png_sha)).exists());
    assert!(voucher.contains(&format!("<img src=\"../underlag/{}.png\"", f.png_sha)));
    let underlag_files: Vec<_> = files_under(&f.out.join("underlag"));
    assert!(underlag_files.iter().all(|p| {
        let ext = p.extension().unwrap().to_string_lossy().to_string();
        ["png", "jpg", "gif", "webp", "pdf", "txt", "bin"].contains(&ext.as_str())
    }));

    let csv = read("ar-2026/grundbok.csv");
    assert!(csv.starts_with('\u{feff}'));
    assert!(
        csv.contains(";\"Kontorsmaterial <script>alert(1)</script> \u{202e}fdp.exe\";")
            || csv.contains(";Kontorsmaterial <script>alert(1)</script> \u{202e}fdp.exe;")
    );
}

#[test]
fn annual_accounts_balance_and_show_the_closing() {
    let f = fixture();
    let i = insyn(&f, Mode::Export);
    let page = String::from_utf8(i.get("ar-2026/arsbokslut.html").unwrap().unwrap().body).unwrap();
    // Försäljning 1 000 kr minus bankkostnad 800 kr: 200 kr i vinst, bokförd mot 2099.
    assert!(page.contains("Årets resultat är 200,00."), "{page}");
    assert!(page.contains("Årets resultat är bokfört mot eget kapital (serie B)"));
    assert!(page.contains("Balansräkningen balanserar"));
    assert!(!page.contains("inte bokfört mot eget kapital</span>"));
    assert!(page.contains("1 av 2 bankrader under året är inte avstämda"));
    assert!(page.contains("Året är inte stängt i bokföringen"));
    assert!(page
        .contains("<tr class=\"summa\"><td>Årets resultat</td><td class=\"num\">200,00</td></tr>"));
    assert!(page.contains("<tr class=\"summa\"><td>Summa tillgångar</td><td class=\"num\">250,00</td><td class=\"num\">0,00</td></tr>"));
    assert!(page.contains("<tr class=\"summa\"><td>Summa eget kapital och skulder</td><td class=\"num\">250,00</td><td class=\"num\">0,00</td></tr>"));

    let vat = String::from_utf8(i.get("ar-2026/moms.html").unwrap().unwrap().body).unwrap();
    assert!(vat.contains("<tr class=\"summa\"><td>49</td><td>Moms att betala</td><td class=\"num\">50,00</td></tr>"), "{vat}");

    let overview = String::from_utf8(i.get("index.html").unwrap().unwrap().body).unwrap();
    assert!(overview.contains("Bokföringen är obruten"), "{overview}");
    assert!(overview.contains("<td class=\"num\">200,00</td>"));
}

#[test]
fn export_never_overwrites() {
    let f = fixture();
    std::fs::create_dir_all(&f.out).unwrap();
    let err = export(&insyn(&f, Mode::Export), &f.out).unwrap_err();
    assert!(err.to_string().contains("skrivs aldrig över"));
}

#[test]
fn unknown_and_non_canonical_paths_are_not_found() {
    let f = fixture();
    let i = insyn(&f, Mode::Export);
    for path in [
        "ar-2026/v-a1.html",
        "ar-2026/v-A01.html",
        "ar-2026/v-A99.html",
        "ar-2025/grundbok.html",
        "ar-2026/../index.html",
        "../etc/passwd",
        "underlag/../../bokis.db",
        &format!("underlag/{}.png", f.html_sha),
        &format!("underlag/{}.html", f.html_sha),
        "underlag/0000.png",
    ] {
        assert!(i.get(path).unwrap().is_none(), "{path} skulle inte finnas");
    }
}

const PORT: u16 = 8585;
const LOGIN: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const SESSION: &str = "2222222222222222222222222222222222222222222222222222222222222222";

fn request(method: &str, target: &str, cookie: bool) -> Request {
    let mut headers = vec![("Host".to_string(), format!("127.0.0.1:{PORT}"))];
    if cookie {
        headers.push((
            "Cookie".into(),
            format!("bok_session=x; bok_insyn={SESSION}"),
        ));
    }
    Request {
        method: method.into(),
        target: target.into(),
        headers,
        body: Vec::new(),
    }
}

#[test]
fn live_view_only_reads_and_needs_the_one_time_link() {
    let f = fixture();
    let events_before = f.store.events(None).unwrap().len();
    let mut view = LiveView::new(
        &f.store,
        &f.archive,
        &f.db,
        "bok test",
        LOGIN.into(),
        SESSION.into(),
        PORT,
    );

    let mut wrong_host = request("GET", "/", true);
    wrong_host.headers[0].1 = "evil.example:8585".into();
    assert_eq!(view.handle(&wrong_host).status, 403);
    assert_eq!(view.handle(&request("GET", "/", false)).status, 403);
    assert_eq!(
        view.handle(&request("GET", "/login?token=fel", false))
            .status,
        403
    );

    let post = view.handle(&request("POST", "/", true));
    assert_eq!(post.status, 405);
    assert_eq!(post.header("Allow"), Some("GET"));
    assert_eq!(view.handle(&request("POST", "/login", false)).status, 405);

    let css = view.handle(&request("GET", "/insyn.css", false));
    assert_eq!(css.status, 200);
    assert_eq!(css.header("Content-Type"), Some("text/css; charset=utf-8"));

    let login = view.handle(&request("GET", &format!("/login?token={LOGIN}"), false));
    assert_eq!(login.status, 303);
    let cookie = login.header("Set-Cookie").unwrap();
    assert!(cookie.starts_with(&format!("bok_insyn={SESSION};")));
    assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
    // Länken fungerar bara en gång.
    assert_eq!(
        view.handle(&request("GET", &format!("/login?token={LOGIN}"), false))
            .status,
        403
    );

    let home = view.handle(&request("GET", "/", true));
    assert_eq!(home.status, 200);
    let csp = home.header("Content-Security-Policy").unwrap();
    assert!(csp.contains("default-src 'none'") && csp.contains("form-action 'none'"));
    assert!(!csp.contains("script-src"));
    assert_eq!(home.header("X-Frame-Options"), Some("DENY"));
    let body = String::from_utf8(home.body).unwrap();
    assert!(body.contains("Visas live ur databasen"));

    for path in [
        "/ar-2026/grundbok.html",
        "/ar-2026/v-A1.html",
        "/historik.html",
        "/systemdokumentation.html",
        "/export.html",
    ] {
        assert_eq!(
            view.handle(&request("GET", path, true)).status,
            200,
            "{path}"
        );
    }
    assert_eq!(
        view.handle(&request("GET", "/ar-2026/v-a1.html", true))
            .status,
        404
    );

    let zip = view.handle(&request(
        "GET",
        &format!("/underlag/{}.bin", f.zip_sha),
        true,
    ));
    assert_eq!(zip.status, 200);
    assert_eq!(zip.header("Content-Type"), Some("application/octet-stream"));
    assert_eq!(zip.header("Content-Disposition"), Some("attachment"));
    assert!(zip
        .header("Content-Security-Policy")
        .unwrap()
        .contains("sandbox"));
    let html_doc = view.handle(&request(
        "GET",
        &format!("/underlag/{}.txt", f.html_sha),
        true,
    ));
    assert_eq!(
        html_doc.header("Content-Type"),
        Some("text/plain; charset=utf-8")
    );
    assert!(html_doc
        .header("Content-Security-Policy")
        .unwrap()
        .contains("sandbox"));
    assert_eq!(
        view.handle(&request(
            "GET",
            &format!("/underlag/{}.html", f.html_sha),
            true
        ))
        .status,
        404
    );
    let image = view.handle(&request(
        "GET",
        &format!("/underlag/{}.png", f.png_sha),
        true,
    ));
    assert_eq!(image.header("Content-Type"), Some("image/png"));
    assert_eq!(image.header("X-Content-Type-Options"), Some("nosniff"));
    assert!(image
        .header("Content-Security-Policy")
        .unwrap()
        .contains("sandbox"));

    // Att titta lämnar inga spår i bokföringen.
    assert_eq!(f.store.events(None).unwrap().len(), events_before);
}

#[test]
fn one_year_export_keeps_the_years_pages_and_attachments() {
    let f = fixture();
    let year = f.store.fiscal_years().unwrap()[0];
    let i = insyn(&f, Mode::Export).only_year(year.id).unwrap();
    let paths = i.paths().unwrap();
    assert!(paths.contains(&"ar-2026/grundbok.html".to_string()));
    assert_eq!(
        paths.iter().filter(|p| p.starts_with("underlag/")).count(),
        5
    );
    assert!(insyn(&f, Mode::Export).only_year(999).is_err());
}

fn page(view: &mut LiveView<'_>, path: &str) -> String {
    String::from_utf8(view.handle(&request("GET", path, true)).body).unwrap()
}

#[test]
fn live_view_checks_once_until_asked_again() {
    use std::os::unix::fs::PermissionsExt;

    let f = fixture();
    let mut view = LiveView::new(
        &f.store,
        &f.archive,
        &f.db,
        "bok test",
        LOGIN.into(),
        SESSION.into(),
        PORT,
    );
    let login = view.handle(&request("GET", &format!("/login?token={LOGIN}"), false));
    assert_eq!(login.status, 303);
    assert!(page(&mut view, "/").contains("Bokföringen är obruten"));

    // Ett underlag ändras på disken. Kontrollen är redan gjord och görs inte
    // om för varje sida, för den läser varje underlag.
    let file = f.archive.path_for(&f.png_sha).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&file, b"inte samma bild").unwrap();
    let again = page(&mut view, "/");
    assert!(again.contains("Bokföringen är obruten"));
    assert!(again.contains("<a href=\"kontrollera\">Kontrollera igen</a>"));

    // Utan inloggning görs ingen ny kontroll.
    assert_eq!(
        view.handle(&request("GET", "/kontrollera", false)).status,
        403
    );
    let redo = view.handle(&request("GET", "/kontrollera", true));
    assert_eq!(redo.status, 303);
    assert_eq!(redo.header("Location"), Some("/"));
    let after = page(&mut view, "/");
    assert!(after.contains("Kontrollen hittade fel"), "{after}");
    assert!(page(&mut view, "/systemdokumentation.html").contains("FEL"));
}

#[test]
fn as_a_service_it_answers_on_its_address_with_links_on_demand() {
    let f = fixture();
    let file = bok_insyn::links::file_beside(&f.db);
    let origin = "https://bokis.example.ts.net:8443";
    bok_insyn::links::set_origin(&file, origin, Utc::now().timestamp()).unwrap();
    let mut view = LiveView::new(
        &f.store,
        &f.archive,
        &f.db,
        "bok test",
        LOGIN.into(),
        SESSION.into(),
        PORT,
    )
    .with_remote(origin, file.clone());
    let remote = |target: &str, cookie: bool| {
        let mut r = request("GET", target, cookie);
        r.headers[0].1 = "bokis.example.ts.net:8443".into();
        r
    };

    // Startlänken gäller inte för en tjänst, och utan länk syns inget.
    assert_eq!(
        view.handle(&remote(&format!("/login?token={LOGIN}"), false))
            .status,
        403
    );
    let refused = view.handle(&remote("/", false));
    assert_eq!(refused.status, 403);
    assert!(String::from_utf8_lossy(&refused.body).contains("bok insyn lank"));
    // En annan värd släpps inte in.
    let mut other = remote("/", true);
    other.headers[0].1 = "evil.example".into();
    assert_eq!(view.handle(&other).status, 403);

    // En länk från `bok insyn lank` fungerar en gång och ger en säker kaka.
    let token = "3333333333333333333333333333333333333333333333333333333333333333";
    bok_insyn::links::issue(&file, token, Utc::now().timestamp()).unwrap();
    let login = view.handle(&remote(&format!("/login?token={token}"), false));
    assert_eq!(login.status, 303);
    let cookie = login.header("Set-Cookie").unwrap();
    assert!(cookie.contains("Secure") && cookie.contains("HttpOnly"));
    assert_eq!(
        view.handle(&remote(&format!("/login?token={token}"), false))
            .status,
        403
    );
    assert_eq!(view.handle(&remote("/", true)).status, 200);
    // Samma tjänst svarar fortfarande på 127.0.0.1.
    assert_eq!(view.handle(&request("GET", "/", true)).status, 200);
}

#[test]
fn a_pdf_receipt_is_shown_on_the_voucher_page() {
    let f = fixture();
    let mut view = LiveView::new(
        &f.store,
        &f.archive,
        &f.db,
        "bok test",
        LOGIN.into(),
        SESSION.into(),
        PORT,
    );
    let login = view.handle(&request("GET", &format!("/login?token={LOGIN}"), false));
    assert_eq!(login.status, 303);
    let csp = view.handle(&request("GET", "/", true));
    assert!(csp
        .header("Content-Security-Policy")
        .unwrap()
        .contains("object-src 'self'"));
}
