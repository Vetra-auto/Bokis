//! Inkorgen, kopiorna och bankfilerna genom `bok`, med signering via
//! `ssh-keygen -Y sign` och en ed25519-nyckel i mjukvara.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    /// None om ssh-keygen saknas.
    fn new() -> Option<Env> {
        let dir = tempfile::tempdir().unwrap();
        let ok = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-C", "testnyckel", "-f"])
            .arg(dir.path().join("nyckel"))
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        ok.then_some(Env { dir })
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn bok_with_db(&self, db: &Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_bok"))
            .args(args)
            .env("BOKIS_DB", db)
            .env("BOKIS_KEY", self.path("nyckel"))
            .env("BOKIS_USER", "testare")
            .current_dir(self.dir.path())
            .output()
            .unwrap()
    }

    fn ok_with_db(&self, db: &Path, args: &[&str]) -> String {
        let out = self.bok_with_db(db, args);
        assert!(
            out.status.success(),
            "bok {args:?} misslyckades:\n{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    fn ok(&self, args: &[&str]) -> String {
        self.ok_with_db(&self.path("data/bokis.db"), args)
    }

    fn fails(&self, args: &[&str]) -> String {
        let out = self.bok_with_db(&self.path("data/bokis.db"), args);
        assert!(!out.status.success(), "bok {args:?} skulle ha misslyckats");
        String::from_utf8(out.stderr).unwrap() + &String::from_utf8(out.stdout).unwrap()
    }

    fn init(&self) {
        std::fs::create_dir_all(self.path("data")).unwrap();
        let year = chrono_year();
        self.ok(&[
            "init",
            "--security",
            "mjukvara",
            "--company",
            "Testbolaget AB",
            "--orgnr",
            "559999-9999",
            "--year",
            &year,
        ]);
    }
}

fn chrono_year() -> String {
    // Samma år som "i dag", så att kopian kan loggas i historiken.
    let out = Command::new("date").arg("+%Y").output().unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

const CAMT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.02">
 <BkToCstmrStmt><Stmt><Id>1</Id>
  <Acct><Id><IBAN>SE4550000000058398257466</IBAN></Id><Ccy>SEK</Ccy></Acct>
  <Bal><Tp><CdOrPrtry><Cd>OPBD</Cd></CdOrPrtry></Tp><Amt Ccy="SEK">1000.00</Amt><CdtDbtInd>CRDT</CdtDbtInd><Dt><Dt>2026-03-01</Dt></Dt></Bal>
  <Bal><Tp><CdOrPrtry><Cd>CLBD</Cd></CdOrPrtry></Tp><Amt Ccy="SEK">500.00</Amt><CdtDbtInd>CRDT</CdtDbtInd><Dt><Dt>2026-03-31</Dt></Dt></Bal>
  <Ntry><Amt Ccy="SEK">500.00</Amt><CdtDbtInd>DBIT</CdtDbtInd><Sts>BOOK</Sts>
   <BookgDt><Dt>2026-03-28</Dt></BookgDt><ValDt><Dt>2026-03-28</Dt></ValDt><AcctSvcrRef>X1</AcctSvcrRef>
   <NtryDtls><TxDtls><RmtInf><Ustrd>Exempel Hosting</Ustrd></RmtInf></TxDtls></NtryDtls></Ntry>
 </Stmt></BkToCstmrStmt></Document>"#;

#[test]
fn inbox_archives_registers_and_moves() {
    let Some(env) = Env::new() else {
        eprintln!("ssh-keygen saknas; hoppar över testet");
        return;
    };
    env.init();
    let inbox = env.path("inkorg");
    std::fs::create_dir_all(&inbox).unwrap();
    std::fs::write(inbox.join("kvitto.pdf"), b"%PDF-1.7 kvitto").unwrap();
    std::fs::write(inbox.join("utdrag.xml"), CAMT).unwrap();
    std::fs::write(inbox.join(".dold"), b"x").unwrap();

    let out = env.ok(&["inkorg", inbox.to_str().unwrap(), "--min-age", "0"]);
    assert!(out.contains("2 nya underlag"), "{out}");
    assert!(inbox.join("inlasta/kvitto.pdf").is_file());
    assert!(!inbox.join("kvitto.pdf").exists());
    assert!(inbox.join(".dold").exists(), "dolda filer lämnas kvar");

    let list = env.ok(&["underlag", "lista"]);
    assert!(
        list.contains("kvitto.pdf") && list.contains("utdrag.xml"),
        "{list}"
    );

    // Samma innehåll igen blir inte ett nytt underlag.
    std::fs::write(inbox.join("kopia-av-kvitto.pdf"), b"%PDF-1.7 kvitto").unwrap();
    let out = env.ok(&["inkorg", inbox.to_str().unwrap(), "--min-age", "0"]);
    assert!(out.contains("0 nya underlag, 1 redan mottagna"), "{out}");

    let verify = env.ok(&["verifiera"]);
    assert!(verify.contains("Bokföringen är intakt"), "{verify}");
}

#[test]
fn backup_check_and_readback() {
    let Some(env) = Env::new() else {
        eprintln!("ssh-keygen saknas; hoppar över testet");
        return;
    };
    env.init();
    let file = env.path("faktura.pdf");
    std::fs::write(&file, b"%PDF-1.7 faktura").unwrap();
    env.ok(&["underlag", "lagg-till", file.to_str().unwrap()]);

    let target = env.path("usb");
    let out = env.ok(&["kopia", "skapa", target.to_str().unwrap()]);
    assert!(out.contains("1 nya underlag kopierade"), "{out}");
    let list = env.ok(&["kopia", "lista", target.to_str().unwrap()]);
    let backup = list.lines().next().unwrap().trim().to_string();
    let out = env.ok(&["kopia", "kontrollera", &backup]);
    assert!(out.contains("Kopian är hel"), "{out}");

    // Kopian loggas i behandlingshistoriken.
    let history = env.ok(&["historik"]);
    assert!(history.contains("export.skapad"), "{history}");

    // Inläsning bara till en tom katalog, och den inlästa bokföringen verifieras.
    let readback = env.path("provlasning");
    let out = env.ok(&["kopia", "las-in", &backup, readback.to_str().unwrap()]);
    assert!(out.contains("Den inlästa bokföringen är intakt"), "{out}");
    let err = env.fails(&["kopia", "las-in", &backup, readback.to_str().unwrap()]);
    assert!(err.contains("inte tom") || err.contains("tom"), "{err}");
    let restored = readback.join("bokis.db");
    let out = env.ok_with_db(&restored, &["verifiera"]);
    assert!(out.contains("Bokföringen är intakt"), "{out}");

    // En ändrad kopia upptäcks.
    let manifest = PathBuf::from(&backup).join("bokis.db");
    let mut perms = std::fs::metadata(&manifest).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(&manifest, perms).unwrap();
    let mut data = std::fs::read(&manifest).unwrap();
    let last = data.len() - 1;
    data[last] ^= 0xff;
    std::fs::write(&manifest, data).unwrap();
    let err = env.fails(&["kopia", "kontrollera", &backup]);
    assert!(err.contains("skadad"), "{err}");
}

#[test]
fn bank_statement_is_imported_and_reconciled() {
    let Some(env) = Env::new() else {
        eprintln!("ssh-keygen saknas; hoppar över testet");
        return;
    };
    env.init();
    let chart = env.path("konton.txt");
    std::fs::write(
        &chart,
        "1930;Företagskonto\n2440;Leverantörsskulder\n5420;Programvaror\n6570;Bankkostnader\n",
    )
    .unwrap();
    env.ok(&["konto", "importera", chart.to_str().unwrap()]);

    let file = env.path("utdrag.xml");
    std::fs::write(&file, CAMT).unwrap();
    let out = env.ok(&["bank", "importera", file.to_str().unwrap()]);
    assert!(out.contains("mottaget som underlag 1"), "{out}");
    assert!(
        out.contains("Saldokontroll: ingående saldo plus raderna ger utgående saldo."),
        "{out}"
    );
    assert!(
        out.contains("1 nya bankrader inlästa mot konto 1930"),
        "{out}"
    );
    let again = env.ok(&["bank", "importera", file.to_str().unwrap()]);
    assert!(again.contains("finns redan som underlag 1"), "{again}");
    assert!(
        again.contains("0 nya bankrader inlästa mot konto 1930, 1 fanns redan"),
        "{again}"
    );

    let rows = env.ok(&["bank", "rader"]);
    assert!(
        rows.contains("-500,00") && rows.contains("ej avstämd"),
        "{rows}"
    );

    // En betalning som bokförs, och avstämningen mot den.
    env.ok(&[
        "foresla",
        "--date",
        "2026-03-27",
        "--text",
        "Exempel Hosting, molntjänst",
        "--line",
        "5420=500,00",
        "--line",
        "1930=-500,00",
    ]);
    env.ok(&["godkann", "1"]);
    let suggestions = env.ok(&["bank", "forslag"]);
    assert!(
        suggestions.contains("A1") && suggestions.contains("-1 dagar"),
        "{suggestions}"
    );
    let err = env.fails(&["bank", "matcha", "1"]);
    assert!(err.contains("--explanation"), "{err}");
    env.ok(&["bank", "matcha", "1", "A1"]);
    let rows = env.ok(&["bank", "rader"]);
    assert!(rows.contains("Alla bankrader är avstämda"), "{rows}");
    let all = env.ok(&["bank", "rader", "--all"]);
    assert!(all.contains("avstämd mot A1"), "{all}");
    let verify = env.ok(&["verifiera"]);
    assert!(
        verify.contains("Bankraderna i stängda perioder är avstämda"),
        "{verify}"
    );

    let csv = env.path("konto.csv");
    std::fs::write(
        &csv,
        "Datum;Text;Belopp;Saldo\n2026-03-02;Kortköp Café;-45,00;955,00\n",
    )
    .unwrap();
    let err = env.fails(&["bank", "importera", csv.to_str().unwrap()]);
    assert!(err.contains("--csv-account"), "{err}");
    let out = env.ok(&[
        "bank",
        "importera",
        csv.to_str().unwrap(),
        "--csv-account",
        "1234-5678",
    ]);
    assert!(
        out.contains("Konto 1234-5678 (SEK): 1 rader, summa -45,00"),
        "{out}"
    );
    env.ok(&[
        "bank",
        "matcha",
        "2",
        "--explanation",
        "Kortköp, kvittot saknas; utlägg",
    ]);
    assert!(
        env.ok(&["bank", "rader"])
            .contains("Alla bankrader är avstämda")
    );
}

#[test]
fn cases_are_answered_and_only_flags_change_status() {
    let Some(env) = Env::new() else {
        eprintln!("ssh-keygen saknas; hoppar över testet");
        return;
    };
    env.init();
    let inbox = env.path("inkorg");
    std::fs::create_dir_all(&inbox).unwrap();
    std::fs::write(inbox.join("reklam.pdf"), b"%PDF-1.7 reklam").unwrap();
    env.ok(&["inkorg", inbox.to_str().unwrap(), "--min-age", "0"]);
    assert!(
        env.ok(&["arende", "lista"])
            .contains("Inga ärenden väntar på svar")
    );

    // Agentens flagga läggs direkt i lagret, som bok-mcp gör.
    let db = env.path("data/bokis.db");
    let mut store = bok_store::Store::open(&db, "test").unwrap();
    store
        .open_case(
            &bok_store::Actor::agent("modell"),
            &bok_store::NewCase {
                kind: bok_store::CaseKind::Flag {
                    reason: "ej_rakenskapsinformation".into(),
                },
                attachment_id: Some(1),
                text: "Reklamblad, ingen affärshändelse".into(),
                model: Some("modell".into()),
                confidence: Some(0.9),
            },
        )
        .unwrap();
    drop(store);

    let list = env.ok(&["arende", "lista"]);
    assert!(
        list.contains("flagga (ej_rakenskapsinformation)") && list.contains("Reklamblad"),
        "{list}"
    );
    let out = env.ok(&[
        "arende",
        "svara",
        "1",
        "Stämmer, reklam",
        "--mark",
        "ej-rakenskapsinformation",
    ]);
    assert!(
        out.contains("markerat som ej_rakenskapsinformation"),
        "{out}"
    );
    let attachments = env.ok(&["underlag", "lista"]);
    assert!(
        attachments.contains("ej_rakenskapsinformation") || attachments.contains("Stämmer, reklam"),
        "{attachments}"
    );
    let all = env.ok(&["arende", "lista", "--all"]);
    assert!(all.contains("svar av testare: Stämmer, reklam"), "{all}");
    let err = env.fails(&["arende", "svara", "1", "igen"]);
    assert!(err.contains("redan besvarat"), "{err}");
    assert!(env.ok(&["verifiera"]).contains("Bokföringen är intakt"));
}

#[test]
fn vat_codes_and_vat_report() {
    let Some(env) = Env::new() else {
        eprintln!("ssh-keygen saknas; hoppar över testet");
        return;
    };
    env.init();
    // Momskoderna läses in med kontoplanen: en tredje kolumn, eller en fil med --vat-codes.
    let chart = env.path("konton.txt");
    std::fs::write(
        &chart,
        "1930;Företagskonto\n2611;Utgående moms 25 %;10\n2641;Debiterad ingående moms\n3001;Försäljning 25 %\n5420;Programvaror\n",
    )
    .unwrap();
    let codes = env.path("momskoder.txt");
    std::fs::write(&codes, "// test\n3001;05\n2641;48\n9999;48\n").unwrap();
    let out = env.ok(&[
        "konto",
        "importera",
        chart.to_str().unwrap(),
        "--vat-codes",
        codes.to_str().unwrap(),
    ]);
    assert!(out.contains("3 av dem har momskod"), "{out}");
    assert!(env.ok(&["moms", "2026-03"]).contains("inga belopp"));

    env.ok(&[
        "foresla",
        "--date",
        "2026-03-05",
        "--text",
        "Försäljning",
        "--line",
        "1930=1250,00",
        "--line",
        "3001=-1000,00",
        "--line",
        "2611=-250,00",
    ]);
    env.ok(&[
        "foresla",
        "--date",
        "2026-03-06",
        "--text",
        "Programvara",
        "--line",
        "5420=800,00",
        "--line",
        "2641=200,00",
        "--line",
        "1930=-1000,00",
    ]);
    env.ok(&["godkann", "1", "2"]);
    let report = env.ok(&["moms", "2026-03"]);
    assert!(
        report.contains("Ruta 05") && report.contains("1 000,00"),
        "{report}"
    );
    assert!(
        report.contains("Ruta 10") && report.contains("250,00"),
        "{report}"
    );
    assert!(
        report.contains("Ruta 48") && report.contains("200,00"),
        "{report}"
    );
    assert!(report.contains("Moms att betala: 50,00"), "{report}");
    let quarter = env.ok(&["moms", "2026-01:2026-03"]);
    assert!(quarter.contains("Moms att betala: 50,00"), "{quarter}");
    assert!(env.ok(&["moms", "2026-04"]).contains("inga belopp"));
    assert!(env.fails(&["moms", "2026-13"]).contains("ingen månad"));

    // En senare ändring gäller från sitt datum och ändrar inte mars.
    std::fs::write(&codes, "3001;42\n").unwrap();
    let out = env.ok(&[
        "konto",
        "momskoder",
        codes.to_str().unwrap(),
        "--from",
        "2026-06-01",
    ]);
    assert!(
        out.contains("Momskod satt på 1 konton, gäller från 2026-06-01"),
        "{out}"
    );
    assert!(env.ok(&["moms", "2026-03"]).contains("Ruta 05"));
    std::fs::write(&codes, "3001;77\n").unwrap();
    assert!(
        env.fails(&["konto", "momskoder", codes.to_str().unwrap()])
            .contains("ingen ruta")
    );

    // En ny version av kontot utan momskod behåller den gamla koden.
    env.ok(&[
        "konto",
        "lagg-till",
        "2641",
        "Ingående moms",
        "--from",
        "2026-08-01",
    ]);
    let rename = env.path("ny-kontoplan.txt");
    std::fs::write(&rename, "2611;Utgående moms 25 procent\n").unwrap();
    let out = env.ok(&[
        "konto",
        "importera",
        rename.to_str().unwrap(),
        "--from",
        "2026-09-01",
    ]);
    assert!(out.contains("1 behållna från kontoplanen"), "{out}");
    env.ok(&[
        "foresla",
        "--date",
        "2026-09-05",
        "--text",
        "Försäljning september",
        "--line",
        "1930=125,00",
        "--line",
        "3001=-100,00",
        "--line",
        "2611=-25,00",
    ]);
    env.ok(&["godkann", "3"]);
    let september = env.ok(&["moms", "2026-09"]);
    assert!(
        september.contains("Ruta 10") && september.contains("25,00"),
        "{september}"
    );

    // Den medföljande filen går att läsa.
    let shipped =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/momskoder-bas.txt");
    let out = env.ok(&[
        "konto",
        "momskoder",
        shipped.to_str().unwrap(),
        "--from",
        "2026-07-01",
    ]);
    assert!(out.contains("Momskod satt på"), "{out}");
}

#[test]
fn system_documentation_is_generated_from_the_book() {
    let Some(env) = Env::new() else {
        eprintln!("ssh-keygen saknas; hoppar över testet");
        return;
    };
    env.init();
    let chart = env.path("konton.txt");
    std::fs::write(
        &chart,
        "1930;Företagskonto\n2641;Debiterad ingående moms;48\n",
    )
    .unwrap();
    env.ok(&["konto", "importera", chart.to_str().unwrap()]);
    let out = env.path("systemdokumentation.md");
    let printed = env.ok(&["systemdokumentation", out.to_str().unwrap()]);
    assert!(
        printed.contains("Systemdokumentationen är skriven"),
        "{printed}"
    );
    let doc = std::fs::read_to_string(&out).unwrap();
    for needle in [
        "# Systemdokumentation för Bokis",
        "Testbolaget AB (559999-9999)",
        "| `lista_underlag` | agenten (läser)",
        "| `bokfor_forslag` | granskaren",
        "Alla tabeller som bara ska kunna växa har spärrar",
        "| `verifikationer` | UPDATE |",
        "2641 → ruta 48",
        "mjukvarunyckel",
        "- OK: Behandlingshistoriken är obruten",
    ] {
        assert!(doc.contains(needle), "saknar {needle:?} i\n{doc}");
    }
    assert!(!doc.contains("FEL"), "{doc}");
    // En befintlig fil skrivs aldrig över, och dokumentet loggas i historiken.
    let err = env.fails(&["systemdokumentation", out.to_str().unwrap()]);
    assert!(err.contains("finns redan"), "{err}");
    assert!(env.ok(&["historik"]).contains("export.skapad"));
}

#[test]
fn closing_proposals_through_the_command_line() {
    let Some(env) = Env::new() else {
        eprintln!("ssh-keygen saknas; hoppar över testet");
        return;
    };
    env.init();
    let chart = env.path("konton.txt");
    std::fs::write(
        &chart,
        "1930;Företagskonto\n2099;Årets resultat\n3001;Försäljning\n5420;Programvaror\n8999;Årets resultat\n",
    )
    .unwrap();
    env.ok(&["konto", "importera", chart.to_str().unwrap()]);
    let year = chrono_year();
    env.ok(&[
        "foresla",
        "--date",
        &format!("{year}-03-05"),
        "--text",
        "Försäljning",
        "--line",
        "1930=1000,00",
        "--line",
        "3001=-1000,00",
    ]);
    env.ok(&["godkann", "1"]);
    let next = (year.parse::<i32>().unwrap() + 1).to_string();
    let err = env.fails(&["bokslut", "ib", "--year", &year]);
    assert!(err.contains("lägg till det först"), "{err}");
    env.ok(&["ar", "lagg-till", &next]);
    let err = env.fails(&["bokslut", "ib", "--year", &year]);
    assert!(err.contains("bokför årets resultat"), "{err}");

    let out = env.ok(&["bokslut", "resultat", "--year", &year]);
    assert!(
        out.contains("Årets resultat") && out.contains("1 000,00") && out.contains("vinst"),
        "{out}"
    );
    assert!(out.contains("bok godkann 2"), "{out}");
    env.ok(&["godkann", "2"]);
    let out = env.ok(&["bokslut", "ib", "--year", &year]);
    assert!(
        out.contains("Ingående balans") && out.contains("bok godkann 3"),
        "{out}"
    );
    env.ok(&["godkann", "3"]);
    let journal = env.ok(&["grundbok", "--year", &next]);
    assert!(journal.contains("I1"), "{journal}");
    assert!(env.ok(&["verifiera"]).contains("Bokföringen är intakt"));
}

#[test]
fn year_archive_is_self_contained_and_verifiable() {
    let Some(env) = Env::new() else {
        eprintln!("ssh-keygen saknas; hoppar över testet");
        return;
    };
    env.init();
    let chart = env.path("konton.txt");
    std::fs::write(&chart, "1930;Företagskonto\n5420;Programvaror\n").unwrap();
    env.ok(&["konto", "importera", chart.to_str().unwrap()]);
    let receipt = env.path("kvitto.pdf");
    std::fs::write(&receipt, b"%PDF-1.7 kvitto").unwrap();
    env.ok(&["underlag", "lagg-till", receipt.to_str().unwrap()]);
    let year = chrono_year();
    env.ok(&[
        "foresla",
        "--date",
        &format!("{year}-03-05"),
        "--text",
        "Programvara",
        "--line",
        "5420=100,00",
        "--line",
        "1930=-100,00",
        "--attachment",
        "1",
    ]);
    env.ok(&["godkann", "1"]);

    let target = env.path("usb");
    let out = env.ok(&["arkivpaket", target.to_str().unwrap(), "--year", &year]);
    assert!(
        out.contains("med 1 underlag") && out.contains("perioder är inte stängda"),
        "{out}"
    );
    let dir = target.join(format!("bokis-{year}"));
    for file in [
        "LASMIG.txt",
        "manifest.sha256",
        "historik.jsonl",
        "systemdokumentation.md",
        "program/bok",
        "rapporter/grundbok.txt",
        "rapporter/huvudbok.txt",
        "rapporter/saldobalans.txt",
        "rapporter/momsrapport.txt",
        "underlag/forteckning.csv",
    ] {
        assert!(dir.join(file).is_file(), "saknar {file}");
    }
    assert!(dir.join(format!("sie/bokis-{year}.se")).is_file());
    let journal = std::fs::read_to_string(dir.join("rapporter/grundbok.txt")).unwrap();
    assert!(
        journal.contains("A1") && journal.contains("Programvara"),
        "{journal}"
    );
    let index = std::fs::read_to_string(dir.join("underlag/forteckning.csv")).unwrap();
    assert!(
        index.contains("kvitto.pdf") && index.contains("A1"),
        "{index}"
    );

    // Kontrollen fungerar med vanliga verktyg, utan Bokis.
    if let Ok(check) = Command::new("sha256sum")
        .args(["-c", "--quiet", "manifest.sha256"])
        .current_dir(&dir)
        .output()
    {
        assert!(
            check.status.success(),
            "{}",
            String::from_utf8_lossy(&check.stdout)
        );
    }
    let err = env.fails(&["arkivpaket", target.to_str().unwrap(), "--year", &year]);
    assert!(err.contains("skrivs aldrig över"), "{err}");
    assert!(env.ok(&["historik"]).contains("arkivpaket"));
}

#[test]
fn reconcile_an_account_against_an_external_balance() {
    let Some(env) = Env::new() else {
        eprintln!("ssh-keygen saknas; hoppar över testet");
        return;
    };
    env.init();
    let chart = env.path("konton.txt");
    std::fs::write(&chart, "1630;Skattekonto\n1930;Företagskonto\n").unwrap();
    env.ok(&["konto", "importera", chart.to_str().unwrap()]);
    let year = chrono_year();
    env.ok(&[
        "foresla",
        "--date",
        &format!("{year}-03-10"),
        "--text",
        "Till skattekontot",
        "--line",
        "1630=5000,00",
        "--line",
        "1930=-5000,00",
    ]);
    env.ok(&["godkann", "1"]);
    let ok = env.ok(&[
        "avstam",
        "1630",
        "--date",
        &format!("{year}-03-31"),
        "--balance",
        "5000,00",
    ]);
    assert!(ok.contains("Kontot stämmer."), "{ok}");
    let off = env.ok(&[
        "avstam",
        "1630",
        "--date",
        &format!("{year}-03-31"),
        "--balance",
        "4875,00",
        "--note",
        "ränta?",
    ]);
    assert!(
        off.contains("stämmer INTE") && off.contains("125,00"),
        "{off}"
    );
    assert!(env.ok(&["historik"]).contains("konto.avstamt"));
}

#[test]
fn rules_are_added_signed_and_listed() {
    let Some(env) = Env::new() else {
        eprintln!("ssh-keygen saknas; hoppar över testet");
        return;
    };
    env.init();
    let out = env.ok(&[
        "regel",
        "lagg-till",
        "molntjanster",
        "--text",
        "Molntjänster: 5420 och 2641.",
        "--from",
        "2026-01-01",
    ]);
    assert!(out.contains("version 1 gäller från 2026-01-01"), "{out}");
    let out = env.ok(&[
        "regel",
        "lagg-till",
        "molntjanster",
        "--text",
        "Från juli: 6540.",
        "--from",
        "2026-07-01",
    ]);
    assert!(out.contains("version 2"), "{out}");
    let march = env.ok(&["regel", "lista", "--date", "2026-03-01"]);
    assert!(
        march.contains("molntjanster v1") && march.contains("5420"),
        "{march}"
    );
    let all = env.ok(&["regel", "lista", "--all"]);
    assert!(all.contains("v1") && all.contains("v2"), "{all}");
    assert!(
        env.fails(&["regel", "lagg-till", "x"])
            .contains("--text eller --file")
    );
    let verify = env.ok(&["verifiera"]);
    assert!(
        verify.contains("Reglerna ändras bara genom signerade systemändringar"),
        "{verify}"
    );
}

#[test]
fn insyn_export_is_readable_without_bokis() {
    let Some(env) = Env::new() else {
        eprintln!("ssh-keygen saknas; hoppar över testet");
        return;
    };
    env.init();
    let chart = env.path("konton.txt");
    std::fs::write(&chart, "1930;Företagskonto\n5420;Programvaror\n").unwrap();
    env.ok(&["konto", "importera", chart.to_str().unwrap()]);
    let receipt = env.path("kvitto.pdf");
    std::fs::write(&receipt, b"%PDF-1.7 kvitto").unwrap();
    env.ok(&["underlag", "lagg-till", receipt.to_str().unwrap()]);
    let year = chrono_year();
    env.ok(&[
        "foresla",
        "--date",
        &format!("{year}-03-05"),
        "--text",
        "Programvara <b>licens</b>",
        "--line",
        "5420=100,00",
        "--line",
        "1930=-100,00",
        "--attachment",
        "1",
        "--approve",
    ]);

    let target = env.path("usb/insyn");
    let out = env.ok(&["insyn", "export", target.to_str().unwrap()]);
    assert!(out.contains("Manifestets SHA-256"), "{out}");
    for file in [
        "index.html".to_string(),
        "insyn.css".to_string(),
        format!("ar-{year}/grundbok.html"),
        format!("ar-{year}/grundbok.csv"),
        format!("ar-{year}/arsbokslut.html"),
        format!("ar-{year}/v-A1.html"),
        format!("sie/bokis-{year}.se"),
        "manifest.sha256".to_string(),
    ] {
        assert!(target.join(&file).is_file(), "saknar {file}");
    }
    let voucher = std::fs::read_to_string(target.join(format!("ar-{year}/v-A1.html"))).unwrap();
    assert!(
        voucher.contains("Programvara &lt;b&gt;licens&lt;/b&gt;"),
        "{voucher}"
    );
    assert!(voucher.contains("Öppna underlaget</a> (PDF"), "{voucher}");
    let sie = std::fs::read(target.join(format!("sie/bokis-{year}.se"))).unwrap();
    assert!(String::from_utf8_lossy(&sie).contains("#VER"));
    if let Ok(check) = Command::new("sha256sum")
        .args(["-c", "--quiet", "manifest.sha256"])
        .current_dir(&target)
        .output()
    {
        assert!(
            check.status.success(),
            "{}",
            String::from_utf8_lossy(&check.stdout)
        );
    }

    // Exporten hamnar i historiken, och en befintlig katalog skrivs aldrig över.
    assert!(env.ok(&["historik"]).contains("insyn"));
    let err = env.fails(&["insyn", "export", target.to_str().unwrap()]);
    assert!(err.contains("skrivs aldrig över"), "{err}");

    // Ett enskilt år, med året i katalogen.
    let one = env.path("usb/insyn-ett-ar");
    env.ok(&["insyn", "export", one.to_str().unwrap(), "--year", &year]);
    assert!(one.join(format!("ar-{year}/grundbok.html")).is_file());
}
