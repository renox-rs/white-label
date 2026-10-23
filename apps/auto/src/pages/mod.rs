//! The back-office pages of the vertical: the vehicles and, with them, the
//! job cards with their estimates and the technician's list, and the public
//! page where a customer approves an estimate. Every back-office route needs
//! a signed-in user; each handler checks the permission of its action.

pub mod accident;
pub mod claims;
pub mod export_sales;
pub mod fleet;
pub mod incidents;
pub mod insurers;
pub mod job_cards;
pub mod labour;
pub mod stops;
pub mod supplier_claims;
pub mod vehicles;

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::parties::directory;

use crate::texts;

/// The texts a template reads as `tx['vehicles.title']`.
pub(crate) type Texts = BTreeMap<&'static str, String>;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .merge(vehicles::routes())
        .merge(job_cards::routes())
        .merge(job_cards::public_routes())
        .merge(labour::routes())
        .merge(stops::routes())
        .merge(accident::routes())
        .merge(incidents::routes())
        .merge(claims::routes())
        .merge(insurers::routes())
        .merge(fleet::routes())
        .merge(export_sales::routes())
        .merge(supplier_claims::routes())
}

/// The app text `key` (without the `auto.` prefix).
pub(crate) fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("auto.{key}"))
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
        .ok_or_else(|| invalid("Give a whole number, for example 12."))
}

/// A whole number typed in a form.
pub(crate) fn parse_number(text: &str) -> std::result::Result<i64, Error> {
    text.trim()
        .parse()
        .map_err(|_| invalid("Give a whole number, for example 12."))
}

/// A reading in kilometres typed in a form (`52000`, `52,000`).
pub(crate) fn parse_km(text: &str) -> std::result::Result<i64, Error> {
    let cleaned: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ',')
        .collect();
    cleaned
        .parse::<i64>()
        .ok()
        .filter(|km| *km >= 0)
        .ok_or_else(|| invalid("Give the odometer reading in kilometres, for example 52000."))
}

/// A whole number typed in a form; a blank field is 0.
pub(crate) fn parse_number_or_zero(text: &str) -> std::result::Result<i64, Error> {
    if text.trim().is_empty() {
        return Ok(0);
    }
    parse_number(text)
}

/// A date typed in a form (`2026-11-10`); `Ok(None)` when it is blank.
pub(crate) fn parse_optional_date(text: &str) -> std::result::Result<Option<NaiveDate>, Error> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    text.parse()
        .map(Some)
        .map_err(|_| invalid("Give the date as year-month-day, for example 2026-11-09."))
}

/// An amount of dirhams typed in a form (`250`, `1,250.50`) as fils; a blank
/// field is 0.
pub(crate) fn parse_money(text: &str) -> std::result::Result<i64, Error> {
    let cleaned: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ',')
        .collect();
    if cleaned.is_empty() {
        return Ok(0);
    }
    let bad = || invalid("Give the amount in dirhams, for example 250 or 250.50.");
    let (whole, cents) = match cleaned.split_once('.') {
        Some((whole, cents)) => (whole, cents),
        None => (cleaned.as_str(), ""),
    };
    if !whole
        .chars()
        .chain(cents.chars())
        .all(|c| c.is_ascii_digit())
        || cents.len() > 2
        || (whole.is_empty() && cents.is_empty())
    {
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
    whole
        .checked_mul(100)
        .and_then(|fils| fils.checked_add(cents))
        .ok_or_else(bad)
}

/// A quantity or a number of hours typed in a form (`2`, `1.5`, `0.25`) as
/// thousandths; a blank field is 0.
pub(crate) fn parse_milli(text: &str) -> std::result::Result<i64, Error> {
    let cleaned: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if cleaned.is_empty() {
        return Ok(0);
    }
    let bad = || invalid("Give a number, for example 2 or 1.5.");
    let (whole, fraction) = match cleaned.split_once('.') {
        Some((whole, fraction)) => (whole, fraction),
        None => (cleaned.as_str(), ""),
    };
    if !whole
        .chars()
        .chain(fraction.chars())
        .all(|c| c.is_ascii_digit())
        || fraction.len() > 3
        || (whole.is_empty() && fraction.is_empty())
    {
        return Err(bad());
    }
    let whole: i64 = if whole.is_empty() {
        0
    } else {
        whole.parse().map_err(|_| bad())?
    };
    let mut padded = fraction.to_owned();
    while padded.len() < 3 {
        padded.push('0');
    }
    let fraction: i64 = padded.parse().map_err(|_| bad())?;
    whole
        .checked_mul(1000)
        .and_then(|milli| milli.checked_add(fraction))
        .ok_or_else(bad)
}

/// A party typed in a form or sent by the picker: its number (`23`) or its
/// code as the Parties list shows it (`P-00023`).
pub(crate) async fn party_of(state: &AppState, text: &str) -> std::result::Result<i64, Error> {
    let text = text.trim();
    let directory = directory(state);
    let found = if let Ok(id) = text.parse::<i64>() {
        directory.find(&state.db, PartyId(id)).await?
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
    found.map(|party| party.id.0).ok_or_else(|| {
        invalid("Pick the person from the list, or give the party code, for example P-00023.")
    })
}

/// The name of the user `id`, else `#id`.
pub(crate) async fn user_name(db: &Db, id: i64) -> String {
    User::find(db, id)
        .await
        .ok()
        .flatten()
        .map_or_else(|| format!("#{id}"), |user| user.name)
}

/// The users holding `role` as options, `[id, name]`, with a leading empty
/// option labelled `blank`.
pub(crate) async fn role_options(db: &Db, role: &str, blank: String) -> Vec<[String; 2]> {
    let mut options = vec![[String::new(), blank]];
    if let Ok(users) = renox::auth::permissions::users_with_role(db, role).await {
        options.extend(
            users
                .into_iter()
                .map(|user| [user.id.to_string(), user.name]),
        );
    }
    options
}

/// The name of party `id` in `lang`, else `#id`.
pub async fn party_name(state: &AppState, lang: &str, id: i64) -> String {
    directory(state)
        .display_name(&state.db, PartyId(id), lang)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("#{id}"))
}

/// The parties the picker offers as the person types (`?q=`), or the labels
/// of the values it holds (`?values=`), as `code · name`.
pub(crate) async fn party_choices(
    state: &AppState,
    query: &renox::select::OptionQuery,
) -> Result<Vec<renox::select::SelectOption>> {
    let directory = directory(state);
    let mut parties = Vec::new();
    if query.is_lookup() {
        for id in query.values_as::<i64>() {
            parties.extend(directory.find(&state.db, PartyId(id)).await?);
        }
    } else {
        parties = directory.search(&state.db, &query.q, 20).await?;
    }
    Ok(parties
        .into_iter()
        .map(|party| {
            renox::select::SelectOption::new(
                party.id.0,
                format!("{} · {}", party.code, party.display_name),
            )
        })
        .collect())
}

/// Whether a ticked checkbox came with the form.
pub(crate) fn ticked(text: &str) -> bool {
    matches!(text.trim(), "on" | "true" | "1" | "yes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_id_is_none_and_a_bad_one_is_refused() {
        assert_eq!(parse_optional_id(" ").unwrap(), None);
        assert_eq!(parse_optional_id("12").unwrap(), Some(12));
        assert!(parse_optional_id("0").is_err());
        assert!(parse_optional_id("abc").is_err());
    }

    #[test]
    fn dirhams_typed_in_a_form_are_fils() {
        assert_eq!(parse_money("250").unwrap(), 25_000);
        assert_eq!(parse_money("1,250.5").unwrap(), 125_050);
        assert_eq!(parse_money("0.05").unwrap(), 5);
        assert_eq!(parse_money(" ").unwrap(), 0);
        assert!(parse_money("12.345").is_err());
        assert!(parse_money("abc").is_err());
        assert!(parse_money("-5").is_err());
    }

    #[test]
    fn hours_and_quantities_typed_in_a_form_are_thousandths() {
        assert_eq!(parse_milli("2").unwrap(), 2_000);
        assert_eq!(parse_milli("1.5").unwrap(), 1_500);
        assert_eq!(parse_milli("0.25").unwrap(), 250);
        assert_eq!(parse_milli(".5").unwrap(), 500);
        assert_eq!(parse_milli("").unwrap(), 0);
        assert!(parse_milli("1.2345").is_err());
        assert!(parse_milli("two").is_err());
    }

    #[test]
    fn a_date_is_year_month_day_or_blank() {
        assert!(parse_optional_date("2026-11-10").unwrap().is_some());
        assert!(parse_optional_date("").unwrap().is_none());
        assert!(parse_optional_date("10/11/2026").is_err());
    }

    #[test]
    fn an_odometer_reading_may_have_commas() {
        assert_eq!(parse_km("52,000").unwrap(), 52_000);
        assert_eq!(parse_km("0").unwrap(), 0);
        assert!(parse_km("-1").is_err());
        assert!(parse_km("lots").is_err());
    }

    #[test]
    fn only_a_ticked_box_is_true() {
        assert!(ticked("on"));
        assert!(!ticked(""));
    }
}
