//! `bok-review`: granskningsvyn som lokal webbsida.
//!
//! ```text
//! bok-review --db <bokis.db> [--nyckel <fil>] [--granskare <namn>] [--port 8484]
//! bok-review --demo [--port 8484]
//! ```
//!
//! Öppna länken som skrivs ut. Den innehåller en engångsnyckel för sessionen.
//! Med `--db` bokför ett godkännande på riktigt: `ssh-keygen` signerar med
//! nyckeln (standard `~/.ssh/id_ed25519_sk`) och väntar på en beröring.

use bok_review::backend::{MemoryReviewBackend, ReviewBackend};
use bok_review::router::Router;
use bok_review::server::{bind, random_token, serve};

fn value<'a>(args: &'a [String], flag: &str) -> Option<&'a String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
}

fn run<B: ReviewBackend>(backend: B, port: u16, what: &str) -> std::io::Result<()> {
    let (listener, port) = bind(port)?;
    let login = random_token()?;
    let session = random_token()?;
    let csrf = random_token()?;
    let mut router = Router::new(backend, login.clone(), session, csrf, port);
    println!("Granskningsvyn körs {what}. Öppna (länken fungerar en gång):\n  http://127.0.0.1:{port}/login?token={login}");
    serve(&mut router, &listener, None)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let port = value(&args, "--port")
        .and_then(|p| p.parse().ok())
        .unwrap_or(8484);
    let result = if args.iter().any(|a| a == "--demo") {
        run(MemoryReviewBackend::demo(), port, "med exempeldata").map_err(|e| e.to_string())
    } else if let Some(db) = value(&args, "--db") {
        run_store(std::path::Path::new(db), &args, port)
    } else {
        eprintln!("bok-review: ange --db <bokis.db>, eller --demo för exempeldata.");
        std::process::exit(2);
    };
    if let Err(e) = result {
        eprintln!("bok-review: {e}");
        std::process::exit(1);
    }
}

#[cfg(feature = "store")]
fn run_store(db: &std::path::Path, args: &[String], port: u16) -> Result<(), String> {
    use std::path::PathBuf;
    let key = value(args, "--nyckel")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default()
                .join(".ssh/id_ed25519_sk")
        });
    let reviewer = value(args, "--granskare")
        .cloned()
        .or_else(|| std::env::var("USER").ok())
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| "granskare".into());
    let signer = Box::new(bok_review::signer::SshKeygenSigner::new(key));
    let backend = bok_review::store::StoreReviewBackend::open(
        db,
        concat!("bok-review ", env!("CARGO_PKG_VERSION")),
        &reviewer,
        signer,
    )
    .map_err(|e| e.to_string())?;
    run(
        backend,
        port,
        &format!("mot {} som {reviewer}", db.display()),
    )
    .map_err(|e| e.to_string())
}

#[cfg(not(feature = "store"))]
fn run_store(_db: &std::path::Path, _args: &[String], _port: u16) -> Result<(), String> {
    Err("byggd utan kopplingen till databasen; bygg med --features store".into())
}
