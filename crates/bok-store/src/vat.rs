//! Momsrapporten: bokförda belopp per ruta i momsdeklarationen.
//!
//! Vilken ruta ett konto hör till är data, inte kod: kontoplanens momskod
//! (`kontoplan.momskod`) är rutans nummer, till exempel `10` för utgående
//! moms 25 % eller `48` för ingående moms. Rapporten summerar de bokförda
//! raderna i perioden per momskod, med det tecken rutan har i blanketten.
//! Ruta 49 räknas fram: utgående moms (10–12, 30–32, 60–62) minus ingående
//! moms (48).
//!
//! Konton med momskod 49 (normalt 2650, redovisningskonto för moms) markerar
//! momsavräkningen. Verifikationer med en sådan rad räknas inte med, annars
//! skulle avräkningen nolla rapporten.

use std::collections::{BTreeMap, HashMap};

use chrono::NaiveDate;
use rusqlite::params;

use bok_core::{AccountNumber, Money};

use crate::ops::account_at;
use crate::{Result, Store, StoreError, parse_date};

/// Rutorna i momsdeklarationen och deras rubriker, i blankettens ordning.
/// Rubrikerna följer Skatteverkets blankett SKV 4700 utgåva 22 (efter
/// mervärdesskattelagen 2023:200), kontrollerade 2026-11-15 mot
/// skatteverket.se ("Fylla i momsdeklarationen"). I rutorna 30–32 och 60–62
/// står avsnittets rubrik med, så att rutan går att förstå ensam.
pub const VAT_BOXES: [(&str, &str); 29] = [
    (
        "05",
        "Momspliktig försäljning som inte ingår i ruta 06, 07 eller 08",
    ),
    ("06", "Momspliktiga uttag"),
    ("07", "Beskattningsunderlag vid vinstmarginalbeskattning"),
    ("08", "Hyresinkomster vid frivillig beskattning"),
    ("10", "Utgående moms 25 %"),
    ("11", "Utgående moms 12 %"),
    ("12", "Utgående moms 6 %"),
    ("20", "Inköp av varor från ett annat EU-land"),
    (
        "21",
        "Inköp av tjänster från ett annat EU-land enligt huvudregeln",
    ),
    ("22", "Inköp av tjänster från ett land utanför EU"),
    (
        "23",
        "Inköp av varor i Sverige som köparen är betalningsskyldig för",
    ),
    (
        "24",
        "Övriga inköp av tjänster i Sverige som köparen är betalningsskyldig för",
    ),
    ("30", "Utgående moms 25 % på inköp i ruta 20–24"),
    ("31", "Utgående moms 12 % på inköp i ruta 20–24"),
    ("32", "Utgående moms 6 % på inköp i ruta 20–24"),
    ("35", "Försäljning av varor till ett annat EU-land"),
    ("36", "Försäljning av varor utanför EU"),
    ("37", "Mellanmans inköp av varor vid trepartshandel"),
    ("38", "Mellanmans försäljning av varor vid trepartshandel"),
    (
        "39",
        "Försäljning av tjänster till beskattningsbar person i annat EU-land enligt huvudregeln",
    ),
    (
        "40",
        "Övrig försäljning av tjänster som tillhandahållits utomlands",
    ),
    (
        "41",
        "Försäljning när köparen är betalningsskyldig i Sverige",
    ),
    ("42", "Övrig försäljning m.m."),
    ("48", "Ingående moms att dra av"),
    ("49", "Moms att betala eller få tillbaka"),
    ("50", "Beskattningsunderlag vid import"),
    ("60", "Utgående moms 25 % på import i ruta 50"),
    ("61", "Utgående moms 12 % på import i ruta 50"),
    ("62", "Utgående moms 6 % på import i ruta 50"),
];

/// Rutor som avser inköp och ingående moms, där debet är positivt. I övriga
/// rutor (försäljning och utgående moms) är kredit positivt.
const DEBIT_BOXES: [&str; 8] = ["20", "21", "22", "23", "24", "37", "48", "50"];

const OUTPUT_VAT_BOXES: [&str; 9] = ["10", "11", "12", "30", "31", "32", "60", "61", "62"];

/// Rubriken för en ruta, eller `None` om rutan inte finns i blanketten.
pub fn box_title(code: &str) -> Option<&'static str> {
    VAT_BOXES.iter().find(|(c, _)| *c == code).map(|(_, t)| *t)
}

/// En ruta med belopp och de konton som bidrar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VatBox {
    pub code: String,
    pub title: String,
    pub amount: Money,
    pub accounts: Vec<(AccountNumber, Money)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VatReport {
    pub from: NaiveDate,
    pub to: NaiveDate,
    /// Rutorna med belopp, i blankettens ordning. Ruta 49 ingår.
    pub boxes: Vec<VatBox>,
    /// Ruta 49: positivt är moms att betala, negativt att få tillbaka.
    pub to_pay: Money,
    /// Momskoder i kontoplanen som inte är någon ruta, med kontona.
    pub unknown_codes: Vec<(String, AccountNumber)>,
    /// Verifikationer som inte räknades med eftersom de är momsavräkningar.
    pub settlements: usize,
}

impl Store {
    /// Momsrapporten för bokförda verifikationer daterade `from`–`to`.
    pub fn vat_report(&self, from: NaiveDate, to: NaiveDate) -> Result<VatReport> {
        if from > to {
            return Err(StoreError::Invalid(format!(
                "perioden börjar {from} men slutar {to}"
            )));
        }
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT v.id, v.datum, r.konto, r.debet - r.kredit
             FROM verifikationer v JOIN rader r ON r.verifikation_id = v.id
             WHERE v.datum BETWEEN ?1 AND ?2
             ORDER BY v.id, r.radnr",
        )?;
        let rows: Vec<(i64, String, i64, i64)> = stmt
            .query_map(params![from.to_string(), to.to_string()], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })?
            .collect::<rusqlite::Result<_>>()?;

        // Momskoden gäller per konto och datum, eftersom kontoplanen ändras över tid.
        let mut codes: HashMap<(i64, NaiveDate), Option<String>> = HashMap::new();
        let mut lines = Vec::with_capacity(rows.len());
        for (voucher, date, account, net) in rows {
            let date = parse_date(&date)?;
            let code = match codes.get(&(account, date)) {
                Some(c) => c.clone(),
                None => {
                    let number = AccountNumber::new(account)?;
                    let c = account_at(conn, number, date)?.and_then(|a| a.vat_code);
                    codes.insert((account, date), c.clone());
                    c
                }
            };
            lines.push((voucher, account, net, code));
        }

        let settlement_vouchers: std::collections::BTreeSet<i64> = lines
            .iter()
            .filter(|(_, _, _, code)| code.as_deref() == Some("49"))
            .map(|(v, _, _, _)| *v)
            .collect();

        let mut sums: BTreeMap<String, BTreeMap<i64, i64>> = BTreeMap::new();
        let mut unknown: Vec<(String, AccountNumber)> = Vec::new();
        for (voucher, account, net, code) in &lines {
            let Some(code) = code.as_deref().map(str::trim).filter(|c| !c.is_empty()) else {
                continue;
            };
            if settlement_vouchers.contains(voucher) || code == "49" {
                continue;
            }
            if box_title(code).is_none() {
                let number = AccountNumber::new(*account)?;
                if !unknown.iter().any(|(c, a)| c == code && *a == number) {
                    unknown.push((code.to_string(), number));
                }
                continue;
            }
            let signed = if DEBIT_BOXES.contains(&code) {
                *net
            } else {
                -*net
            };
            *sums
                .entry(code.to_string())
                .or_default()
                .entry(*account)
                .or_default() += signed;
        }

        let mut boxes = Vec::new();
        for (code, title) in VAT_BOXES {
            let Some(accounts) = sums.get(code) else {
                continue;
            };
            let total: i64 = accounts.values().sum();
            boxes.push(VatBox {
                code: code.to_string(),
                title: title.to_string(),
                amount: Money::from_ore(total),
                accounts: accounts
                    .iter()
                    .map(|(a, m)| Ok((AccountNumber::new(*a)?, Money::from_ore(*m))))
                    .collect::<Result<_>>()?,
            });
        }
        let amount_of = |code: &str| -> i64 {
            boxes
                .iter()
                .find(|b| b.code == code)
                .map_or(0, |b| b.amount.ore())
        };
        let output: i64 = OUTPUT_VAT_BOXES.iter().map(|c| amount_of(c)).sum();
        let to_pay = Money::from_ore(output - amount_of("48"));
        let pos = boxes
            .iter()
            .position(|b| b.code.as_str() > "49")
            .unwrap_or(boxes.len());
        boxes.insert(
            pos,
            VatBox {
                code: "49".into(),
                title: box_title("49").unwrap_or_default().into(),
                amount: to_pay,
                accounts: vec![],
            },
        );
        Ok(VatReport {
            from,
            to,
            boxes,
            to_pay,
            unknown_codes: unknown,
            settlements: settlement_vouchers.len(),
        })
    }
}
