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

/// A quantity in thousandths of a unit: `8`, `8.5` and `8.125` are 8000, 8500
/// and 8125; more than three decimals, a sign or anything but digits is
/// `None`. An empty text is `None` too: a quantity is always given.
pub fn parse_milli(text: &str) -> Option<i64> {
    let text = text.trim();
    let (whole, parts) = match text.split_once('.') {
        Some((whole, parts)) => (whole, parts),
        None => (text, ""),
    };
    if whole.is_empty() || parts.len() > 3 || !parts.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let whole: i64 = whole.parse().ok()?;
    let parts: i64 = format!("{parts:0<3}").parse().ok()?;
    whole.checked_mul(1_000)?.checked_add(parts)
}

/// Thousandths as text: `8.5`, `8` for a whole number.
pub fn milli_text(milli: i64) -> String {
    let sign = if milli < 0 { "-" } else { "" };
    let (whole, parts) = (milli.abs() / 1_000, milli.abs() % 1_000);
    if parts == 0 {
        return format!("{sign}{whole}");
    }
    let fraction = format!("{parts:03}");
    format!("{sign}{whole}.{}", fraction.trim_end_matches('0'))
}

/// A temperature in tenths of a degree: `9`, `8.5` and `-18.5` are 90, 85 and
/// -185; more than one decimal or anything but digits is `None`.
pub fn parse_tenths(text: &str) -> Option<i64> {
    let text = text.trim();
    let (negative, unsigned) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (whole, tenth) = match unsigned.split_once('.') {
        Some((whole, tenth)) => (whole, tenth),
        None => (unsigned, ""),
    };
    if whole.is_empty() || tenth.len() > 1 || !tenth.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let whole: i64 = whole.parse().ok()?;
    let tenth: i64 = if tenth.is_empty() { 0 } else { tenth.parse().ok()? };
    let value = whole.checked_mul(10)?.checked_add(tenth)?;
    Some(if negative { -value } else { value })
}

/// Tenths of a degree as text: `9`, `8.5`, `-18.5`.
pub fn tenths_text(tenths: i64) -> String {
    let sign = if tenths < 0 { "-" } else { "" };
    let (whole, tenth) = (tenths.abs() / 10, tenths.abs() % 10);
    if tenth == 0 {
        format!("{sign}{whole}")
    } else {
        format!("{sign}{whole}.{tenth}")
    }
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
    fn quantities_and_temperatures_are_read_and_written() {
        assert_eq!(parse_milli("8"), Some(8_000));
        assert_eq!(parse_milli("8.125"), Some(8_125));
        assert_eq!(parse_milli("8.1255"), None);
        assert_eq!(parse_milli(""), None);
        assert_eq!(milli_text(8_500), "8.5");
        assert_eq!(milli_text(8_000), "8");
        assert_eq!(parse_tenths("9"), Some(90));
        assert_eq!(parse_tenths("-18.5"), Some(-185));
        assert_eq!(parse_tenths("5.55"), None);
        assert_eq!(tenths_text(85), "8.5");
        assert_eq!(tenths_text(-185), "-18.5");
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
