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
//! | `POST /scheduling/bookings/{id}/reassign` | `wl_scheduling.booking_reassign` | `bookings.manage` |
//! | `POST /scheduling/bookings/{id}/cancel` | `wl_scheduling.booking_cancel` | `bookings.manage` |
//! | `POST /scheduling/bookings/{id}/mark-no-show` | `wl_scheduling.booking_no_show` | `bookings.manage` |
//! | `POST /scheduling/bookings/{id}/complete` | `wl_scheduling.booking_complete` | `bookings.manage` |
//! | `POST /scheduling/bookings/{id}/waive-deposit` | `wl_scheduling.booking_waive_deposit` | `bookings.waive_deposit` |
//! | `POST /scheduling/bookings/{id}/check-in` | `wl_scheduling.booking_check_in` | `bookings.checkin` |
//! | `POST /scheduling/bookings/{id}/start` | `wl_scheduling.booking_start` | `bookings.checkin` |
//!
//! Decision: the permissions are global; a scoped grant (a front desk of one
//! branch) is a later refinement.

use std::collections::BTreeMap;

use renox::chrono::NaiveDateTime;
use renox::db::DateTime;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::json;
use serde::Serialize;
use wl_kernel::calendar::BusinessCalendar;
use wl_kernel::documents::dubai_text;
use wl_kernel::scheduling::Initiator;
use wl_kernel::{PartyId, Subject};

use crate::booking::{Booking, BookingStatus, REASON_CODES, WALKED_OUT};
use crate::deposit::{self, money_text};
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
                .post("/scheduling/bookings/{id}/reassign", reassign)
                .name("wl_scheduling.booking_reassign")
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
                .post("/scheduling/bookings/{id}/waive-deposit", waive_deposit)
                .name("wl_scheduling.booking_waive_deposit")
                .require_permission(permissions::WAIVE_DEPOSIT),
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

/// The local text of an instant (`2026-11-03T10:30`, Asia/Dubai), as a form
/// field and [`parse_local`] read it.
pub fn local_text(at: DateTime) -> String {
    BusinessCalendar::default()
        .local(at)
        .format("%Y-%m-%dT%H:%M")
        .to_string()
}

pub(crate) fn field<'a>(form: &'a BTreeMap<String, String>, name: &str) -> &'a str {
    form.get(name).map_or("", |value| value.trim())
}

/// A form field that is missing or wrong, as a 422 the sheet shows in words
/// (never the name of the field).
pub(crate) fn bad_field(lang: &Lang, name: &str) -> Error {
    let key = format!("wl_scheduling.problem.field.{name}");
    let message = texts::t(lang, &key);
    let message = if message == key {
        texts::t(lang, "wl_scheduling.problem.field")
    } else {
        message
    };
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message)
}

pub(crate) fn number(form: &BTreeMap<String, String>, name: &str, lang: &Lang) -> Result<i64> {
    field(form, name).parse().map_err(|_| bad_field(lang, name))
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
    bookings: GridPage<Booking>,
    can_manage: bool,
}

/// The first resource of a booking, as a scalar subquery (`{T}` is the
/// bookings table).
const FIRST_RESOURCE: &str = "(SELECT \"r\".\"name\" FROM \"wl_scheduling_booking_resources\" \"br\" \
     JOIN \"wl_scheduling_resources\" \"r\" ON \"r\".\"id\" = \"br\".\"resource_id\" \
     WHERE \"br\".\"booking_id\" = {T}.\"id\" ORDER BY \"br\".\"id\" LIMIT 1)";

/// The bookings grid: search on the number and resource,
/// filters by service, status and start. All the resources of a booking are
/// drawn by the template (`custom`); the search and the filter read the first.
fn bookings_grid(lang: &Lang, services: Vec<(String, String)>, can_export: bool) -> Grid {
    let statuses: Vec<(String, String)> = BookingStatus::ALL
        .iter()
        .map(|status| (status.as_str().to_owned(), status_text(lang, *status)))
        .collect();
    let grid = Grid::new("scheduling-bookings")
        .title(&text(lang, "bookings.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("number", &text(lang, "bookings.number"))
                .frozen()
                .mobile()
                .searchable()
                .copyable()
                .link("/scheduling/bookings/{id}"),
        )
        // The party's name comes from the kernel's directory (the parties module
        // is not a dependency), so the template draws it.
        .column(Column::custom("party", &text(lang, "bookings.party")).mobile())
        .column(Column::number("party_id", &text(lang, "bookings.party_id")).hidden())
        .column(
            Column::select("service_type_id", &text(lang, "bookings.service"), services).mobile(),
        )
        .column(Column::datetime("starts_at", &text(lang, "bookings.when")).mobile())
        .column(
            Column::text("resource_name", &text(lang, "bookings.first_resource"))
                .computed(FIRST_RESOURCE)
                .searchable()
                .hidden(),
        )
        .column(Column::custom(
            "resources",
            &text(lang, "bookings.resources"),
        ))
        .column(
            Column::select("status", &text(lang, "fields.status"), statuses)
                .badges(&[
                    ("requested", "info"),
                    ("booked", "info"),
                    ("confirmed", "info"),
                    ("checked_in", "info"),
                    ("in_progress", "info"),
                    ("completed", "success"),
                    ("no_show", "danger"),
                    ("cancelled", "warning"),
                ])
                .mobile(),
        )
        .column(Column::datetime("ends_at", &text(lang, "bookings.ends")).hidden())
        .column(Column::text("source", &text(lang, "bookings.source")).hidden())
        .sort_by("-starts_at")
        .row_url("/scheduling/bookings/{id}")
        .cards_on_mobile()
        .empty_state(
            &text(lang, "bookings.empty"),
            Some(&text(lang, "bookings.empty_hint")),
        );
    if can_export { grid.exports() } else { grid }
}

async fn row_of(state: &AppState, lang: &Lang, booking: &Booking) -> Result<BookingRow> {
    let service = ServiceType::find(&state.db, booking.service_type_id).await?;
    Ok(BookingRow {
        id: booking.id,
        number: booking.number.clone(),
        party: party_name(state, lang, booking.party()).await,
        service: service.map_or_else(String::new, |s| crate::names::name_in(&s, &lang.locale)),
        when: dubai_text(booking.starts_at),
        resources: resource_names(state, booking).await?,
        status: status_text(lang, booking.status),
        status_kind: status_kind(booking.status),
    })
}

/// `GET /scheduling/bookings`: every booking, newest start first, searched,
/// filtered, sorted and paged by the grid.
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    let can_manage = user.has_permission(permissions::MANAGE);
    let services = ServiceType::query()
        .order_by(ServiceType::NAME_EN)
        .get(&state.db)
        .await?
        .iter()
        .map(|s| (s.id.to_string(), crate::names::name_in(s, &lang.locale)))
        .collect();
    let grid = bookings_grid(&lang, services, can_manage);
    if request.param("export").is_some() && !can_manage {
        return Err(Error::Forbidden);
    }
    let query = Booking::query();
    if let Some(file) = grid.export(query.clone(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let mut details = BTreeMap::new();
    for booking in page.items() {
        details.insert(
            booking.id,
            (
                party_name(&state, &lang, booking.party()).await,
                resource_names(&state, booking).await?,
            ),
        );
    }
    let bookings = page.extend(|booking| {
        let found = details.get(&booking.id);
        json!({
            "party": found.map(|d| d.0.as_str()),
            "resources": found.map(|d| d.1.as_str()),
        })
    });
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        bookings,
        can_manage,
    })
    .into_response())
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
    deposit: String,
    work_done: String,
    can_waive: bool,
    can_reassign: bool,
    on_booking: Vec<(String, String)>,
    others: Vec<(String, String)>,
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
    // The resources a reassign can replace, and those that could take over.
    let held = booking.resource_ids(&state.db).await?;
    let mut on_booking = Vec::new();
    for id in &held {
        if let Some(resource) = Resource::find(&state.db, *id).await? {
            on_booking.push((id.to_string(), resource.name));
        }
    }
    let others: Vec<(String, String)> = crate::resource::eligible_resources(&state.db, service.id)
        .await?
        .into_iter()
        .filter(|resource| !held.contains(&resource.id))
        .map(|resource| (resource.id.to_string(), resource.name))
        .collect();
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
        starts_input: local_text(booking.starts_at),
        deposit: deposit_text(&lang, &booking),
        work_done: work_done_text(&booking),
        // Offered only when another resource could take the booking over.
        can_reassign: manage
            && !others.is_empty()
            && matches!(
                status,
                BookingStatus::Requested | BookingStatus::Booked | BookingStatus::Confirmed
            ),
        on_booking,
        others,
        can_waive: user.has_permission(permissions::WAIVE_DEPOSIT) && booking.deposit_forfeited > 0,
    }))
}

/// The deposit in one line: the amount and what became of it, empty for a
/// booking without one.
fn deposit_text(lang: &Lang, booking: &Booking) -> String {
    if booking.deposit_amount <= 0 {
        return String::new();
    }
    let state = match booking.deposit_outcome.as_deref() {
        Some(outcome) => text(lang, &format!("deposit.{outcome}")),
        None => text(lang, "deposit.held"),
    };
    let mut line = format!("{} · {state}", money_text(booking.deposit_amount));
    if booking.deposit_applied > 0 {
        line.push_str(" · ");
        line.push_str(&texts::t_with(
            lang,
            "wl_scheduling.deposit.applied_part",
            &[("amount", &money_text(booking.deposit_applied))],
        ));
    }
    line
}

/// The work done before a cancellation after the check-in, empty otherwise.
fn work_done_text(booking: &Booking) -> String {
    let after_check_in = matches!(
        booking.cancel_stage.as_deref(),
        Some("checked_in" | "in_progress")
    );
    if after_check_in {
        money_text(booking.work_done_amount)
    } else {
        String::new()
    }
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
    let starts_at =
        parse_local(field(&form, "starts_at")).ok_or_else(|| bad_field(&lang, "starts_at"))?;
    let mut new = NewBooking::new(
        PartyId(number(&form, "party", &lang)?),
        number(&form, "service", &lang)?,
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
    let starts_at =
        parse_local(field(&form, "starts_at")).ok_or_else(|| bad_field(&lang, "starts_at"))?;
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

/// `POST /scheduling/bookings/{id}/reassign`: `from` (a resource of the
/// booking), `to` (the one that takes over) and `reason_code`.
async fn reassign(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    service::reassign(
        &state,
        id,
        number(&form, "from", &lang)?,
        number(&form, "to", &lang)?,
        field(&form, "reason_code"),
        Actor::staff(user.id),
        renox::db::now(),
    )
    .await
    .map_err(|refusal| refusal.into_error(&lang))?;
    done(&lang, "booking.reassigned", id)
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

/// `POST /scheduling/bookings/{id}/waive-deposit`: `reason` is required; the
/// route needs `bookings.waive_deposit`.
async fn waive_deposit(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    deposit::waive(&state, id, field(&form, "reason"), Actor::staff(user.id))
        .await
        .map_err(|refusal| refusal.into_error(&lang))?;
    done(&lang, "deposit.waived", id)
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
