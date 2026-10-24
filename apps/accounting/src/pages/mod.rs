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

use renox::grid::Column;
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
        .merge(
            Routes::new()
                .get("/accounting/parties/options", party_options)
                .name("accounting.party_options")
                .get("/accounting/engagements/options", engagement_options)
                .name("accounting.engagement_options")
                .require_auth(),
        )
}

/// `GET /accounting/parties/options`: the parties a picker offers as the
/// person types their name or code (`?q=`), or the labels of the values it
/// holds (`?values=`): the client of a new engagement or of a trial balance,
/// a UBO or director of a CDD file. Asked by name, never as a party number
/// (#495).
async fn party_options(
    State(state): State<AppState>,
    user: AuthUser,
    query: renox::select::OptionQuery,
) -> Result<Json<Vec<renox::select::SelectOption>>> {
    use crate::permissions::{CDD_PREPARE, ENGAGEMENTS_MANAGE, KPI_MANAGE};
    if ![ENGAGEMENTS_MANAGE, KPI_MANAGE, CDD_PREPARE]
        .iter()
        .any(|permission| user.has_permission(permission))
    {
        return Err(Error::Forbidden);
    }
    let directory = directory(&state);
    let mut parties = Vec::new();
    if query.is_lookup() {
        for id in query.values_as::<i64>() {
            parties.extend(directory.find(&state.db, PartyId(id)).await?);
        }
    } else {
        parties = directory.search(&state.db, &query.q, 20).await?;
    }
    Ok(Json(
        parties
            .into_iter()
            .map(|party| {
                renox::select::SelectOption::new(
                    party.id.0,
                    format!("{} · {}", party.code, party.display_name),
                )
            })
            .collect(),
    ))
}

/// `GET /accounting/engagements/options`: the engagements a return can be
/// prepared under, by number, client or services (`?q=`), or the labels of
/// the values held (`?values=`).
async fn engagement_options(
    State(state): State<AppState>,
    user: AuthUser,
    query: renox::select::OptionQuery,
) -> Result<Json<Vec<renox::select::SelectOption>>> {
    require(&user, crate::permissions::RETURNS_PREPARE)?;
    let engagements = if query.is_lookup() {
        let ids = query.values_as::<i64>();
        crate::Engagement::query()
            .where_(crate::Engagement::ID.is_in(ids))
            .get(&state.db)
            .await?
    } else {
        crate::Engagement::query()
            .where_(crate::Engagement::STATUS.is_in([
                crate::EngagementStatus::Active,
                crate::EngagementStatus::Suspended,
            ]))
            .order_by(crate::Engagement::ID)
            .get(&state.db)
            .await?
    };
    let directory = directory(&state);
    let needle = query.q.trim().to_lowercase();
    let mut options = Vec::new();
    for engagement in engagements {
        let client = directory
            .find(&state.db, engagement.client())
            .await?
            .map(|party| party.display_name)
            .unwrap_or_default();
        let label = format!(
            "{} · {} · {}",
            engagement.number(),
            client,
            engagement.title
        );
        if needle.is_empty() || query.is_lookup() || label.to_lowercase().contains(&needle) {
            options.push(renox::select::SelectOption::new(engagement.id, label));
        }
        if options.len() >= 20 {
            break;
        }
    }
    Ok(Json(options))
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

/// The grid column of a record's number (`ENG-0007`): its id padded to four
/// digits, so people search and sort by what they read. It is the frozen link
/// to `url` (`{id}` is the row's id).
pub(crate) fn number_column(label: &str, prefix: &str, url: &str) -> Column {
    Column::text("number", label)
        .computed(&format!(
            "'{prefix}-' || CASE WHEN {{T}}.\"id\" < 10 THEN '000' \
             WHEN {{T}}.\"id\" < 100 THEN '00' WHEN {{T}}.\"id\" < 1000 THEN '0' \
             ELSE '' END || CAST({{T}}.\"id\" AS TEXT)"
        ))
        .frozen()
        .mobile()
        .searchable()
        .link(url)
}

/// The grid column of the client company's name, read from its party.
pub(crate) fn client_column(label: &str, foreign_key: &str) -> Column {
    Column::related(
        "client",
        label,
        "wl_parties_parties",
        foreign_key,
        "display_name",
    )
    .mobile()
    .searchable()
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
