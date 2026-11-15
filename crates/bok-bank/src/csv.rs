//! CSV-exporter från banker. Kolumnerna anges med ett [`CsvFormat`],
//! eftersom varje bank har sitt eget upplägg.

use crate::{assign_entry_ids, parse_amount_with_decimal, parse_date, BankError, BankRow};

/// Teckenkodning i CSV-filen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEncoding {
    Utf8,
    /// Windows-1252, som också täcker ISO-8859-1 för svenska tecken.
    Windows1252,
}

/// I vilken ordning banken skriver raderna.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowOrder {
    /// Okänd. Raderna läses i filens ordning, och saldot kontrolleras inte.
    Unknown,
    /// Äldsta raden först.
    OldestFirst,
    /// Senaste raden först. Raderna vänds, så att de kommer i tidsordning.
    NewestFirst,
}

/// Rubrikraden i Lunar Business export av kontoutdrag som CSV.
const LUNAR_HEADER: &str = "Datum,Tid,Titel,Belopp,Balans,Transaktions-ID";

/// Beskriver hur en banks CSV-export är uppställd.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsvFormat {
    /// Namnet som visas när filen läses in.
    pub name: &'static str,
    /// Kontot raderna hör till, till exempel IBAN.
    pub account: String,
    pub currency: String,
    pub delimiter: char,
    pub has_header: bool,
    pub encoding: TextEncoding,
    /// chrono-format, till exempel `%Y-%m-%d`.
    pub date_format: String,
    pub decimal_char: char,
    pub col_booking_date: usize,
    pub col_value_date: Option<usize>,
    pub col_text: usize,
    pub col_amount: usize,
    pub col_balance: Option<usize>,
    pub col_reference: Option<usize>,
    /// Bankens eget, unika id för raden. Då känns samma rad igen i en ny
    /// export som överlappar en tidigare, också om texten har ändrats.
    pub col_transaction_id: Option<usize>,
    pub order: RowOrder,
}

impl CsvFormat {
    /// Ett enkelt svenskt format: `Datum;Text;Belopp;Saldo`, UTF-8,
    /// ISO-datum och decimalkomma. Bankspecifika format bör läggas till
    /// först när de har kontrollerats mot en riktig export.
    pub fn simple(account: impl Into<String>) -> Self {
        Self {
            name: "enkel CSV (datum;text;belopp;saldo)",
            account: account.into(),
            currency: "SEK".into(),
            delimiter: ';',
            has_header: true,
            encoding: TextEncoding::Utf8,
            date_format: "%Y-%m-%d".into(),
            decimal_char: ',',
            col_booking_date: 0,
            col_value_date: None,
            col_text: 1,
            col_amount: 2,
            col_balance: Some(3),
            col_reference: None,
            col_transaction_id: None,
            order: RowOrder::Unknown,
        }
    }

    /// Lunar Business: "Exportera kontoutdrag" under kontots Detaljer, med
    /// filformatet CSV. Rubrikraden är `Datum,Tid,Titel,Belopp,Balans,Transaktions-ID`,
    /// filen är UTF-8 med BOM, fälten skiljs med komma, belopp och saldo står
    /// inom citattecken med decimalkomma, och senaste raden kommer först.
    /// Kontrollerat mot en riktig export 2026-11-15.
    pub fn lunar(account: impl Into<String>) -> Self {
        Self {
            name: "Lunar Business",
            account: account.into(),
            currency: "SEK".into(),
            delimiter: ',',
            has_header: true,
            encoding: TextEncoding::Utf8,
            date_format: "%Y-%m-%d".into(),
            decimal_char: ',',
            col_booking_date: 0,
            col_value_date: None,
            col_text: 2,
            col_amount: 3,
            col_balance: Some(4),
            col_reference: None,
            col_transaction_id: Some(5),
            order: RowOrder::NewestFirst,
        }
    }

    /// Väljer format efter filens rubrikrad. Lunar Business känns igen; allt
    /// annat läses som det enkla formatet.
    pub fn detect(data: &[u8], account: impl Into<String>) -> Self {
        let text = decode(data, TextEncoding::Utf8);
        let header = text.lines().next().unwrap_or("").trim();
        if header == LUNAR_HEADER {
            Self::lunar(account)
        } else {
            Self::simple(account)
        }
    }
}

/// Ingående och utgående saldo ur radernas saldon, när ordningen är känd och
/// varje rad har ett saldo: saldot efter första raden minus dess belopp, och
/// saldot efter sista raden. Raderna ska vara i tidsordning, som
/// [`parse_csv`] lämnar dem.
pub fn balances_from_rows(rows: &[BankRow], order: RowOrder) -> Option<(i64, i64)> {
    if order == RowOrder::Unknown {
        return None;
    }
    let first = rows.first()?;
    let last = rows.last()?;
    if rows.iter().any(|r| r.balance_ore.is_none()) {
        return None;
    }
    Some((first.balance_ore? - first.amount_ore, last.balance_ore?))
}

fn decode(data: &[u8], encoding: TextEncoding) -> String {
    let without_bom = data.strip_prefix(b"\xef\xbb\xbf").unwrap_or(data);
    match encoding {
        TextEncoding::Utf8 => String::from_utf8_lossy(without_bom).into_owned(),
        TextEncoding::Windows1252 => encoding_rs::WINDOWS_1252.decode(without_bom).0.into_owned(),
    }
}

/// Delar en CSV-rad i fält. Hanterar citerade fält med avgränsare och
/// dubblerade citattecken inuti.
fn split_line(line: &str, delimiter: char) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                current.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            c if c == delimiter && !quoted => fields.push(std::mem::take(&mut current)),
            c => current.push(c),
        }
    }
    fields.push(current);
    fields
}

fn field(fields: &[String], col: usize, line: usize) -> Result<&str, BankError> {
    fields
        .get(col)
        .map(|s| s.trim())
        .ok_or_else(|| BankError::Line {
            line,
            error: format!("kolumn {col} saknas"),
        })
}

/// Läser en CSV-export till bankrader, i tidsordning när formatet säger att
/// banken skriver senaste raden först och annars i filens ordning.
pub fn parse_csv(data: &[u8], format: &CsvFormat) -> Result<Vec<BankRow>, BankError> {
    let text = decode(data, format.encoding);
    let mut rows = Vec::new();
    let mut bank_refs = Vec::new();
    for (index, line_text) in text.lines().enumerate() {
        let line = index + 1;
        if (format.has_header && index == 0) || line_text.trim().is_empty() {
            continue;
        }
        let fields = split_line(line_text, format.delimiter);
        let at_line = |e: BankError| BankError::Line {
            line,
            error: e.to_string(),
        };
        let booking_date = parse_date(
            field(&fields, format.col_booking_date, line)?,
            &format.date_format,
        )
        .map_err(at_line)?;
        let value_date = match format.col_value_date {
            Some(c) => {
                let v = field(&fields, c, line)?;
                if v.is_empty() {
                    None
                } else {
                    Some(parse_date(v, &format.date_format).map_err(at_line)?)
                }
            }
            None => None,
        };
        let amount_ore = parse_amount_with_decimal(
            field(&fields, format.col_amount, line)?,
            Some(format.decimal_char),
        )
        .map_err(at_line)?;
        let balance_ore = match format.col_balance {
            Some(c) => {
                let v = field(&fields, c, line)?;
                if v.is_empty() {
                    None
                } else {
                    Some(parse_amount_with_decimal(v, Some(format.decimal_char)).map_err(at_line)?)
                }
            }
            None => None,
        };
        let reference = match format.col_reference {
            Some(c) => Some(field(&fields, c, line)?.to_string()).filter(|s| !s.is_empty()),
            None => None,
        };
        bank_refs.push(match format.col_transaction_id {
            Some(c) => Some(field(&fields, c, line)?.to_string()).filter(|s| !s.is_empty()),
            None => None,
        });
        rows.push(BankRow {
            account: format.account.clone(),
            booking_date,
            value_date,
            amount_ore,
            currency: format.currency.clone(),
            text: field(&fields, format.col_text, line)?.to_string(),
            reference,
            counterparty: None,
            balance_ore,
            entry_id: String::new(),
        });
    }
    if format.order == RowOrder::NewestFirst {
        rows.reverse();
        bank_refs.reverse();
    }
    assign_entry_ids(&mut rows, "csv", &bank_refs);
    Ok(rows)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn simple_csv_with_quotes_and_swedish_amounts() {
        let data = "Datum;Text;Belopp;Saldo\n\
                    2026-11-01;\"Hyra; kontor\";-12 500,00;87 500,00\n\
                    2026-11-02;\"Kund AB \"\"faktura 7\"\"\";1 234,56;88 734,56\n\
                    \n\
                    2026-11-02;Öresavrundning;-0,40;88 734,16\n";
        let rows = parse_csv(
            data.as_bytes(),
            &CsvFormat::simple("SE45 5000 0000 0583 9825 7466"),
        )
        .unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].text, "Hyra; kontor");
        assert_eq!(rows[0].amount_ore, -1_250_000);
        assert_eq!(rows[0].balance_ore, Some(8_750_000));
        assert_eq!(rows[1].text, "Kund AB \"faktura 7\"");
        assert_eq!(rows[1].amount_ore, 123_456);
        assert_eq!(rows[2].amount_ore, -40);
        assert_eq!(rows[2].text, "Öresavrundning");
    }

    #[test]
    fn windows_1252_and_bom() {
        let mut data = b"\xef\xbb\xbfDatum;Text;Belopp;Saldo\n".to_vec();
        // "Löneutbetalning" i Windows-1252 (ö = 0xF6).
        data.extend_from_slice(b"2026-11-25;L\xf6neutbetalning;-30 000,00;\n");
        let mut format = CsvFormat::simple("konto");
        format.encoding = TextEncoding::Windows1252;
        let rows = parse_csv(&data, &format).unwrap();
        assert_eq!(rows[0].text, "Löneutbetalning");
        assert_eq!(rows[0].balance_ore, None);
    }

    #[test]
    fn identical_rows_get_distinct_but_stable_ids() {
        let data =
            "Datum;Text;Belopp;Saldo\n2026-11-03;Kortköp;-50,00;\n2026-11-03;Kortköp;-50,00;\n";
        let format = CsvFormat::simple("konto");
        let a = parse_csv(data.as_bytes(), &format).unwrap();
        let b = parse_csv(data.as_bytes(), &format).unwrap();
        assert_ne!(a[0].entry_id, a[1].entry_id);
        assert_eq!(a[0].entry_id, b[0].entry_id);
        assert_eq!(a[1].entry_id, b[1].entry_id);
    }

    #[test]
    fn errors_report_the_line_number() {
        let data = "Datum;Text;Belopp;Saldo\n2026-11-01;Ok;10,00;\n2026-13-01;Fel datum;10,00;\n";
        let err = parse_csv(data.as_bytes(), &CsvFormat::simple("konto")).unwrap_err();
        assert!(matches!(err, BankError::Line { line: 3, .. }), "{err:?}");
        let data = "Datum;Text;Belopp;Saldo\n2026-11-01;Tre decimaler;10,005;\n";
        assert!(matches!(
            parse_csv(data.as_bytes(), &CsvFormat::simple("konto")),
            Err(BankError::Line { line: 2, .. })
        ));
    }

    #[test]
    fn other_format_with_point_and_value_date() {
        let format = CsvFormat {
            name: "test",
            account: "1234".into(),
            currency: "SEK".into(),
            delimiter: ',',
            has_header: false,
            encoding: TextEncoding::Utf8,
            date_format: "%d.%m.%Y".into(),
            decimal_char: '.',
            col_booking_date: 0,
            col_value_date: Some(1),
            col_text: 2,
            col_amount: 3,
            col_balance: None,
            col_reference: Some(4),
            col_transaction_id: None,
            order: RowOrder::Unknown,
        };
        let data = "02.11.2026,03.11.2026,\"Bankgiro, Telia\",-150.50,OCR123\n";
        let rows = parse_csv(data.as_bytes(), &format).unwrap();
        assert_eq!(rows[0].amount_ore, -15_050);
        assert_eq!(rows[0].text, "Bankgiro, Telia");
        assert_eq!(rows[0].reference.as_deref(), Some("OCR123"));
        assert_eq!(
            rows[0].value_date.map(|d| d.to_string()).as_deref(),
            Some("2026-11-03")
        );
    }

    /// Påhittade rader i samma form som Lunar Business export: senaste först,
    /// belopp och saldo inom citattecken, ett id per rad.
    const LUNAR: &str = "\u{feff}Datum,Tid,Titel,Belopp,Balans,Transaktions-ID\n\
        2026-05-20,13:45,Kontorsmaterial AB,\"-249,00\",\"12 751,00\",11111111-2222-4333-8444-555555555555-aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee\n\
        2026-05-02,09:10,\"Kund AB, faktura 12\",\"12 500,00\",\"13 000,00\",11111111-2222-4333-8444-555555555555-ffffffff-0000-4111-8222-333333333333\n\
        2026-04-27,16:02,Bankavgift,\"-0,50\",\"500,00\",11111111-2222-4333-8444-555555555555-44444444-5555-4666-8777-888888888888\n";

    #[test]
    fn lunar_business_is_recognised_and_read_oldest_first() {
        let format = CsvFormat::detect(LUNAR.as_bytes(), "97100000001");
        assert_eq!(format.name, "Lunar Business");
        let rows = parse_csv(LUNAR.as_bytes(), &format).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].booking_date.to_string(), "2026-04-27");
        assert_eq!(rows[0].text, "Bankavgift");
        assert_eq!(rows[0].amount_ore, -50);
        assert_eq!(rows[0].balance_ore, Some(50_000));
        assert_eq!(rows[1].text, "Kund AB, faktura 12");
        assert_eq!(rows[1].amount_ore, 1_250_000);
        assert_eq!(rows[2].amount_ore, -24_900);
        assert_eq!(rows[2].balance_ore, Some(1_275_100));
        assert!(rows
            .iter()
            .all(|r| r.account == "97100000001" && r.currency == "SEK"));
        // Ingående saldo är saldot efter första raden minus dess belopp.
        assert_eq!(
            balances_from_rows(&rows, format.order),
            Some((50_050, 1_275_100))
        );
    }

    #[test]
    fn a_lunar_row_keeps_its_id_when_the_text_changes() {
        let format = CsvFormat::lunar("97100000001");
        let before = parse_csv(LUNAR.as_bytes(), &format).unwrap();
        let renamed = LUNAR.replace("Kontorsmaterial AB", "Kontorsmaterial Sverige AB");
        let after = parse_csv(renamed.as_bytes(), &format).unwrap();
        assert_eq!(before[2].entry_id, after[2].entry_id);
        assert_ne!(before[0].entry_id, before[1].entry_id);
    }

    #[test]
    fn other_files_keep_the_simple_format_and_an_empty_lunar_file_has_no_balances() {
        let simple = "Datum;Text;Belopp;Saldo\n2026-11-01;Hyra;-100,00;900,00\n";
        let format = CsvFormat::detect(simple.as_bytes(), "konto");
        assert_eq!(format, CsvFormat::simple("konto"));
        let rows = parse_csv(simple.as_bytes(), &format).unwrap();
        assert_eq!(balances_from_rows(&rows, format.order), None);
        let empty = "\u{feff}Datum,Tid,Titel,Belopp,Balans,Transaktions-ID\n";
        let format = CsvFormat::detect(empty.as_bytes(), "konto");
        let rows = parse_csv(empty.as_bytes(), &format).unwrap();
        assert!(rows.is_empty());
        assert_eq!(balances_from_rows(&rows, format.order), None);
    }
}
