//! A member leaves, and the programme changes or ends (wl-sales §4.7, §7;
//! BL-SAL-26).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `POST /loyalty/accounts/{id}/leave` | `wl_sales.loyalty_leave` | `loyalty.enrol` |
//! | `GET /portal/loyalty/{id}` | `wl_sales.portal_loyalty` | a user acting for the member |
//! | `POST /portal/loyalty/{id}/leave` | `wl_sales.portal_loyalty_leave` | a user acting for the member |
//! | `POST /loyalty/programme/changes` | `wl_sales.loyalty_terms_change` | `loyalty.manage` |
//! | `POST /loyalty/programme/off` | `wl_sales.loyalty_programme_off` | `loyalty.manage` |
//!
//! A member who asks to leave is shown the points that will be forfeited and
//! confirms: at the counter the cashier records the confirmation (`confirm`),
//! in the portal the member does. It is never refused and needs no approval.
//! The portal pages answer 404 to anyone who does not act for the member
//! (`PortalUsers`). A change of the terms with less than the notice ahead is
//! refused on save.

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::validation::{Errors, ValidationError};
use serde::Serialize;
use wl_kernel::documents::today;
use wl_kernel::portal::portal_users;

use crate::closure::{self, VIA_COUNTER, VIA_PORTAL};
use crate::loyalty_account::{AccountStatus, LoyaltyAccount};
use crate::loyalty_programme::LoyaltyProgramme;
use crate::notices::aed;
use crate::pages::campaigns::parse_fils;
use crate::terms::{self, TermsChange};
use crate::{permissions, texts};

/// The texts a template reads as `tx['closure.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The staff routes, each behind its permission, and the portal routes,
/// which need only a signed-in user acting for the member.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .post("/loyalty/accounts/{id}/leave", staff_leave)
                .name("wl_sales.loyalty_leave")
                .require_permission(permissions::LOYALTY_ENROL),
        )
        .merge(
            Routes::new()
                .post("/loyalty/programme/changes", change_terms)
                .name("wl_sales.loyalty_terms_change")
                .post("/loyalty/programme/off", turn_off)
                .name("wl_sales.loyalty_programme_off")
                .require_permission(permissions::LOYALTY_MANAGE),
        )
        .merge(
            Routes::new()
                .get("/portal/loyalty/{id}", portal_page)
                .name("wl_sales.portal_loyalty")
                .post("/portal/loyalty/{id}/leave", portal_leave)
                .name("wl_sales.portal_loyalty_leave"),
        )
}

/// The package text `key` (without the `wl_sales.` prefix).
fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_sales.{key}"))
}

/// A 422 with `message` under `field`, keeping what was typed.
fn invalid<F: Serialize>(field: &str, message: String, form: &F) -> Error {
    let mut errors = Errors::new();
    errors.add(field, message);
    Error::Validation(ValidationError::new(errors).with_input(form))
}

/// Whether the form says the member confirmed.
fn confirmed(form: &BTreeMap<String, String>) -> bool {
    form.get("confirm")
        .is_some_and(|v| matches!(v.trim(), "1" | "on" | "true" | "yes"))
}

/// `POST /loyalty/accounts/{id}/leave`: the cashier records that the member
/// asked to leave and confirmed. Without `confirm` nothing is forfeited.
async fn staff_leave(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(account): Found<LoyaltyAccount>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    if !confirmed(&form) {
        return Err(invalid("confirm", text(&lang, "closure.errors.confirm"), &form));
    }
    closure::leave(&state, account.id, Some(user.id), VIA_COUNTER).await?;
    Ok((
        Toast::success(text(&lang, "closure.done")),
        Redirect::route("wl_sales.loyalty", &[])?,
    )
        .into_response())
}

/// The account `id` when the signed-in user acts for its member in the
/// portal; 404 otherwise, so nobody learns which accounts exist.
async fn own_account(state: &AppState, user: &AuthUser, id: i64) -> Result<LoyaltyAccount> {
    let account = LoyaltyAccount::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    let acting = portal_users(state)
        .acting_for(&state.db, user.id, account.party(), today())
        .await?;
    if acting.is_none() {
        return Err(Error::NotFound);
    }
    Ok(account)
}

/// What the portal page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-sales/portal_loyalty.html")]
pub(crate) struct PortalPage {
    tx: Texts,
    account_id: i64,
    code: String,
    open: bool,
    points: i64,
    value: String,
}

/// `GET /portal/loyalty/{id}`: the member sees the points that leaving
/// forfeits.
async fn portal_page(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    let account = own_account(&state, &user, id).await?;
    let forfeit = closure::forfeit_of(&state.db, &account).await?;
    Ok(View::typed(PortalPage {
        tx: texts::all(&lang),
        account_id: account.id,
        code: account.code.clone(),
        open: account.status == AccountStatus::Active,
        points: forfeit.points,
        value: format!("AED {}", aed(forfeit.value)),
    }))
}

/// `POST /portal/loyalty/{id}/leave`: the member confirms; the account
/// closes at once, without approval.
async fn portal_leave(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let account = own_account(&state, &user, id).await?;
    if !confirmed(&form) {
        return Err(invalid("confirm", text(&lang, "closure.errors.confirm"), &form));
    }
    closure::leave(&state, account.id, Some(user.id), VIA_PORTAL).await?;
    Ok((
        Toast::success(text(&lang, "closure.done")),
        Redirect::route("wl_sales.portal_loyalty", &[&account.id])?,
    )
        .into_response())
}

/// The programme the staff change: the one that is running.
async fn running(db: &Db) -> Result<LoyaltyProgramme> {
    LoyaltyProgramme::first_active(db)
        .await?
        .ok_or(Error::NotFound)
}

/// `POST /loyalty/programme/changes`: saves a change of the terms. Fields:
/// `effective_from` (a date at least 30 days ahead), and the ones that change:
/// `earn_rate`, `minimum` (AED), `validity_months`, `excluded_items` (codes
/// separated by commas), `terms_version`, `terms_en`, `terms_ar`.
async fn change_terms(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let get = |key: &str| form.get(key).map_or("", |v| v.trim());
    let number = |key: &str| -> Result<Option<i64>> {
        match get(key) {
            "" => Ok(None),
            v => v
                .parse::<i64>()
                .map(Some)
                .map_err(|_| invalid(key, text(&lang, "loyalty.errors.number"), &form)),
        }
    };
    let effective_from = get("effective_from")
        .parse::<NaiveDate>()
        .map_err(|_| invalid("effective_from", text(&lang, "errors.date"), &form))?;
    let mut change = TermsChange::new(effective_from);
    change.earn_rate = number("earn_rate")?;
    change.validity_months = number("validity_months")?;
    change.minimum_amount = match get("minimum") {
        "" => None,
        v => Some(
            parse_fils(v)
                .ok_or_else(|| invalid("minimum", text(&lang, "loyalty.errors.number"), &form))?,
        ),
    };
    change.excluded_items = form.get("excluded_items").map(|items| {
        items
            .split(',')
            .map(|i| i.trim().to_owned())
            .filter(|i| !i.is_empty())
            .collect()
    });
    if !get("terms_version").is_empty() {
        change.terms_version = Some(get("terms_version").to_owned());
        change.terms_text_en = Some(get("terms_en").to_owned());
        change.terms_text_ar = Some(get("terms_ar").to_owned());
    }
    let programme = running(&state.db).await?;
    let saved = terms::change_terms(&state, programme.id, Some(user.id), change).await?;
    let message = texts::t_with(
        &lang,
        "wl_sales.terms.saved",
        &[("members", &saved.notified_members)],
    );
    Ok((Toast::success(message), Redirect::route("wl_sales.loyalty", &[])?).into_response())
}

/// `POST /loyalty/programme/off`: turns the programme off once `confirm` is
/// given. Earning stops now; the closing date is 90 days ahead.
async fn turn_off(
    State(state): State<AppState>,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    if !confirmed(&form) {
        return Err(invalid("confirm", text(&lang, "terms.errors.confirm"), &form));
    }
    let programme = running(&state.db).await?;
    let off = terms::turn_off(&state, programme.id).await?;
    let closing = off.closing_date.map(|d| d.to_string()).unwrap_or_default();
    let message = texts::t_with(&lang, "wl_sales.terms.turned_off", &[("date", &closing)]);
    Ok((Toast::success(message), Redirect::route("wl_sales.loyalty", &[])?).into_response())
}
