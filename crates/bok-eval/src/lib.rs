//! Mätning av agentens konteringar mot facit.
//!
//! Uppgifterna kommer från bokföringssviten i Accounted Bench
//! (<https://github.com/erp-mafia/accounted-bench>), uppgifter och facit
//! © 2025–2026 Jakob Wennberg, CC BY 4.0. De läses från en lokal kopia och
//! ingår inte i Bokis. Benchen ger modellen en uppgift i taget och ber om ett
//! konto och en momsbehandling. Här arbetar agenten i stället genom Bokis
//! MCP-server, som i verklig användning: varje uppgift blir ett underlag i
//! inkorgen, agenten lägger ett verifikationsförslag med hela konteringen, och
//! konto och moms härleds ur raderna (se [`score`]). Resultatet är därför inte
//! direkt jämförbart med benchens topplista.
//!
//! - [`tasks`] läser uppgifterna och visar dem som underlag.
//! - [`chart`] är kontoplanen agenten ser.
//! - [`record`] loggar förslagen under en körning.
//! - [`score`] poängsätter loggen mot facit.

use std::path::PathBuf;

use bok_mcp::backend::{Account, Document, DocumentInfo, Rule};
use bok_mcp::memory::MemoryBackend;

pub mod chart;
pub mod record;
pub mod score;
pub mod tasks;

#[derive(Debug, thiserror::Error)]
pub enum EvalError {
    #[error("{}: {}", .0.display(), .1)]
    Io(PathBuf, #[source] std::io::Error),
    #[error("uppgifterna: {0}")]
    Tasks(String),
    #[error("loggen, rad {line}: {message}")]
    Records { line: usize, message: String },
    #[error("kontoplanen: {0}")]
    Chart(String),
}

/// Reglerna agenten får i `hamta_regler` under utvärderingen. De beskriver
/// uppgiftens form och hur momsen ska synas i konteringen, inte vilket konto
/// eller vilken behandling som är rätt i ett enskilt fall.
pub fn eval_rules() -> Vec<Rule> {
    let rule = |id: &str, text: &str| Rule {
        id: id.into(),
        version: 1,
        text: text.into(),
        valid_from: None,
        valid_to: None,
    };
    vec![
        rule(
            "U1",
            "Utvärdering: varje underlag är en egen uppgift med ett eget företag. Företagets förhållanden \
             (verksamhet, bolagsform, momsregistrering, bokföringsmetod) står i underlaget och gäller bara det.",
        ),
        rule(
            "U2",
            "Lägg exakt ett verifikationsförslag per underlag med underlagets id i underlag_ids. Bokför hela \
             affärshändelsen mot företagskontot 1930; inga fakturor är bokförda sedan tidigare. Lägg hellre ett \
             förslag med hög osäkerhet än en fråga: en fråga utan förslag räknas som obesvarad.",
        ),
        rule(
            "U3",
            "Momsen syns i konteringen med BAS-konton: 2641 för avdragsgill ingående moms i Sverige; 2645 och \
             2614 för omvänd betalningsskyldighet vid köp från utlandet; 2647 och 2614 för omvänd \
             betalningsskyldighet inom Sverige; 2645 och 2615 för import av varor. Moms som inte får dras av \
             ingår i kostnaden.",
        ),
    ]
}

/// Ett lager i minnet med en uppgift per underlag, utan tidigare konteringar
/// och utan bankrader.
pub fn build_backend(tasks: &[tasks::Task], chart: Vec<Account>) -> MemoryBackend {
    let documents = tasks
        .iter()
        .map(|t| {
            let data = tasks::render_document(t).into_bytes();
            Document {
                info: DocumentInfo {
                    id: t.id.clone(),
                    sha256: bok_mcp::sha256_hex(&data),
                    file_name: format!("{}.txt", t.id),
                    media_type: "text/plain".into(),
                    size: data.len() as u64,
                    received_at: format!("{}T00:00:00Z", t.input.transaction.date),
                    channel: "utvardering".into(),
                    sender: None,
                    status: "oppen".into(),
                },
                data,
            }
        })
        .collect();
    MemoryBackend {
        documents,
        account_list: chart,
        rule_list: eval_rules(),
        history: Vec::new(),
        bank_rows: Vec::new(),
        cases: Vec::new(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use bok_mcp::server::Server;
    use serde_json::{json, Value};

    fn call(
        server: &mut Server<impl bok_mcp::backend::AgentBackend>,
        id: u64,
        name: &str,
        args: Value,
    ) -> Value {
        let request = json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": args}});
        let reply = server.handle_line(&request.to_string()).unwrap();
        serde_json::from_str(&reply).unwrap()
    }

    #[test]
    fn agent_sees_tasks_as_documents_and_proposals_are_logged() {
        let tasks: Vec<tasks::Task> = serde_json::from_str(tasks::tests::SAMPLE).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("forslag.jsonl");
        let chart = chart::parse_chart(
            "1930 Företagskonto\n2641 Debiterad ingående moms\n6530 Redovisningstjänster\n",
        )
        .unwrap();
        let backend = record::RecordingBackend::open(build_backend(&tasks, chart), &log).unwrap();
        let mut server = Server::new(backend);
        let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}});
        server.handle_line(&init.to_string()).unwrap();

        let listed = call(&mut server, 2, "lista_underlag", json!({}));
        let text = listed["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("booking-001") && text.contains("booking-216"));
        assert!(!text.contains("domestic_25"));

        let doc = call(
            &mut server,
            3,
            "hamta_underlag",
            json!({"id": "booking-101"}),
        );
        let doc_text = doc["result"]["content"].to_string();
        assert!(doc_text.contains("EXEMPELMOLN*KONTOR"));
        assert!(!doc_text.contains("5420"));

        let proposal = json!({
            "affarshandelse_datum": "2026-05-12",
            "beskrivning": "Redovisningstjänster oktober, Exempel Redovisning AB",
            "motpart": {"namn": "Exempel Redovisning AB"},
            "rader": [
                {"konto": 6530, "debet_ore": 500000},
                {"konto": 2641, "debet_ore": 125000},
                {"konto": 1930, "kredit_ore": 625000}
            ],
            "underlag_ids": ["booking-001"],
            "harkomst": {"modell": "testmodell", "motivering": "Redovisningstjänst med 25 % moms.", "osakerhet": 0.1}
        });
        let reply = call(&mut server, 4, "foresla_verifikation", proposal);
        assert_eq!(reply["result"]["isError"], json!(false), "{reply}");

        let records = record::parse_records(&std::fs::read_to_string(&log).unwrap()).unwrap();
        let report = score::score(&tasks[..2], &records);
        assert_eq!((report.summary.answered, report.summary.passed), (1, 1));
        assert_eq!(report.results[1].status, score::Status::Missing);
    }
}
