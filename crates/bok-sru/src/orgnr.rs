//! Organisationsnumret i SRU-form: tolv siffror utan bindestreck, med
//! sekelsiffrorna 16 för juridiska personer (556000-0100 blir 165560000100).

use bok_core::{IdKind, IdNumber};

use crate::SruError;

/// Ger organisationsnumret i SRU-form. Godtar tio siffror (med eller utan
/// bindestreck), som får prefixet 16, eller tolv siffror som redan har det.
/// Kontrollsiffran prövas i `bok_core::id_number`, och ett personnummer
/// godtas inte: inkomstdeklaration 2 lämnas av juridiska personer.
pub fn sru_orgnr(input: &str) -> Result<String, SruError> {
    let bad = || SruError::OrgNumber(input.to_string());
    let number = IdNumber::parse(input).map_err(|_| bad())?;
    if number.kind() != IdKind::Organisation {
        return Err(bad());
    }
    number.twelve_digits().ok_or_else(bad)
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
