//! Reading what people type: dirhams, areas and dates.

use renox::chrono::NaiveDate;

/// A decimal text as an integer in `scale` decimal places: `"25,000.5"` with
/// scale 2 is 2_500_050. `None` for an empty text, a sign, a letter, or more
/// decimals than the scale holds (unless they are zeros).
fn parse_decimal(text: &str, scale: usize) -> Option<i64> {
    let cleaned: String = text.chars().filter(|c| !matches!(c, ',' | ' ' | '_')).collect();
    if cleaned.is_empty() {
        return None;
    }
    let (whole, fraction) = cleaned.split_once('.').unwrap_or((cleaned.as_str(), ""));
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if (whole.is_empty() && fraction.is_empty()) || !digits(whole) || !digits(fraction) {
        return None;
    }
    let (kept, extra) = fraction.split_at(fraction.len().min(scale));
    if extra.bytes().any(|b| b != b'0') {
        return None;
    }
    let mut value: i64 = if whole.is_empty() { 0 } else { whole.parse().ok()? };
    for _ in 0..scale {
        value = value.checked_mul(10)?;
    }
    let mut fractional: i64 = if kept.is_empty() { 0 } else { kept.parse().ok()? };
    for _ in kept.len()..scale {
        fractional = fractional.checked_mul(10)?;
    }
    value.checked_add(fractional)
}

/// Dirhams as fils: `"25,000.50"` is 2_500_050.
pub fn parse_aed(text: &str) -> Option<i64> {
    parse_decimal(text, 2)
}

/// A quantity as thousandths: `"1,250.5"` is 1_250_500.
pub fn parse_milli(text: &str) -> Option<i64> {
    parse_decimal(text, 3)
}

/// A date typed as `YYYY-MM-DD`; `None` for blank or malformed text.
pub fn parse_date(text: &str) -> Option<NaiveDate> {
    text.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirhams_become_fils() {
        assert_eq!(parse_aed("25000"), Some(2_500_000));
        assert_eq!(parse_aed("25,000.50"), Some(2_500_050));
        assert_eq!(parse_aed("0.5"), Some(50));
        assert_eq!(parse_aed(".75"), Some(75));
        assert_eq!(parse_aed("10.500"), Some(1_050));
    }

    #[test]
    fn bad_amounts_are_refused() {
        for bad in ["", "abc", "-5", "1.234", "1.2.3", "."] {
            assert_eq!(parse_aed(bad), None, "{bad}");
        }
    }

    #[test]
    fn areas_become_thousandths() {
        assert_eq!(parse_milli("1250.5"), Some(1_250_500));
        assert_eq!(parse_milli("900"), Some(900_000));
    }

    #[test]
    fn dates_are_iso() {
        assert_eq!(parse_date("2027-02-28"), NaiveDate::from_ymd_opt(2027, 2, 28));
        assert_eq!(parse_date("28/02/2027"), None);
        assert_eq!(parse_date(""), None);
    }
}
