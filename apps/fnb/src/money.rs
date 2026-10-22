//! Dirhams and fils: money is fils (`i64`) everywhere; people type and read
//! dirhams.

/// Dirhams as fils: `150`, `150.5` and `150.50` are 15000, 15050 and 15050; an
/// empty text is 0; more than two decimals, or anything but digits, is
/// `None`.
pub fn parse_dirhams(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.is_empty() {
        return Some(0);
    }
    let (whole, cents) = match text.split_once('.') {
        Some((whole, cents)) => (whole, cents),
        None => (text, ""),
    };
    if cents.len() > 2 || !cents.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let whole: i64 = whole.parse().ok()?;
    let cents: i64 = format!("{cents:0<2}").parse().ok()?;
    whole.checked_mul(100)?.checked_add(cents)
}

/// Fils as text: `AED 25.00`.
pub fn aed(fils: i64) -> String {
    let sign = if fils < 0 { "-" } else { "" };
    format!("AED {sign}{}.{:02}", fils.abs() / 100, fils.abs() % 100)
}

/// The VAT rate of the F&B client's sales: 5% (UAE).
pub const VAT_PERCENT: i64 = 5;

/// `gross` (with VAT) without the VAT, rounded half up to the fil.
pub fn net_of(gross: i64) -> i64 {
    let divisor = 100 + VAT_PERCENT;
    let scaled = i128::from(gross) * 100;
    let rounded = if scaled >= 0 {
        (scaled * 2 + i128::from(divisor)) / (2 * i128::from(divisor))
    } else {
        -((-scaled * 2 + i128::from(divisor)) / (2 * i128::from(divisor)))
    };
    i64::try_from(rounded).unwrap_or(gross)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirhams_are_read_as_fils() {
        assert_eq!(parse_dirhams("150"), Some(15_000));
        assert_eq!(parse_dirhams("150.5"), Some(15_050));
        assert_eq!(parse_dirhams("30.00"), Some(3_000));
        assert_eq!(parse_dirhams(""), Some(0));
        assert_eq!(parse_dirhams("1.234"), None);
        assert_eq!(parse_dirhams("abc"), None);
    }

    #[test]
    fn fils_are_written_as_dirhams() {
        assert_eq!(aed(8_500), "AED 85.00");
        assert_eq!(aed(-5), "AED -0.05");
    }

    #[test]
    fn the_net_is_the_gross_without_five_percent_vat() {
        assert_eq!(net_of(10_500), 10_000);
        assert_eq!(net_of(3_000), 2_857);
        assert_eq!(net_of(0), 0);
        assert_eq!(net_of(-2_100), -2_000);
    }
}
