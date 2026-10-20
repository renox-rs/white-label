//! The back-office pages of the vertical: the engagements and the CDD file.
//! Every route needs a signed-in user; each handler checks the permission
//! of its action.

pub mod aml;
pub mod cdd;
pub mod deadline_notices;
pub mod engagements;
pub mod kpi;
pub mod letters;
pub mod portal_returns;
pub mod returns;
pub mod tax_profiles;

use std::collections::BTreeMap;

use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::parties::directory;

use crate::texts;

/// The texts a template reads as `tx['engagements.title']`.
pub(crate) type Texts = BTreeMap<&'static str, String>;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .merge(engagements::routes())
        .merge(cdd::routes())
        .merge(deadline_notices::routes())
        .merge(returns::routes())
        .merge(tax_profiles::routes())
        .merge(portal_returns::routes())
        .merge(letters::routes())
        .merge(aml::routes())
        .merge(kpi::routes())
}

/// The app text `key` (without the `accounting.` prefix).
pub(crate) fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("accounting.{key}"))
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

/// Dirhams typed in (`5000`, `5,000.50`) as fils.
pub(crate) fn parse_aed(text: &str) -> Option<i64> {
    let clean: String = text.trim().chars().filter(|c| *c != ',').collect();
    if clean.is_empty() {
        return Some(0);
    }
    let (whole, part) = clean.split_once('.').unwrap_or((clean.as_str(), ""));
    if part.len() > 2 || !whole.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let whole: i64 = whole.parse().ok()?;
    let part: i64 = if part.is_empty() {
        0
    } else {
        format!("{part:0<2}").parse().ok()?
    };
    whole.checked_mul(100)?.checked_add(part)
}

/// An id typed in a form.
pub(crate) fn parse_id(text: &str) -> Option<i64> {
    text.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirhams_read_as_fils() {
        assert_eq!(parse_aed("5000"), Some(500_000));
        assert_eq!(parse_aed("5,000.5"), Some(500_050));
        assert_eq!(parse_aed(""), Some(0));
        assert_eq!(parse_aed("-3"), None);
        assert_eq!(parse_aed("1.234"), None);
    }
}
