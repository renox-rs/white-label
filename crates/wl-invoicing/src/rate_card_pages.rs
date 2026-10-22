//! The rate card routes (wl-invoicing §3, `FIN.RAT.01`; 09 `MPW.ACL.16`):
//! JSON for the back office, actions answer 303 to the card. The client
//! accepts a version here as a portal user; staff record the signed
//! acceptance.
//!
//! A `GET` answers the page of [`crate::rate_card_screens`] when the request
//! asks for HTML and the JSON otherwise. The actions answer 303 to the page they
//! came from, with a toast: the version after a new version, a line or the
//! acceptance, the card after it is started.

use std::collections::BTreeMap;

use renox::axum::http::HeaderMap;
use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::Serialize;
use wl_kernel::negotiate::wants_html;
use wl_kernel::{PartyId, Subject};

use crate::budget_category::refused;
use crate::permissions;
use crate::rate_card::{RateCard, RateCardLine, RateCardVersion, Unit};
use crate::rate_card_screens::{self, card_url, version_url};
use crate::rate_cards::{self, LineInput};
use crate::texts::{self, english};

/// The module's rate card routes. Viewing and managing are behind their
/// permissions; accepting is open to a signed-in user, the service decides who
/// may accept for the client.
pub fn routes() -> Routes {
    // The account manager holds the permissions for the clients they look
    // after, not globally: the guard lets in anyone who holds one anywhere and
    // each handler checks it in the scope of the contract the card prices
    // ([`allowed`]).
    Routes::new()
        .merge(wl_kernel::scopes::anywhere(
            Routes::new()
                .get("/rate-cards/{id}", show)
                .name("wl_invoicing.rate_card")
                .get("/rate-card-versions/{id}", show_version)
                .name("wl_invoicing.rate_card_version"),
            permissions::RATE_CARDS_VIEW,
        ))
        .merge(wl_kernel::scopes::anywhere(
            Routes::new()
                .post("/rate-cards", create_card)
                .name("wl_invoicing.rate_card_create")
                .post("/rate-cards/{id}/versions", create_version)
                .name("wl_invoicing.rate_card_version_create")
                .post("/rate-card-versions/{id}/lines", add_line)
                .name("wl_invoicing.rate_card_line_add")
                .post("/rate-card-lines/{id}", edit_line)
                .name("wl_invoicing.rate_card_line_edit"),
            permissions::RATE_CARDS_MANAGE,
        ))
        .merge(
            Routes::new()
                .post("/rate-card-versions/{id}/accept", accept)
                .name("wl_invoicing.rate_card_version_accept"),
        )
}

/// 403 unless `user` holds `permission` in the scope of the contract `source`
/// (the client company for a manpower contract), or globally.
pub(crate) async fn allowed(
    state: &AppState,
    user: &AuthUser,
    permission: &str,
    source: &Subject,
) -> Result {
    let scope = wl_kernel::scopes::scope_of(&state.db, source).await?;
    if user.has_permission_in(permission, &scope) || user.has_permission(permission) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
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

/// `GET /rate-cards/{id}`: the card with its versions and their lines, as a
/// page for a browser.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Response> {
    let card = RateCard::find_or_404(&state.db, id).await?;
    allowed(&state, &user, permissions::RATE_CARDS_VIEW, &card.source()).await?;
    if wants_html(&headers) {
        return Ok(rate_card_screens::card_page(&state, &user, &lang, &card)
            .await?
            .into_response());
    }
    let mut versions = Vec::new();
    for version in RateCardVersion::of_card(&state.db, card.id).await? {
        versions.push(version_view(&state, version).await?);
    }
    Ok(Json(CardView { card, versions }).into_response())
}

/// `GET /rate-card-versions/{id}`: one version with its lines, as a page for a
/// browser (with the price preview of `?line=&date=&regular=&overtime=&holiday=`).
async fn show_version(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Query(query): Query<rate_card_screens::PreviewQuery>,
) -> Result<Response> {
    let version = RateCardVersion::find_or_404(&state.db, id).await?;
    let card = RateCard::find_or_404(&state.db, version.rate_card_id).await?;
    allowed(&state, &user, permissions::RATE_CARDS_VIEW, &card.source()).await?;
    if wants_html(&headers) {
        return Ok(
            rate_card_screens::version_page(&state, &user, &lang, &version, &query)
                .await?
                .into_response(),
        );
    }
    Ok(Json(version_view(&state, version).await?).into_response())
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

/// What a person types as a role or shift (`Security guard`) as its key
/// (`security_guard`), the way the lines are stored and found.
fn key_of(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join("_")
        .to_lowercase()
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
        role: key_of(field(form, "role")),
        shift_type: key_of(field(form, "shift_type")),
        unit: unit_of(field(form, "unit"))?,
        rate,
        ot_multiplier: multiplier("ot_multiplier")?,
        holiday_multiplier: multiplier("holiday_multiplier")?,
        min_hours_milli: minimum,
    })
}

/// A form that worked: the toast `key` and a 303 to the page `url`.
fn done(lang: &Lang, url: &str, key: &str) -> Response {
    (
        Toast::success(texts::t(lang, &format!("wl_invoicing.rate_card.{key}"))),
        Redirect::to(url),
    )
        .into_response()
}

/// `POST /rate-cards`: `party`, `source` (`manpower.client_contract:4`).
async fn create_card(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let party: i64 = field(&form, "party").parse().map_err(|_| bad())?;
    let source: Subject = field(&form, "source").parse().map_err(|_| bad())?;
    allowed(&state, &user, permissions::RATE_CARDS_MANAGE, &source).await?;
    let card = rate_cards::create_card(&state.db, Some(user.id), PartyId(party), &source).await?;
    Ok(done(&lang, &card_url(card.id), "screen.start_done"))
}

/// `POST /rate-cards/{id}/versions`: `effective_from`, and `copy` to start
/// from the latest accepted version.
async fn create_version(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let card = RateCard::find_or_404(&state.db, id).await?;
    allowed(
        &state,
        &user,
        permissions::RATE_CARDS_MANAGE,
        &card.source(),
    )
    .await?;
    let from = date_of(&form, "effective_from")?;
    let copy = matches!(field(&form, "copy"), "1" | "true" | "on");
    let version = rate_cards::new_version(&state.db, Some(user.id), id, from, copy).await?;
    Ok(done(&lang, &version_url(version.id), "screen.created"))
}

/// `POST /rate-card-versions/{id}/lines`: `role`, `shift_type`, `unit`,
/// `rate` (AED), `ot_multiplier`, `holiday_multiplier`, `min_hours`.
async fn add_line(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let version = RateCardVersion::find_or_404(&state.db, id).await?;
    let card = RateCard::find_or_404(&state.db, version.rate_card_id).await?;
    allowed(
        &state,
        &user,
        permissions::RATE_CARDS_MANAGE,
        &card.source(),
    )
    .await?;
    rate_cards::add_line(&state.db, id, line_of(&form)?).await?;
    Ok(done(&lang, &version_url(id), "screen.line_saved"))
}

/// `POST /rate-card-lines/{id}`: the same fields; refused with *new version*
/// offered unless the version is still pending.
async fn edit_line(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let found = RateCardLine::find_or_404(&state.db, id).await?;
    let version = RateCardVersion::find_or_404(&state.db, found.version_id).await?;
    let card = RateCard::find_or_404(&state.db, version.rate_card_id).await?;
    allowed(
        &state,
        &user,
        permissions::RATE_CARDS_MANAGE,
        &card.source(),
    )
    .await?;
    let line = rate_cards::edit_line(&state.db, id, line_of(&form)?).await?;
    Ok(done(
        &lang,
        &version_url(line.version_id),
        "screen.line_saved",
    ))
}

/// `POST /rate-card-versions/{id}/accept`: the client's acceptance
/// (`DOC.SIG.01`); staff add `document`, the signed acceptance.
async fn accept(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let document = Some(field(&form, "document")).filter(|d| !d.is_empty());
    let version = rate_cards::accept(&state, &user, id, document).await?;
    Ok(done(
        &lang,
        &version_url(version.id),
        "screen.accepted_done",
    ))
}

#[cfg(test)]
mod tests {
    use super::hundredths;

    #[test]
    fn a_typed_role_becomes_its_key() {
        assert_eq!(super::key_of(" Security  guard "), "security_guard");
        assert_eq!(super::key_of("night"), "night");
        assert_eq!(super::key_of(""), "");
    }

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
