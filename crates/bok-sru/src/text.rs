//! Text i SRU-filer: ISO 8859-1 utan styrtecken, och inget `#` i strängar.

use crate::SruError;

/// Kontrollerar att en sträng går att skriva som värde: inga styrtecken,
/// inget `#` och bara tecken som finns i ISO 8859-1.
pub fn check_value(field: &str, value: &str) -> Result<(), SruError> {
    if value.contains('#') {
        return Err(SruError::Hash(field.to_string()));
    }
    if let Some(ch) = value
        .chars()
        .find(|c| c.is_control() || u32::from(*c) > 0xFF)
    {
        return Err(SruError::BadChar {
            field: field.to_string(),
            ch,
        });
    }
    Ok(())
}

/// Kodar text till ISO 8859-1. Varje tecken med kodpunkt upp till 0xFF blir
/// samma byte; andra tecken är ett fel.
pub fn to_latin1(text: &str) -> Result<Vec<u8>, SruError> {
    text.chars()
        .map(|c| {
            u8::try_from(u32::from(c)).map_err(|_| SruError::BadChar {
                field: "filen".into(),
                ch: c,
            })
        })
        .collect()
}

/// Avkodar ISO 8859-1, där varje byte är ett tecken.
pub fn from_latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|b| char::from(*b)).collect()
}

/// Tar bort inledande och avslutande blanktecken och ersätter radbrytningar
/// och tabbar inuti med mellanslag, så att en sträng ryms på en rad.
pub fn one_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn swedish_letters_are_one_byte_each() {
        let bytes = to_latin1("Åsa Öberg är här").unwrap();
        assert_eq!(bytes.len(), "Åsa Öberg är här".chars().count());
        assert_eq!(bytes[0], 0xC5);
        assert_eq!(from_latin1(&bytes), "Åsa Öberg är här");
    }

    #[test]
    fn characters_outside_latin1_and_hashes_are_refused() {
        assert!(to_latin1("€").is_err());
        assert!(check_value("#NAMN", "Bolag € AB").is_err());
        assert!(check_value("#NAMN", "Bolag #1 AB").is_err());
        assert!(check_value("#NAMN", "Rad\nbrytning").is_err());
        assert!(check_value("#NAMN", "Vetra Automation AB").is_ok());
    }

    #[test]
    fn whitespace_is_collapsed() {
        assert_eq!(one_line("  Gatan 1\n  Stad\t"), "Gatan 1 Stad");
    }
}
