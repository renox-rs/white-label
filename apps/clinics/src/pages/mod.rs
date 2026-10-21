//! The back-office pages of the vertical: the insurance policies, the
//! appointment's pre-check-in screen and the visit. Every route needs a
//! signed-in user; each handler checks the permission of its action.

pub mod appointments;
pub mod authorisations;
pub mod call_list;
pub mod claims;
pub mod contracts;
pub mod permits;
pub mod policies;
pub mod recalls;
pub mod visits;

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::parties::directory;

use crate::texts;

/// The texts a template reads as `tx['policies.title']`.
pub(crate) type Texts = BTreeMap<&'static str, String>;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .merge(policies::routes())
        .merge(appointments::routes())
        .merge(authorisations::routes())
        .merge(call_list::routes())
        .merge(claims::routes())
        .merge(contracts::routes())
        .merge(permits::routes())
        .merge(recalls::routes())
        .merge(visits::routes())
}

/// The app text `key` (without the `clinics.` prefix).
pub(crate) fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("clinics.{key}"))
}

/// The app text `key` with `:name` placeholders filled in from `params`.
pub(crate) fn text_with(
    lang: &Lang,
    key: &str,
    params: &[(&str, &dyn std::fmt::Display)],
) -> String {
    texts::t_with(lang, &format!("clinics.{key}"), params)
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

/// The name of `party` in the page's language, else `#id`.
pub(crate) async fn party_name(state: &AppState, lang: &Lang, party: PartyId) -> String {
    directory(state)
        .display_name(&state.db, party, &lang.locale)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("#{}", party.0))
}

/// The name of the user `id`, else `#id`.
pub(crate) async fn user_name(db: &Db, id: i64) -> String {
    User::find(db, id)
        .await
        .ok()
        .flatten()
        .map_or_else(|| format!("#{id}"), |u| u.name)
}

/// A moment typed in a form (`2026-11-10T09:30`, Asia/Dubai wall time).
pub(crate) fn parse_moment(text: &str) -> Option<renox::db::DateTime> {
    let naive = renox::chrono::NaiveDateTime::parse_from_str(text.trim(), "%Y-%m-%dT%H:%M").ok()?;
    Some(
        (naive
            - renox::chrono::Duration::seconds(i64::from(
                wl_kernel::documents::DUBAI_OFFSET_SECONDS,
            )))
        .and_utc(),
    )
}

/// An id typed in a form.
pub(crate) fn parse_id(text: &str) -> Option<i64> {
    text.trim().parse().ok()
}

/// A date typed in a form (`2026-11-10`); `Ok(None)` when it is blank.
pub(crate) fn parse_date(text: &str) -> std::result::Result<Option<NaiveDate>, Error> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    text.parse()
        .map(Some)
        .map_err(|_| invalid("date: YYYY-MM-DD"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_date_is_none_and_a_bad_one_is_refused() {
        assert_eq!(parse_date(" ").unwrap(), None);
        assert_eq!(
            parse_date("2026-11-10").unwrap(),
            NaiveDate::from_ymd_opt(2026, 11, 10)
        );
        assert!(parse_date("10/11/2026").is_err());
    }

    #[test]
    fn a_dubai_wall_time_is_four_hours_ahead_of_utc() {
        let at = parse_moment("2026-11-10T09:30").expect("a moment");
        assert_eq!(at.to_rfc3339(), "2026-11-10T05:30:00+00:00");
        assert!(parse_moment("yesterday").is_none());
    }
}
