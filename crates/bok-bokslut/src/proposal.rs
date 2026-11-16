//! Det gemensamma för bokslutsförslagen: serie B, en idempotensnyckel ur
//! raderna och härkomsten.

use chrono::NaiveDate;

use bok_core::Series;
use bok_core::canonical::sha256_hex;
use bok_core::voucher::Line;
use bok_store::{Actor, NewProposal, Provenance, Store};

use crate::Result;

/// Ett lagt förslag och det granskaren bör veta om det.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposed {
    /// Förslagets id.
    pub id: i64,
    /// Sådant som granskaren bör se innan förslaget godkänns.
    pub notes: Vec<String>,
}

/// En nyckel ur raderna, så att samma förslag inte läggs två gånger men ett
/// ändrat belopp ger ett nytt.
pub fn fingerprint(lines: &[Line]) -> String {
    let text = lines
        .iter()
        .map(|l| format!("{}:{}:{}", l.account, l.debit.ore(), l.credit.ore()))
        .collect::<Vec<_>>()
        .join(";");
    sha256_hex(text.as_bytes())
}

/// Lägger ett bokslutsförslag i serie B.
pub fn propose(
    store: &mut Store,
    actor: &Actor,
    kind: &str,
    date: NaiveDate,
    description: String,
    lines: Vec<Line>,
    provenance: Provenance,
) -> Result<i64> {
    let key = format!("bokslut:{kind}:{date}:{}", fingerprint(&lines));
    let proposal = NewProposal {
        series: Series::B,
        date,
        description,
        counterparty: None,
        lines,
        attachments: vec![],
        idempotency_key: Some(key),
        provenance,
    };
    Ok(store.create_proposal(actor, &proposal)?)
}
