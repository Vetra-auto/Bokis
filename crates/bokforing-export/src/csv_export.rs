use std::io::Write;

pub struct CsvExporter {
    delimiter: u8,
}

impl Default for CsvExporter {
    fn default() -> Self {
        Self::new()
    }
}

impl CsvExporter {
    pub fn new() -> Self {
        Self { delimiter: b';' }
    }

    pub fn export_journal<W: Write>(
        &self,
        writer: W,
        rows: &[JournalRow],
    ) -> Result<(), csv::Error> {
        let mut wtr = csv::WriterBuilder::new()
            .delimiter(self.delimiter)
            .from_writer(writer);

        wtr.write_record(["Datum", "Serie", "Nummer", "Beskrivning", "Konto", "Debet", "Kredit"])?;

        for row in rows {
            wtr.write_record([
                &row.date,
                &row.series,
                &row.number.to_string(),
                &row.description,
                &row.account_number.to_string(),
                &format_amount(row.debit_ore),
                &format_amount(row.credit_ore),
            ])?;
        }

        wtr.flush()?;
        Ok(())
    }

    pub fn export_general_ledger<W: Write>(
        &self,
        writer: W,
        rows: &[GeneralLedgerRow],
    ) -> Result<(), csv::Error> {
        let mut wtr = csv::WriterBuilder::new()
            .delimiter(self.delimiter)
            .from_writer(writer);

        wtr.write_record(["Konto", "Datum", "Serie", "Nummer", "Beskrivning", "Debet", "Kredit", "Saldo"])?;

        for row in rows {
            wtr.write_record([
                &row.account_number.to_string(),
                &row.date,
                &row.series,
                &row.voucher_number.to_string(),
                &row.description,
                &format_amount(row.debit_ore),
                &format_amount(row.credit_ore),
                &format_amount(row.balance_ore),
            ])?;
        }

        wtr.flush()?;
        Ok(())
    }

    pub fn export_accounts<W: Write>(
        &self,
        writer: W,
        rows: &[AccountRow],
    ) -> Result<(), csv::Error> {
        let mut wtr = csv::WriterBuilder::new()
            .delimiter(self.delimiter)
            .from_writer(writer);

        wtr.write_record(["Kontonummer", "Namn", "Typ", "SRU-kod", "Aktiv"])?;

        for row in rows {
            wtr.write_record([
                row.number.to_string(),
                row.name.clone(),
                row.account_type.clone(),
                row.sru_code.as_deref().unwrap_or("").to_string(),
                if row.is_active { "Ja".to_string() } else { "Nej".to_string() },
            ])?;
        }

        wtr.flush()?;
        Ok(())
    }
}

pub struct JournalRow {
    pub date: String,
    pub series: String,
    pub number: i64,
    pub description: String,
    pub account_number: u16,
    pub debit_ore: i64,
    pub credit_ore: i64,
}

pub struct GeneralLedgerRow {
    pub account_number: u16,
    pub date: String,
    pub series: String,
    pub voucher_number: i64,
    pub description: String,
    pub debit_ore: i64,
    pub credit_ore: i64,
    pub balance_ore: i64,
}

pub struct AccountRow {
    pub number: u16,
    pub name: String,
    pub account_type: String,
    pub sru_code: Option<String>,
    pub is_active: bool,
}

/// Belopp med decimalkomma, tomt för noll. Tecknet bärs separat så att
/// belopp mellan -0,99 och -0,01 kr behåller sitt minus.
fn format_amount(ore: i64) -> String {
    crate::amount::format_decimal_comma(ore)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_csv_output() {
        let exporter = CsvExporter::new();
        let rows = vec![
            JournalRow {
                date: "2026-01-15".to_string(),
                series: "A".to_string(),
                number: 1,
                description: "Kontorsmaterial".to_string(),
                account_number: 5410,
                debit_ore: 10000,
                credit_ore: 0,
            },
            JournalRow {
                date: "2026-01-15".to_string(),
                series: "A".to_string(),
                number: 1,
                description: "Kontorsmaterial".to_string(),
                account_number: 2440,
                debit_ore: 0,
                credit_ore: 10000,
            },
        ];

        let mut output = Vec::new();
        exporter.export_journal(&mut output, &rows).unwrap();
        let csv_str = String::from_utf8(output).unwrap();
        assert!(csv_str.contains("Datum;Serie;Nummer"));
        assert!(csv_str.contains("5410;100,00;"));
        assert!(csv_str.contains("2440;;100,00"));
    }

    #[test]
    fn small_negative_amounts_keep_their_sign() {
        assert_eq!(format_amount(-1), "-0,01");
        assert_eq!(format_amount(-99), "-0,99");
        assert_eq!(format_amount(-100), "-1,00");
    }
}
