use bokforing_core::types::Money;

pub struct Sie4Writer {
    output: Vec<u8>,
}

pub struct Sie4Company {
    pub name: String,
    pub org_number: String,
    pub fiscal_year_start: String,
    pub fiscal_year_end: String,
}

pub struct Sie4Account {
    pub number: u16,
    pub name: String,
    pub sru_code: Option<String>,
}

pub struct Sie4Voucher {
    pub series: String,
    pub number: i64,
    pub date: String,
    pub description: String,
    pub lines: Vec<Sie4VoucherLine>,
}

pub struct Sie4VoucherLine {
    pub account_number: u16,
    pub amount: Money,
    pub date: String,
    pub description: String,
}

impl Sie4Writer {
    pub fn new() -> Self {
        Self { output: Vec::new() }
    }

    pub fn write_header(&mut self, company: &Sie4Company, program_name: &str, program_version: &str) {
        self.write_line("#FLAGGA 0");
        self.write_line("#FORMAT PC8");
        self.write_line("#SIETYP 4");
        self.write_line(&format!("#PROGRAM \"{}\" \"{}\"", program_name, program_version));
        self.write_line(&format!("#GEN {}", chrono::Utc::now().format("%Y%m%d")));
        self.write_line(&format!("#FNAMN \"{}\"", company.name));
        self.write_line(&format!("#ORGNR {}", company.org_number.replace('-', "")));
        self.write_line(&format!(
            "#RAR 0 {} {}",
            company.fiscal_year_start, company.fiscal_year_end
        ));
    }

    pub fn write_accounts(&mut self, accounts: &[Sie4Account]) {
        for acc in accounts {
            self.write_line(&format!("#KONTO {} \"{}\"", acc.number, acc.name));
            if let Some(ref sru) = acc.sru_code {
                self.write_line(&format!("#SRU {} {}", acc.number, sru));
            }
        }
    }

    pub fn write_opening_balance(&mut self, account_number: u16, amount_ore: i64) {
        self.write_line(&format!("#IB 0 {} {}", account_number, format_sie_amount(amount_ore)));
    }

    pub fn write_closing_balance(&mut self, account_number: u16, amount_ore: i64) {
        self.write_line(&format!("#UB 0 {} {}", account_number, format_sie_amount(amount_ore)));
    }

    pub fn write_result_balance(&mut self, account_number: u16, amount_ore: i64) {
        self.write_line(&format!("#RES 0 {} {}", account_number, format_sie_amount(amount_ore)));
    }

    pub fn write_vouchers(&mut self, vouchers: &[Sie4Voucher]) {
        for v in vouchers {
            self.write_line(&format!(
                "#VER {} {} {} \"{}\"",
                v.series, v.number, v.date, v.description
            ));
            self.write_line("{");
            for line in &v.lines {
                self.write_line(&format!(
                    "\t#TRANS {} {{}} {} {} \"{}\"",
                    line.account_number,
                    format_sie_amount(line.amount.ore()),
                    line.date,
                    line.description
                ));
            }
            self.write_line("}");
        }
    }

    pub fn finish(self) -> Vec<u8> {
        self.output
    }

    fn write_line(&mut self, line: &str) {
        let encoded = utf8_to_cp437(line);
        self.output.extend_from_slice(&encoded);
        self.output.push(b'\r');
        self.output.push(b'\n');
    }
}

fn format_sie_amount(ore: i64) -> String {
    let kronor = ore / 100;
    let rest = (ore % 100).abs();
    if rest == 0 {
        format!("{}", kronor)
    } else {
        format!("{}.{:02}", kronor, rest)
    }
}

fn utf8_to_cp437(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| match c {
            '\u{00E5}' => 0x86u8, // å
            '\u{00E4}' => 0x84,   // ä
            '\u{00F6}' => 0x94,   // ö
            '\u{00C5}' => 0x8F,   // Å
            '\u{00C4}' => 0x8E,   // Ä
            '\u{00D6}' => 0x99,   // Ö
            '\u{00E9}' => 0x82,   // é
            '\u{00FC}' => 0x81,   // ü
            c if c.is_ascii() => c as u8,
            _ => b'?',
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sie_amount_formatting() {
        assert_eq!(format_sie_amount(10000), "100");
        assert_eq!(format_sie_amount(-5075), "-50.75");
        assert_eq!(format_sie_amount(0), "0");
        assert_eq!(format_sie_amount(50), "0.50");
    }

    #[test]
    fn cp437_encoding() {
        let encoded = utf8_to_cp437("Köp från Ö-vik");
        assert!(encoded.contains(&0x94)); // ö
        assert!(encoded.contains(&0x86)); // å (from "från")
        assert!(encoded.contains(&0x99)); // Ö
    }

    #[test]
    fn basic_sie4_generation() {
        let mut writer = Sie4Writer::new();
        writer.write_header(
            &Sie4Company {
                name: "Test AB".to_string(),
                org_number: "559000-1234".to_string(),
                fiscal_year_start: "20260101".to_string(),
                fiscal_year_end: "20261231".to_string(),
            },
            "Bokforing",
            "0.1.0",
        );
        writer.write_accounts(&[Sie4Account {
            number: 1910,
            name: "Kassa".to_string(),
            sru_code: Some("7201".to_string()),
        }]);

        let output = writer.finish();
        let text = String::from_utf8_lossy(&output);
        assert!(text.contains("#SIETYP 4"));
        assert!(text.contains("#ORGNR 5590001234"));
        assert!(text.contains("#KONTO 1910"));
        assert!(text.contains("#SRU 1910 7201"));
    }
}
