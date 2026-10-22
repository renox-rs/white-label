//! The back-office pages of the vertical: the courses, the batches and their
//! sessions, the enrolments and the certificates, and the public page that
//! verifies a certificate. Every back-office route needs a signed-in user; each
//! handler checks the permission of its action.

pub mod batches;
pub mod certificates;
pub mod courses;
pub mod enrolments;
pub mod sessions;

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;

use crate::texts;

/// The texts a template reads as `tx['courses.title']`.
pub(crate) type Texts = BTreeMap<&'static str, String>;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .merge(courses::routes())
        .merge(batches::routes())
        .merge(sessions::routes())
        .merge(enrolments::routes())
        .merge(certificates::routes())
        .merge(certificates::public_routes())
}

/// The app text `key` (without the `edtech.` prefix).
pub(crate) fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("edtech.{key}"))
}

/// A 422 with `message`.
pub(crate) fn invalid(message: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.to_owned())
}

/// `Forbidden` unless `user` holds `permission`.
pub(crate) fn require(user: &AuthUser, permission: &str) -> Result {
    if user.has_permission(permission) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

/// An id typed in a form; `Ok(None)` when it is blank.
pub(crate) fn parse_optional_id(text: &str) -> std::result::Result<Option<i64>, Error> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    text.parse()
        .ok()
        .filter(|id| *id > 0)
        .map(Some)
        .ok_or_else(|| invalid("number: 1, 2, 3 ..."))
}

/// A whole number typed in a form.
pub(crate) fn parse_number(text: &str) -> std::result::Result<i64, Error> {
    text.trim()
        .parse()
        .map_err(|_| invalid("number: 1, 2, 3 ..."))
}

/// A date typed in a form (`2026-11-10`).
pub(crate) fn parse_date(text: &str) -> std::result::Result<NaiveDate, Error> {
    text.trim().parse().map_err(|_| invalid("date: YYYY-MM-DD"))
}

/// The weekdays typed in a form: `1,3` or `1 3`, 1 = Monday ... 7 = Sunday.
pub(crate) fn parse_weekdays(text: &str) -> std::result::Result<Vec<i64>, Error> {
    let mut days = Vec::new();
    for part in text.split(|c: char| c == ',' || c.is_whitespace()) {
        if part.is_empty() {
            continue;
        }
        let day: i64 = part
            .parse()
            .ok()
            .filter(|day| (1..=7).contains(day))
            .ok_or_else(|| invalid("weekdays: 1 = Monday ... 7 = Sunday"))?;
        if !days.contains(&day) {
            days.push(day);
        }
    }
    days.sort_unstable();
    Ok(days)
}

/// An amount of dirhams typed in a form (`2520`, `2,520.50`) as fils; a blank
/// field is 0.
pub(crate) fn parse_money(text: &str) -> std::result::Result<i64, Error> {
    let cleaned: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ',')
        .collect();
    if cleaned.is_empty() {
        return Ok(0);
    }
    let bad = || invalid("amount: 2520 or 2520.50");
    let (whole, cents) = match cleaned.split_once('.') {
        Some((whole, cents)) => (whole, cents),
        None => (cleaned.as_str(), ""),
    };
    if cents.len() > 2 || (whole.is_empty() && cents.is_empty()) {
        return Err(bad());
    }
    let whole: i64 = if whole.is_empty() {
        0
    } else {
        whole.parse().map_err(|_| bad())?
    };
    let cents: i64 = match cents.len() {
        0 => 0,
        1 => cents.parse::<i64>().map_err(|_| bad())? * 10,
        _ => cents.parse().map_err(|_| bad())?,
    };
    if whole < 0 {
        return Err(bad());
    }
    whole
        .checked_mul(100)
        .and_then(|fils| fils.checked_add(cents))
        .ok_or_else(bad)
}

/// A party typed in a form or sent by the picker: its number (`23`) or its
/// code as the Parties list shows it (`P-00023`).
pub(crate) async fn party_of(state: &AppState, text: &str) -> std::result::Result<i64, Error> {
    let text = text.trim();
    let directory = wl_kernel::parties::directory(state);
    let found = if let Ok(id) = text.parse::<i64>() {
        directory.find(&state.db, wl_kernel::PartyId(id)).await?
    } else {
        let digits = text
            .get(..2)
            .filter(|prefix| prefix.eq_ignore_ascii_case("p-"))
            .and_then(|_| text[2..].trim().parse::<u32>().ok());
        match digits {
            Some(number) => {
                directory
                    .find_by_code(&state.db, &format!("P-{number:05}"))
                    .await?
            }
            None => None,
        }
    };
    found
        .map(|party| party.id.0)
        .ok_or_else(|| invalid("party: pick one from the list or give its number, P-00023"))
}

/// A party that may be left blank.
pub(crate) async fn optional_party_of(
    state: &AppState,
    text: &str,
) -> std::result::Result<Option<i64>, Error> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    party_of(state, text).await.map(Some)
}

/// Whether a ticked checkbox came with the form.
pub(crate) fn ticked(text: &str) -> bool {
    matches!(text.trim(), "on" | "true" | "1" | "yes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weekdays_are_listed_by_number() {
        assert_eq!(parse_weekdays("3, 1 3").unwrap(), [1, 3]);
        assert!(parse_weekdays("monday").is_err());
        assert!(parse_weekdays("8").is_err());
        assert!(parse_weekdays("").unwrap().is_empty());
    }

    #[test]
    fn a_blank_id_is_none_and_a_bad_one_is_refused() {
        assert_eq!(parse_optional_id(" ").unwrap(), None);
        assert_eq!(parse_optional_id("12").unwrap(), Some(12));
        assert!(parse_optional_id("0").is_err());
        assert!(parse_optional_id("tutor").is_err());
    }

    #[test]
    fn a_date_is_year_month_day() {
        assert!(parse_date("2026-11-10").is_ok());
        assert!(parse_date("10/11/2026").is_err());
    }

    #[test]
    fn dirhams_typed_in_a_form_are_fils() {
        assert_eq!(parse_money("2520").unwrap(), 252_000);
        assert_eq!(parse_money("2,520.5").unwrap(), 252_050);
        assert_eq!(parse_money("0.05").unwrap(), 5);
        assert_eq!(parse_money(" ").unwrap(), 0);
        assert!(parse_money("12.345").is_err());
        assert!(parse_money("abc").is_err());
        assert!(parse_money("-5").is_err());
    }

    #[test]
    fn only_a_ticked_box_is_true() {
        assert!(ticked("on"));
        assert!(!ticked(""));
    }
}
