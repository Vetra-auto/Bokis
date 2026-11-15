//! Formatering av belopp i öre.
//!
//! Tecknet bärs separat från kronorna. Den gamla formateringen räknade fram
//! tecknet ur kronorna (`ore / 100`), och för belopp mellan -0,99 och
//! -0,01 kr är kronorna 0, så minustecknet försvann.

/// Tecken, kronor och öre för ett belopp i öre.
pub(crate) fn split(ore: i64) -> (&'static str, u64, u64) {
    let abs = ore.unsigned_abs();
    (if ore < 0 { "-" } else { "" }, abs / 100, abs % 100)
}

/// `1234,56`, `-0,50`; tomt för noll. Används i PDF och CSV.
pub(crate) fn format_decimal_comma(ore: i64) -> String {
    if ore == 0 {
        return String::new();
    }
    let (sign, kronor, rest) = split(ore);
    format!("{sign}{kronor},{rest:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_negative_amounts_keep_their_sign() {
        assert_eq!(format_decimal_comma(-1), "-0,01");
        assert_eq!(format_decimal_comma(-50), "-0,50");
        assert_eq!(format_decimal_comma(-99), "-0,99");
        assert_eq!(format_decimal_comma(-100), "-1,00");
        assert_eq!(format_decimal_comma(-5075), "-50,75");
        assert_eq!(format_decimal_comma(1), "0,01");
        assert_eq!(format_decimal_comma(0), "");
        assert_eq!(format_decimal_comma(i64::MIN), "-92233720368547758,08");
    }
}
