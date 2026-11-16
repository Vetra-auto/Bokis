//! Kopplingen från konton till fält, som data: `FÄLTKOD;KONTON` per rad.
//!
//! KONTON är enskilda konton och intervall åtskilda med komma, till exempel
//! `1200-1279,1290-1299`. Två koder med snedstreck (`7411/7510`) betyder ett
//! fält med plus- och minuskod, där nettot av radens konton avgör vilken
//! kod beloppet hamnar på. Ett konto får bara förekomma på en rad.

use bok_core::AccountNumber;

use crate::DeclError;

/// INK2R-kopplingen för BAS, `docs/ink2r-bas.txt`.
pub const INK2R_BAS: &str = include_str!("../../../docs/ink2r-bas.txt");
/// INK2S-kopplingen för BAS, `docs/ink2s-bas.txt`.
pub const INK2S_BAS: &str = include_str!("../../../docs/ink2s-bas.txt");

/// Fältet eller fälten en rad leder till.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Ett fält.
    Single(String),
    /// Ett fält med pluskod (kreditnetto) och minuskod (debetnetto).
    Pair {
        /// Koden när radens konton netto har kreditsaldo.
        plus: String,
        /// Koden när radens konton netto har debetsaldo.
        minus: String,
    },
}

/// En rad i kopplingen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    /// Fältet eller fälten.
    pub target: Target,
    /// Kontointervallen, båda ändarna inräknade.
    pub ranges: Vec<(u16, u16)>,
}

impl Rule {
    fn covers(&self, account: u16) -> bool {
        self.ranges
            .iter()
            .any(|(lo, hi)| (*lo..=*hi).contains(&account))
    }
}

/// En hel koppling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountMap {
    /// Raderna i filens ordning.
    pub rules: Vec<Rule>,
}

fn code(file: &str, line: usize, s: &str) -> Result<String, DeclError> {
    let s = s.trim();
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_digit()) {
        return Err(DeclError::Data {
            file: file.to_string(),
            line,
            message: format!("{s} är ingen fältkod"),
        });
    }
    Ok(s.to_string())
}

fn account(file: &str, line: usize, s: &str) -> Result<u16, DeclError> {
    AccountNumber::new(s.trim().parse::<i64>().unwrap_or(0))
        .map(AccountNumber::value)
        .map_err(|_| DeclError::Data {
            file: file.to_string(),
            line,
            message: format!("{} är inget kontonummer", s.trim()),
        })
}

impl AccountMap {
    /// Läser en koppling. `file` används i felmeddelanden.
    pub fn parse(file: &str, text: &str) -> Result<AccountMap, DeclError> {
        let mut rules: Vec<Rule> = Vec::new();
        for (i, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with("//") {
                continue;
            }
            let err = |message: String| DeclError::Data {
                file: file.to_string(),
                line: i + 1,
                message,
            };
            let (codes, accounts) = line
                .split_once(';')
                .ok_or_else(|| err("skriv FÄLTKOD;KONTON".into()))?;
            let target = match codes.split_once('/') {
                Some((plus, minus)) => Target::Pair {
                    plus: code(file, i + 1, plus)?,
                    minus: code(file, i + 1, minus)?,
                },
                None => Target::Single(code(file, i + 1, codes)?),
            };
            let mut ranges = Vec::new();
            for part in accounts.split(',') {
                let part = part.trim();
                let (lo, hi) = match part.split_once('-') {
                    Some((lo, hi)) => (account(file, i + 1, lo)?, account(file, i + 1, hi)?),
                    None => {
                        let a = account(file, i + 1, part)?;
                        (a, a)
                    }
                };
                if lo > hi {
                    return Err(err(format!("intervallet {part} går baklänges")));
                }
                for rule in &rules {
                    if let Some((a, b)) = rule.ranges.iter().find(|(a, b)| lo <= *b && *a <= hi) {
                        return Err(err(format!(
                            "{part} överlappar {a}-{b}, som redan har ett fält"
                        )));
                    }
                }
                if ranges.iter().any(|(a, b)| lo <= *b && *a <= hi) {
                    return Err(err(format!("{part} förekommer två gånger på raden")));
                }
                ranges.push((lo, hi));
            }
            rules.push(Rule { target, ranges });
        }
        Ok(AccountMap { rules })
    }

    /// INK2R-kopplingen för BAS.
    pub fn ink2r_bas() -> Result<AccountMap, DeclError> {
        AccountMap::parse("docs/ink2r-bas.txt", INK2R_BAS)
    }

    /// INK2S-kopplingen för BAS.
    pub fn ink2s_bas() -> Result<AccountMap, DeclError> {
        AccountMap::parse("docs/ink2s-bas.txt", INK2S_BAS)
    }

    /// Numret på raden som kontot hör till, om någon.
    pub fn rule_for(&self, account: AccountNumber) -> Option<usize> {
        self.rules.iter().position(|r| r.covers(account.value()))
    }

    /// Alla fältkoder som kopplingen leder till.
    pub fn codes(&self) -> impl Iterator<Item = &str> {
        self.rules.iter().flat_map(|r| match &r.target {
            Target::Single(c) => vec![c.as_str()],
            Target::Pair { plus, minus } => vec![plus.as_str(), minus.as_str()],
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::catalog::{Catalog, FormKind, Sign};

    fn acc(n: i64) -> AccountNumber {
        AccountNumber::new(n).unwrap()
    }

    #[test]
    fn the_bas_files_read_and_point_at_real_fields() {
        let catalog =
            Catalog::for_year_end(chrono::NaiveDate::from_ymd_opt(2026, 12, 31).unwrap()).unwrap();
        let r = AccountMap::ink2r_bas().unwrap();
        for code in r.codes() {
            assert!(catalog.field(FormKind::Ink2R, code).is_some(), "{code}");
        }
        for rule in &r.rules {
            if let Target::Pair { plus, minus } = &rule.target {
                assert_eq!(
                    catalog.field(FormKind::Ink2R, plus).unwrap().sign,
                    Sign::Plus
                );
                assert_eq!(
                    catalog.field(FormKind::Ink2R, minus).unwrap().sign,
                    Sign::Minus
                );
            }
        }
        let s = AccountMap::ink2s_bas().unwrap();
        for code in s.codes() {
            assert!(catalog.field(FormKind::Ink2S, code).is_some(), "{code}");
        }
    }

    #[test]
    fn every_account_but_the_year_result_has_an_ink2r_field() {
        let r = AccountMap::ink2r_bas().unwrap();
        // De vanligaste kontona i ett aktiebolag.
        for n in [
            1220, 1229, 1510, 1630, 1790, 1930, 2081, 2091, 2099, 2440, 2510, 2512, 2611, 2641,
            2650, 2893, 2990, 3001, 3973, 4010, 5410, 6072, 6570, 7010, 7510, 7832, 7973, 8314,
            8423, 8910,
        ] {
            assert!(r.rule_for(acc(n)).is_some(), "{n}");
        }
        assert!(r.rule_for(acc(8999)).is_none());
    }

    #[test]
    fn overlaps_and_bad_lines_are_refused() {
        assert!(AccountMap::parse("t", "7215;1200-1279\n7216;1279").is_err());
        assert!(AccountMap::parse("t", "7215;1279-1200").is_err());
        assert!(AccountMap::parse("t", "7215 1200").is_err());
        assert!(AccountMap::parse("t", "72a;1200").is_err());
        assert!(AccountMap::parse("t", "7215;12").is_err());
        let m = AccountMap::parse("t", "// kommentar\n7411/7510;4900-4909\n").unwrap();
        assert!(matches!(m.rules[0].target, Target::Pair { .. }));
        assert_eq!(m.rule_for(acc(4905)), Some(0));
    }
}
