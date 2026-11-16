//! Försäljning och utrangering av ett inventarium: avgången registreras och
//! verifikationen läggs som förslag, daterad avgångsdagen.

use bok_core::Money;
use bok_inventarier::disposal::disposal_voucher;
use bok_inventarier::{Disposal, DisposalAccounts, DisposalVoucher};
use bok_store::{Actor, Provenance, Store};

use crate::proposal::{Proposed, propose};
use crate::{ClosingError, Result};

/// Lägger verifikationen för avgången som förslag och registrerar den i
/// inventarieregistret. En felregistrering registreras utan förslag.
/// `vat` är den utgående momsen enligt fakturan.
pub fn dispose(
    store: &mut Store,
    actor: &Actor,
    asset_id: i64,
    disposal: &Disposal,
    vat: Money,
    accounts: &DisposalAccounts,
    provenance: Provenance,
) -> Result<Option<(Proposed, DisposalVoucher)>> {
    let asset = store.asset(asset_id)?;
    if asset.disposal.is_some() {
        return Err(ClosingError::Invalid(format!(
            "inventarie {asset_id} har redan lämnat registret"
        )));
    }
    let year = store.fiscal_year_for_date(disposal.date)?.ok_or_else(|| {
        ClosingError::Invalid(format!("inget räkenskapsår omfattar {}", disposal.date))
    })?;
    let voucher = disposal_voucher(&asset.details, disposal, year.start, vat, accounts)?;
    let proposed = match voucher {
        None => None,
        Some(v) => {
            let what = match disposal.kind {
                bok_inventarier::DisposalKind::Sold { .. } => "Försäljning",
                _ => "Utrangering",
            };
            let id = propose(
                store,
                actor,
                &format!("avgang-{asset_id}"),
                disposal.date,
                format!(
                    "{what} av inventarie {asset_id}: {}",
                    asset.details.description
                ),
                v.lines.clone(),
                provenance,
            )?;
            let notes = vec![format!(
                "Tidigare års avskrivningar antas bokförda enligt plan; bokfört värde vid avgången {}.",
                v.book_value
            )];
            Some((Proposed { id, notes }, v))
        }
    };
    store.dispose_asset(actor, asset_id, disposal)?;
    Ok(proposed)
}
