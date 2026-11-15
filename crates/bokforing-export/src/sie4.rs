use std::collections::BTreeMap;

use bokforing_core::types::Money;

use crate::amount::split;

pub struct Sie4Writer {
    output: Vec<u8>,
}

impl Default for Sie4Writer {
    fn default() -> Self {
        Self::new()
    }
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

/// En verifikation med registreringsdatum och signatur (`#VER … regdatum sign`).
pub struct Sie4VoucherRecord {
    pub voucher: Sie4Voucher,
    /// Registreringsdatum, `YYYYMMDD`.
    pub reg_date: Option<String>,
    /// Vem som bokförde.
    pub sign: Option<String>,
}

/// Allt som behövs för en SIE 4-fil för ett räkenskapsår.
pub struct Sie4Export {
    pub company: Sie4Company,
    pub program_name: String,
    pub program_version: String,
    /// Datum då filen skapas, `YYYYMMDD`.
    pub generated: String,
    pub accounts: Vec<Sie4Account>,
    /// Ingående balans per konto i öre. Den skrivs som `#IB`, inte som
    /// verifikationer, så att den inte räknas två gånger av den som läser filen.
    pub opening_balances: Vec<(u16, i64)>,
    /// Årets verifikationer utom ingående balans.
    pub vouchers: Vec<Sie4VoucherRecord>,
}

impl Sie4Writer {
    pub fn new() -> Self {
        Self { output: Vec::new() }
    }

    pub fn write_header(&mut self, company: &Sie4Company, program_name: &str, program_version: &str) {
        let generated = chrono::Utc::now().format("%Y%m%d").to_string();
        self.write_header_generated(company, program_name, program_version, &generated);
    }

    /// Som [`Sie4Writer::write_header`] men med ett angivet `#GEN`-datum.
    pub fn write_header_generated(
        &mut self,
        company: &Sie4Company,
        program_name: &str,
        program_version: &str,
        generated: &str,
    ) {
        self.write_line("#FLAGGA 0");
        self.write_line("#FORMAT PC8");
        self.write_line("#SIETYP 4");
        self.write_line(&format!("#PROGRAM {} {}", quote(program_name), quote(program_version)));
        self.write_line(&format!("#GEN {}", generated));
        self.write_line(&format!("#FNAMN {}", quote(&company.name)));
        self.write_line(&format!("#ORGNR {}", company.org_number.replace('-', "")));
        self.write_line(&format!(
            "#RAR 0 {} {}",
            company.fiscal_year_start, company.fiscal_year_end
        ));
    }

    pub fn write_accounts(&mut self, accounts: &[Sie4Account]) {
        for acc in accounts {
            self.write_line(&format!("#KONTO {} {}", acc.number, quote(&acc.name)));
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
            self.write_voucher(v, None, None);
        }
    }

    /// `#VER serie nr datum "text" regdatum "sign"` följt av raderna.
    pub fn write_voucher(&mut self, v: &Sie4Voucher, reg_date: Option<&str>, sign: Option<&str>) {
        let mut head = format!("#VER {} {} {} {}", v.series, v.number, v.date, quote(&v.description));
        match (reg_date, sign) {
            (Some(r), Some(s)) => head.push_str(&format!(" {} {}", r, quote(s))),
            (Some(r), None) => head.push_str(&format!(" {}", r)),
            (None, Some(s)) => head.push_str(&format!(" \"\" {}", quote(s))),
            (None, None) => {}
        }
        self.write_line(&head);
        self.write_line("{");
        for line in &v.lines {
            self.write_line(&format!(
                "\t#TRANS {} {{}} {} {} {}",
                line.account_number,
                format_sie_amount(line.amount.ore()),
                line.date,
                quote(&line.description)
            ));
        }
        self.write_line("}");
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

/// Skriver en hel SIE 4-fil för ett räkenskapsår. Balanskonton (klass 1–2)
/// får `#IB` och `#UB`, resultatkonton (klass 3 och uppåt) får `#RES`.
/// Utgående balans är ingående balans plus årets verifikationer.
pub fn export_year(e: &Sie4Export) -> Vec<u8> {
    let mut opening: BTreeMap<u16, i64> = BTreeMap::new();
    for (account, ore) in &e.opening_balances {
        *opening.entry(*account).or_default() += ore;
    }
    let mut movement: BTreeMap<u16, i64> = BTreeMap::new();
    for record in &e.vouchers {
        for line in &record.voucher.lines {
            *movement.entry(line.account_number).or_default() += line.amount.ore();
        }
    }
    let mut accounts: Vec<u16> = opening.keys().chain(movement.keys()).copied().collect();
    accounts.sort_unstable();
    accounts.dedup();

    let mut w = Sie4Writer::new();
    w.write_header_generated(&e.company, &e.program_name, &e.program_version, &e.generated);
    w.write_accounts(&e.accounts);
    let is_balance = |account: u16| account < 3000;
    for &a in accounts.iter().filter(|a| is_balance(**a)) {
        w.write_opening_balance(a, opening.get(&a).copied().unwrap_or(0));
    }
    for &a in accounts.iter().filter(|a| is_balance(**a)) {
        let ub = opening.get(&a).copied().unwrap_or(0) + movement.get(&a).copied().unwrap_or(0);
        w.write_closing_balance(a, ub);
    }
    for &a in accounts.iter().filter(|a| !is_balance(**a)) {
        let res = opening.get(&a).copied().unwrap_or(0) + movement.get(&a).copied().unwrap_or(0);
        w.write_result_balance(a, res);
    }
    for record in &e.vouchers {
        w.write_voucher(&record.voucher, record.reg_date.as_deref(), record.sign.as_deref());
    }
    w.finish()
}

/// Belopp i SIE: punkt som decimaltecken, decimaler bara när de behövs.
/// Tecknet tas från beloppet, inte från kronorna, så att -0,50 blir `-0.50`.
fn format_sie_amount(ore: i64) -> String {
    let (sign, kronor, rest) = split(ore);
    if rest == 0 {
        format!("{sign}{kronor}")
    } else {
        format!("{sign}{kronor}.{rest:02}")
    }
}

/// En sträng inom citattecken. Citattecken och bakåtsnedstreck i texten
/// skrivs med bakåtsnedstreck (SIE 4B), och radbrytningar och andra
/// styrtecken blir mellanslag så att posten håller sig på en rad.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out.push('"');
    out
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
    fn small_negative_amounts_keep_their_sign() {
        assert_eq!(format_sie_amount(-1), "-0.01");
        assert_eq!(format_sie_amount(-50), "-0.50");
        assert_eq!(format_sie_amount(-99), "-0.99");
        assert_eq!(format_sie_amount(-100), "-1");
        assert_eq!(format_sie_amount(-101), "-1.01");
    }

    #[test]
    fn quotes_and_backslashes_are_escaped() {
        assert_eq!(quote("Kassa"), "\"Kassa\"");
        assert_eq!(quote("Bokhandeln \"Ord\" AB"), "\"Bokhandeln \\\"Ord\\\" AB\"");
        assert_eq!(quote("C:\\temp"), "\"C:\\\\temp\"");
        assert_eq!(quote("två\nrader"), "\"två rader\"");
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

    fn line(account: u16, ore: i64, text: &str) -> Sie4VoucherLine {
        Sie4VoucherLine {
            account_number: account,
            amount: Money::from_ore(ore),
            date: "20260315".into(),
            description: text.into(),
        }
    }

    fn sample_year() -> Sie4Export {
        Sie4Export {
            company: Sie4Company {
                name: "Bokhandeln \"Ord & Bild\" AB".into(),
                org_number: "559000-1234".into(),
                fiscal_year_start: "20260101".into(),
                fiscal_year_end: "20261231".into(),
            },
            program_name: "Bokis".into(),
            program_version: "0.1.0".into(),
            generated: "20261113".into(),
            accounts: vec![
                Sie4Account { number: 1930, name: "Företagskonto".into(), sru_code: None },
                Sie4Account { number: 2081, name: "Aktiekapital".into(), sru_code: None },
                Sie4Account { number: 3010, name: "Försäljning".into(), sru_code: None },
                Sie4Account { number: 6570, name: "Bankkostnader".into(), sru_code: None },
            ],
            opening_balances: vec![(1930, 2_500_000), (2081, -2_500_000)],
            vouchers: vec![
                Sie4VoucherRecord {
                    voucher: Sie4Voucher {
                        series: "A".into(),
                        number: 1,
                        date: "20260315".into(),
                        description: "Sålt \"special\" till kund".into(),
                        lines: vec![line(1930, 125_000, "Inbetalning"), line(3010, -125_000, "")],
                    },
                    reg_date: Some("20260316".into()),
                    sign: Some("henrik".into()),
                },
                Sie4VoucherRecord {
                    voucher: Sie4Voucher {
                        series: "A".into(),
                        number: 2,
                        date: "20260331".into(),
                        description: "Bankavgift".into(),
                        lines: vec![line(6570, 50, "Avgift"), line(1930, -50, "")],
                    },
                    reg_date: Some("20260401".into()),
                    sign: Some("henrik".into()),
                },
            ],
        }
    }

    #[test]
    fn year_export_has_opening_closing_and_result_balances() {
        let text = String::from_utf8_lossy(&export_year(&sample_year())).replace("\r\n", "\n");
        assert!(text.contains("#IB 0 1930 25000\n"), "{text}");
        assert!(text.contains("#IB 0 2081 -25000\n"));
        assert!(text.contains("#UB 0 1930 26249.50\n"));
        assert!(text.contains("#UB 0 2081 -25000\n"));
        assert!(text.contains("#RES 0 3010 -1250\n"));
        assert!(text.contains("#RES 0 6570 0.50\n"));
        assert!(!text.contains("#IB 0 3010"), "resultatkonton har ingen ingående balans");
        assert!(text.contains("#VER A 2 20260331 \"Bankavgift\" 20260401 \"henrik\"\n"));
        assert!(text.contains("#TRANS 1930 {} -0.50 20260315 \"\"\n"), "litet negativt belopp behåller tecknet");
        assert!(text.contains("#GEN 20261113\n"));
    }

    #[test]
    fn export_round_trips_through_sie_parser() {
        let bytes = export_year(&sample_year());
        assert_eq!(sie_parser::detect_encoding(&bytes), sie_parser::Encoding::Cp437);
        let src = sie_parser::decode_cp437(&bytes);
        let doc = sie_parser::document::read(&src).expect("sie-parser läser exporten");
        assert_eq!(doc.company.name, "Bokhandeln \"Ord & Bild\" AB");
        assert_eq!(doc.ib[&(0, 1930)].to_string(), "25000");
        assert_eq!(doc.ub[&(0, 1930)].to_string(), "26249.50");
        assert_eq!(doc.res[&(0, 6570)].to_string(), "0.50");
        assert_eq!(doc.accounts[&1930].name, "Företagskonto");
        assert_eq!(doc.vouchers.len(), 2);
        let first = &doc.vouchers[0];
        assert_eq!(first.text, "Sålt \"special\" till kund");
        assert_eq!(first.registration_date.map(|d| d.to_string()).as_deref(), Some("20260316"));
        assert_eq!(doc.vouchers[1].rows[1].amount.to_string(), "-0.50");
        // Utgående balans = ingående balans + årets verifikationer.
        let computed = doc
            .vouchers
            .iter()
            .flat_map(|v| v.rows.iter())
            .filter(|r| r.account == 1930)
            .fold(doc.ib[&(0, 1930)], |acc, r| acc + r.amount);
        assert_eq!(computed, doc.ub[&(0, 1930)]);
    }
}
