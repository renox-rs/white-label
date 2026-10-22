//! The rate card routes (wl-invoicing §3, `FIN.RAT.01`; 09 `MPW.ACL.16`):
//! JSON for the back office, actions answer 303 to the card. The client
//! accepts a version here as a portal user; staff record the signed
//! acceptance.
//!
//! Decision: there is no page template yet; the routes answer JSON and the
//! 303 to the card, like the other back-office actions that have no screen.

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::Serialize;
use wl_kernel::{PartyId, Subject};

use crate::budget_category::refused;
use crate::permissions;
use crate::rate_card::{RateCard, RateCardLine, RateCardVersion, Unit};
use crate::rate_cards::{self, LineInput};
use crate::texts::english;

/// The module's rate card routes. Viewing and managing are behind their
/// permissions; accepting is open to a signed-in user, the service decides who
/// may accept for the client.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .get("/rate-cards/{id}", show)
                .name("wl_invoicing.rate_card")
                .get("/rate-card-versions/{id}", show_version)
                .name("wl_invoicing.rate_card_version")
                .require_permission(permissions::RATE_CARDS_VIEW),
        )
        .merge(
            Routes::new()
                .post("/rate-cards", create_card)
                .name("wl_invoicing.rate_card_create")
                .post("/rate-cards/{id}/versions", create_version)
                .name("wl_invoicing.rate_card_version_create")
                .post("/rate-card-versions/{id}/lines", add_line)
                .name("wl_invoicing.rate_card_line_add")
                .post("/rate-card-lines/{id}", edit_line)
                .name("wl_invoicing.rate_card_line_edit")
                .require_permission(permissions::RATE_CARDS_MANAGE),
        )
        .merge(
            Routes::new()
                .post("/rate-card-versions/{id}/accept", accept)
                .name("wl_invoicing.rate_card_version_accept"),
        )
}

/// A version with its lines.
#[derive(Serialize)]
struct VersionView {
    version: RateCardVersion,
    lines: Vec<RateCardLine>,
}

/// A card with its versions.
#[derive(Serialize)]
struct CardView {
    card: RateCard,
    versions: Vec<VersionView>,
}

async fn version_view(state: &AppState, version: RateCardVersion) -> Result<VersionView> {
    let lines = version.lines(&state.db).await?;
    Ok(VersionView { version, lines })
}

/// `GET /rate-cards/{id}`: the card with its versions and their lines.
async fn show(State(state): State<AppState>, Path(id): Path<i64>) -> Result<Json<CardView>> {
    let card = RateCard::find_or_404(&state.db, id).await?;
    let mut versions = Vec::new();
    for version in RateCardVersion::of_card(&state.db, card.id).await? {
        versions.push(version_view(&state, version).await?);
    }
    Ok(Json(CardView { card, versions }))
}

/// `GET /rate-card-versions/{id}`: one version with its lines.
async fn show_version(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<VersionView>> {
    let version = RateCardVersion::find_or_404(&state.db, id).await?;
    Ok(Json(version_view(&state, version).await?))
}

fn field<'a>(form: &'a BTreeMap<String, String>, name: &str) -> &'a str {
    form.get(name).map_or("", |value| value.trim())
}

fn bad() -> Error {
    refused(english("wl_invoicing.rate_card.problem.bad_input"))
}

fn date_of(form: &BTreeMap<String, String>, name: &str) -> Result<NaiveDate> {
    field(form, name).parse().map_err(|_| bad())
}

/// `12.5` as 1250 (fils or hundredths of a unit): at most two decimals.
pub(crate) fn hundredths(text: &str) -> Option<i64> {
    let text = text.trim();
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    if whole.is_empty() || fraction.len() > 2 || !whole.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let whole: i64 = whole.parse().ok()?;
    let fraction: i64 = format!("{fraction:0<2}").parse().ok()?;
    whole.checked_mul(100)?.checked_add(fraction)
}

fn unit_of(text: &str) -> Result<Unit> {
    match text {
        "" | "hour" => Ok(Unit::Hour),
        "day" => Ok(Unit::Day),
        "month" => Ok(Unit::Month),
        _ => Err(bad()),
    }
}

fn line_of(form: &BTreeMap<String, String>) -> Result<LineInput> {
    let rate = hundredths(field(form, "rate")).ok_or_else(bad)?;
    let multiplier = |name: &str| match field(form, name) {
        "" => Ok(100),
        text => hundredths(text).ok_or_else(bad),
    };
    let minimum = match field(form, "min_hours") {
        "" => 0,
        text => hundredths(text).ok_or_else(bad)? * 10,
    };
    Ok(LineInput {
        role: field(form, "role").to_owned(),
        shift_type: field(form, "shift_type").to_owned(),
        unit: unit_of(field(form, "unit"))?,
        rate,
        ot_multiplier: multiplier("ot_multiplier")?,
        holiday_multiplier: multiplier("holiday_multiplier")?,
        min_hours_milli: minimum,
    })
}

fn to_card(id: i64) -> Result<Response> {
    Redirect::route("wl_invoicing.rate_card", &[&id]).map(IntoResponse::into_response)
}

/// `POST /rate-cards`: `party`, `source` (`manpower.client_contract:4`).
async fn create_card(
    State(state): State<AppState>,
    user: AuthUser,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let party: i64 = field(&form, "party").parse().map_err(|_| bad())?;
    let source: Subject = field(&form, "source").parse().map_err(|_| bad())?;
    let card = rate_cards::create_card(&state.db, Some(user.id), PartyId(party), &source).await?;
    to_card(card.id)
}

/// `POST /rate-cards/{id}/versions`: `effective_from`, and `copy` to start
/// from the latest accepted version.
async fn create_version(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let from = date_of(&form, "effective_from")?;
    let copy = matches!(field(&form, "copy"), "1" | "true" | "on");
    rate_cards::new_version(&state.db, Some(user.id), id, from, copy).await?;
    to_card(id)
}

/// `POST /rate-card-versions/{id}/lines`: `role`, `shift_type`, `unit`,
/// `rate` (AED), `ot_multiplier`, `holiday_multiplier`, `min_hours`.
async fn add_line(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let version = RateCardVersion::find_or_404(&state.db, id).await?;
    rate_cards::add_line(&state.db, id, line_of(&form)?).await?;
    to_card(version.rate_card_id)
}

/// `POST /rate-card-lines/{id}`: the same fields; refused with *new version*
/// offered unless the version is still pending.
async fn edit_line(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let line = rate_cards::edit_line(&state.db, id, line_of(&form)?).await?;
    let version = RateCardVersion::find_or_404(&state.db, line.version_id).await?;
    to_card(version.rate_card_id)
}

/// `POST /rate-card-versions/{id}/accept`: the client's acceptance
/// (`DOC.SIG.01`); staff add `document`, the signed acceptance.
async fn accept(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let document = Some(field(&form, "document")).filter(|d| !d.is_empty());
    let version = rate_cards::accept(&state, &user, id, document).await?;
    to_card(version.rate_card_id)
}

#[cfg(test)]
mod tests {
    use super::hundredths;

    #[test]
    fn amounts_have_at_most_two_decimals() {
        assert_eq!(hundredths("27"), Some(2700));
        assert_eq!(hundredths("27.5"), Some(2750));
        assert_eq!(hundredths("1.25"), Some(125));
        assert_eq!(hundredths("3,100"), None);
        assert_eq!(hundredths("1.255"), None);
        assert_eq!(hundredths("-1"), None);
        assert_eq!(hundredths(""), None);
    }
}
