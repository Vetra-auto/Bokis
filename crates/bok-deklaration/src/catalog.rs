//! Fältförteckningen: varje fält i INK2, INK2R och INK2S med fältnummer,
//! SRU-kod, blankettens tecken, datatyp och kontonas normala sida.
//!
//! Förteckningen är data per beskattningsperiod. Perioden avgörs av
//! räkenskapsårets sista dag: P1 januari–april, P2 maj–juni, P3 juli–augusti
//! och P4 september–december, med året då räkenskapsåret slutar. Ett
//! räkenskapsår som slutar 2026-12-31 deklareras alltså på `INK2-2026P4`.

use chrono::{Datelike, NaiveDate};

use crate::DeclError;

/// Förteckningarna som finns, som (period, filens innehåll).
const CATALOGS: &[(&str, &str)] = &[("2026P4", include_str!("../data/ink2-2026p4.txt"))];

/// Blanketten ett fält hör till.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FormKind {
    /// Huvudblanketten.
    Ink2,
    /// Räkenskapsschemat: balans- och resultaträkning.
    Ink2R,
    /// Skattemässiga justeringar.
    Ink2S,
}

impl FormKind {
    /// Blankettens namn utan period, till exempel `INK2R`.
    pub fn as_str(self) -> &'static str {
        match self {
            FormKind::Ink2 => "INK2",
            FormKind::Ink2R => "INK2R",
            FormKind::Ink2S => "INK2S",
        }
    }

    fn parse(s: &str) -> Option<FormKind> {
        match s {
            "INK2" => Some(FormKind::Ink2),
            "INK2R" => Some(FormKind::Ink2R),
            "INK2S" => Some(FormKind::Ink2S),
            _ => None,
        }
    }
}

/// Blankettens tryckta tecken för fältet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sign {
    /// Ökar summan.
    Plus,
    /// Minskar summan.
    Minus,
    /// Inget tryckt tecken.
    None,
}

/// Kontonas normala sida för ett fält som räknas ur bokföringen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// Tillgångar och kostnader: debetsaldot är positivt.
    Debit,
    /// Eget kapital, skulder och intäkter: kreditsaldot är positivt.
    Credit,
    /// Fältet räknas inte ur kontosaldon.
    Neither,
}

/// Vilka värden ett fält tar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataType {
    /// Heltal, också negativa (Numeriskt_A).
    Signed,
    /// Heltal 0–999 999 999 999 (Numeriskt_B).
    Unsigned,
    /// Heltal med högst så många siffror (Numeriskt_10, Numeriskt_13).
    Digits(u32),
    /// Datum ÅÅÅÅMMDD.
    Date,
    /// Text, till exempel X i en kryssruta.
    Text,
}

/// Ett fält i blanketterna.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// Blanketten.
    pub form: FormKind,
    /// Fältnumret på blanketten, till exempel `2.4` eller `4.14a`; tomt för
    /// fält utan nummer (räkenskapsårets början och slut, kryssrutor).
    pub number: String,
    /// SRU-koden, till exempel `7215`.
    pub code: String,
    /// Blankettens tecken.
    pub sign: Sign,
    /// Datatypen.
    pub data_type: DataType,
    /// Kontonas normala sida.
    pub side: Side,
    /// Blankettens text.
    pub text: String,
}

impl Field {
    /// Kontrollerar att ett heltal ryms i fältets datatyp.
    pub fn accepts(&self, value: i64) -> bool {
        const MAX: i64 = 999_999_999_999;
        match self.data_type {
            DataType::Signed => (-MAX..=MAX).contains(&value),
            DataType::Unsigned => (0..=MAX).contains(&value),
            DataType::Digits(n) => value >= 0 && value < 10_i64.pow(n),
            DataType::Date | DataType::Text => false,
        }
    }

    /// Fältnumret och koden för visning: "2.4 (7215)".
    pub fn label(&self) -> String {
        if self.number.is_empty() {
            self.code.clone()
        } else {
            format!("{} ({})", self.number, self.code)
        }
    }
}

/// Fältförteckningen för en beskattningsperiod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    /// Perioden, till exempel `2026P4`.
    pub period: String,
    /// Fälten i blankettordning.
    pub fields: Vec<Field>,
}

/// Beskattningsperioden för ett räkenskapsår som slutar `end`.
pub fn period_of(end: NaiveDate) -> String {
    let p = match end.month() {
        1..=4 => 1,
        5..=6 => 2,
        7..=8 => 3,
        _ => 4,
    };
    format!("{}P{p}", end.year())
}

fn data_error(period: &str, line: usize, message: impl Into<String>) -> DeclError {
    DeclError::Data {
        file: format!("ink2-{}.txt", period.to_lowercase()),
        line,
        message: message.into(),
    }
}

impl Catalog {
    /// Förteckningen för räkenskapsåret som slutar `end`.
    pub fn for_year_end(end: NaiveDate) -> Result<Catalog, DeclError> {
        let period = period_of(end);
        let Some((_, text)) = CATALOGS.iter().find(|(p, _)| *p == period) else {
            let known = CATALOGS
                .iter()
                .map(|(p, _)| *p)
                .collect::<Vec<_>>()
                .join(", ");
            return Err(DeclError::NoCatalog(period, known));
        };
        Catalog::parse(&period, text)
    }

    /// Läser en förteckning: `BLANKETT;FÄLT;KOD;TECKEN;DATATYP;SALDO;TEXT`.
    pub fn parse(period: &str, text: &str) -> Result<Catalog, DeclError> {
        let mut fields = Vec::new();
        for (i, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with("//") {
                continue;
            }
            let parts: Vec<&str> = line.splitn(7, ';').collect();
            let [form, number, code, sign, data_type, side, text] = parts.as_slice() else {
                return Err(data_error(period, i + 1, "raden ska ha sju kolumner"));
            };
            let form = FormKind::parse(form)
                .ok_or_else(|| data_error(period, i + 1, format!("okänd blankett {form}")))?;
            if code.is_empty() || !code.chars().all(|c| c.is_ascii_digit()) {
                return Err(data_error(
                    period,
                    i + 1,
                    format!("{code} är ingen fältkod"),
                ));
            }
            let sign = match *sign {
                "+" => Sign::Plus,
                "-" => Sign::Minus,
                "*" => Sign::None,
                other => return Err(data_error(period, i + 1, format!("okänt tecken {other}"))),
            };
            let data_type = match *data_type {
                "Numeriskt_A" => DataType::Signed,
                "Numeriskt_B" => DataType::Unsigned,
                "Numeriskt_10" => DataType::Digits(10),
                "Numeriskt_13" => DataType::Digits(13),
                "Datum_D" => DataType::Date,
                "Str_X" => DataType::Text,
                other => {
                    return Err(data_error(period, i + 1, format!("okänd datatyp {other}")));
                }
            };
            let side = match *side {
                "D" => Side::Debit,
                "K" => Side::Credit,
                "-" => Side::Neither,
                other => return Err(data_error(period, i + 1, format!("okänd sida {other}"))),
            };
            if fields
                .iter()
                .any(|f: &Field| f.form == form && f.code == *code)
            {
                return Err(data_error(
                    period,
                    i + 1,
                    format!("koden {code} finns två gånger"),
                ));
            }
            fields.push(Field {
                form,
                number: (*number).to_string(),
                code: (*code).to_string(),
                sign,
                data_type,
                side,
                text: (*text).to_string(),
            });
        }
        Ok(Catalog {
            period: period.to_string(),
            fields,
        })
    }

    /// Blankettens namn med period, till exempel `INK2R-2026P4`.
    pub fn form_name(&self, form: FormKind) -> String {
        format!("{}-{}", form.as_str(), self.period)
    }

    /// Fältet med en viss kod på en blankett.
    pub fn field(&self, form: FormKind, code: &str) -> Option<&Field> {
        self.fields
            .iter()
            .find(|f| f.form == form && f.code == code)
    }

    /// Fälten för en blankett.
    pub fn form_fields(&self, form: FormKind) -> impl Iterator<Item = &Field> {
        self.fields.iter().filter(move |f| f.form == form)
    }

    /// Slår upp ett fält på nummer (`4.14a`) eller kod (`7763`). Ett nummer
    /// med både plus- och minuskod måste anges med koden. Koderna 7011 och
    /// 7012 finns på alla tre blanketterna och ger huvudblankettens.
    pub fn lookup(&self, key: &str) -> Result<&Field, DeclError> {
        let key = key.trim();
        if key.chars().all(|c| c.is_ascii_digit()) {
            return self
                .fields
                .iter()
                .find(|f| f.code == key)
                .ok_or_else(|| DeclError::UnknownField(key.to_string()));
        }
        let hits: Vec<&Field> = self.fields.iter().filter(|f| f.number == key).collect();
        match hits.as_slice() {
            [] => Err(DeclError::UnknownField(key.to_string())),
            [one] => Ok(one),
            many => Err(DeclError::AmbiguousField(
                key.to_string(),
                many.iter()
                    .map(|f| f.code.as_str())
                    .collect::<Vec<_>>()
                    .join(" eller "),
            )),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        Catalog::for_year_end(NaiveDate::from_ymd_opt(2026, 12, 31).unwrap()).unwrap()
    }

    #[test]
    fn periods_follow_the_last_day_of_the_year() {
        let d = |y, m, day| NaiveDate::from_ymd_opt(y, m, day).unwrap();
        assert_eq!(period_of(d(2026, 12, 31)), "2026P4");
        assert_eq!(period_of(d(2026, 9, 1)), "2026P4");
        assert_eq!(period_of(d(2026, 8, 31)), "2026P3");
        assert_eq!(period_of(d(2027, 6, 30)), "2027P2");
        assert_eq!(period_of(d(2027, 4, 30)), "2027P1");
        assert!(matches!(
            Catalog::for_year_end(d(2027, 4, 30)),
            Err(DeclError::NoCatalog(..))
        ));
    }

    #[test]
    fn the_2026p4_catalog_reads_and_has_the_known_codes() {
        let c = catalog();
        assert_eq!(c.form_name(FormKind::Ink2R), "INK2R-2026P4");
        assert_eq!(c.form_fields(FormKind::Ink2R).count(), 86);
        // Stickprov mot Skatteverkets förteckning.
        let f = c.field(FormKind::Ink2R, "7215").unwrap();
        assert_eq!((f.number.as_str(), f.side), ("2.4", Side::Debit));
        let f = c.field(FormKind::Ink2R, "7302").unwrap();
        assert_eq!((f.number.as_str(), f.side), ("2.28", Side::Credit));
        let f = c.field(FormKind::Ink2R, "7450").unwrap();
        assert_eq!(
            (f.number.as_str(), f.data_type),
            ("3.26", DataType::Unsigned)
        );
        assert_eq!(c.field(FormKind::Ink2S, "7653").unwrap().number, "4.3c");
        assert_eq!(c.field(FormKind::Ink2S, "7770").unwrap().sign, Sign::Minus);
        assert_eq!(c.field(FormKind::Ink2, "7104").unwrap().number, "1.1");
        assert_eq!(c.field(FormKind::Ink2, "1581").unwrap().number, "1.16");
        // Varje fält i 2.1–2.50 och 3.1–3.27 har en sida.
        assert!(
            c.form_fields(FormKind::Ink2R)
                .filter(|f| !f.number.is_empty())
                .all(|f| f.side != Side::Neither)
        );
    }

    #[test]
    fn lookup_by_number_or_code() {
        let c = catalog();
        assert_eq!(c.lookup("4.14a").unwrap().code, "7763");
        assert_eq!(c.lookup("7763").unwrap().number, "4.14a");
        assert!(matches!(
            c.lookup("4.13"),
            Err(DeclError::AmbiguousField(..))
        ));
        assert!(matches!(c.lookup("9.9"), Err(DeclError::UnknownField(_))));
        assert_eq!(c.lookup("7011").unwrap().form, FormKind::Ink2);
    }

    #[test]
    fn ranges_of_the_data_types() {
        let c = catalog();
        assert!(!c.field(FormKind::Ink2S, "7670").unwrap().accepts(-1));
        assert!(c.field(FormKind::Ink2R, "7281").unwrap().accepts(-1));
        assert!(
            !c.field(FormKind::Ink2, "1581")
                .unwrap()
                .accepts(10_000_000_000)
        );
    }

    #[test]
    fn broken_data_is_refused() {
        assert!(Catalog::parse("X", "INK2;1.1;7104;*;Numeriskt_B;-").is_err());
        assert!(Catalog::parse("X", "INK9;1.1;7104;*;Numeriskt_B;-;x").is_err());
        assert!(Catalog::parse("X", "INK2;1.1;71a;*;Numeriskt_B;-;x").is_err());
        assert!(
            Catalog::parse(
                "X",
                "INK2;1.1;7104;*;Numeriskt_B;-;x\nINK2;1.1;7104;*;Numeriskt_B;-;x"
            )
            .is_err()
        );
    }
}
