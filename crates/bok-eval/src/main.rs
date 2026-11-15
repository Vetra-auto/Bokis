//! `bok-eval`: mäter agentens konteringar mot facit.
//!
//! ```text
//! bok-eval serve <uppgifter>... --logg <fil> [--kontoplan <fil>] [--med-stagade] [--forsta N]
//! bok-eval score <uppgifter>... --logg <fil> [--med-stagade] [--forsta N] [--json]
//! ```
//!
//! `<uppgifter>` är filer eller kataloger med uppgifter, normalt
//! `accounted-bench/tasks/booking`. `serve` är en MCP-server över stdio med en
//! uppgift per underlag; den kopplas till agenten som vilken MCP-server som
//! helst. `score` jämför loggen med facit.

use std::path::PathBuf;
use std::process::ExitCode;

use bok_eval::{build_backend, chart, record, score, tasks, EvalError};

const USAGE: &str = "Användning:
  bok-eval serve <uppgifter>... --logg <fil> [--kontoplan <fil>] [--med-stagade] [--forsta N]
  bok-eval score <uppgifter>... --logg <fil> [--med-stagade] [--forsta N] [--json]

<uppgifter> är filer eller kataloger, normalt accounted-bench/tasks/booking.
Utan --kontoplan används benchens context-accounts.txt bredvid uppgifterna.";

#[derive(Debug, Default, PartialEq)]
struct Options {
    command: String,
    sources: Vec<PathBuf>,
    log: Option<PathBuf>,
    chart: Option<PathBuf>,
    include_staged: bool,
    first: Option<usize>,
    json: bool,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut options = Options::default();
    let mut iter = args.iter();
    options.command = iter.next().cloned().ok_or("ange serve eller score")?;
    if options.command != "serve" && options.command != "score" {
        return Err(format!("okänt kommando: {}", options.command));
    }
    while let Some(arg) = iter.next() {
        let mut value = |name: &str| {
            iter.next()
                .cloned()
                .ok_or(format!("{name} kräver ett värde"))
        };
        match arg.as_str() {
            "--logg" => options.log = Some(PathBuf::from(value("--logg")?)),
            "--kontoplan" => options.chart = Some(PathBuf::from(value("--kontoplan")?)),
            "--med-stagade" => options.include_staged = true,
            "--forsta" => {
                let n = value("--forsta")?;
                options.first = Some(
                    n.parse()
                        .map_err(|_| format!("--forsta: {n} är inget antal"))?,
                );
            }
            "--json" => options.json = true,
            flag if flag.starts_with("--") => return Err(format!("okänd flagga: {flag}")),
            path => options.sources.push(PathBuf::from(path)),
        }
    }
    if options.sources.is_empty() {
        return Err("ange var uppgifterna finns".into());
    }
    if options.log.is_none() {
        return Err("ange loggfilen med --logg".into());
    }
    Ok(options)
}

fn load(options: &Options) -> Result<Vec<tasks::Task>, EvalError> {
    let mut tasks = tasks::load_tasks(&options.sources, options.include_staged)?;
    if let Some(n) = options.first {
        tasks.truncate(n);
    }
    Ok(tasks)
}

fn serve(options: &Options, log: &std::path::Path) -> Result<(), EvalError> {
    let chart_path = match &options.chart {
        Some(path) => path.clone(),
        None => tasks::bench_chart_path(&options.sources).ok_or_else(|| {
            EvalError::Chart(
                "ingen context-accounts.txt bredvid uppgifterna; ange kontoplanen med --kontoplan <fil>"
                    .into(),
            )
        })?,
    };
    let chart = read_chart(&chart_path)?;
    let tasks = load(options)?;
    eprintln!(
        "bok-eval: {} uppgifter, {} konton, loggar till {}",
        tasks.len(),
        chart.len(),
        log.display()
    );
    let backend = record::RecordingBackend::open(build_backend(&tasks, chart), log)?;
    bok_mcp::server::run_stdio(bok_mcp::server::Server::new(backend))
        .map_err(|e| EvalError::Io(log.to_path_buf(), e))
}

fn read_chart(path: &std::path::Path) -> Result<Vec<bok_mcp::backend::Account>, EvalError> {
    let text = std::fs::read_to_string(path).map_err(|e| EvalError::Io(path.to_path_buf(), e))?;
    chart::parse_chart(&text).map_err(|e| EvalError::Chart(format!("{}: {e}", path.display())))
}

fn score_command(options: &Options, log: &std::path::Path) -> Result<String, EvalError> {
    let tasks = load(options)?;
    let text = std::fs::read_to_string(log).map_err(|e| EvalError::Io(log.to_path_buf(), e))?;
    let records = record::parse_records(&text)?;
    let report = score::score(&tasks, &records);
    if options.json {
        serde_json::to_string_pretty(&report).map_err(|e| EvalError::Tasks(e.to_string()))
    } else {
        Ok(score::render_text(&report))
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match parse(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("bok-eval: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let Some(log) = options.log.clone() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let result = if options.command == "serve" {
        serve(&options, &log)
    } else {
        score_command(&options, &log).map(|text| println!("{text}"))
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("bok-eval: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn parses_commands_and_flags() {
        let o = parse(&args(
            "serve tasks/booking --logg f.jsonl --forsta 5 --med-stagade --kontoplan plan.txt",
        ))
        .unwrap();
        assert_eq!(o.command, "serve");
        assert_eq!(o.sources, vec![PathBuf::from("tasks/booking")]);
        assert_eq!(o.log, Some(PathBuf::from("f.jsonl")));
        assert_eq!(
            (o.first, o.include_staged, o.chart.as_deref()),
            (Some(5), true, Some(std::path::Path::new("plan.txt")))
        );
        let o = parse(&args("score a.json b.json --logg f.jsonl --json")).unwrap();
        assert_eq!((o.sources.len(), o.json), (2, true));
        assert!(parse(&args("score a.json")).is_err());
        assert!(parse(&args("score --logg f.jsonl")).is_err());
        assert!(parse(&args("run a.json --logg f.jsonl")).is_err());
        assert!(parse(&args("score a.json --logg")).is_err());
        assert!(parse(&args("score a.json --logg f --forsta x")).is_err());
        assert!(parse(&args("score a.json --logg f --okand")).is_err());
    }
}
