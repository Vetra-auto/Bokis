//! Bolagsskatten.
//!
//! Överskottet av näringsverksamhet (INK2S 4.15) avrundas nedåt till helt
//! tiotal kronor och är den beskattningsbara inkomsten (IL 1 kap. 7 §).
//! Skatten för juridiska personer är 20,6 procent av den (IL 65 kap. 10 §).
//! Eftersom inkomsten är ett tiotal blir skatten ett helt antal öre.
//! Lagen säger inget om avrundning av själva skatten; beloppet här är en
//! beräkning att bokföra, och Skatteverkets slutliga skatt gäller.

/// Skattesatsen i promille: 20,6 % (IL 65 kap. 10 §, för beskattningsår som
/// börjar efter den 31 december 2020).
pub const RATE_PER_MILLE: i64 = 206;

/// Beräkningen av bolagsskatten.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorporateTax {
    /// Överskottet enligt INK2S 4.15, i kronor.
    pub surplus: i64,
    /// Den beskattningsbara inkomsten, avrundad nedåt till tiotal kronor.
    pub taxable: i64,
    /// Skatten i öre.
    pub tax_ore: i64,
}

/// Bolagsskatten på ett överskott i hela kronor. Ett underskott (negativt
/// eller noll) ger ingen skatt.
pub fn corporate_tax(surplus: i64) -> CorporateTax {
    let taxable = if surplus > 0 {
        surplus - surplus % 10
    } else {
        0
    };
    // taxable · 100 öre · 206 / 1000 = taxable / 10 · 206, exakt.
    let tax_ore = taxable / 10 * RATE_PER_MILLE;
    CorporateTax {
        surplus,
        taxable,
        tax_ore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounded_down_to_tens_then_twenty_point_six_percent() {
        let t = corporate_tax(142_009);
        assert_eq!(t.taxable, 142_000);
        assert_eq!(t.tax_ore, 2_925_200);
        assert_eq!(corporate_tax(100_000).tax_ore, 2_060_000);
        // 10 kr ger 2,06 kr.
        assert_eq!(corporate_tax(19).tax_ore, 206);
    }

    #[test]
    fn no_tax_on_a_deficit() {
        assert_eq!(corporate_tax(0).tax_ore, 0);
        assert_eq!(corporate_tax(-5_000).tax_ore, 0);
        assert_eq!(corporate_tax(9).taxable, 0);
    }
}
