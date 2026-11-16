//! En läsbar sammanställning av deklarationen i Markdown: kontrollerna,
//! varje blankett med fältnummer, kod och belopp, bolagsskatten och vilka
//! konton varje fält består av.

use std::fmt::Write as _;

use crate::catalog::FormKind;
use crate::declaration::Declaration;

/// Ett heltal med mellanslag som tusentalsavgränsare: 142 000.
pub fn grouped(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(c);
    }
    if n < 0 { format!("−{out}") } else { out }
}

/// Öre som kronor med två decimaler: 29 252,00.
pub fn ore_text(ore: i64) -> String {
    let sign = if ore < 0 { "−" } else { "" };
    let abs = ore.unsigned_abs();
    format!(
        "{sign}{},{:02}",
        grouped(i64::try_from(abs / 100).unwrap_or(i64::MAX)),
        abs % 100
    )
}

fn title(form: FormKind) -> &'static str {
    match form {
        FormKind::Ink2 => "INK2 – huvudblankett",
        FormKind::Ink2R => "INK2R – räkenskapsschema",
        FormKind::Ink2S => "INK2S – skattemässiga justeringar",
    }
}

/// Sammanställningen som Markdown.
pub fn markdown(d: &Declaration) -> String {
    let mut b = String::new();
    let _ = writeln!(b, "# Inkomstdeklaration 2 – utkast\n");
    let _ = writeln!(
        b,
        "{}, {}. Räkenskapsår {} – {}, blanketterna {}, {} och {}.\n",
        d.company_name,
        d.org_number,
        d.year_start,
        d.year_end,
        d.catalog.form_name(FormKind::Ink2),
        d.catalog.form_name(FormKind::Ink2R),
        d.catalog.form_name(FormKind::Ink2S),
    );
    let _ = writeln!(
        b,
        "Bokis har räknat fram utkastet ur bokföringen. Det är du som lämnar deklarationen: \
         granska varje fält mot årsredovisningen och fyll i det som inte syns i bokföringen \
         (till exempel underskott från tidigare år, 4.14a) innan filerna lämnas in.\n"
    );

    let _ = writeln!(b, "## Kontroller\n");
    for c in &d.checks {
        let mark = if c.ok { "[x]" } else { "[ ] **FEL:**" };
        if c.detail.is_empty() {
            let _ = writeln!(b, "- {mark} {}", c.name);
        } else {
            let _ = writeln!(b, "- {mark} {} – {}", c.name, c.detail);
        }
    }
    if !d.warnings.is_empty() {
        let _ = writeln!(b, "\n## Att se över\n");
        for w in &d.warnings {
            let _ = writeln!(b, "- {w}");
        }
    }

    for form in [FormKind::Ink2, FormKind::Ink2R, FormKind::Ink2S] {
        let _ = writeln!(b, "\n## {}\n", title(form));
        let _ = writeln!(b, "| Fält | Kod | Text | Belopp, kr |");
        let _ = writeln!(b, "|------|-----|------|-----------:|");
        let _ = writeln!(b, "|  | 7011 | Räkenskapsårets början | {} |", d.year_start);
        let _ = writeln!(b, "|  | 7012 | Räkenskapsårets slut | {} |", d.year_end);
        for (f, k, _) in d.amounts(form) {
            let _ = writeln!(
                b,
                "| {} | {} | {} | {} |",
                f.number,
                f.code,
                f.text,
                grouped(k)
            );
        }
        for ((kind, code), text) in &d.texts {
            if *kind == form {
                let label = d.catalog.field(form, code).map_or("", |f| f.text.as_str());
                let _ = writeln!(b, "|  | {code} | {label} | {text} |");
            }
        }
    }

    let _ = writeln!(b, "\n## Bolagsskatt\n");
    let _ = writeln!(
        b,
        "Överskott (4.15) {} kr. Beskattningsbar inkomst, avrundad nedåt till helt tiotal kronor \
         (IL 1 kap. 7 §): {} kr. Skatt 20,6 % (IL 65 kap. 10 §): **{} kr**. Bokför den med \
         `bok bokslut skatt`; Skatteverkets beslut om slutlig skatt gäller.",
        grouped(d.tax.surplus),
        grouped(d.tax.taxable),
        ore_text(d.tax.tax_ore)
    );

    let _ = writeln!(b, "\n## Specifikation per fält\n");
    for form in [FormKind::Ink2R, FormKind::Ink2S, FormKind::Ink2] {
        for (f, k, v) in d.amounts(form) {
            let _ = writeln!(
                b,
                "**{} {} ({})** – {} kr\n",
                form.as_str(),
                f.number,
                f.code,
                grouped(k)
            );
            for p in &v.parts {
                let _ = writeln!(b, "- {}: {}", p.label, ore_text(p.ore));
            }
            b.push('\n');
        }
    }
    b
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::testdata::input;

    #[test]
    fn numbers_read_naturally() {
        assert_eq!(grouped(142_000), "142 000");
        assert_eq!(grouped(-1_234_567), "−1 234 567");
        assert_eq!(grouped(12), "12");
        assert_eq!(ore_text(2_925_200), "29 252,00");
        assert_eq!(ore_text(-5), "−0,05");
    }

    #[test]
    fn the_summary_has_every_part() {
        let d = Declaration::compute_bas(&input()).unwrap();
        let md = markdown(&d);
        for needle in [
            "INK2R-2026P4",
            "| 3.26 | 7450 |",
            "| 4.15 | 7670 |",
            "| 1.1 | 7104 |",
            "29 252,00 kr",
            "6072 Representation, ej avdragsgill: 10 000,00",
            "[x] Balansräkningen balanserar",
        ] {
            assert!(md.contains(needle), "{needle} saknas i\n{md}");
        }
    }
}
