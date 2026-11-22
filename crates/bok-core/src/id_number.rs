//! Svenska identitetsbeteckningar: organisationsnummer, personnummer och
//! samordningsnummer.
//!
//! Ett organisationsnummer består av tio siffror, och den sista är en
//! kontrollsiffra (lag 1974:174 om identitetsbeteckning för juridiska
//! personer m.fl., 4 §). Ett personnummer består av födelsetid (sex siffror),
//! födelsenummer (tre siffror) och kontrollsiffra, med bindestreck som byts
//! mot plustecken det år personen fyller 100 (folkbokföringslagen 18 §). En
//! enskild näringsidkare har inget eget organisationsnummer och identifieras
//! med sitt personnummer. I ett samordningsnummer är dagen ökad med 60 (lag
//! 2022:1697 om samordningsnummer, 2 kap. 8 §).
//!
//! Lagtexterna säger inte hur kontrollsiffran räknas. Bokis använder
//! Luhn-algoritmen (modulus 10 med vikterna 2 och 1), som stämmer med
//! Skatteverkets exempel 556000-0100 i SRU-specifikationen.
//!
//! Tolv siffror godtas också: sekelsiffror (18, 19, 20) framför ett
//! personnummer och prefixet 16 framför ett organisationsnummer, som i
//! Skatteverkets filformat.

use std::fmt;

use chrono::NaiveDate;
use thiserror::Error;

/// Vilket slags nummer det är.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdKind {
    /// Organisationsnummer för en juridisk person.
    Organisation,
    /// Personnummer, till exempel för en enskild näringsidkare.
    Person,
    /// Samordningsnummer, för den som inte är eller har varit folkbokförd.
    Coordination,
}

impl IdKind {
    /// Namnet på svenska, för meddelanden.
    pub fn title(self) -> &'static str {
        match self {
            IdKind::Organisation => "organisationsnummer",
            IdKind::Person => "personnummer",
            IdKind::Coordination => "samordningsnummer",
        }
    }
}

/// Ett kontrollerat svenskt identitetsnummer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdNumber {
    kind: IdKind,
    /// De tio siffrorna utan sekel eller prefix.
    ten: String,
    /// Sekelsiffrorna om de angavs (bara för person- och samordningsnummer).
    century: Option<String>,
    /// Plustecken i stället för bindestreck: personen har fyllt 100.
    plus: bool,
}

/// Varför ett nummer inte godtas.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IdNumberError {
    /// Fel antal siffror eller tecken som inte hör hemma i ett nummer.
    #[error(
        "{0:?} är inget svenskt organisations- eller personnummer; skriv tio siffror som NNNNNN-NNNN"
    )]
    Format(String),
    /// Kontrollsiffran stämmer inte, oftast ett skrivfel.
    #[error("kontrollsiffran i {0:?} stämmer inte; kontrollera numret mot underlaget")]
    CheckDigit(String),
    /// Sekelsiffrorna pekar på ett personnummer men datumet finns inte.
    #[error("{0:?} innehåller inget giltigt födelsedatum")]
    Date(String),
}

impl IdNumber {
    /// Tolkar och kontrollerar ett nummer. Mellanslag tas bort; bindestreck
    /// eller plustecken får stå före de fyra sista siffrorna.
    pub fn parse(input: &str) -> Result<IdNumber, IdNumberError> {
        let trimmed: String = input.chars().filter(|c| !c.is_whitespace()).collect();
        let format = || IdNumberError::Format(input.trim().to_string());
        let (body, sep) = match trimmed.char_indices().find(|(_, c)| *c == '-' || *c == '+') {
            Some((i, c)) => {
                let digits_after = trimmed.len() - i - 1;
                if digits_after != 4 {
                    return Err(format());
                }
                (trimmed.replacen(c, "", 1), Some(c))
            }
            None => (trimmed.clone(), None),
        };
        if body.is_empty() || !body.bytes().all(|b| b.is_ascii_digit()) {
            return Err(format());
        }
        let (century, ten) = match body.len() {
            10 => (None, body.clone()),
            12 => (Some(body[..2].to_string()), body[2..].to_string()),
            _ => return Err(format()),
        };
        if sep == Some('+') && century.is_some() {
            return Err(format());
        }
        if !luhn(&ten) {
            return Err(IdNumberError::CheckDigit(input.trim().to_string()));
        }
        let birth_kind = birth_kind(&ten);
        let kind = match century.as_deref() {
            Some("16") => IdKind::Organisation,
            Some(c @ ("18" | "19" | "20")) => {
                let kind = birth_kind.ok_or_else(|| IdNumberError::Date(input.trim().into()))?;
                if !valid_date(c, &ten, kind) {
                    return Err(IdNumberError::Date(input.trim().into()));
                }
                kind
            }
            Some(_) => return Err(format()),
            None => birth_kind.unwrap_or(IdKind::Organisation),
        };
        let century = century.filter(|_| kind != IdKind::Organisation);
        Ok(IdNumber {
            kind,
            ten,
            century,
            plus: sep == Some('+'),
        })
    }

    /// Vilket slags nummer det är.
    pub fn kind(&self) -> IdKind {
        self.kind
    }

    /// De tio siffrorna utan bindestreck, sekel eller prefix.
    pub fn ten_digits(&self) -> &str {
        &self.ten
    }

    /// Numret med tolv siffror som i Skatteverkets filer: 16 framför ett
    /// organisationsnummer, sekelsiffrorna framför ett personnummer om de är
    /// kända.
    pub fn twelve_digits(&self) -> Option<String> {
        match (self.kind, &self.century) {
            (IdKind::Organisation, _) => Some(format!("16{}", self.ten)),
            (_, Some(c)) => Some(format!("{c}{}", self.ten)),
            (_, None) => None,
        }
    }
}

/// Visar numret på normalformen: NNNNNN-NNNN, ÅÅÅÅMMDD-NNNN när seklet är
/// känt och ÅÅMMDD+NNNN för den som fyllt 100.
impl fmt::Display for IdNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sep = if self.plus { '+' } else { '-' };
        let century = self.century.as_deref().unwrap_or("");
        write!(f, "{century}{}{sep}{}", &self.ten[..6], &self.ten[6..])
    }
}

/// Om siffrorna 3–6 är en månad och en dag är numret ett personnummer (dag
/// 1–31) eller ett samordningsnummer (dag 61–91). Annars är det inget
/// födelsedatum, och numret tolkas som ett organisationsnummer.
fn birth_kind(ten: &str) -> Option<IdKind> {
    let month: u32 = ten[2..4].parse().ok()?;
    let day: u32 = ten[4..6].parse().ok()?;
    if !(1..=12).contains(&month) {
        return None;
    }
    match day {
        1..=31 => Some(IdKind::Person),
        61..=91 => Some(IdKind::Coordination),
        _ => None,
    }
}

/// Finns datumet när seklet är känt?
fn valid_date(century: &str, ten: &str, kind: IdKind) -> bool {
    let parse = |s: &str| s.parse::<u32>().ok();
    let (Some(year), Some(month), Some(day)) = (
        parse(&format!("{century}{}", &ten[..2])),
        parse(&ten[2..4]),
        parse(&ten[4..6]),
    ) else {
        return false;
    };
    let day = if kind == IdKind::Coordination {
        day - 60
    } else {
        day
    };
    i32::try_from(year)
        .ok()
        .and_then(|y| NaiveDate::from_ymd_opt(y, month, day))
        .is_some()
}

/// Luhn-kontrollen över tio siffror: varannan siffra från vänster dubblas,
/// siffersumman tas och summan ska vara jämnt delbar med tio.
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

/// Ett utländskt nummer börjar med landskoden, till exempel DK12345678. Det
/// kan Bokis inte kontrollera och tar därför emot som det är.
pub fn is_foreign(input: &str) -> bool {
    let mut chars = input.trim().chars();
    matches!(
        (chars.next(), chars.next()),
        (Some(a), Some(b)) if a.is_ascii_alphabetic() && b.is_ascii_alphabetic()
    )
}

/// Kontrollerar en motparts nummer och ger det på normalform. Ett svenskt
/// nummer måste ha rätt kontrollsiffra; ett utländskt nummer måste börja med
/// landskoden och sparas med versaler och utan mellanslag.
pub fn normalize_party_number(input: &str) -> Result<String, IdNumberError> {
    if is_foreign(input) {
        return Ok(input
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_uppercase());
    }
    IdNumber::parse(input).map(|n| n.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn organisation_numbers() {
        let n = IdNumber::parse("556000-0100").unwrap();
        assert_eq!(n.kind(), IdKind::Organisation);
        assert_eq!(n.to_string(), "556000-0100");
        assert_eq!(n.twelve_digits().as_deref(), Some("165560000100"));
        assert_eq!(IdNumber::parse("5560000100").unwrap(), n);
        assert_eq!(IdNumber::parse(" 556000 0100 ").unwrap(), n);
        assert_eq!(IdNumber::parse("16556000-0100").unwrap(), n);
        assert_eq!(IdNumber::parse("165560000100").unwrap(), n);
    }

    #[test]
    fn the_number_that_got_through_at_the_start_is_refused() {
        // 559000-0000 gick igenom vid starten 2026-11-16; kontrollsiffran är fel.
        assert!(matches!(
            IdNumber::parse("559000-0000"),
            Err(IdNumberError::CheckDigit(_))
        ));
        assert!(matches!(
            IdNumber::parse("556000-0101"),
            Err(IdNumberError::CheckDigit(_))
        ));
    }

    #[test]
    fn wrong_shapes_are_refused() {
        for bad in [
            "",
            "abc",
            "556000",
            "556000-01000",
            "55600-00100",
            "5560000-100",
            "556000--0100",
            "17556000-0100",
            "+5560000100",
        ] {
            assert!(
                matches!(IdNumber::parse(bad), Err(IdNumberError::Format(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn personal_numbers_for_sole_traders() {
        let n = IdNumber::parse("800101-1231").unwrap();
        assert_eq!(n.kind(), IdKind::Person);
        assert_eq!(n.to_string(), "800101-1231");
        assert_eq!(n.twelve_digits(), None);
        let full = IdNumber::parse("19800101-1231").unwrap();
        assert_eq!(full.to_string(), "19800101-1231");
        assert_eq!(full.twelve_digits().as_deref(), Some("198001011231"));
        assert_eq!(IdNumber::parse("198001011231").unwrap(), full);
        let old = IdNumber::parse("800101+1231").unwrap();
        assert_eq!(old.to_string(), "800101+1231");
        // Plustecken och sekelsiffror säger samma sak två gånger.
        assert!(IdNumber::parse("19800101+1231").is_err());
        // Den 30 februari finns inte.
        assert!(matches!(
            IdNumber::parse("19800230-1235"),
            Err(IdNumberError::Date(_))
        ));
    }

    #[test]
    fn coordination_numbers_add_sixty_to_the_day() {
        let n = IdNumber::parse("800161-1238").unwrap();
        assert_eq!(n.kind(), IdKind::Coordination);
        assert!(IdNumber::parse("19800161-1238").is_ok());
        // Dag 61 + 30 finns inte i februari.
        assert!(IdNumber::parse("19800290-1232").is_err());
    }

    #[test]
    fn foreign_numbers_keep_their_country_code() {
        assert!(is_foreign("DK 12345678"));
        assert!(!is_foreign("556000-0100"));
        assert_eq!(
            normalize_party_number("dk 1234 5678").as_deref(),
            Ok("DK12345678")
        );
        assert_eq!(
            normalize_party_number("5560000100").as_deref(),
            Ok("556000-0100")
        );
        assert!(normalize_party_number("559000-0000").is_err());
    }

    #[test]
    fn luhn_matches_known_examples() {
        assert!(luhn("5560000100"));
        assert!(luhn("8001011231"));
        assert!(!luhn("8001011232"));
    }
}
