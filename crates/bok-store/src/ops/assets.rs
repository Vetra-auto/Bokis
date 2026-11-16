//! Inventarieregistret (schemaversion 4): registrering, avgång och uppslag.
//!
//! Bara en människa registrerar och avregistrerar inventarier, och varje
//! skrivning blir en händelse i behandlingshistoriken. Avskrivningarna
//! räknas av `bok-inventarier` och bokförs som vanliga förslag; registret
//! självt innehåller inga belopp som ändras över tiden.

use rusqlite::{Row, params};
use serde_json::json;

use bok_core::{AccountNumber, Money};
use bok_inventarier::{Asset, AssetAccounts, Disposal, DisposalKind, NewAsset};

use super::{account_at, require_human};
use crate::write::{Actor, EventDraft};
use crate::{Result, Store, StoreError, parse_date};

/// Händelsen när ett inventarium registreras.
pub const ASSET_REGISTERED: &str = "inventarie.registrerad";
/// Händelsen när ett inventarium lämnar registret.
pub const ASSET_DISPOSED: &str = "inventarie.avgang";

const ASSET_COLUMNS: &str = "i.id, i.beskrivning, i.anskaffad, i.anskaffningsvarde, i.restvarde, \
    i.nyttjandeperiod, i.avskrivning_fran, i.tillgangskonto, i.avskrivningskonto, i.kostnadskonto, \
    v.serie, v.nummer, r.start, r.slut, a.datum, a.typ, a.pris, a.forklaring";

const ASSET_FROM: &str = "FROM inventarier i \
    LEFT JOIN verifikationer v ON v.id = i.verifikation_id \
    LEFT JOIN rakenskapsar r ON r.id = v.rakenskapsar_id \
    LEFT JOIN inventarieavgangar a ON a.inventarie_id = i.id";

fn account(r: &Row<'_>, i: usize) -> rusqlite::Result<AccountNumber> {
    AccountNumber::new(r.get(i)?).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(i, rusqlite::types::Type::Integer, Box::new(e))
    })
}

fn asset_from_row(r: &Row<'_>) -> rusqlite::Result<Asset> {
    let voucher = match (
        r.get::<_, Option<String>>(10)?,
        r.get::<_, Option<i64>>(11)?,
    ) {
        (Some(series), Some(number)) => {
            let start = parse_date(&r.get::<_, String>(12)?)?;
            let end = parse_date(&r.get::<_, String>(13)?)?;
            let year = crate::FiscalYear { id: 0, start, end };
            Some(format!("{series}{number} ({})", year.label()))
        }
        _ => None,
    };
    let disposal = match r.get::<_, Option<String>>(14)? {
        None => None,
        Some(date) => {
            let kind = match r.get::<_, String>(15)?.as_str() {
                "forsaljning" => DisposalKind::Sold {
                    price: Money::from_ore(r.get(16)?),
                },
                "utrangering" => DisposalKind::Scrapped,
                _ => DisposalKind::Void,
            };
            Some(Disposal {
                date: parse_date(&date)?,
                kind,
                note: r.get(17)?,
            })
        }
    };
    Ok(Asset {
        id: r.get(0)?,
        details: NewAsset {
            description: r.get(1)?,
            acquired: parse_date(&r.get::<_, String>(2)?)?,
            cost: Money::from_ore(r.get(3)?),
            residual: Money::from_ore(r.get(4)?),
            life_months: r.get(5)?,
            depreciation_start: parse_date(&r.get::<_, String>(6)?)?,
            accounts: AssetAccounts {
                asset: account(r, 7)?,
                accumulated: account(r, 8)?,
                expense: account(r, 9)?,
            },
        },
        voucher,
        disposal,
    })
}

impl Store {
    /// Registrerar ett inventarium. Kontona måste finnas i kontoplanen vid
    /// anskaffningen. `voucher_id` är verifikationen som anskaffningen
    /// bokfördes på, om den finns i Bokis. Ger numret i registret.
    pub fn register_asset(
        &mut self,
        actor: &Actor,
        asset: &NewAsset,
        voucher_id: Option<i64>,
    ) -> Result<i64> {
        require_human(actor, "registrera inventarier")?;
        asset
            .validate()
            .map_err(|e| StoreError::Invalid(e.to_string()))?;
        let a = asset.accounts;
        for number in [a.asset, a.accumulated, a.expense] {
            if account_at(self.conn(), number, asset.acquired)?.is_none() {
                return Err(StoreError::UnknownAccount(number, asset.acquired));
            }
        }
        if let Some(id) = voucher_id {
            let exists: bool = self.conn().query_row(
                "SELECT EXISTS (SELECT 1 FROM verifikationer WHERE id = ?1)",
                params![id],
                |r| r.get(0),
            )?;
            if !exists {
                return Err(StoreError::missing("verifikation", id));
            }
        }
        self.write(actor, |ctx| {
            ctx.tx.execute(
                "INSERT INTO inventarier (beskrivning, anskaffad, anskaffningsvarde, restvarde,
                     nyttjandeperiod, avskrivning_fran, tillgangskonto, avskrivningskonto,
                     kostnadskonto, verifikation_id, aktor, registrerad, handelse)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    asset.description.trim(),
                    asset.acquired.to_string(),
                    asset.cost.ore(),
                    asset.residual.ore(),
                    asset.life_months,
                    asset.depreciation_start.to_string(),
                    i64::from(a.asset),
                    i64::from(a.accumulated),
                    i64::from(a.expense),
                    voucher_id,
                    ctx.actor.name,
                    ctx.now_str,
                    ctx.event_seq,
                ],
            )?;
            let id = ctx.tx.last_insert_rowid();
            let event =
                EventDraft::new(ASSET_REGISTERED, format!("inventarie:{id}")).new_values(json!({
                    "beskrivning": asset.description.trim(),
                    "anskaffad": asset.acquired.to_string(),
                    "anskaffningsvarde_ore": asset.cost.ore(),
                    "restvarde_ore": asset.residual.ore(),
                    "nyttjandeperiod_manader": asset.life_months,
                    "avskrivning_fran": asset.depreciation_start.to_string(),
                    "konton": [i64::from(a.asset), i64::from(a.accumulated), i64::from(a.expense)],
                    "verifikation": voucher_id,
                }));
            Ok((id, event))
        })
    }

    /// Registrerar att ett inventarium har sålts, utrangerats eller var
    /// felregistrerat. Ett inventarium kan bara lämna registret en gång.
    pub fn dispose_asset(
        &mut self,
        actor: &Actor,
        asset_id: i64,
        disposal: &Disposal,
    ) -> Result<()> {
        require_human(actor, "avregistrera inventarier")?;
        let asset = self.asset(asset_id)?;
        if asset.disposal.is_some() {
            return Err(StoreError::Invalid(format!(
                "inventarie {asset_id} har redan lämnat registret"
            )));
        }
        if disposal.date < asset.details.acquired {
            return Err(StoreError::Invalid(format!(
                "avgången {} är före anskaffningen {}",
                disposal.date, asset.details.acquired
            )));
        }
        let note = disposal
            .note
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty());
        let price = match disposal.kind {
            DisposalKind::Sold { price } if price.is_negative() => {
                return Err(StoreError::Invalid("priset kan inte vara negativt".into()));
            }
            DisposalKind::Sold { price } => price,
            DisposalKind::Void if note.is_none() => {
                return Err(StoreError::Invalid(
                    "en felregistrering måste ha en förklaring".into(),
                ));
            }
            _ => Money::ZERO,
        };
        let kind = disposal.kind.as_str();
        self.write(actor, |ctx| {
            ctx.tx.execute(
                "INSERT INTO inventarieavgangar (inventarie_id, datum, typ, pris, forklaring,
                     aktor, registrerad, handelse)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    asset_id,
                    disposal.date.to_string(),
                    kind,
                    price.ore(),
                    note,
                    ctx.actor.name,
                    ctx.now_str,
                    ctx.event_seq,
                ],
            )?;
            let event = EventDraft::new(ASSET_DISPOSED, format!("inventarie:{asset_id}"))
                .new_values(json!({
                    "datum": disposal.date.to_string(),
                    "typ": kind,
                    "pris_ore": price.ore(),
                    "forklaring": note,
                }));
            Ok(((), event))
        })
    }

    /// Hela registret i nummerordning, med avgångar.
    pub fn assets(&self) -> Result<Vec<Asset>> {
        let mut stmt = self.conn().prepare(&format!(
            "SELECT {ASSET_COLUMNS} {ASSET_FROM} ORDER BY i.id"
        ))?;
        let rows = stmt
            .query_map([], asset_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Ett inventarium.
    pub fn asset(&self, id: i64) -> Result<Asset> {
        use rusqlite::OptionalExtension;
        self.conn()
            .query_row(
                &format!("SELECT {ASSET_COLUMNS} {ASSET_FROM} WHERE i.id = ?1"),
                params![id],
                asset_from_row,
            )
            .optional()?
            .ok_or_else(|| StoreError::missing("inventarie", id))
    }
}
