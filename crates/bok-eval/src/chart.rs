//! Kontoplanen som agenten ser under utvärderingen.
//!
//! Den läses från en fil med en rad per konto: fyra siffror, mellanslag, namn.
//! Utan `--kontoplan` används benchens egen fil, `context-accounts.txt` bredvid
//! uppgifterna. Bokis har ingen inbyggd kopia av BAS-kontoplanen.

use bok_mcp::backend::Account;

/// Kontotyp efter kontoklass, som den visas för agenten.
pub fn kind(number: u32) -> &'static str {
    match number {
        1000..=1999 => "tillgang",
        2000..=2099 => "eget_kapital",
        2100..=2999 => "skuld",
        3000..=3999 | 8000..=8399 => "intakt",
        _ => "kostnad",
    }
}

fn account(number: u32, name: &str) -> Account {
    Account {
        number,
        name: name.into(),
        kind: kind(number).into(),
        active: true,
    }
}

/// Läser en kontoplan med en rad per konto: fyra siffror, mellanslag, namn.
/// Rader i annat format hoppas över. Ett konto som förekommer två gånger är
/// ett fel, liksom en fil utan konton.
pub fn parse_chart(text: &str) -> Result<Vec<Account>, String> {
    let mut accounts: Vec<Account> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        let Some((number, name)) = line.split_once(' ') else {
            continue;
        };
        if number.len() != 4
            || !number.bytes().all(|b| b.is_ascii_digit())
            || name.trim().is_empty()
        {
            continue;
        }
        let number: u32 = number
            .parse()
            .map_err(|_| format!("rad {}: ogiltigt kontonummer", i + 1))?;
        if number < 1000 {
            continue;
        }
        if accounts.iter().any(|a| a.number == number) {
            return Err(format!("rad {}: konto {number} finns redan", i + 1));
        }
        accounts.push(account(number, name.trim()));
    }
    if accounts.is_empty() {
        return Err("kontoplanen innehåller inga konton".into());
    }
    accounts.sort_by_key(|a| a.number);
    Ok(accounts)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn parses_text_chart() {
        let text = "BAS 2026\n1930 Företagskonto\n\n  6530 Redovisningstjänster  \n123 för kort\nabcd Bokstäver\n2641 Debiterad ingående moms\n";
        let chart = parse_chart(text).unwrap();
        let numbers: Vec<u32> = chart.iter().map(|a| a.number).collect();
        assert_eq!(numbers, vec![1930, 2641, 6530]);
        assert_eq!(chart[2].name, "Redovisningstjänster");
        assert!(parse_chart("1930 A\n1930 B\n").is_err());
        assert!(parse_chart("ingenting här\n").is_err());
    }

    #[test]
    fn kind_follows_the_account_class() {
        let kinds = [1930, 2018, 2641, 3001, 6530, 8310, 8410].map(kind);
        assert_eq!(
            kinds,
            [
                "tillgang",
                "eget_kapital",
                "skuld",
                "intakt",
                "kostnad",
                "intakt",
                "kostnad"
            ]
        );
    }
}
