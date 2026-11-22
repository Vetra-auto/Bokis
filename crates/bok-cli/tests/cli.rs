//! Hela flödet genom `bok`, med signering via `ssh-keygen -Y sign` och en
//! ed25519-nyckel i mjukvara (i skarp drift en ed25519-sk-nyckel).

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

    fn db(&self) -> PathBuf {
        self.path("data/bokis.db")
    }

    fn bok(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_bok"))
            .args(args)
            .env("BOKIS_DB", self.db())
            .env("BOKIS_KEY", self.path("nyckel"))
            .env("BOKIS_USER", "testare")
            .current_dir(self.dir.path())
            .output()
            .unwrap()
    }

    /// Kör och kräver att kommandot lyckas; ger stdout.
    fn ok(&self, args: &[&str]) -> String {
        let out = self.bok(args);
        assert!(
            out.status.success(),
            "bok {args:?} misslyckades:\n{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    /// Kör och kräver att kommandot misslyckas; ger stderr.
    fn fails(&self, args: &[&str]) -> String {
        let out = self.bok(args);
        assert!(!out.status.success(), "bok {args:?} skulle ha misslyckats");
        String::from_utf8(out.stderr).unwrap() + &String::from_utf8(out.stdout).unwrap()
    }
}

fn write(path: &Path, text: &[u8]) {
    std::fs::write(path, text).unwrap();
}

#[test]
fn the_whole_flow_through_the_command_line() {
    let Some(env) = Env::new() else {
        eprintln!("ssh-keygen saknas; hoppar över testet");
        return;
    };

    let out = env.ok(&[
        "init",
        "--security",
        "mjukvara",
        "--company",
        "Testbolaget AB",
        "--orgnr",
        "559999-9991",
        "--year",
        "2026",
    ]);
    assert!(out.contains("Räkenskapsår 2026"), "{out}");
    let size = std::fs::metadata(env.db()).unwrap().len();
    let err = env.fails(&[
        "init",
        "--security",
        "mjukvara",
        "--company",
        "Annat AB",
        "--orgnr",
        "556000-0001",
    ]);
    assert!(err.contains("finns redan"), "{err}");
    assert!(std::fs::metadata(env.db()).unwrap().len() >= size);
    assert!(env.ok(&["key", "list"]).contains("testare"));

    write(
        &env.path("kontoplan.txt"),
        "1930;Företagskonto\n2081;Aktiekapital\n5410;Förbrukningsinventarier\n6570;Bankkostnader\n"
            .as_bytes(),
    );
    assert!(
        env.ok(&["account", "import", "kontoplan.txt"])
            .contains("4 konton")
    );
    assert!(
        env.ok(&["konto", "lista"])
            .contains("5410 Förbrukningsinventarier")
    );

    write(&env.path("kvitto.pdf"), b"%PDF-1.4 kvitto");
    assert!(
        env.ok(&["attachment", "add", "kvitto.pdf"])
            .contains("Underlag 1 mottaget")
    );
    assert!(
        env.ok(&["attachment", "add", "kvitto.pdf"])
            .contains("redan mottagen som underlag 1")
    );

    let out = env.ok(&[
        "propose",
        "--date",
        "2026-03-15",
        "--text",
        "Papper \"special\"",
        "--line",
        "5410=80,00",
        "--line",
        "1930=-80,00",
        "--attachment",
        "1",
        "--counterparty-name",
        "Papper AB",
        "--orgnr",
        "556000-0001",
    ]);
    assert!(out.contains("Förslag 1 lagt"), "{out}");
    env.ok(&[
        "propose",
        "--date",
        "2026-03-16",
        "--text",
        "Fel",
        "--line",
        "5410=0,50",
        "--line",
        "1930=-0,50",
    ]);
    env.ok(&["reject", "2", "--reason", "dubblett"]);
    let out = env.ok(&["approve", "1"]);
    assert!(out.contains("Bokförd som A1 i perioden 2026-03"), "{out}");
    let err = env.fails(&["approve", "2"]);
    assert!(err.contains("redan ett beslut"), "{err}");

    let out = env.ok(&[
        "correct",
        "A1",
        "--reason",
        "fel konto",
        "--line",
        "6570=80",
        "--line",
        "1930=-80",
        "--approve",
    ]);
    assert!(out.contains("Bokförd som R1"), "{out}");

    let journal = env.ok(&["journal", "--year", "2026"]);
    assert!(
        journal.contains("A1     2026-03-15  Papper \"special\""),
        "{journal}"
    );
    assert!(journal.contains("registrerad"), "{journal}");
    assert!(
        journal.contains("Motpart: Papper AB (556000-0001)"),
        "{journal}"
    );
    assert!(journal.contains("Rättad genom R1 (fel konto)"), "{journal}");
    assert!(journal.find("A1 ").unwrap() < journal.find("R1 ").unwrap());
    let ledger = env.ok(&["huvudbok", "--account", "5410"]);
    assert!(
        ledger.contains("Ingående balans") && ledger.contains("[rättad genom R1]"),
        "{ledger}"
    );
    let tb = env.ok(&["trial-balance"]);
    assert!(
        tb.contains("6570 Bankkostnader") && tb.contains("80,00"),
        "{tb}"
    );
    let shown = env.ok(&["show", "A1"]);
    assert!(
        shown.contains("Signerad av testare") && shown.contains("Kanonisk hash"),
        "{shown}"
    );

    env.ok(&["period", "review", "2026-02"]);
    env.ok(&["period", "close", "2026-02"]);
    env.ok(&["period", "lock", "2026-02"]);
    let err = env.fails(&["period", "reopen", "2026-02"]);
    assert!(err.contains("låst period"), "{err}");
    let err = env.fails(&[
        "propose",
        "--date",
        "2026-02-10",
        "--text",
        "Sent",
        "--line",
        "5410=1",
        "--line",
        "1930=-1",
        "--approve",
    ]);
    assert!(err.contains("tar inte emot poster"), "{err}");

    let out = env.ok(&["verify"]);
    assert!(out.contains("Bokföringen är intakt"), "{out}");
    assert!(
        out.contains("mjukvarunyckel"),
        "en mjukvarunyckel flaggas: {out}"
    );

    env.ok(&["export", "sie", "--year", "2026", "--out", "bok.se"]);
    let sie = sie_parser::decode_cp437(&std::fs::read(env.path("bok.se")).unwrap());
    assert!(
        sie.contains("#VER A 1 20260315 \"Papper \\\"special\\\" (rättad genom R1)\""),
        "{sie}"
    );
    assert!(sie.contains("#RES 0 6570 80"), "{sie}");
    assert!(sie.contains("#UB 0 1930 -80\r\n"), "{sie}");
    let doc = sie_parser::document::read(&sie).unwrap();
    let vouchers: Vec<String> = doc
        .vouchers
        .iter()
        .map(|v| format!("{}{}", v.series, v.number))
        .collect();
    assert_eq!(
        vouchers,
        ["A1", "R1"],
        "det avvisade och det vägrade förslaget finns inte i filen"
    );
    let err = env.fails(&["export", "sie", "--year", "2026", "--out", "bok.se"]);
    assert!(err.contains("skrivs aldrig över"), "{err}");

    let history = env.ok(&["historik"]);
    for action in [
        "databas.skapad",
        "kontoplan.andrad",
        "forslag.avvisat",
        "verifikation.bokford",
        "period.last",
        "export.skapad",
    ] {
        assert!(
            history.contains(action),
            "{action} saknas i historiken:\n{history}"
        );
    }

    // Ett ändrat underlag i arkivet upptäcks.
    let sha = std::fs::read_dir(env.path("data/underlag"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let file = std::fs::read_dir(&sha)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut perms = std::fs::metadata(&file).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(&file, perms).unwrap();
    write(&file, b"%PDF-1.4 ett annat kvitto");
    let out = env.bok(&["verify"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("underlag 1: filen har ändrats"));
}

#[test]
fn signing_with_an_unregistered_key_posts_nothing() {
    let Some(env) = Env::new() else {
        return;
    };
    env.ok(&[
        "init",
        "--security",
        "mjukvara",
        "--company",
        "Testbolaget AB",
        "--orgnr",
        "559999-9991",
        "--year",
        "2026",
    ]);
    write(&env.path("k.txt"), b"1930 Bank\n5410 Material\n");
    env.ok(&["account", "import", "k.txt"]);
    env.ok(&[
        "propose",
        "--date",
        "2026-03-15",
        "--text",
        "Papper",
        "--line",
        "5410=10",
        "--line",
        "1930=-10",
    ]);
    Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(env.path("annan"))
        .status()
        .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_bok"))
        .args(["approve", "1"])
        .env("BOKIS_DB", env.db())
        .env("BOKIS_KEY", env.path("annan"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("inte registrerad"));
    let out = env.ok(&["approve", "1"]);
    assert!(
        out.contains("Bokförd som A1"),
        "numret förbrukades inte av försöket: {out}"
    );
}

#[test]
fn the_security_level_is_chosen_at_init_and_changed_with_a_signature() {
    let Some(env) = Env::new() else {
        eprintln!("ssh-keygen saknas; hoppar över testet");
        return;
    };
    // Standardnivån kräver en hårdvarunyckel; testets nyckel är en fil.
    let err = env.fails(&[
        "init",
        "--company",
        "Testbolaget AB",
        "--orgnr",
        "559999-9991",
    ]);
    assert!(err.contains("kräver en hårdvarunyckel"), "{err}");
    assert!(err.contains("--security mjukvara"), "{err}");
    assert!(!env.db().exists());

    let out = env.ok(&[
        "init",
        "--sakerhet",
        "mjukvara",
        "--company",
        "Testbolaget AB",
        "--orgnr",
        "559999-9991",
    ]);
    assert!(out.contains("Säkerhetsnivå: mjukvarunyckel."), "{out}");
    let shown = env.ok(&["sakerhet", "visa"]);
    assert!(
        shown.contains("Säkerhetsnivå: mjukvarunyckel, sedan"),
        "{shown}"
    );
    assert!(
        shown.contains("mjukvarunyckel när bokföringen skapades"),
        "{shown}"
    );

    // Nyckelfilen kan inte höja nivån, och en okänd nivå avvisas.
    let err = env.fails(&["sakerhet", "satt", "hårdvara"]);
    assert!(err.contains("kommer från en mjukvarunyckel"), "{err}");
    let err = env.fails(&["sakerhet", "satt", "yubikey"]);
    assert!(err.contains("finns inte"), "{err}");
    let err = env.fails(&["sakerhet", "satt", "mjukvara"]);
    assert!(err.contains("redan"), "{err}");
    assert!(env.ok(&["verify"]).contains("Bokföringen är intakt"));
}

#[test]
fn organisation_numbers_are_checked() {
    let Some(env) = Env::new() else {
        eprintln!("ssh-keygen saknas; hoppar över testet");
        return;
    };
    let init = |orgnr: &str| {
        vec![
            "init".to_string(),
            "--security".into(),
            "mjukvara".into(),
            "--company".into(),
            "Testbolaget AB".into(),
            "--orgnr".into(),
            orgnr.into(),
            "--year".into(),
            "2026".into(),
        ]
    };
    let args = init("559000-0000");
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    // Numret som gick igenom vid starten 2026-11-16 har fel kontrollsiffra.
    let err = env.fails(&args);
    assert!(err.contains("kontrollsiffran"), "{err}");
    assert!(
        !env.db().exists(),
        "en misslyckad init lämnar ingen databas"
    );

    let args = init("5599999991");
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = env.ok(&args);
    assert!(out.contains("(559999-9991)"), "{out}");

    write(
        &env.path("kontoplan.txt"),
        "1930;Företagskonto\n6570;Bankkostnader\n".as_bytes(),
    );
    env.ok(&["account", "import", "kontoplan.txt"]);
    let err = env.fails(&[
        "propose",
        "--date",
        "2026-03-15",
        "--text",
        "Bankavgift mars",
        "--line",
        "6570=10,00",
        "--line",
        "1930=-10,00",
        "--counterparty-name",
        "Banken AB",
        "--orgnr",
        "556000-0000",
    ]);
    assert!(err.contains("kontrollsiffran"), "{err}");
    let out = env.ok(&[
        "propose",
        "--date",
        "2026-03-15",
        "--text",
        "Bankavgift mars",
        "--line",
        "6570=10,00",
        "--line",
        "1930=-10,00",
        "--counterparty-name",
        "Banken A/S",
        "--orgnr",
        "dk 1234 5678",
    ]);
    assert!(out.contains("Förslag"), "{out}");
}
