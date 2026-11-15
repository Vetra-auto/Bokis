//! `bok-mcp`: MCP-server för Bokis över stdio.
//!
//! ```text
//! bok-mcp --db ~/.local/share/bokis/bokis.db    mot bokföringen (funktionen store)
//! bok-mcp --demo                                med exempeldata
//! ```
//!
//! Registreras i agentens MCP-klient, till exempel:
//!
//! ```text
//! claude mcp add bokis -- /sökväg/till/bok-mcp --db ~/.local/share/bokis/bokis.db
//! ```

use bok_mcp::memory::MemoryBackend;
use bok_mcp::server::{run_stdio, Server};

const USAGE: &str =
    "bok-mcp: ange --db <bokis.db> för att arbeta mot bokföringen, eller --demo för exempeldata.";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = if args.iter().any(|a| a == "--demo") {
        run_stdio(Server::new(MemoryBackend::demo())).map_err(|e| e.to_string())
    } else if let Some(db) = args
        .iter()
        .position(|a| a == "--db")
        .and_then(|i| args.get(i + 1))
    {
        run_store(std::path::Path::new(db))
    } else {
        eprintln!("{USAGE}");
        std::process::exit(2);
    };
    if let Err(e) = result {
        eprintln!("bok-mcp: {e}");
        std::process::exit(1);
    }
}

#[cfg(feature = "store")]
fn run_store(db: &std::path::Path) -> Result<(), String> {
    let backend =
        bok_mcp::store::StoreBackend::open(db, concat!("bok-mcp ", env!("CARGO_PKG_VERSION")))
            .map_err(|e| e.to_string())?;
    run_stdio(Server::new(backend)).map_err(|e| e.to_string())
}

#[cfg(not(feature = "store"))]
fn run_store(_db: &std::path::Path) -> Result<(), String> {
    Err("byggd utan kopplingen till databasen; bygg med --features store".into())
}
