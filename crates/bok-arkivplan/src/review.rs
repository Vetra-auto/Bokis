//! Iakttagelser om planen mot bokföringslagen och BFN:s allmänna råd, och
//! bevarandetiden för ett räkenskapsår.
//!
//! Iakttagelserna stoppar inte planen. Planen ska beskriva hur arkivet
//! faktiskt ser ut, också när något saknas; systemdokumentationen visar då
//! vad som behöver ses över.

use chrono::{Datelike, NaiveDate};

use crate::plan::{ArchivePlan, Kind};

/// Något i planen som behöver ses över, med lagrummet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    /// Vad som saknas eller bör ändras.
    pub text: String,
    /// Var kravet står.
    pub source: &'static str,
}

/// Iakttagelserna för en plan, i den ordning de står här.
pub fn observations(plan: &ArchivePlan) -> Vec<Observation> {
    let mut out = Vec::new();
    let mut push = |text: String, source| out.push(Observation { text, source });
    let has = |k: Kind| plan.parts.iter().any(|p| p.kind == k);
    if !has(Kind::Original) {
        push(
            "planen säger inte var originalet, databasen och underlagen, förvaras".into(),
            "BFNAR 2013:2 punkt 8.3",
        );
    }
    if !has(Kind::Sakerhetskopia) {
        push(
            "planen har ingen säkerhetskopia av den elektroniska räkenskapsinformationen".into(),
            "BFNAR 2013:2 punkt 8.2",
        );
    } else if !plan
        .parts
        .iter()
        .any(|p| p.kind == Kind::Sakerhetskopia && p.separate)
    {
        push(
            "ingen säkerhetskopia är markerad som åtskild från originalet".into(),
            "BFNAR 2013:2 punkt 8.2",
        );
    }
    for p in &plan.parts {
        if !p.in_sweden() && p.notified.is_none() {
            push(
                format!(
                    "{} förvaras i {} men planen säger inte när platsen anmäldes till Skatteverket",
                    p.what.trim(),
                    p.country.trim()
                ),
                "BFL 7 kap. 3 a §",
            );
        }
        if p.kind == Kind::Sakerhetskopia && p.check.is_none() {
            push(
                format!(
                    "planen säger inte hur kopian \"{}\" kontrolleras",
                    p.what.trim()
                ),
                "BFL 7 kap. 2 § (betryggande sätt)",
            );
        }
    }
    if plan.readable.is_none() {
        push(
            "planen säger inte hur informationen tas fram i läsbar form".into(),
            "BFL 7 kap. 1 §",
        );
    }
    out
}

/// Sista dagen räkenskapsinformationen för ett räkenskapsår med det här
/// slutdatumet ska bevaras: den 31 december det sjunde året efter det
/// kalenderår då räkenskapsåret avslutades (bokföringslagen 7 kap. 2 §).
pub fn retained_through(year_end: NaiveDate) -> NaiveDate {
    NaiveDate::from_ymd_opt(year_end.year() + 7, 12, 31).unwrap_or(NaiveDate::MAX)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::plan::tests::example;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn seven_years_after_the_calendar_year() {
        assert_eq!(retained_through(d(2026, 12, 31)), d(2033, 12, 31));
        // Brutet räkenskapsår: det kalenderår då året avslutades räknas.
        assert_eq!(retained_through(d(2027, 4, 30)), d(2034, 12, 31));
    }

    #[test]
    fn a_complete_plan_has_no_observations() {
        let plan = ArchivePlan::from_json(example()).unwrap();
        assert_eq!(observations(&plan), vec![]);
    }

    #[test]
    fn missing_pieces_are_pointed_out() {
        let mut plan = ArchivePlan::from_json(example()).unwrap();
        plan.parts[1].separate = false;
        plan.parts[1].check = None;
        plan.parts[2].country = "Danmark".into();
        plan.readable = None;
        let sources: Vec<_> = observations(&plan).iter().map(|o| o.source).collect();
        assert_eq!(
            sources,
            vec![
                "BFNAR 2013:2 punkt 8.2",
                "BFL 7 kap. 2 § (betryggande sätt)",
                "BFL 7 kap. 3 a §",
                "BFL 7 kap. 1 §"
            ]
        );
        plan.parts[2].notified = Some("2026-11-20".into());
        plan.parts.retain(|p| p.kind == Kind::Arkivpaket);
        let texts: Vec<_> = observations(&plan).into_iter().map(|o| o.source).collect();
        assert_eq!(
            texts,
            vec![
                "BFNAR 2013:2 punkt 8.3",
                "BFNAR 2013:2 punkt 8.2",
                "BFL 7 kap. 1 §"
            ]
        );
    }
}
