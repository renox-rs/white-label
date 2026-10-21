//! Cancelling as the party (wl-scheduling §4.1, §5.2 step 4, `SCH.ACL.01`,
//! `SCH.ACL.02`): from the portal, from the link in a confirmation or
//! reminder, and from the link in a message about a change the business made.
//!
//! | Route | Name | Who |
//! |---|---|---|
//! | `GET/POST /scheduling/links/{id}/cancel` | `wl_scheduling.link_cancel_page`, `wl_scheduling.link_cancel` | a signed link: `party_request`, initiated by the party, before the cut-off |
//! | `GET/POST /scheduling/links/{id}/cancel/{code}` | `wl_scheduling.link_change_page`, `wl_scheduling.link_change_cancel` | a signed link in a change message: reason `code`, initiated by the business, at any time before the check-in |
//! | `GET /scheduling/links/{id}/accept` | `wl_scheduling.link_accept_page` | a signed link in a change message: *accept* keeps the booking as it is |
//! | `GET/POST /scheduling/links/{id}/reschedule` | `wl_scheduling.link_reschedule_page`, `wl_scheduling.link_reschedule` | a signed link in a change message: *pick another time* lists the nearest free times of the same resources, and posting one moves the booking, also after the reschedule cut-off |
//! | `POST /scheduling/portal/bookings/{id}/cancel` | `wl_scheduling.portal_cancel` | a portal user acting for the booking's party: `party_request` |
//! | `POST /scheduling/portal/bookings` | `wl_scheduling.portal_book` | a portal user acting for `party`: books, or requests when the service needs acceptance; a slot just taken answers 409 with the nearest free slots |
//! | `POST /scheduling/portal/waiting-list` | `wl_scheduling.portal_waiting_join` | a portal user acting for `party`: joins the waiting list |
//! | `POST /scheduling/portal/waiting-list/{id}/withdraw` | `wl_scheduling.portal_waiting_withdraw` | a portal user acting for the entry's party |
//!
//! The signature is the credential of a link ([`renox::signed`]); a booking
//! of another party answers 404 in the portal. Neither route lets the party
//! pick a code or a side: the ordinary routes record `party_request`, the
//! change route records the code of the change as `business`.
//!
//! Decision: a link opens a page with a button, and the button posts to the
//! same signed URL, so a mail scanner that opens the link cancels nothing.

use std::collections::BTreeMap;
use std::time::Duration;

use renox::axum::extract::OriginalUri;
use renox::prelude::*;
use renox::signed::ValidSignature;
use serde::Serialize;
use wl_kernel::PartyId;
use wl_kernel::documents::{dubai_text, today};

use crate::booking::{Booking, default_initiator};
use crate::config::config_of;
use crate::pages::{field, local_text, number, parse_local};
use crate::resource::{Resource, ServiceType};
use crate::service::{self, Actor, CancelRequest, NewBooking};
use crate::slots;
use crate::texts;
use crate::waiting::{self, NewEntry, WaitingEntry};

/// The link in a confirmation or reminder that cancels `booking_id`, valid
/// for `ttl`.
///
/// # Errors
///
/// When the route is not registered.
pub fn cancel_url(state: &AppState, booking_id: i64, ttl: Duration) -> Result<String> {
    state.signed_url("wl_scheduling.link_cancel_page", &[&booking_id], ttl)
}

/// The link in a message about the change `reason_code` that cancels
/// `booking_id` as initiated by the business, valid for `ttl`.
///
/// # Errors
///
/// When the route is not registered.
pub fn change_cancel_url(
    state: &AppState,
    booking_id: i64,
    reason_code: &str,
    ttl: Duration,
) -> Result<String> {
    state.signed_url(
        "wl_scheduling.link_change_page",
        &[&booking_id, &reason_code],
        ttl,
    )
}

/// The link in a change message that keeps the booking as it is, valid for
/// `ttl`.
///
/// # Errors
///
/// When the route is not registered.
pub fn accept_url(state: &AppState, booking_id: i64, ttl: Duration) -> Result<String> {
    state.signed_url("wl_scheduling.link_accept_page", &[&booking_id], ttl)
}

/// The link in a change message that opens the choice of another time, valid
/// for `ttl`.
///
/// # Errors
///
/// When the route is not registered.
pub fn reschedule_url(state: &AppState, booking_id: i64, ttl: Duration) -> Result<String> {
    state.signed_url("wl_scheduling.link_reschedule_page", &[&booking_id], ttl)
}

/// The routes of the links (public: the signature is the credential) and of
/// the portal.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .get("/scheduling/links/{id}/cancel", cancel_page)
                .name("wl_scheduling.link_cancel_page")
                .post("/scheduling/links/{id}/cancel", cancel_by_link)
                .name("wl_scheduling.link_cancel")
                .get("/scheduling/links/{id}/cancel/{code}", change_page)
                .name("wl_scheduling.link_change_page")
                .post("/scheduling/links/{id}/cancel/{code}", cancel_by_change)
                .name("wl_scheduling.link_change_cancel")
                .get("/scheduling/links/{id}/accept", accept_page)
                .name("wl_scheduling.link_accept_page")
                .get("/scheduling/links/{id}/reschedule", reschedule_page)
                .name("wl_scheduling.link_reschedule_page")
                .post("/scheduling/links/{id}/reschedule", reschedule_by_link)
                .name("wl_scheduling.link_reschedule"),
        )
        .merge(
            Routes::new()
                .post("/scheduling/portal/bookings/{id}/cancel", portal_cancel)
                .name("wl_scheduling.portal_cancel")
                .post("/scheduling/portal/bookings", portal_book)
                .name("wl_scheduling.portal_book")
                .post("/scheduling/portal/waiting-list", portal_join)
                .name("wl_scheduling.portal_waiting_join")
                .post(
                    "/scheduling/portal/waiting-list/{id}/withdraw",
                    portal_withdraw,
                )
                .name("wl_scheduling.portal_waiting_withdraw")
                .require_auth(),
        )
}

/// What the link page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-scheduling/link.html")]
pub(crate) struct LinkPage {
    tx: BTreeMap<&'static str, String>,
    summary: String,
    action: String,
    can_cancel: bool,
    message: String,
}

async fn page(
    state: &AppState,
    lang: &Lang,
    id: i64,
    action: String,
    message: Option<String>,
) -> Result<View> {
    let booking = Booking::find_or_404(&state.db, id).await?;
    let service = ServiceType::find_or_404(&state.db, booking.service_type_id).await?;
    let can_cancel = message.is_none()
        && booking.status.is_open()
        && !booking.status.is_after_check_in();
    Ok(View::typed(LinkPage {
        tx: texts::all(lang),
        summary: format!("{} · {}", service.name_en, dubai_text(booking.starts_at)),
        action,
        can_cancel,
        message: message.unwrap_or_default(),
    }))
}

/// `GET /scheduling/links/{id}/cancel`.
async fn cancel_page(
    _signature: ValidSignature,
    State(state): State<AppState>,
    lang: Lang,
    OriginalUri(uri): OriginalUri,
    Path(id): Path<i64>,
) -> Result<View> {
    page(&state, &lang, id, uri.to_string(), None).await
}

/// `POST /scheduling/links/{id}/cancel`: the party cancels, `party_request`.
async fn cancel_by_link(
    _signature: ValidSignature,
    State(state): State<AppState>,
    lang: Lang,
    OriginalUri(uri): OriginalUri,
    Path(id): Path<i64>,
) -> Result<View> {
    service::cancel(
        &state,
        id,
        CancelRequest::by_party(),
        Actor::party(None),
        renox::db::now(),
    )
    .await
    .map_err(|refusal| refusal.into_error(&lang))?;
    let message = texts::t(&lang, "wl_scheduling.link.cancelled");
    page(&state, &lang, id, uri.to_string(), Some(message)).await
}

/// The reason code of a change message, if the module knows it.
fn known_code(state: &AppState, code: &str) -> Result<()> {
    default_initiator(code, &config_of(state).extra_reason_codes)
        .map(|_| ())
        .ok_or(Error::NotFound)
}

/// `GET /scheduling/links/{id}/cancel/{code}`.
async fn change_page(
    _signature: ValidSignature,
    State(state): State<AppState>,
    lang: Lang,
    OriginalUri(uri): OriginalUri,
    Path((id, code)): Path<(i64, String)>,
) -> Result<View> {
    known_code(&state, &code)?;
    page(&state, &lang, id, uri.to_string(), None).await
}

/// `POST /scheduling/links/{id}/cancel/{code}`: the party cancels from a
/// message about a change the business made: the code of the change,
/// initiated by the business, also after the cancellation cut-off.
async fn cancel_by_change(
    _signature: ValidSignature,
    State(state): State<AppState>,
    lang: Lang,
    OriginalUri(uri): OriginalUri,
    Path((id, code)): Path<(i64, String)>,
) -> Result<View> {
    known_code(&state, &code)?;
    service::cancel(
        &state,
        id,
        CancelRequest::from_change_message(&code),
        Actor::party(None),
        renox::db::now(),
    )
    .await
    .map_err(|refusal| refusal.into_error(&lang))?;
    let message = texts::t(&lang, "wl_scheduling.link.cancelled");
    page(&state, &lang, id, uri.to_string(), Some(message)).await
}

/// `GET /scheduling/links/{id}/accept`: the party keeps the booking as it is.
/// Nothing changes; the page says so.
async fn accept_page(
    _signature: ValidSignature,
    State(state): State<AppState>,
    lang: Lang,
    OriginalUri(uri): OriginalUri,
    Path(id): Path<i64>,
) -> Result<View> {
    let message = texts::t(&lang, "wl_scheduling.link.accepted");
    page(&state, &lang, id, uri.to_string(), Some(message)).await
}

/// One time the party can pick.
#[derive(Serialize)]
pub(crate) struct SlotChoice {
    value: String,
    label: String,
}

/// What the page of *pick another time* needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-scheduling/link_slots.html")]
pub(crate) struct SlotsPage {
    tx: BTreeMap<&'static str, String>,
    summary: String,
    action: String,
    message: String,
    slots: Vec<SlotChoice>,
    no_slots: bool,
}

async fn slots_page(
    state: &AppState,
    lang: &Lang,
    id: i64,
    action: String,
    message: Option<String>,
) -> Result<View> {
    let booking = Booking::find_or_404(&state.db, id).await?;
    let service = ServiceType::find_or_404(&state.db, booking.service_type_id).await?;
    let open = booking.status.is_open() && !booking.status.is_after_check_in();
    let mut choices = Vec::new();
    if message.is_none() && open {
        let ids = booking.resource_ids(&state.db).await?;
        let mut resources = Vec::new();
        for resource_id in &ids {
            if let Some(resource) = Resource::find(&state.db, *resource_id).await? {
                resources.push(resource);
            }
        }
        let now = renox::db::now();
        // Up to three times of the same resources, as the party was told.
        for slot in slots::nearest(state, &service, &resources, booking.starts_at, now, slots::SHOWN)
            .await?
        {
            let mut same = slot.resource_ids.clone();
            same.sort_unstable();
            if same == ids {
                choices.push(SlotChoice {
                    value: local_text(slot.start),
                    label: dubai_text(slot.start),
                });
            }
        }
    }
    let no_slots = message.is_none() && choices.is_empty();
    Ok(View::typed(SlotsPage {
        tx: texts::all(lang),
        summary: format!("{} · {}", service.name_en, dubai_text(booking.starts_at)),
        action,
        message: message.unwrap_or_default(),
        slots: choices,
        no_slots,
    }))
}

/// `GET /scheduling/links/{id}/reschedule`: the nearest free times of the
/// booking's own resources, each with a button.
async fn reschedule_page(
    _signature: ValidSignature,
    State(state): State<AppState>,
    lang: Lang,
    OriginalUri(uri): OriginalUri,
    Path(id): Path<i64>,
) -> Result<View> {
    slots_page(&state, &lang, id, uri.to_string(), None).await
}

/// `POST /scheduling/links/{id}/reschedule`: the party picks `starts_at`. The
/// booking moves, the slot is checked again under the lock, and the conflict
/// clears; the reschedule cut-off does not apply to a change message.
async fn reschedule_by_link(
    _signature: ValidSignature,
    State(state): State<AppState>,
    lang: Lang,
    OriginalUri(uri): OriginalUri,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<View> {
    let start = parse_local(field(&form, "starts_at"))
        .ok_or_else(|| Error::BadRequest("starts_at".to_owned()))?;
    service::reschedule_from_change(&state, id, start, renox::db::now())
        .await
        .map_err(|refusal| refusal.into_error(&lang))?;
    let message = texts::t(&lang, "wl_scheduling.link.rescheduled");
    slots_page(&state, &lang, id, uri.to_string(), Some(message)).await
}

/// `POST /scheduling/portal/bookings/{id}/cancel`: a portal user cancels a
/// booking of the party they act for (`SCH.ACL.01`); a booking of anyone
/// else answers 404. The code is `party_request` and the party picks none.
async fn portal_cancel(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    let booking = Booking::find_or_404(&state.db, id).await?;
    acts_for(&state, &user, booking.party()).await?;
    service::cancel(
        &state,
        id,
        CancelRequest::by_party(),
        Actor::party(Some(user.id)),
        renox::db::now(),
    )
    .await
    .map_err(|refusal| refusal.into_error(&lang))?;
    Ok((
        Toast::success(texts::t(&lang, "wl_scheduling.link.cancelled")),
        renox::HxRefresh,
    )
        .into_response())
}

/// Whether the portal user acts for `party` today; a party of anyone else is
/// a 404.
async fn acts_for(state: &AppState, user: &AuthUser, party: PartyId) -> Result<()> {
    let acting = wl_kernel::portal::portal_users(state)
        .acting_for(&state.db, user.id, party, today())
        .await?;
    acting.map(|_| ()).ok_or(Error::NotFound)
}

/// `POST /scheduling/portal/bookings`: a portal user books a slot for the
/// party they act for (`SCH.SLF.01`). Fields: `party`, `service`,
/// `starts_at` (`2026-11-03T10:30`, Asia/Dubai) and optionally `resource`
/// (the first free one of the service otherwise).
///
/// A service that needs acceptance is requested and holds the slot for the
/// request hold time. When another booker took the last seat first the answer
/// is 409, with the nearest free slots (up to three) and the waiting list
/// (`POST /scheduling/portal/waiting-list`).
async fn portal_book(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let party = PartyId(number(&form, "party")?);
    acts_for(&state, &user, party).await?;
    let service = ServiceType::find_or_404(&state.db, number(&form, "service")?).await?;
    let start = parse_local(field(&form, "starts_at"))
        .ok_or_else(|| Error::BadRequest("starts_at".to_owned()))?;
    let at = renox::db::now();
    let resources = match field(&form, "resource").parse::<i64>() {
        Ok(id) => vec![id],
        Err(_) => service::pick_resources(&state, &service, start, at)
            .await
            .map_err(|refusal| refusal.into_error(&lang))?,
    };
    let new = NewBooking::new(party, service.id, &resources, start).source("portal");
    let actor = Actor::party(Some(user.id));
    let made = if service.requires_acceptance {
        service::request(&state, new, actor, at).await
    } else {
        service::book(&state, new, actor, at).await
    };
    made.map_err(|refusal| refusal.into_error(&lang))?;
    Ok((
        Toast::success(texts::t(&lang, "wl_scheduling.booking.booked")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /scheduling/portal/waiting-list`: joins the waiting list. Fields:
/// `party`, `service`, and optionally `resource`, `window_from`,
/// `window_until` (`2026-11-03T09:00`) and `expires_on` (`2026-11-10`).
async fn portal_join(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let party = PartyId(number(&form, "party")?);
    acts_for(&state, &user, party).await?;
    let mut entry = NewEntry::new(party, number(&form, "service")?);
    if let Ok(resource) = field(&form, "resource").parse::<i64>() {
        entry = entry.with(resource);
    }
    entry.window_from = parse_local(field(&form, "window_from"));
    entry.window_until = parse_local(field(&form, "window_until"));
    entry.expires_on = field(&form, "expires_on").parse().ok();
    waiting::join(&state, entry)
        .await
        .map_err(|refusal| refusal.into_error(&lang))?;
    Ok((
        Toast::success(texts::t(&lang, "wl_scheduling.waiting.joined")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /scheduling/portal/waiting-list/{id}/withdraw`.
async fn portal_withdraw(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    let entry = WaitingEntry::find_or_404(&state.db, id).await?;
    acts_for(&state, &user, entry.party()).await?;
    waiting::withdraw(&state, id, Actor::party(Some(user.id)))
        .await
        .map_err(|refusal| refusal.into_error(&lang))?;
    Ok((
        Toast::success(texts::t(&lang, "wl_scheduling.waiting.withdrawn")),
        renox::HxRefresh,
    )
        .into_response())
}
