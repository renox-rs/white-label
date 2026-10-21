//! The staff pages and actions of bookings (wl-scheduling §5.1).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /scheduling/bookings` | `wl_scheduling.bookings` | `bookings.view` |
//! | `GET /scheduling/bookings/{id}` | `wl_scheduling.booking` | `bookings.view` |
//! | `POST /scheduling/bookings` | `wl_scheduling.booking_create` | `bookings.manage` |
//! | `POST /scheduling/bookings/{id}/accept` | `wl_scheduling.booking_accept` | `bookings.manage` |
//! | `POST /scheduling/bookings/{id}/decline` | `wl_scheduling.booking_decline` | `bookings.manage` |
//! | `POST /scheduling/bookings/{id}/confirm` | `wl_scheduling.booking_confirm` | `bookings.manage` |
//! | `POST /scheduling/bookings/{id}/reschedule` | `wl_scheduling.booking_reschedule` | `bookings.manage` |
//! | `POST /scheduling/bookings/{id}/cancel` | `wl_scheduling.booking_cancel` | `bookings.manage` |
//! | `POST /scheduling/bookings/{id}/mark-no-show` | `wl_scheduling.booking_no_show` | `bookings.manage` |
//! | `POST /scheduling/bookings/{id}/complete` | `wl_scheduling.booking_complete` | `bookings.manage` |
//! | `POST /scheduling/bookings/{id}/check-in` | `wl_scheduling.booking_check_in` | `bookings.checkin` |
//! | `POST /scheduling/bookings/{id}/start` | `wl_scheduling.booking_start` | `bookings.checkin` |
//!
//! Decision: the permissions are global; a scoped grant (a front desk of one
//! branch) is a later refinement.

use std::collections::BTreeMap;

use renox::chrono::NaiveDateTime;
use renox::db::DateTime;
use renox::prelude::*;
use serde::Serialize;
use wl_kernel::calendar::BusinessCalendar;
use wl_kernel::documents::dubai_text;
use wl_kernel::scheduling::Initiator;
use wl_kernel::{PartyId, Subject};

use crate::booking::{Booking, BookingStatus, REASON_CODES, WALKED_OUT};
use crate::resource::{Resource, ServiceType};
use crate::service::{self, Actor, CancelRequest, NewBooking};
use crate::{permissions, texts};

/// The texts a template reads as `tx['bookings.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .get("/scheduling/bookings", index)
                .name("wl_scheduling.bookings")
                .get("/scheduling/bookings/{id}", show)
                .name("wl_scheduling.booking")
                .require_permission(permissions::VIEW),
        )
        .merge(
            Routes::new()
                .post("/scheduling/bookings", create)
                .name("wl_scheduling.booking_create")
                .post("/scheduling/bookings/{id}/accept", accept)
                .name("wl_scheduling.booking_accept")
                .post("/scheduling/bookings/{id}/decline", decline)
                .name("wl_scheduling.booking_decline")
                .post("/scheduling/bookings/{id}/confirm", confirm)
                .name("wl_scheduling.booking_confirm")
                .post("/scheduling/bookings/{id}/reschedule", reschedule)
                .name("wl_scheduling.booking_reschedule")
                .post("/scheduling/bookings/{id}/cancel", cancel)
                .name("wl_scheduling.booking_cancel")
                .post("/scheduling/bookings/{id}/mark-no-show", no_show)
                .name("wl_scheduling.booking_no_show")
                .post("/scheduling/bookings/{id}/complete", complete)
                .name("wl_scheduling.booking_complete")
                .require_permission(permissions::MANAGE),
        )
        .merge(
            Routes::new()
                .post("/scheduling/bookings/{id}/check-in", check_in)
                .name("wl_scheduling.booking_check_in")
                .post("/scheduling/bookings/{id}/start", start)
                .name("wl_scheduling.booking_start")
                .require_permission(permissions::CHECKIN),
        )
}

fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_scheduling.{key}"))
}

/// The badge kind of a status.
pub(crate) fn status_kind(status: BookingStatus) -> &'static str {
    match status {
        BookingStatus::Completed => "success",
        BookingStatus::NoShow => "error",
        BookingStatus::Cancelled => "warning",
        _ => "info",
    }
}

/// The status in the page's language.
pub(crate) fn status_text(lang: &Lang, status: BookingStatus) -> String {
    text(lang, &format!("status.{}", status.as_str()))
}

/// A local time such as `2026-11-03T10:30` (Asia/Dubai) as an instant.
pub fn parse_local(text: &str) -> Option<DateTime> {
    let text = text.trim();
    ["%Y-%m-%dT%H:%M", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M"]
        .iter()
        .find_map(|format| NaiveDateTime::parse_from_str(text, format).ok())
        .map(|local| BusinessCalendar::default().from_local(local))
}

pub(crate) fn field<'a>(form: &'a BTreeMap<String, String>, name: &str) -> &'a str {
    form.get(name).map_or("", |value| value.trim())
}

pub(crate) fn number(form: &BTreeMap<String, String>, name: &str) -> Result<i64> {
    field(form, name)
        .parse()
        .map_err(|_| Error::BadRequest(name.to_owned()))
}

async fn party_name(state: &AppState, lang: &Lang, party: PartyId) -> String {
    wl_kernel::parties::directory(state)
        .display_name(&state.db, party, &lang.locale)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("#{}", party.0))
}

async fn resource_names(state: &AppState, booking: &Booking) -> Result<String> {
    let mut names = Vec::new();
    for id in booking.resource_ids(&state.db).await? {
        if let Some(resource) = Resource::find(&state.db, id).await? {
            names.push(resource.name);
        }
    }
    Ok(names.join(", "))
}

/// One booking in the list.
#[derive(Serialize, Debug, Clone)]
pub struct BookingRow {
    /// The primary key.
    pub id: i64,
    /// The number.
    pub number: String,
    /// Who booked.
    pub party: String,
    /// What is booked.
    pub service: String,
    /// The start, in Asia/Dubai.
    pub when: String,
    /// The resources.
    pub resources: String,
    /// The status in the page's language.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-scheduling/bookings.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    bookings: Vec<BookingRow>,
}

async fn row_of(state: &AppState, lang: &Lang, booking: &Booking) -> Result<BookingRow> {
    let service = ServiceType::find(&state.db, booking.service_type_id).await?;
    Ok(BookingRow {
        id: booking.id,
        number: booking.number.clone(),
        party: party_name(state, lang, booking.party()).await,
        service: service.map_or_else(String::new, |s| s.name_en),
        when: dubai_text(booking.starts_at),
        resources: resource_names(state, booking).await?,
        status: status_text(lang, booking.status),
        status_kind: status_kind(booking.status),
    })
}

/// `GET /scheduling/bookings`.
async fn index(State(state): State<AppState>, lang: Lang) -> Result<View> {
    let mut bookings = Vec::new();
    for booking in Booking::query()
        .order_by_desc(Booking::STARTS_AT)
        .limit(200)
        .get(&state.db)
        .await?
    {
        bookings.push(row_of(&state, &lang, &booking).await?);
    }
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        bookings,
    }))
}

/// What the booking page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-scheduling/booking.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    booking: BookingRow,
    ends: String,
    source: String,
    notes: String,
    cancel_reason: String,
    reasons: Vec<(String, String)>,
    sides: Vec<(String, String)>,
    can_manage: bool,
    can_checkin: bool,
    is_requested: bool,
    is_booked: bool,
    can_reschedule: bool,
    can_check_in: bool,
    can_start: bool,
    can_complete: bool,
    can_no_show: bool,
    can_cancel: bool,
    starts_input: String,
}

/// `GET /scheduling/bookings/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    let booking = Booking::find_or_404(&state.db, id).await?;
    let service = ServiceType::find_or_404(&state.db, booking.service_type_id).await?;
    let status = booking.status;
    let manage = user.has_permission(permissions::MANAGE);
    let checkin = user.has_permission(permissions::CHECKIN);
    // `walked_out` is offered only once the party is checked in (§4.1).
    let reasons = REASON_CODES
        .iter()
        .filter(|(code, _)| *code != WALKED_OUT || status.is_after_check_in())
        .map(|(code, _)| ((*code).to_owned(), text(&lang, &format!("reason.{code}"))))
        .collect();
    let sides = vec![
        (String::new(), text(&lang, "booking.side_default")),
        ("party".to_owned(), text(&lang, "booking.side_party")),
        ("business".to_owned(), text(&lang, "booking.side_business")),
    ];
    let local = BusinessCalendar::default().local(booking.starts_at);
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        booking: row_of(&state, &lang, &booking).await?,
        ends: dubai_text(booking.ends_at),
        source: booking.source.clone(),
        notes: booking.notes.clone().unwrap_or_default(),
        cancel_reason: booking
            .cancel_code
            .as_deref()
            .map(|code| text(&lang, &format!("reason.{code}")))
            .unwrap_or_default(),
        reasons,
        sides,
        can_manage: manage,
        can_checkin: checkin,
        is_requested: manage && status == BookingStatus::Requested,
        is_booked: manage && status == BookingStatus::Booked,
        can_reschedule: manage
            && matches!(
                status,
                BookingStatus::Requested | BookingStatus::Booked | BookingStatus::Confirmed
            ),
        can_check_in: checkin
            && service.check_in_required
            && matches!(status, BookingStatus::Booked | BookingStatus::Confirmed),
        can_start: checkin && service.track_in_progress && status == BookingStatus::CheckedIn,
        can_complete: manage
            && matches!(
                status,
                BookingStatus::Booked
                    | BookingStatus::Confirmed
                    | BookingStatus::CheckedIn
                    | BookingStatus::InProgress
            ),
        can_no_show: manage && matches!(status, BookingStatus::Booked | BookingStatus::Confirmed),
        can_cancel: manage && status.is_open(),
        starts_input: local.format("%Y-%m-%dT%H:%M").to_string(),
    }))
}

fn done(lang: &Lang, key: &str, id: i64) -> Result<Response> {
    Ok((
        Toast::success(text(lang, key)),
        Redirect::route("wl_scheduling.booking", &[&id])?,
    )
        .into_response())
}

/// `POST /scheduling/bookings`: books (or, with `kind=request`, requests) a
/// slot. Fields: `party`, `service`, `resources` (ids, comma separated),
/// `starts_at`, and optionally `subject_type`, `subject_id`, `notes`.
async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let resources: Vec<i64> = field(&form, "resources")
        .split(',')
        .filter_map(|id| id.trim().parse().ok())
        .collect();
    let starts_at = parse_local(field(&form, "starts_at"))
        .ok_or_else(|| Error::BadRequest("starts_at".to_owned()))?;
    let mut new = NewBooking::new(
        PartyId(number(&form, "party")?),
        number(&form, "service")?,
        &resources,
        starts_at,
    );
    if let Ok(subject) = format!(
        "{}:{}",
        field(&form, "subject_type"),
        field(&form, "subject_id")
    )
    .parse::<Subject>()
    {
        new = new.for_subject(subject);
    }
    let notes = field(&form, "notes");
    if !notes.is_empty() {
        new.notes = Some(notes.to_owned());
    }
    let actor = Actor::staff(user.id);
    let at = renox::db::now();
    let booking = if field(&form, "kind") == "request" {
        service::request(&state, new, actor, at).await
    } else {
        service::book(&state, new, actor, at).await
    }
    .map_err(|refusal| refusal.into_error(&lang))?;
    done(&lang, "booking.booked", booking.id)
}

/// `POST /scheduling/bookings/{id}/accept`.
async fn accept(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    service::accept(&state, id, Actor::staff(user.id), renox::db::now())
        .await
        .map_err(|refusal| refusal.into_error(&lang))?;
    done(&lang, "booking.accepted", id)
}

/// `POST /scheduling/bookings/{id}/decline`: `reason` is required.
async fn decline(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    service::decline(
        &state,
        id,
        field(&form, "reason"),
        Actor::staff(user.id),
        renox::db::now(),
    )
    .await
    .map_err(|refusal| refusal.into_error(&lang))?;
    done(&lang, "booking.declined", id)
}

/// `POST /scheduling/bookings/{id}/confirm`.
async fn confirm(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    service::confirm(&state, id, Actor::staff(user.id), renox::db::now())
        .await
        .map_err(|refusal| refusal.into_error(&lang))?;
    done(&lang, "booking.confirmed", id)
}

/// `POST /scheduling/bookings/{id}/reschedule`: `starts_at`, and optionally
/// `resources`.
async fn reschedule(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let starts_at = parse_local(field(&form, "starts_at"))
        .ok_or_else(|| Error::BadRequest("starts_at".to_owned()))?;
    let resources: Vec<i64> = field(&form, "resources")
        .split(',')
        .filter_map(|id| id.trim().parse().ok())
        .collect();
    let resources = (!resources.is_empty()).then_some(resources);
    service::reschedule(
        &state,
        id,
        starts_at,
        resources,
        Actor::staff(user.id),
        renox::db::now(),
    )
    .await
    .map_err(|refusal| refusal.into_error(&lang))?;
    done(&lang, "booking.rescheduled", id)
}

/// `POST /scheduling/bookings/{id}/cancel`: `reason_code` (and `note`).
async fn cancel(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let mut request = CancelRequest::new(field(&form, "reason_code"));
    let note = field(&form, "note");
    if !note.is_empty() {
        request = request.note(note);
    }
    // Staff may record the other side than the code's default, with a note.
    if let Some(side) = Initiator::parse(field(&form, "initiated_by")) {
        request = request.initiated_by(side);
    }
    service::cancel(&state, id, request, Actor::staff(user.id), renox::db::now())
        .await
        .map_err(|refusal| refusal.into_error(&lang))?;
    done(&lang, "booking.cancelled", id)
}

/// `POST /scheduling/bookings/{id}/mark-no-show`.
async fn no_show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    service::mark_no_show(&state, id, Actor::staff(user.id), renox::db::now())
        .await
        .map_err(|refusal| refusal.into_error(&lang))?;
    done(&lang, "booking.no_show_done", id)
}

/// `POST /scheduling/bookings/{id}/complete`.
async fn complete(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    service::complete(&state, id, Actor::staff(user.id), renox::db::now())
        .await
        .map_err(|refusal| refusal.into_error(&lang))?;
    done(&lang, "booking.completed", id)
}

/// `POST /scheduling/bookings/{id}/check-in`.
async fn check_in(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    service::check_in(&state, id, Actor::staff(user.id), renox::db::now())
        .await
        .map_err(|refusal| refusal.into_error(&lang))?;
    done(&lang, "booking.checked_in", id)
}

/// `POST /scheduling/bookings/{id}/start`.
async fn start(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    service::start(&state, id, Actor::staff(user.id), renox::db::now())
        .await
        .map_err(|refusal| refusal.into_error(&lang))?;
    done(&lang, "booking.started", id)
}
