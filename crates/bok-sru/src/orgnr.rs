//! Organisationsnumret i SRU-form: tolv siffror utan bindestreck, med
//! sekelsiffrorna 16 för juridiska personer (556000-0100 blir 165560000100).

use crate::SruError;

/// Ger organisationsnumret i SRU-form. Godtar tio siffror (med eller utan
/// bindestreck), som får prefixet 16, eller tolv siffror som redan har det.
/// Kontrollsiffran prövas med Luhn-algoritmen.
pub fn sru_orgnr(input: &str) -> Result<String, SruError> {
    let digits: String = input.chars().filter(|c| *c != '-' && *c != ' ').collect();
    let bad = || SruError::OrgNumber(input.to_string());
    if !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err(bad());
    }
    let ten = match digits.len() {
        10 => digits.as_str(),
        12 if digits.starts_with("16") => &digits[2..],
        _ => return Err(bad()),
    };
    // Tredje siffran är minst 2 för juridiska personer.
    if ten.as_bytes()[2] < b'2' || !luhn(ten) {
        return Err(bad());
    }
    Ok(format!("16{ten}"))
}

/// Luhn-kontrollen över tio siffror.
fn luhn(ten: &str) -> bool {
    let sum: u32 = ten
        .bytes()
        .enumerate()
        .map(|(i, b)| {
            let d = u32::from(b - b'0');
            if i % 2 == 0 {
                let x = d * 2;
                x / 10 + x % 10
            } else {
                d
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_example_from_skatteverket() {
        assert_eq!(sru_orgnr("556000-0100").as_deref(), Ok("165560000100"));
        assert_eq!(sru_orgnr("5560000100").as_deref(), Ok("165560000100"));
        assert_eq!(sru_orgnr("165560000100").as_deref(), Ok("165560000100"));
    }

    #[test]
    fn wrong_numbers_are_refused() {
        assert!(sru_orgnr("556000-0101").is_err());
        assert!(sru_orgnr("556000").is_err());
        assert!(sru_orgnr("19800101-1234").is_err());
        assert!(sru_orgnr("abc").is_err());
        // Ett personnummer (tredje siffran under 2) är inget organisationsnummer.
        assert!(sru_orgnr("800101-1231").is_err());
    }
}
