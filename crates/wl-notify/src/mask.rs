//! Masking of addresses in staff-facing text (#56 A8): a task shows where a
//! message went without giving the whole address away.

use wl_kernel::notify::NotifyChannel;

/// The mask of `address` on `channel`: an e-mail address keeps the first
/// letter of its mailbox and its domain (`s***@example.com`); a phone number
/// keeps its last 4 digits (`****4567`).
pub fn address(channel: NotifyChannel, address: &str) -> String {
    match channel {
        NotifyChannel::Email => email(address),
        NotifyChannel::Sms | NotifyChannel::Whatsapp => phone(address),
    }
}

/// `s***@example.com` for `sara@example.com`; `***` when there is no `@`.
pub fn email(address: &str) -> String {
    let address = address.trim();
    match address.rsplit_once('@') {
        Some((mailbox, domain)) => match mailbox.chars().next() {
            Some(first) => format!("{first}***@{domain}"),
            None => format!("***@{domain}"),
        },
        None => "***".to_owned(),
    }
}

/// `****4567` for `+971 50 123 4567`; only `****` for fewer than 5 digits.
pub fn phone(number: &str) -> String {
    let digits: Vec<char> = number.chars().filter(char::is_ascii_digit).collect();
    if digits.len() < 5 {
        return "****".to_owned();
    }
    let tail: String = digits[digits.len() - 4..].iter().collect();
    format!("****{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_email_keeps_its_first_letter_and_domain() {
        assert_eq!(email("sara@example.com"), "s***@example.com");
        assert_eq!(email("  Sara.M@Example.com "), "S***@Example.com");
        assert_eq!(email("a@b.co"), "a***@b.co");
    }

    #[test]
    fn a_malformed_email_shows_no_part_of_it() {
        assert_eq!(email("nobody"), "***");
        assert_eq!(email("@example.com"), "***@example.com");
        assert_eq!(email(""), "***");
    }

    #[test]
    fn a_phone_number_keeps_its_last_four_digits() {
        assert_eq!(phone("+971501234567"), "****4567");
        assert_eq!(phone("+971 50 123 4567"), "****4567");
    }

    #[test]
    fn a_short_number_is_fully_hidden() {
        assert_eq!(phone("1234"), "****");
        assert_eq!(phone(""), "****");
    }

    #[test]
    fn the_channel_picks_the_mask() {
        assert_eq!(
            address(NotifyChannel::Email, "sara@example.com"),
            "s***@example.com"
        );
        assert_eq!(address(NotifyChannel::Sms, "+971501234567"), "****4567");
        assert_eq!(
            address(NotifyChannel::Whatsapp, "+971501234567"),
            "****4567"
        );
    }
}
