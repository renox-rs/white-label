//! The back-office pages of the vertical: the courses, the batches and their
//! sessions. Every route needs a signed-in user; each handler checks the
//! permission of its action.

pub mod batches;
pub mod courses;
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
    fn only_a_ticked_box_is_true() {
        assert!(ticked("on"));
        assert!(!ticked(""));
    }
}
