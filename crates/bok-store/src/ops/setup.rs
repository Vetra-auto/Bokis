//! Grunduppgifter: företaget, nycklar, räkenskapsår, kontoplan, motparter
//! och mottagna underlag.

use std::collections::BTreeSet;

use chrono::NaiveDate;
use rusqlite::{OptionalExtension, params};
use serde_json::json;

use bok_core::decision::{ChartEntry, chart_change_hash, key_registration_hash};
use bok_core::signature::{NS_KEY, NS_SYSTEM, Signer, parse_public_key, verify};
use bok_core::voucher::{Counterparty, validate_counterparty};
use bok_core::{Series, fiscal_year};

use super::{counterparty_by_id, require_human, verify_registered};
use crate::model::*;
use crate::write::{Actor, EventDraft};
use crate::{NewBook, Result, Store, StoreError, parse_timestamp};

fn clean(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

impl Store {
    /// Första transaktionen i en ny databas: företaget och den första
    /// nyckeln, som signerar sin egen registrering.
    pub(crate) fn initialize(&mut self, book: NewBook<'_>) -> Result<()> {
        let key = parse_public_key(book.public_key)?;
        let name = book.key_name.trim();
        if name.is_empty()
            || book.company_name.trim().is_empty()
            || book.org_number.trim().is_empty()
        {
            return Err(StoreError::Invalid(
                "företagsnamn, organisationsnummer och nyckelns ägare måste anges".into(),
            ));
        }
        let level = book.security_level;
        if !level.admits_key(key.hardware) {
            return Err(StoreError::Invalid(format!(
                "säkerhetsnivån {} kräver en hårdvarunyckel (ed25519-sk), men nyckeln är en mjukvarunyckel",
                level.title()
            )));
        }
        let actor = Actor::human(name);
        self.write(&actor, |ctx| {
            ctx.tx.execute(
                "INSERT INTO foretag (namn, orgnr, registrerad) VALUES (?1, ?2, ?3)",
                params![book.company_name.trim(), book.org_number.trim(), ctx.now_str],
            )?;
            let hash = key_registration_hash(name, &key.openssh, &key.fingerprint, &ctx.now_str);
            let sig = book.signer.sign(NS_KEY, hash.as_bytes())?;
            verify(&key.openssh, NS_KEY, hash.as_bytes(), &sig)?;
            ctx.tx.execute(
                "INSERT INTO nycklar (namn, publik_nyckel, fingeravtryck, algoritm, hardvara, registrerad, handelse)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![name, key.openssh, key.fingerprint, key.algorithm, key.hardware as i64, ctx.now_str, ctx.event_seq],
            )?;
            let key_id = ctx.tx.last_insert_rowid();
            let event = EventDraft::new("databas.skapad", format!("nyckel:{key_id}"))
                .new_values(json!({
                    "schemaversion": crate::SCHEMA_VERSION,
                    "foretag": book.company_name.trim(),
                    "orgnr": book.org_number.trim(),
                    "nyckel": {
                        "namn": name,
                        "fingeravtryck": key.fingerprint,
                        "algoritm": key.algorithm,
                        "hardvara": key.hardware,
                        "beslutshash": hash,
                        "signerad_av": key.fingerprint,
                    },
                }))
                .signed(sig);
            Ok(((), event))
        })?;
        // Nivån är bokföringens första systemändring, signerad av samma nyckel.
        self.decide_security_level(&actor, level, None, book.signer)?;
        Ok(())
    }

    /// Registrerar en ny nyckel. En redan registrerad nyckel måste signera.
    pub fn add_key(
        &mut self,
        actor: &Actor,
        name: &str,
        public_key: &str,
        signer: &dyn Signer,
    ) -> Result<KeyRow> {
        require_human(actor, "registrera nycklar")?;
        let key = parse_public_key(public_key)?;
        let name = name.trim();
        if name.is_empty() {
            return Err(StoreError::Invalid("nyckeln måste ha en ägare".into()));
        }
        let level = self.security_level()?;
        if !level.admits_key(key.hardware) {
            return Err(StoreError::Invalid(format!(
                "säkerhetsnivån är {}, så en mjukvarunyckel kan inte signera och registreras inte; \
                 sänk nivån först med bok sakerhet satt mjukvara",
                level.title()
            )));
        }
        let id = self.write(actor, |ctx| {
            let exists: bool = ctx.tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM nycklar WHERE fingeravtryck = ?1)",
                params![key.fingerprint],
                |r| r.get(0),
            )?;
            if exists {
                return Err(StoreError::Invalid(format!("nyckeln {} är redan registrerad", key.fingerprint)));
            }
            let hash = key_registration_hash(name, &key.openssh, &key.fingerprint, &ctx.now_str);
            let sig = signer.sign(NS_KEY, hash.as_bytes())?;
            let authorizer = verify_registered(ctx.tx, NS_KEY, hash.as_bytes(), &sig)?;
            ctx.tx.execute(
                "INSERT INTO nycklar (namn, publik_nyckel, fingeravtryck, algoritm, hardvara, registrerad, handelse)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![name, key.openssh, key.fingerprint, key.algorithm, key.hardware as i64, ctx.now_str, ctx.event_seq],
            )?;
            let id = ctx.tx.last_insert_rowid();
            let event = EventDraft::new("nyckel.registrerad", format!("nyckel:{id}"))
                .new_values(json!({
                    "namn": name,
                    "fingeravtryck": key.fingerprint,
                    "algoritm": key.algorithm,
                    "hardvara": key.hardware,
                    "beslutshash": hash,
                    "signerad_av": authorizer.fingerprint,
                }))
                .signed(sig);
            Ok((id, event))
        })?;
        super::key_by_id(self.conn(), id)
    }

    /// Lägger till ett räkenskapsår med en öppen period per månad och
    /// nummerserierna A, B, I och R, som alla börjar på 1.
    pub fn add_fiscal_year(
        &mut self,
        actor: &Actor,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<FiscalYear> {
        require_human(actor, "lägga till räkenskapsår")?;
        fiscal_year::validate(start, end)?;
        self.write(actor, |ctx| {
            ctx.tx.execute(
                "INSERT INTO rakenskapsar (start, slut, registrerad) VALUES (?1, ?2, ?3)",
                params![start.to_string(), end.to_string(), ctx.now_str],
            )?;
            let id = ctx.tx.last_insert_rowid();
            let periods = fiscal_year::monthly_periods(start, end);
            for (ps, pe) in &periods {
                ctx.tx.execute(
                    "INSERT INTO perioder (rakenskapsar_id, start, slut) VALUES (?1, ?2, ?3)",
                    params![id, ps.to_string(), pe.to_string()],
                )?;
            }
            for series in Series::ALL {
                ctx.tx.execute(
                    "INSERT INTO nummerserier (rakenskapsar_id, serie, nasta) VALUES (?1, ?2, 1)",
                    params![id, series.as_str()],
                )?;
            }
            let event = EventDraft::new("rakenskapsar.skapat", format!("rakenskapsar:{id}"))
                .new_values(json!({
                    "start": start.to_string(),
                    "slut": end.to_string(),
                    "perioder": periods.len(),
                    "serier": ["A", "B", "I", "R"],
                }));
            Ok((FiscalYear { id, start, end }, event))
        })
    }

    /// Ändrar kontoplanen från och med `valid_from`. Ändringen är en
    /// signerad systemändring: en registrerad nyckel signerar hela
    /// ändringen på en gång.
    pub fn change_chart(
        &mut self,
        actor: &Actor,
        valid_from: NaiveDate,
        changes: &[AccountChange],
        signer: &dyn Signer,
    ) -> Result<usize> {
        require_human(actor, "ändra kontoplanen")?;
        if changes.is_empty() {
            return Err(StoreError::Invalid(
                "ändringen innehåller inga konton".into(),
            ));
        }
        let mut seen = BTreeSet::new();
        for c in changes {
            if !seen.insert(c.number) {
                return Err(StoreError::Invalid(format!(
                    "konto {} förekommer två gånger",
                    c.number
                )));
            }
            if c.name.trim().is_empty() {
                return Err(StoreError::Invalid(format!(
                    "konto {} saknar namn",
                    c.number
                )));
            }
        }
        let cleaned: Vec<AccountChange> = changes
            .iter()
            .map(|c| AccountChange {
                number: c.number,
                name: c.name.trim().to_string(),
                active: c.active,
                sru: clean(c.sru.as_deref()),
                vat_code: clean(c.vat_code.as_deref()),
            })
            .collect();
        self.write(actor, |ctx| {
            for c in &cleaned {
                let exists: bool = ctx.tx.query_row(
                    "SELECT EXISTS (SELECT 1 FROM kontoplan WHERE konto = ?1 AND giltig_fran = ?2)",
                    params![i64::from(c.number), valid_from.to_string()],
                    |r| r.get(0),
                )?;
                if exists {
                    return Err(StoreError::Invalid(format!(
                        "konto {} har redan en version som gäller från {valid_from}",
                        c.number
                    )));
                }
            }
            let entries: Vec<ChartEntry<'_>> = cleaned
                .iter()
                .map(|c| ChartEntry {
                    number: c.number,
                    name: &c.name,
                    active: c.active,
                    sru: c.sru.as_deref(),
                    vat_code: c.vat_code.as_deref(),
                })
                .collect();
            let hash = chart_change_hash(valid_from, &entries);
            let sig = signer.sign(NS_SYSTEM, hash.as_bytes())?;
            let key = verify_registered(ctx.tx, NS_SYSTEM, hash.as_bytes(), &sig)?;
            for c in &cleaned {
                ctx.tx.execute(
                    "INSERT INTO kontoplan (konto, namn, aktiv, sru, momskod, giltig_fran, andring)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        i64::from(c.number),
                        c.name,
                        c.active as i64,
                        c.sru,
                        c.vat_code,
                        valid_from.to_string(),
                        ctx.event_seq
                    ],
                )?;
            }
            let accounts: Vec<_> = cleaned
                .iter()
                .map(|c| json!({"konto": c.number.value(), "namn": c.name, "aktiv": c.active}))
                .collect();
            let event = EventDraft::new("kontoplan.andrad", format!("kontoplan:{valid_from}"))
                .new_values(json!({
                    "giltig_fran": valid_from.to_string(),
                    "antal": cleaned.len(),
                    "beslutshash": hash,
                    "nyckel": key.fingerprint,
                    "konton": accounts,
                }))
                .signed(sig);
            Ok((cleaned.len(), event))
        })
    }

    pub fn add_counterparty(&mut self, actor: &Actor, counterparty: &Counterparty) -> Result<i64> {
        require_human(actor, "ändra motpartsregistret")?;
        validate_counterparty(counterparty)?;
        let c = Counterparty {
            name: counterparty.name.trim().to_string(),
            org_number: clean(counterparty.org_number.as_deref()),
            vat_number: clean(counterparty.vat_number.as_deref()),
        };
        self.write(actor, |ctx| {
            ctx.tx.execute(
                "INSERT INTO motparter (namn, orgnr, momsnr, registrerad) VALUES (?1, ?2, ?3, ?4)",
                params![c.name, c.org_number, c.vat_number, ctx.now_str],
            )?;
            let id = ctx.tx.last_insert_rowid();
            let event = EventDraft::new("motpart.skapad", format!("motpart:{id}"))
                .new_values(serde_json::to_value(&c)?);
            Ok((id, event))
        })
    }

    /// Ändrar en motpart i registret. Redan bokförda verifikationer behåller
    /// kopian från bokföringstillfället.
    pub fn update_counterparty(
        &mut self,
        actor: &Actor,
        id: i64,
        counterparty: &Counterparty,
    ) -> Result<()> {
        require_human(actor, "ändra motpartsregistret")?;
        validate_counterparty(counterparty)?;
        let c = Counterparty {
            name: counterparty.name.trim().to_string(),
            org_number: clean(counterparty.org_number.as_deref()),
            vat_number: clean(counterparty.vat_number.as_deref()),
        };
        self.write(actor, |ctx| {
            let old = counterparty_by_id(ctx.tx, id)?;
            ctx.tx.execute(
                "UPDATE motparter SET namn = ?1, orgnr = ?2, momsnr = ?3, andrad = ?4 WHERE id = ?5",
                params![c.name, c.org_number, c.vat_number, ctx.now_str, id],
            )?;
            let event = EventDraft::new("motpart.andrad", format!("motpart:{id}"))
                .old(serde_json::to_value(&old)?)
                .new_values(serde_json::to_value(&c)?);
            Ok(((), event))
        })
    }

    /// Loggar en export i behandlingshistoriken, med filens SHA-256.
    pub fn record_export(
        &mut self,
        actor: &Actor,
        format: &str,
        fiscal_year_id: i64,
        file_name: &str,
        sha256: &str,
    ) -> Result<()> {
        self.write(actor, |_ctx| {
            let event = EventDraft::new("export.skapad", format!("rakenskapsar:{fiscal_year_id}"))
                .new_values(json!({ "format": format, "fil": file_name, "sha256": sha256 }));
            Ok(((), event))
        })
    }

    /// Loggar en ändring av fjärråtkomsten: en agentanslutning som godkänns
    /// eller återkallas, eller en uppladdningslänk som skapas eller
    /// återkallas. Nycklarna själva finns inte i bokföringen, bara att de
    /// finns, vem som gav dem och när.
    pub fn record_remote_access(
        &mut self,
        actor: &Actor,
        action: &str,
        name: &str,
        values: serde_json::Value,
    ) -> Result<()> {
        // Händelsens namn ur listan, så att bara kända händelser kan skrivas.
        let action: &'static str = REMOTE_ACCESS_ACTIONS
            .iter()
            .copied()
            .find(|a| *a == action)
            .ok_or_else(|| {
                StoreError::Invalid(format!("okänd händelse för fjärråtkomst: {action}"))
            })?;
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 100 {
            return Err(StoreError::Invalid(
                "anslutningen måste ha ett namn på högst 100 tecken".into(),
            ));
        }
        self.write(actor, |_ctx| {
            let event = EventDraft::new(action, format!("fjarr:{name}")).new_values(values);
            Ok(((), event))
        })
    }

    /// Godtar att klockan står före den senaste händelsen. Det är rätt bara
    /// när klockan nu går rätt och det är den senaste händelsen som fick fel
    /// tid, för att klockan gick före när den skrevs. Bara en människa kan
    /// godta det, med en förklaring. Händelsen sparar båda tiderna, och
    /// skrivningar därefter jämförs med den. Ger den senaste händelsens tid
    /// och den nya händelsens, som de lagras.
    pub fn accept_clock(&mut self, actor: &Actor, reason: &str) -> Result<(String, String)> {
        require_human(actor, "godta klockan")?;
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(StoreError::Invalid(
                "skriv varför klockan står före den senaste händelsen".into(),
            ));
        }
        let last: String = self
            .conn()
            .query_row(
                "SELECT tid FROM handelser ORDER BY seq DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::Invalid("historiken är tom".into()))?;
        if self.now() >= parse_timestamp(&last)? {
            return Err(StoreError::Invalid(
                "klockan står inte före den senaste händelsen; det finns inget att godta".into(),
            ));
        }
        self.write_despite_clock(actor, |ctx| {
            let event = EventDraft::new(CLOCK_ACCEPTED, "historik").new_values(json!({
                "senaste_handelse": last,
                "klockan": ctx.now_str,
                "forklaring": reason,
            }));
            Ok(((last.clone(), ctx.now_str.clone()), event))
        })
    }

    /// Registrerar ett mottaget underlag. Anroparen har redan sparat filen
    /// oförändrad i arkivet under dess SHA-256.
    pub fn register_attachment(&mut self, actor: &Actor, a: &NewAttachment) -> Result<i64> {
        let sha = a.sha256.to_ascii_lowercase();
        if sha.len() != 64 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(StoreError::Invalid(
                "underlagets SHA-256 måste vara 64 hexadecimala tecken".into(),
            ));
        }
        if let Some(existing) = self
            .conn()
            .query_row(
                "SELECT id FROM underlag WHERE sha256 = ?1",
                params![sha],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
        {
            return Err(StoreError::DuplicateAttachment(existing));
        }
        self.write(actor, |ctx| {
            ctx.tx.execute(
                "INSERT INTO underlag (sha256, storlek, mottagen, kanal, avsandare, originalnamn, filtyp)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    sha,
                    a.size,
                    ctx.now_str,
                    a.channel.as_db_str(),
                    clean(a.sender.as_deref()),
                    clean(a.original_name.as_deref()),
                    clean(a.file_type.as_deref())
                ],
            )?;
            let id = ctx.tx.last_insert_rowid();
            ctx.tx.execute(
                "INSERT INTO underlag_status (underlag_id, status, text, tid, handelse)
                 VALUES (?1, 'mottagen', NULL, ?2, ?3)",
                params![id, ctx.now_str, ctx.event_seq],
            )?;
            let event = EventDraft::new("underlag.mottaget", format!("underlag:{id}")).new_values(json!({
                "sha256": sha,
                "storlek": a.size,
                "kanal": a.channel.as_db_str(),
                "originalnamn": a.original_name,
            }));
            Ok((id, event))
        })
    }
}
