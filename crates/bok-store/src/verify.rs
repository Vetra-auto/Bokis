//! `bok verify`: kontrollerar att historiken är obruten, att serierna saknar
//! luckor och att varje bokförd verifikation har en giltig signatur över sin
//! kanoniska form. Kontrollen läser bara; den ändrar ingenting.

use std::collections::BTreeMap;

use rusqlite::{OptionalExtension, params};
use serde_json::Value;

use bok_core::chain::{EventRecord, GENESIS, verify_events, voucher_chain_hash};
use bok_core::decision::{
    ChartEntry, chart_change_hash, key_registration_hash, period_decision_hash,
};
use bok_core::security::SecurityLevel;
use bok_core::signature::{
    NS_KEY, NS_PERIOD, NS_SYSTEM, NS_VOUCHER, signer_fingerprint, sk_flags, verify,
};
use bok_core::voucher::{Counterparty, PostedVoucher, validate_lines};
use bok_core::{AccountNumber, PeriodStatus};

use crate::ops::security::{SECURITY_LEVEL_DECIDED, security_level_hash};
use crate::ops::{
    KEY_COLUMNS, corrected_original, fiscal_year_by_id, key_by_id, key_from_row, parse_series,
    period_by_id, voucher_lines,
};
use crate::reports::attachment_hashes;
use crate::{
    CLOCK_ACCEPTED, Result, Store, display_timestamp, format_timestamp, parse_date, parse_timestamp,
};

/// En del av kontrollen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Check {
    pub name: &'static str,
    /// Antal kontrollerade poster.
    pub checked: usize,
    pub problems: Vec<String>,
    /// Sådant som inte är fel men bör ses över, till exempel att
    /// säkerhetsnivån godtar nyckelfiler.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VerifyReport {
    pub checks: Vec<Check>,
}

impl VerifyReport {
    pub fn is_ok(&self) -> bool {
        self.checks.iter().all(|c| c.problems.is_empty())
    }

    pub fn problems(&self) -> impl Iterator<Item = &String> {
        self.checks.iter().flat_map(|c| c.problems.iter())
    }
}

/// Bankrad, bokföringsdag, periodens status, matchning, verifikation, konto, belopp.
type BankCheckRow = (
    i64,
    String,
    Option<String>,
    Option<i64>,
    Option<i64>,
    i64,
    i64,
);

fn check(name: &'static str) -> Check {
    Check {
        name,
        ..Check::default()
    }
}

struct ChartRow {
    change: i64,
    number: i64,
    name: String,
    active: bool,
    sru: Option<String>,
    vat_code: Option<String>,
    valid_from: String,
}

struct EventSig {
    action: String,
    time: String,
    old: Option<Value>,
    new: Option<Value>,
    signature: Option<String>,
}

impl Store {
    pub fn verify(&self) -> Result<VerifyReport> {
        Ok(VerifyReport {
            checks: vec![
                self.verify_events()?,
                self.verify_series()?,
                self.verify_vouchers()?,
                self.verify_periods()?,
                self.verify_chart()?,
                self.verify_keys()?,
                self.verify_security_level()?,
                self.verify_bank()?,
                self.verify_rules()?,
            ],
        })
    }

    /// Bankraderna: avstämda om de ligger i en stängd eller låst period, och
    /// varje avstämning mot en verifikation har rätt belopp på bankens konto.
    fn verify_bank(&self) -> Result<Check> {
        let mut c = check("Bankraderna i stängda perioder är avstämda");
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT b.id, b.bokforingsdag, p.status, m.id, m.verifikation_id, b.konto, b.belopp
             FROM bankrader b
             LEFT JOIN perioder p ON b.bokforingsdag BETWEEN p.start AND p.slut
             LEFT JOIN bankmatchningar m ON m.bankrad_id = b.id
             ORDER BY b.id",
        )?;
        let rows: Vec<BankCheckRow> = stmt
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        for (id, date, status, matched, voucher, account, amount) in rows {
            c.checked += 1;
            if matched.is_none() && matches!(status.as_deref(), Some("stangd" | "last")) {
                c.problems.push(format!(
                    "bankrad {id} ({date}) ligger i en {} period men är inte avstämd",
                    if status.as_deref() == Some("last") {
                        "låst"
                    } else {
                        "stängd"
                    }
                ));
            }
            if let Some(v) = voucher {
                let net: i64 = conn.query_row(
                    "SELECT COALESCE(SUM(debet - kredit), 0) FROM rader WHERE verifikation_id = ?1 AND konto = ?2",
                    params![v, account],
                    |r| r.get(0),
                )?;
                if net != amount {
                    c.problems.push(format!(
                        "bankrad {id} är avstämd mot verifikation id {v}, men beloppen skiljer sig ({net} och {amount} öre)"
                    ));
                }
            }
        }
        Ok(c)
    }

    fn event_sig(&self, seq: i64) -> Result<Option<EventSig>> {
        let parse = |s: Option<String>| s.and_then(|t| serde_json::from_str::<Value>(&t).ok());
        Ok(self
            .conn()
            .query_row(
                "SELECT handling, tid, gamla, nya, signatur FROM handelser WHERE seq = ?1",
                params![seq],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()?
            .map(|(action, time, old, new, signature)| EventSig {
                action,
                time,
                old: parse(old),
                new: parse(new),
                signature,
            }))
    }

    fn key_by_fingerprint(&self, fingerprint: &str) -> Result<Option<crate::KeyRow>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {KEY_COLUMNS} FROM nycklar WHERE fingeravtryck = ?1"),
                params![fingerprint],
                key_from_row,
            )
            .optional()?)
    }

    /// Historiken: obruten följd, rätt föregående hash, hash som stämmer.
    fn verify_events(&self) -> Result<Check> {
        let mut c = check("Behandlingshistoriken är obruten");
        let mut stmt = self.conn().prepare(
            "SELECT seq, tid, aktor_typ, aktor, roll, handling, objekt, gamla, nya, programversion, regelversion,
                    signatur, foregaende_hash, hash FROM handelser ORDER BY seq",
        )?;
        let events: Vec<(EventRecord, String)> = stmt
            .query_map([], |r| {
                Ok((
                    EventRecord {
                        seq: r.get(0)?,
                        time: r.get(1)?,
                        actor_kind: r.get(2)?,
                        actor: r.get(3)?,
                        role: r.get(4)?,
                        action: r.get(5)?,
                        object: r.get(6)?,
                        old_values: r.get(7)?,
                        new_values: r.get(8)?,
                        program_version: r.get(9)?,
                        rule_version: r.get(10)?,
                        signature: r.get(11)?,
                        prev_hash: r.get(12)?,
                    },
                    r.get(13)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        c.checked = events.len();
        if let Err(e) = verify_events(events.iter().map(|(e, h)| (e, h.as_str()))) {
            c.problems.push(e.to_string());
        }
        // Tiderna går framåt, utom där en människa har godtagit att klockan
        // stod före den senaste händelsen.
        let mut previous = None;
        for (e, _) in &events {
            let Ok(t) = parse_timestamp(&e.time) else {
                c.problems.push(format!(
                    "händelse {}: tiden {} går inte att läsa",
                    e.seq, e.time
                ));
                continue;
            };
            if let Some(p) = previous
                && t < p
                && e.action != CLOCK_ACCEPTED
            {
                c.problems.push(format!(
                    "händelse {}: tiden {} är före föregående händelse ({}), och ingen har godtagit det",
                    e.seq,
                    display_timestamp(&e.time),
                    display_timestamp(&format_timestamp(p))
                ));
            }
            previous = Some(t);
        }
        Ok(c)
    }

    /// Varje serie är 1, 2, 3 … utan luckor, och räknaren står på nästa nummer.
    fn verify_series(&self) -> Result<Check> {
        let mut c = check("Verifikationsserierna saknar luckor");
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT n.rakenskapsar_id, r.start, n.serie, n.nasta FROM nummerserier n
             JOIN rakenskapsar r ON r.id = n.rakenskapsar_id ORDER BY r.start, n.serie",
        )?;
        let series: Vec<(i64, String, String, i64)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (year_id, year_start, serie, next) in series {
            let mut numbers = conn.prepare_cached(
                "SELECT nummer FROM verifikationer WHERE rakenskapsar_id = ?1 AND serie = ?2 ORDER BY nummer",
            )?;
            let found: Vec<i64> = numbers
                .query_map(params![year_id, serie], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            c.checked += found.len();
            for (i, n) in found.iter().enumerate() {
                if *n != i as i64 + 1 {
                    c.problems.push(format!(
                        "serie {serie} i året från {year_start}: väntade nummer {}, fann {n}",
                        i + 1
                    ));
                    break;
                }
            }
            if next != found.len() as i64 + 1 {
                c.problems.push(format!(
                    "serie {serie} i året från {year_start}: räknaren står på {next} men serien har {} verifikationer",
                    found.len()
                ));
            }
        }
        let orphans: i64 = conn.query_row(
            "SELECT COUNT(*) FROM verifikationer v WHERE NOT EXISTS
               (SELECT 1 FROM nummerserier n WHERE n.rakenskapsar_id = v.rakenskapsar_id AND n.serie = v.serie)",
            [],
            |r| r.get(0),
        )?;
        if orphans > 0 {
            c.problems
                .push(format!("{orphans} verifikationer saknar nummerserie"));
        }
        Ok(c)
    }

    /// Varje bokförd verifikation: raderna, den kanoniska formen, signaturen,
    /// beslutet och verifikationskedjan.
    fn verify_vouchers(&self) -> Result<Check> {
        let mut c = check("Varje bokförd verifikation har en giltig signatur");
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, rakenskapsar_id, serie, nummer, period_id, datum, sammanstallningsdag, registrerad,
                    beskrivning, motpart_namn, motpart_orgnr, motpart_momsnr, forslag_id, antal_rader,
                    kanonisk_hash, signatur, nyckel_id, kedjehash
             FROM verifikationer ORDER BY id",
        )?;
        type Row = (
            i64,
            i64,
            String,
            i64,
            i64,
            String,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            i64,
            i64,
            String,
            String,
            i64,
            String,
        );
        let rows: Vec<Row> = stmt
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                    r.get(10)?,
                    r.get(11)?,
                    r.get(12)?,
                    r.get(13)?,
                    r.get(14)?,
                    r.get(15)?,
                    r.get(16)?,
                    r.get(17)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut previous_chain = GENESIS.to_string();
        for (
            id,
            year_id,
            serie,
            nummer,
            period_id,
            datum,
            compiled,
            registered,
            description,
            cp_name,
            cp_org,
            cp_vat,
            proposal_id,
            line_count,
            stored_hash,
            signature,
            key_id,
            chain_hash,
        ) in rows
        {
            c.checked += 1;
            let label = format!("{serie}{nummer} (id {id})");
            let lines = voucher_lines(conn, id)?;
            if lines.len() as i64 != line_count {
                c.problems.push(format!(
                    "{label}: {} rader, men verifikationen anger {line_count}",
                    lines.len()
                ));
            }
            if let Err(e) = validate_lines(&lines) {
                c.problems.push(format!("{label}: {e}"));
            }
            let year = fiscal_year_by_id(conn, year_id)?;
            let period = period_by_id(conn, period_id)?;
            let original: Option<i64> = conn
                .query_row(
                    "SELECT original_id FROM rattelser WHERE rattelse_id = ?1",
                    params![id],
                    |r| r.get(0),
                )
                .optional()?;
            let corrects = match original {
                Some(o) => Some(corrected_original(conn, o)?),
                None => None,
            };
            if (serie == "R") != corrects.is_some() {
                c.problems.push(format!(
                    "{label}: serie R och kopplingen till ett original hör inte ihop"
                ));
            }
            let voucher = PostedVoucher {
                series: parse_series(&serie)?,
                number: nummer,
                fiscal_year: (year.start, year.end),
                period: (period.start, period.end),
                date: parse_date(&datum)?,
                compiled_on: parse_date(&compiled)?,
                registered_at: registered.clone(),
                description,
                counterparty: cp_name.map(|name| Counterparty {
                    name,
                    org_number: cp_org,
                    vat_number: cp_vat,
                }),
                proposal_id,
                attachments: attachment_hashes(conn, id)?,
                corrects,
                lines,
            };
            let recomputed = voucher.canonical_hash();
            if recomputed != stored_hash {
                c.problems.push(format!(
                    "{label}: innehållet stämmer inte med den kanoniska hashen"
                ));
            }
            match key_by_id(conn, key_id) {
                Ok(key) => {
                    if let Err(e) = verify(
                        &key.public_key,
                        NS_VOUCHER,
                        recomputed.as_bytes(),
                        &signature,
                    ) {
                        c.problems
                            .push(format!("{label}: signaturen är inte giltig: {e}"));
                    }
                    if key.registered_at > registered {
                        c.problems
                            .push(format!("{label}: nyckeln registrerades efter bokföringen"));
                    }
                }
                Err(e) => c.problems.push(format!("{label}: {e}")),
            }
            let approved: bool = conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM beslut WHERE verifikation_id = ?1 AND forslag_id = ?2
                                AND utfall = 'godkant' AND signatur = ?3)",
                params![id, proposal_id, signature],
                |r| r.get(0),
            )?;
            if !approved {
                c.problems
                    .push(format!("{label}: det signerade godkännandet saknas"));
            }
            let expected_chain = voucher_chain_hash(&previous_chain, &stored_hash);
            if expected_chain != chain_hash {
                c.problems
                    .push(format!("{label}: verifikationskedjan är bruten"));
            }
            previous_chain = chain_hash;
        }
        Ok(c)
    }

    /// Stängda och låsta perioder har signerade beslut, och varje periods
    /// status är den som historiken senast gav den.
    fn verify_periods(&self) -> Result<Check> {
        let mut c = check("Stängda och låsta perioder har signerade beslut");
        let conn = self.conn();
        let mut history = conn.prepare(
            "SELECT p.start, p.status,
                    COALESCE((SELECT json_extract(h.nya, '$.status') FROM handelser h
                              WHERE h.objekt = 'period:' || p.id AND h.handling LIKE 'period.%'
                              ORDER BY h.seq DESC LIMIT 1), 'oppen')
             FROM perioder p ORDER BY p.start",
        )?;
        let statuses: Vec<(String, String, String)> = history
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (start, status, logged) in statuses {
            if status != logged {
                c.problems.push(format!(
                    "period {}: status {status}, men historiken säger {logged}",
                    &start[..7]
                ));
            }
        }
        let mut stmt = conn.prepare(
            "SELECT id, start, slut, status FROM perioder WHERE status IN ('stangd', 'last') ORDER BY start",
        )?;
        let periods: Vec<(i64, String, String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (id, start, end, status) in periods {
            c.checked += 1;
            let label = &start[..7];
            let mut needed = vec![("period.stangd", PeriodStatus::Closed)];
            if status == "last" {
                needed.push(("period.last", PeriodStatus::Locked));
            }
            for (action, to) in needed {
                let seq: Option<i64> = conn
                    .query_row(
                        "SELECT seq FROM handelser WHERE objekt = ?1 AND handling = ?2 ORDER BY seq DESC LIMIT 1",
                        params![format!("period:{id}"), action],
                        |r| r.get(0),
                    )
                    .optional()?;
                let Some(event) = (match seq {
                    Some(s) => self.event_sig(s)?,
                    None => None,
                }) else {
                    c.problems.push(format!(
                        "period {label}: beslutet ({action}) saknas i historiken"
                    ));
                    continue;
                };
                let from = event
                    .old
                    .as_ref()
                    .and_then(|v| v["status"].as_str())
                    .and_then(|s| PeriodStatus::from_db_str(s).ok());
                let stored = event
                    .new
                    .as_ref()
                    .and_then(|v| v["beslutshash"].as_str().map(str::to_string));
                let fingerprint = event
                    .new
                    .as_ref()
                    .and_then(|v| v["nyckel"].as_str().map(str::to_string));
                let (Some(from), Some(stored), Some(fingerprint), Some(signature)) =
                    (from, stored, fingerprint, event.signature)
                else {
                    c.problems.push(format!(
                        "period {label}: beslutet ({action}) är inte signerat"
                    ));
                    continue;
                };
                let hash = period_decision_hash(
                    parse_date(&start)?,
                    parse_date(&end)?,
                    from,
                    to,
                    &event.time,
                );
                if hash != stored {
                    c.problems.push(format!(
                        "period {label}: beslutet ({action}) stämmer inte med perioden"
                    ));
                    continue;
                }
                match self.key_by_fingerprint(&fingerprint)? {
                    Some(key) => {
                        if let Err(e) =
                            verify(&key.public_key, NS_PERIOD, hash.as_bytes(), &signature)
                        {
                            c.problems.push(format!(
                                "period {label}: signaturen på beslutet är inte giltig: {e}"
                            ));
                        }
                    }
                    None => c.problems.push(format!(
                        "period {label}: nyckeln {fingerprint} är inte registrerad"
                    )),
                }
            }
        }
        Ok(c)
    }

    /// Varje version av kontoplanen kommer från en signerad systemändring.
    fn verify_chart(&self) -> Result<Check> {
        let mut c = check("Kontoplanen ändras bara genom signerade systemändringar");
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT andring, konto, namn, aktiv, sru, momskod, giltig_fran FROM kontoplan ORDER BY andring, konto",
        )?;
        let rows: Vec<ChartRow> = stmt
            .query_map([], |r| {
                Ok(ChartRow {
                    change: r.get(0)?,
                    number: r.get(1)?,
                    name: r.get(2)?,
                    active: r.get::<_, i64>(3)? == 1,
                    sru: r.get(4)?,
                    vat_code: r.get(5)?,
                    valid_from: r.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut batches: BTreeMap<i64, Vec<ChartRow>> = BTreeMap::new();
        for row in rows {
            batches.entry(row.change).or_default().push(row);
        }
        for (seq, rows) in batches {
            c.checked += rows.len();
            let Some(event) = self.event_sig(seq)? else {
                c.problems
                    .push(format!("kontoplanen: ändring {seq} saknas i historiken"));
                continue;
            };
            if event.action != "kontoplan.andrad" {
                c.problems.push(format!(
                    "kontoplanen: händelse {seq} är ingen ändring av kontoplanen"
                ));
                continue;
            }
            let valid_from = &rows[0].valid_from;
            if rows.iter().any(|r| &r.valid_from != valid_from) {
                c.problems.push(format!(
                    "kontoplanen: ändring {seq} har flera giltighetsdatum"
                ));
                continue;
            }
            let mut entries = Vec::new();
            for r in &rows {
                entries.push(ChartEntry {
                    number: AccountNumber::new(r.number)?,
                    name: &r.name,
                    active: r.active,
                    sru: r.sru.as_deref(),
                    vat_code: r.vat_code.as_deref(),
                });
            }
            let hash = chart_change_hash(parse_date(valid_from)?, &entries);
            let stored = event
                .new
                .as_ref()
                .and_then(|v| v["beslutshash"].as_str().map(str::to_string));
            let fingerprint = event
                .new
                .as_ref()
                .and_then(|v| v["nyckel"].as_str().map(str::to_string));
            if stored.as_deref() != Some(hash.as_str()) {
                c.problems.push(format!(
                    "kontoplanen: ändring {seq} stämmer inte med de lagrade kontona"
                ));
                continue;
            }
            let (Some(fingerprint), Some(signature)) = (fingerprint, event.signature) else {
                c.problems
                    .push(format!("kontoplanen: ändring {seq} är inte signerad"));
                continue;
            };
            match self.key_by_fingerprint(&fingerprint)? {
                Some(key) => {
                    if let Err(e) = verify(&key.public_key, NS_SYSTEM, hash.as_bytes(), &signature)
                    {
                        c.problems.push(format!(
                            "kontoplanen: signaturen på ändring {seq} är inte giltig: {e}"
                        ));
                    }
                }
                None => c.problems.push(format!(
                    "kontoplanen: nyckeln {fingerprint} är inte registrerad"
                )),
            }
        }
        Ok(c)
    }

    /// Varje regelversion är en signerad systemändring, och hashen i
    /// Säkerhetsnivån: varje beslut är signerat och hänger ihop med beslutet
    /// före, och varje signatur i historiken klarar nivån som gällde när den
    /// gjordes.
    fn verify_security_level(&self) -> Result<Check> {
        let mut c = check("Signaturerna klarar säkerhetsnivån");
        let decisions = crate::ops::security::decisions(self.conn())?;
        let mut previous: Option<SecurityLevel> = None;
        for d in &decisions {
            c.checked += 1;
            let label = format!(
                "beslutet om säkerhetsnivån {} ({})",
                d.level.title(),
                display_timestamp(&d.decided)
            );
            if d.previous != previous {
                c.problems.push(format!(
                    "{label}: nivån före stämmer inte med beslutet innan"
                ));
            }
            // Ett beslut ska klara både nivån före och den nya.
            let required = previous.unwrap_or(SecurityLevel::Software).max(d.level);
            previous = Some(d.level);
            let Some(event) = self.event_sig(d.event)? else {
                c.problems
                    .push(format!("{label}: beslutet saknas i historiken"));
                continue;
            };
            if event.action != SECURITY_LEVEL_DECIDED {
                c.problems.push(format!(
                    "{label}: händelse {} är inget beslut om säkerhetsnivån",
                    d.event
                ));
                continue;
            }
            let hash = security_level_hash(d.level, d.previous, &d.decided);
            let stored = event
                .new
                .as_ref()
                .and_then(|v| v["beslutshash"].as_str().map(str::to_string));
            if stored.as_deref() != Some(hash.as_str()) {
                c.problems
                    .push(format!("{label} stämmer inte med sitt signerade beslut"));
                continue;
            }
            let Some(signature) = event.signature.as_deref() else {
                c.problems.push(format!("{label} är inte signerat"));
                continue;
            };
            match self.signing_key(signature)? {
                Some(key) => {
                    if let Err(e) = verify(&key.public_key, NS_SYSTEM, hash.as_bytes(), signature) {
                        c.problems
                            .push(format!("{label}: signaturen är inte giltig: {e}"));
                    } else if let Err(e) =
                        required.check(key.hardware, sk_flags(signature).ok().flatten())
                    {
                        c.problems.push(format!("{label}: {e}"));
                    }
                }
                None => c
                    .problems
                    .push(format!("{label}: signerat med en okänd nyckel")),
            }
        }
        // Varje annan signatur prövas mot nivån som gällde vid sin händelse.
        let signed: Vec<(i64, String, String)> = self
            .conn()
            .prepare(
                "SELECT seq, handling, signatur FROM handelser WHERE signatur IS NOT NULL ORDER BY seq",
            )?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (seq, action, signature) in signed {
            if action == SECURITY_LEVEL_DECIDED {
                continue;
            }
            c.checked += 1;
            let level = decisions
                .iter()
                .take_while(|d| d.event < seq)
                .last()
                .map_or(SecurityLevel::Software, |d| d.level);
            if level == SecurityLevel::Software {
                continue;
            }
            // En okänd nyckel rapporteras av kontrollen för respektive post.
            let Some(key) = self.signing_key(&signature)? else {
                continue;
            };
            if let Err(e) = level.check(key.hardware, sk_flags(&signature).ok().flatten()) {
                c.problems.push(format!("händelse {seq} ({action}): {e}"));
            }
        }
        if previous.unwrap_or(SecurityLevel::Software) == SecurityLevel::Software {
            let since = match decisions.last() {
                Some(d) => format!("sedan {}", display_timestamp(&d.decided)),
                None => "eftersom bokföringen skapades före nivåerna".to_string(),
            };
            c.warnings.push(format!(
                "säkerhetsnivån är mjukvarunyckel {since}: en signatur visar vilken nyckel som \
                 godkände, men inte att någon rörde en hårdvarunyckel"
            ));
        }
        Ok(c)
    }

    /// Den registrerade nyckeln som gjorde signaturen, om den finns.
    fn signing_key(&self, signature: &str) -> Result<Option<crate::KeyRow>> {
        match signer_fingerprint(signature) {
            Ok(fingerprint) => self.key_by_fingerprint(&fingerprint),
            Err(_) => Ok(None),
        }
    }

    /// händelsen stämmer med den lagrade regeln.
    fn verify_rules(&self) -> Result<Check> {
        let mut c = check("Reglerna ändras bara genom signerade systemändringar");
        let rows: Vec<(i64, String, i64, String, Option<String>, String)> = self
            .conn()
            .prepare(
                "SELECT andring, namn, version, giltig_fran, giltig_till, innehall FROM regler ORDER BY andring",
            )?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (seq, name, version, from, to, content) in rows {
            c.checked += 1;
            let Some(event) = self.event_sig(seq)? else {
                c.problems.push(format!(
                    "regeln {name} v{version}: ändringen saknas i historiken"
                ));
                continue;
            };
            if event.action != "regel.andrad" {
                c.problems.push(format!(
                    "regeln {name} v{version}: händelse {seq} är ingen regeländring"
                ));
                continue;
            }
            let to = match to.as_deref().map(parse_date).transpose() {
                Ok(t) => t,
                Err(e) => {
                    c.problems
                        .push(format!("regeln {name} v{version}: ogiltigt slutdatum: {e}"));
                    continue;
                }
            };
            let hash =
                crate::ops::rules::rule_hash(&name, version, parse_date(&from)?, to, &content);
            let stored = event
                .new
                .as_ref()
                .and_then(|v| v["beslutshash"].as_str().map(str::to_string));
            if stored.as_deref() != Some(hash.as_str()) {
                c.problems.push(format!(
                    "regeln {name} v{version} stämmer inte med sin signerade ändring"
                ));
                continue;
            }
            let fingerprint = event
                .new
                .as_ref()
                .and_then(|v| v["nyckel"].as_str().map(str::to_string));
            let (Some(fingerprint), Some(signature)) = (fingerprint, event.signature) else {
                c.problems
                    .push(format!("regeln {name} v{version} är inte signerad"));
                continue;
            };
            match self.key_by_fingerprint(&fingerprint)? {
                Some(key) => {
                    if let Err(e) = verify(&key.public_key, NS_SYSTEM, hash.as_bytes(), &signature)
                    {
                        c.problems.push(format!(
                            "regeln {name} v{version}: signaturen är inte giltig: {e}"
                        ));
                    }
                }
                None => c.problems.push(format!(
                    "regeln {name} v{version}: nyckeln {fingerprint} är inte registrerad"
                )),
            }
        }
        Ok(c)
    }

    /// Den första nyckeln signerade sin egen registrering; varje senare
    /// nyckel registrerades med en signatur från en tidigare nyckel.
    fn verify_keys(&self) -> Result<Check> {
        let mut c = check("Nycklarna är registrerade med signatur");
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {KEY_COLUMNS}, handelse FROM nycklar ORDER BY id"
        ))?;
        let keys: Vec<(crate::KeyRow, i64)> = stmt
            .query_map([], |r| Ok((key_from_row(r)?, r.get(7)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (key, seq) in &keys {
            c.checked += 1;
            let Some(event) = self.event_sig(*seq)? else {
                c.problems.push(format!(
                    "nyckel {}: registreringen saknas i historiken",
                    key.name
                ));
                continue;
            };
            let info = match event.action.as_str() {
                "databas.skapad" => event.new.as_ref().map(|v| v["nyckel"].clone()),
                "nyckel.registrerad" => event.new.clone(),
                _ => None,
            };
            let stored = info
                .as_ref()
                .and_then(|v| v["beslutshash"].as_str().map(str::to_string));
            let signer_fp = info
                .as_ref()
                .and_then(|v| v["signerad_av"].as_str().map(str::to_string));
            let hash = key_registration_hash(
                &key.name,
                &key.public_key,
                &key.fingerprint,
                &key.registered_at,
            );
            if stored.as_deref() != Some(hash.as_str()) {
                c.problems.push(format!(
                    "nyckel {}: registreringen stämmer inte med nyckeln",
                    key.name
                ));
                continue;
            }
            let (Some(signer_fp), Some(signature)) = (signer_fp, event.signature) else {
                c.problems.push(format!(
                    "nyckel {}: registreringen är inte signerad",
                    key.name
                ));
                continue;
            };
            let signer = keys
                .iter()
                .map(|(k, _)| k)
                .find(|k| k.fingerprint == signer_fp);
            match signer {
                Some(s) if s.id < key.id || (s.id == key.id && key.id == keys[0].0.id) => {
                    if let Err(e) = verify(&s.public_key, NS_KEY, hash.as_bytes(), &signature) {
                        c.problems.push(format!(
                            "nyckel {}: signaturen på registreringen är inte giltig: {e}",
                            key.name
                        ));
                    }
                }
                _ => c.problems.push(format!(
                    "nyckel {}: registrerades inte av en tidigare registrerad nyckel",
                    key.name
                )),
            }
        }
        Ok(c)
    }
}
