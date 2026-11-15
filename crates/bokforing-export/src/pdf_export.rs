use genpdf::{Document, Element as _, SimplePageDecorator};
use genpdf::elements::{Paragraph, TableLayout, Break};
use genpdf::style::Style;
use genpdf::fonts;

use crate::csv_export::{JournalRow, GeneralLedgerRow};

pub struct PdfExporter {
    font_family: fonts::FontFamily<fonts::FontData>,
}

#[derive(Debug, thiserror::Error)]
pub enum PdfError {
    #[error("Font-fel: {0}")]
    Font(String),
    #[error("PDF-generering misslyckades: {0}")]
    Render(String),
}

impl PdfExporter {
    pub fn new(font_dir: &std::path::Path) -> Result<Self, PdfError> {
        let font_family = fonts::from_files(font_dir, "LiberationSans", None)
            .map_err(|e| PdfError::Font(format!("{}", e)))?;
        Ok(Self { font_family })
    }

    pub fn export_journal(
        &self,
        company_name: &str,
        fiscal_year: &str,
        rows: &[JournalRow],
    ) -> Result<Vec<u8>, PdfError> {
        let mut doc = Document::new(self.font_family.clone());
        doc.set_title(format!("Grundbok — {}", company_name));

        let mut decorator = SimplePageDecorator::new();
        decorator.set_margins(15);
        doc.set_page_decorator(decorator);

        doc.push(Paragraph::new(format!("Grundbok — {}", company_name))
            .styled(Style::new().bold().with_font_size(16)));
        doc.push(Paragraph::new(format!("Räkenskapsår: {}", fiscal_year))
            .styled(Style::new().with_font_size(10)));
        doc.push(Break::new(1));

        let mut table = TableLayout::new(vec![2, 1, 1, 4, 2, 2, 2]);
        table.set_cell_decorator(genpdf::elements::FrameCellDecorator::new(true, false, false));

        let header_style = Style::new().bold().with_font_size(8);
        let mut header_row = table.row();
        header_row.push_element(Paragraph::new("Datum").styled(header_style));
        header_row.push_element(Paragraph::new("Serie").styled(header_style));
        header_row.push_element(Paragraph::new("Nr").styled(header_style));
        header_row.push_element(Paragraph::new("Beskrivning").styled(header_style));
        header_row.push_element(Paragraph::new("Konto").styled(header_style));
        header_row.push_element(Paragraph::new("Debet").styled(header_style));
        header_row.push_element(Paragraph::new("Kredit").styled(header_style));
        header_row.push().map_err(|e| PdfError::Render(format!("{}", e)))?;

        let cell_style = Style::new().with_font_size(7);
        for row in rows {
            let mut tr = table.row();
            tr.push_element(Paragraph::new(&row.date).styled(cell_style));
            tr.push_element(Paragraph::new(&row.series).styled(cell_style));
            tr.push_element(Paragraph::new(row.number.to_string()).styled(cell_style));
            tr.push_element(Paragraph::new(&row.description).styled(cell_style));
            tr.push_element(Paragraph::new(row.account_number.to_string()).styled(cell_style));
            tr.push_element(Paragraph::new(format_amount(row.debit_ore)).styled(cell_style));
            tr.push_element(Paragraph::new(format_amount(row.credit_ore)).styled(cell_style));
            tr.push().map_err(|e| PdfError::Render(format!("{}", e)))?;
        }

        doc.push(table);

        let mut buf = Vec::new();
        doc.render(&mut buf)
            .map_err(|e| PdfError::Render(format!("{}", e)))?;
        Ok(buf)
    }

    pub fn export_general_ledger(
        &self,
        company_name: &str,
        fiscal_year: &str,
        rows: &[GeneralLedgerRow],
    ) -> Result<Vec<u8>, PdfError> {
        let mut doc = Document::new(self.font_family.clone());
        doc.set_title(format!("Huvudbok — {}", company_name));

        let mut decorator = SimplePageDecorator::new();
        decorator.set_margins(15);
        doc.set_page_decorator(decorator);

        doc.push(Paragraph::new(format!("Huvudbok — {}", company_name))
            .styled(Style::new().bold().with_font_size(16)));
        doc.push(Paragraph::new(format!("Räkenskapsår: {}", fiscal_year))
            .styled(Style::new().with_font_size(10)));
        doc.push(Break::new(1));

        let mut table = TableLayout::new(vec![2, 2, 1, 1, 4, 2, 2, 2]);
        table.set_cell_decorator(genpdf::elements::FrameCellDecorator::new(true, false, false));

        let header_style = Style::new().bold().with_font_size(8);
        let mut header_row = table.row();
        header_row.push_element(Paragraph::new("Konto").styled(header_style));
        header_row.push_element(Paragraph::new("Datum").styled(header_style));
        header_row.push_element(Paragraph::new("Serie").styled(header_style));
        header_row.push_element(Paragraph::new("Nr").styled(header_style));
        header_row.push_element(Paragraph::new("Beskrivning").styled(header_style));
        header_row.push_element(Paragraph::new("Debet").styled(header_style));
        header_row.push_element(Paragraph::new("Kredit").styled(header_style));
        header_row.push_element(Paragraph::new("Saldo").styled(header_style));
        header_row.push().map_err(|e| PdfError::Render(format!("{}", e)))?;

        let cell_style = Style::new().with_font_size(7);
        for row in rows {
            let mut tr = table.row();
            tr.push_element(Paragraph::new(row.account_number.to_string()).styled(cell_style));
            tr.push_element(Paragraph::new(&row.date).styled(cell_style));
            tr.push_element(Paragraph::new(&row.series).styled(cell_style));
            tr.push_element(Paragraph::new(row.voucher_number.to_string()).styled(cell_style));
            tr.push_element(Paragraph::new(&row.description).styled(cell_style));
            tr.push_element(Paragraph::new(format_amount(row.debit_ore)).styled(cell_style));
            tr.push_element(Paragraph::new(format_amount(row.credit_ore)).styled(cell_style));
            tr.push_element(Paragraph::new(format_amount(row.balance_ore)).styled(cell_style));
            tr.push().map_err(|e| PdfError::Render(format!("{}", e)))?;
        }

        doc.push(table);

        let mut buf = Vec::new();
        doc.render(&mut buf)
            .map_err(|e| PdfError::Render(format!("{}", e)))?;
        Ok(buf)
    }

    pub fn export_trial_balance(
        &self,
        company_name: &str,
        fiscal_year: &str,
        accounts: &[TrialBalanceRow],
    ) -> Result<Vec<u8>, PdfError> {
        let mut doc = Document::new(self.font_family.clone());
        doc.set_title(format!("Balansrapport — {}", company_name));

        let mut decorator = SimplePageDecorator::new();
        decorator.set_margins(15);
        doc.set_page_decorator(decorator);

        doc.push(Paragraph::new(format!("Balansrapport — {}", company_name))
            .styled(Style::new().bold().with_font_size(16)));
        doc.push(Paragraph::new(format!("Räkenskapsår: {}", fiscal_year))
            .styled(Style::new().with_font_size(10)));
        doc.push(Break::new(1));

        let mut table = TableLayout::new(vec![2, 5, 3, 3, 3]);
        table.set_cell_decorator(genpdf::elements::FrameCellDecorator::new(true, false, false));

        let header_style = Style::new().bold().with_font_size(8);
        let mut header_row = table.row();
        header_row.push_element(Paragraph::new("Konto").styled(header_style));
        header_row.push_element(Paragraph::new("Namn").styled(header_style));
        header_row.push_element(Paragraph::new("Debet").styled(header_style));
        header_row.push_element(Paragraph::new("Kredit").styled(header_style));
        header_row.push_element(Paragraph::new("Saldo").styled(header_style));
        header_row.push().map_err(|e| PdfError::Render(format!("{}", e)))?;

        let cell_style = Style::new().with_font_size(7);
        let mut total_debit: i64 = 0;
        let mut total_credit: i64 = 0;

        for row in accounts {
            total_debit += row.total_debit_ore;
            total_credit += row.total_credit_ore;

            let mut tr = table.row();
            tr.push_element(Paragraph::new(row.account_number.to_string()).styled(cell_style));
            tr.push_element(Paragraph::new(&row.account_name).styled(cell_style));
            tr.push_element(Paragraph::new(format_amount(row.total_debit_ore)).styled(cell_style));
            tr.push_element(Paragraph::new(format_amount(row.total_credit_ore)).styled(cell_style));
            tr.push_element(Paragraph::new(format_amount(row.balance_ore)).styled(cell_style));
            tr.push().map_err(|e| PdfError::Render(format!("{}", e)))?;
        }

        let bold_style = Style::new().bold().with_font_size(8);
        let mut total_row = table.row();
        total_row.push_element(Paragraph::new(""));
        total_row.push_element(Paragraph::new("SUMMA").styled(bold_style));
        total_row.push_element(Paragraph::new(format_amount(total_debit)).styled(bold_style));
        total_row.push_element(Paragraph::new(format_amount(total_credit)).styled(bold_style));
        total_row.push_element(Paragraph::new(format_amount(total_debit - total_credit)).styled(bold_style));
        total_row.push().map_err(|e| PdfError::Render(format!("{}", e)))?;

        doc.push(table);

        let mut buf = Vec::new();
        doc.render(&mut buf)
            .map_err(|e| PdfError::Render(format!("{}", e)))?;
        Ok(buf)
    }
}

pub struct TrialBalanceRow {
    pub account_number: u16,
    pub account_name: String,
    pub total_debit_ore: i64,
    pub total_credit_ore: i64,
    pub balance_ore: i64,
}

/// Belopp med decimalkomma, tomt för noll. Tecknet bärs separat så att
/// belopp mellan -0,99 och -0,01 kr behåller sitt minus.
fn format_amount(ore: i64) -> String {
    crate::amount::format_decimal_comma(ore)
}

#[cfg(test)]
mod tests {
    use super::format_amount;

    #[test]
    fn small_negative_amounts_keep_their_sign() {
        assert_eq!(format_amount(-1), "-0,01");
        assert_eq!(format_amount(-50), "-0,50");
        assert_eq!(format_amount(-99), "-0,99");
        assert_eq!(format_amount(-12345), "-123,45");
        assert_eq!(format_amount(12345), "123,45");
        assert_eq!(format_amount(0), "");
    }
}
