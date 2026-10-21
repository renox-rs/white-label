//! The booking services (wl-scheduling §4.1, §5.1, §5.2): one function per
//! action. Each checks what the machine cannot (the slot, the service type's
//! rules, the cut-offs), then runs the transition of
//! [`crate::booking_machine`] and, after the commit, emits the events.
//!
//! Who may act is decided by the caller: the staff routes sit behind
//! `bookings.manage` or `bookings.checkin`, the portal routes check the grant
//! of the party, the link routes check the signature. The services take an
//! [`Actor`] and refuse only what an actor of that kind may never do (a party
//! accepting a request).
//!
//! Every function that reads or writes the slots takes the moment as `at`
//! (routes pass [`renox::db::now`]), which makes them testable.

use renox::chrono::TimeDelta;
use renox::db::{DateTime, Transaction};
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::documents::dubai_date;
use wl_kernel::scheduling::{Eligibility, EligibilityQuery, resource_eligibility};
use wl_kernel::{PartyId, Subject};

use crate::booking::{BOOKING_SERIES, Booking, BookingResource, BookingStatus};
use crate::booking_machine::{
    ACCEPT, ActorKind, BookingInput, CANCEL, CHECK_IN, COMPLETE, CONFIRM, DECLINE, MARK_NO_SHOW,
    Occupy, RESCHEDULE, RESCHEDULE_REQUEST, START,
};
use crate::config::config_of;
use crate::events;
use crate::refusal::Refusal;
use crate::resource::{Resource, ServiceResource, ServiceType};
use crate::slots::{self, Exclude, Unfit};

/// Who runs a service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Actor {
    /// Staff, the party or the system.
    pub kind: ActorKind,
    /// The user, when a user acts.
    pub user: Option<i64>,
}

impl Actor {
    /// A member of staff.
    pub fn staff(user: i64) -> Self {
        Actor {
            kind: ActorKind::Staff,
            user: Some(user),
        }
    }

    /// The party: a portal user, or nobody for a signed link.
    pub fn party(user: Option<i64>) -> Self {
        Actor {
            kind: ActorKind::Party,
            user,
        }
    }

    /// A timer or a listener.
    pub fn system() -> Self {
        Actor {
            kind: ActorKind::System,
            user: None,
        }
    }

    fn meta(&self) -> renox::serde_json::Value {
        json!({ "actor": self.kind.as_str(), "user": self.user })
    }
}

/// A booking to make.
#[derive(Debug, Clone)]
pub struct NewBooking {
    /// Who books.
    pub party: PartyId,
    /// The patient, vehicle or student, if any.
    pub subject: Option<Subject>,
    /// What is booked.
    pub service_type_id: i64,
    /// The resources, one of each type the service type needs.
    pub resource_ids: Vec<i64>,
    /// The start.
    pub starts_at: DateTime,
    /// `staff`, `portal`, `website` or `phone`.
    pub source: String,
    /// Notes of the person who books.
    pub notes: Option<String>,
    /// The branch, workshop or campus.
    pub location: Option<Subject>,
    /// The link of an online meeting.
    pub meeting_link: Option<String>,
}

impl NewBooking {
    /// A booking by staff for `party` of `service_type_id` with these
    /// resources, from `starts_at`.
    pub fn new(
        party: PartyId,
        service_type_id: i64,
        resource_ids: &[i64],
        starts_at: DateTime,
    ) -> Self {
        NewBooking {
            party,
            subject: None,
            service_type_id,
            resource_ids: resource_ids.to_vec(),
            starts_at,
            source: "staff".to_owned(),
            notes: None,
            location: None,
            meeting_link: None,
        }
    }

    /// For this patient, vehicle or student.
    pub fn for_subject(mut self, subject: Subject) -> Self {
        self.subject = Some(subject);
        self
    }

    /// Made through this channel (`portal`, `website`, `phone`).
    pub fn source(mut self, source: &str) -> Self {
        self.source = source.to_owned();
        self
    }

    /// At this location.
    pub fn at(mut self, location: Subject) -> Self {
        self.location = Some(location);
        self
    }
}

/// Whether the booking asked for staff acceptance (`request`) or is made
/// (`book`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Book,
    Request,
}

/// Whether `at` is at least `hours` before `start`.
pub fn hours_before(start: DateTime, at: DateTime, hours: i64) -> bool {
    start - at >= TimeDelta::hours(hours)
}

fn staff_only(actor: Actor) -> std::result::Result<(), Refusal> {
    if actor.kind == ActorKind::Party {
        Err(Refusal::Transition(TransitionError::Forbidden))
    } else {
        Ok(())
    }
}

async fn service_of(state: &AppState, booking: &Booking) -> std::result::Result<ServiceType, Refusal> {
    Ok(ServiceType::find_or_404(&state.db, booking.service_type_id).await?)
}

fn input_for(state: &AppState, actor: Actor, at: DateTime, service: &ServiceType) -> BookingInput {
    BookingInput {
        actor: actor.kind,
        actor_user: actor.user,
        now: Some(at),
        check_in_required: service.check_in_required,
        track_in_progress: service.track_in_progress,
        grace_minutes: config_of(state).no_show_grace_minutes,
        ..BookingInput::default()
    }
}

/// The resources `ids` for `service`: active, offered for it, covering the
/// resource types it needs, in the order of their ids.
async fn resolve_resources(
    state: &AppState,
    service: &ServiceType,
    ids: &[i64],
) -> std::result::Result<Vec<Resource>, Refusal> {
    let mut ids = ids.to_vec();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Err(Refusal::invalid("wl_scheduling.problem.no_resource"));
    }
    let mut found = Vec::new();
    for id in ids {
        let resource = Resource::find(&state.db, id)
            .await?
            .filter(|resource| resource.active)
            .ok_or_else(|| Refusal::invalid("wl_scheduling.problem.unknown_resource"))?;
        let offered = ServiceResource::query()
            .where_(ServiceResource::SERVICE_TYPE_ID.eq(service.id))
            .where_(ServiceResource::RESOURCE_ID.eq(resource.id))
            .exists(&state.db)
            .await?;
        if !offered {
            return Err(Refusal::invalid("wl_scheduling.problem.not_offered"));
        }
        found.push(resource);
    }
    let required = service.required();
    let covers = required
        .iter()
        .all(|kind| found.iter().any(|resource| resource.resource_type == *kind));
    let only_required = found
        .iter()
        .all(|resource| required.contains(&resource.resource_type));
    if !covers || !only_required {
        return Err(Refusal::invalid("wl_scheduling.problem.wrong_resources"));
    }
    Ok(found)
}

/// Asks the vertical's [`ResourceEligibility`](wl_kernel::scheduling::ResourceEligibility)
/// for every resource at `at`: the booking's start, or now at check-in
/// (`SCH.ACL.04`). The default says yes.
async fn check_eligible(
    state: &AppState,
    service: &ServiceType,
    resources: &[Resource],
    at: DateTime,
) -> std::result::Result<(), Refusal> {
    let strategy = resource_eligibility(state);
    for resource in resources {
        let query = EligibilityQuery {
            resource_id: resource.id,
            employee_id: resource.employee_id,
            service_type: service.code.clone(),
            location: resource.location(),
            at,
        };
        if let Eligibility::Refused(text) = strategy.check(state, query).await? {
            return Err(Refusal::Ineligible(text));
        }
    }
    Ok(())
}

/// What the refused booker is told: the slot is closed, blocked or taken.
fn refusal_for(unfit: Unfit) -> Refusal {
    match unfit {
        Unfit::Taken => Refusal::Taken {
            nearest: Vec::new(),
            waiting_list: false,
        },
        other => Refusal::Unavailable(other),
    }
}

/// The occupied time of each of `resources` for a booking of `service`.
fn occupy_rows(
    service: &ServiceType,
    resources: &[Resource],
    start: DateTime,
    end: DateTime,
) -> Vec<Occupy> {
    let span = slots::occupied_span(service, start, end);
    resources
        .iter()
        .map(|resource| Occupy {
            resource_id: resource.id,
            from: span.from,
            until: span.until,
        })
        .collect()
}

/// A staff member books (`book`), or a party books a service that needs no
/// acceptance. The slot must be free for all required resources.
pub async fn book(
    state: &AppState,
    new: NewBooking,
    actor: Actor,
    at: DateTime,
) -> std::result::Result<Booking, Refusal> {
    create(state, new, actor, at, Kind::Book).await
}

/// A request for a service that needs staff acceptance (`request`). It holds
/// the slot for the request hold time.
pub async fn request(
    state: &AppState,
    new: NewBooking,
    actor: Actor,
    at: DateTime,
) -> std::result::Result<Booking, Refusal> {
    create(state, new, actor, at, Kind::Request).await
}

async fn create(
    state: &AppState,
    new: NewBooking,
    actor: Actor,
    at: DateTime,
    kind: Kind,
) -> std::result::Result<Booking, Refusal> {
    let cfg = config_of(state);
    let service = ServiceType::find(&state.db, new.service_type_id)
        .await?
        .filter(|service| service.active)
        .ok_or_else(|| Refusal::invalid("wl_scheduling.problem.unknown_service"))?;
    match kind {
        Kind::Request if !service.requires_acceptance => {
            return Err(Refusal::invalid("wl_scheduling.problem.no_request_needed"));
        }
        Kind::Book if service.requires_acceptance && actor.kind == ActorKind::Party => {
            return Err(Refusal::invalid("wl_scheduling.problem.request_needed"));
        }
        _ => {}
    }
    if actor.kind == ActorKind::Party && !service.self_bookable {
        return Err(Refusal::invalid("wl_scheduling.problem.not_self_bookable"));
    }
    let start = new.starts_at;
    let end = start + TimeDelta::minutes(service.duration_minutes);
    if start < at {
        return Err(Refusal::invalid("wl_scheduling.problem.in_the_past"));
    }
    if actor.kind == ActorKind::Party {
        if !hours_before(start, at, cfg.min_notice_hours) {
            return Err(Refusal::invalid("wl_scheduling.problem.too_soon"));
        }
        if start - at > TimeDelta::days(cfg.horizon_days) {
            return Err(Refusal::invalid("wl_scheduling.problem.too_far"));
        }
    }
    let resources = resolve_resources(state, &service, &new.resource_ids).await?;
    check_eligible(state, &service, &resources, start).await?;
    let number = BOOKING_SERIES
        .next_allowing_gaps(&state.db, None, None, dubai_date(at))
        .await?;
    let ids: Vec<i64> = resources.iter().map(|resource| resource.id).collect();
    let mut tx = state.db.begin().await?;
    let locked = slots::lock(&mut tx, &ids).await?;
    if let Err(unfit) =
        slots::check_free(&mut tx, &service, &locked, start, end, at, Exclude::default()).await?
    {
        tx.rollback().await?;
        return Err(refusal_for(unfit));
    }
    let (status, reason, hold_until) = match kind {
        Kind::Book => (BookingStatus::Booked, "book", None),
        Kind::Request => (
            BookingStatus::Requested,
            "request",
            Some(at + TimeDelta::minutes(cfg.request_hold_minutes)),
        ),
    };
    let mut booking = Booking {
        number,
        party_id: new.party.0,
        subject_type: new.subject.as_ref().map(|s| s.subject_type.clone()),
        subject_id: new.subject.as_ref().map(|s| s.subject_id),
        service_type_id: service.id,
        location_type: new.location.as_ref().map(|s| s.subject_type.clone()),
        location_id: new.location.as_ref().map(|s| s.subject_id),
        meeting_link: new.meeting_link.clone(),
        starts_at: start,
        ends_at: end,
        source: new.source.clone(),
        notes: new.notes.clone(),
        status,
        hold_until,
        created_by: actor.user,
        deposit_amount: service.deposit_amount,
        ..Booking::default()
    };
    booking.insert_in_state(&mut tx, status, reason).await?;
    insert_rows(&mut tx, booking.id, &occupy_rows(&service, &locked, start, end)).await?;
    tx.commit().await?;
    let booking = Booking::find_or_404(&state.db, booking.id).await?;
    if kind == Kind::Book {
        events::booked(state, &booking, &service, ids).await?;
    }
    Ok(booking)
}

async fn insert_rows(
    tx: &mut Transaction,
    booking_id: i64,
    rows: &[Occupy],
) -> std::result::Result<(), Refusal> {
    for row in rows {
        BookingResource {
            booking_id,
            resource_id: row.resource_id,
            occupies_from: row.from,
            occupies_until: row.until,
            released: false,
            ..BookingResource::default()
        }
        .save(&mut *tx)
        .await?;
    }
    Ok(())
}

/// Runs `name` on `booking` in its own transaction, then emits the
/// transition's event.
async fn run_simple(
    state: &AppState,
    booking: &Booking,
    name: &str,
    input: &BookingInput,
    actor: Actor,
    reason: Option<&str>,
) -> std::result::Result<Booking, Refusal> {
    let mut tx = state.db.begin().await?;
    let mut run = booking
        .transition(name)
        .by_system()
        .input(input)
        .meta(actor.meta());
    if let Some(reason) = reason {
        run = run.reason(reason);
    }
    let outcome = run.run_in(&mut tx).await?;
    tx.commit().await?;
    outcome.emit(state).await?;
    Ok(Booking::find_or_404(&state.db, booking.id).await?)
}

/// Staff accept a `requested` booking (`accept`). The slot is checked again
/// under the lock: a hold that lapsed no longer protects it.
pub async fn accept(
    state: &AppState,
    id: i64,
    actor: Actor,
    at: DateTime,
) -> std::result::Result<Booking, Refusal> {
    staff_only(actor)?;
    let booking = Booking::find_or_404(&state.db, id).await?;
    let service = service_of(state, &booking).await?;
    let ids = booking.resource_ids(&state.db).await?;
    let resources = resolve_resources(state, &service, &ids).await?;
    check_eligible(state, &service, &resources, booking.starts_at).await?;
    let mut tx = state.db.begin().await?;
    let locked = slots::lock(&mut tx, &ids).await?;
    let exclude = Exclude {
        booking: Some(booking.id),
    };
    if let Err(unfit) = slots::check_free(
        &mut tx,
        &service,
        &locked,
        booking.starts_at,
        booking.ends_at,
        at,
        exclude,
    )
    .await?
    {
        tx.rollback().await?;
        return Err(refusal_for(unfit));
    }
    let input = input_for(state, actor, at, &service);
    let outcome = booking
        .transition(ACCEPT)
        .by_system()
        .input(&input)
        .meta(actor.meta())
        .run_in(&mut tx)
        .await?;
    tx.commit().await?;
    outcome.emit(state).await?;
    let booking = Booking::find_or_404(&state.db, id).await?;
    events::booked(state, &booking, &service, ids).await?;
    Ok(booking)
}

/// Staff decline a `requested` booking with a reason (`decline`); the slot is
/// released and offered to the waiting list.
pub async fn decline(
    state: &AppState,
    id: i64,
    reason: &str,
    actor: Actor,
    at: DateTime,
) -> std::result::Result<Booking, Refusal> {
    staff_only(actor)?;
    let booking = Booking::find_or_404(&state.db, id).await?;
    let service = service_of(state, &booking).await?;
    let ids = booking.resource_ids(&state.db).await?;
    let mut input = input_for(state, actor, at, &service);
    input.note = Some(reason.to_owned());
    let done = run_simple(state, &booking, DECLINE, &input, actor, Some(reason)).await?;
    events::slot_released(state, &service, ids, booking.starts_at, booking.ends_at).await?;
    Ok(done)
}

/// The party (or staff for them) confirms they will come (`confirm`).
pub async fn confirm(
    state: &AppState,
    id: i64,
    actor: Actor,
    at: DateTime,
) -> std::result::Result<Booking, Refusal> {
    let booking = Booking::find_or_404(&state.db, id).await?;
    let service = service_of(state, &booking).await?;
    let input = input_for(state, actor, at, &service);
    run_simple(state, &booking, CONFIRM, &input, actor, None).await
}

/// Moves a booking to `new_start` (`reschedule`), with the same resources or
/// `new_resources`. A party may do it before the reschedule cut-off, staff
/// always. The old slot is released; the booking returns to `booked`, or to
/// `requested` when the service type needs acceptance.
pub async fn reschedule(
    state: &AppState,
    id: i64,
    new_start: DateTime,
    new_resources: Option<Vec<i64>>,
    actor: Actor,
    at: DateTime,
) -> std::result::Result<Booking, Refusal> {
    let cfg = config_of(state);
    let old = Booking::find_or_404(&state.db, id).await?;
    let service = service_of(state, &old).await?;
    if !old.status.is_open() || old.status.is_after_check_in() {
        return Err(Refusal::Transition(TransitionError::NotFrom(
            old.status.as_str().to_owned(),
        )));
    }
    let old_ids = old.resource_ids(&state.db).await?;
    let wanted = new_resources.unwrap_or_else(|| old_ids.clone());
    let end = new_start + TimeDelta::minutes(service.duration_minutes);
    if new_start < at {
        return Err(Refusal::invalid("wl_scheduling.problem.in_the_past"));
    }
    if actor.kind == ActorKind::Party {
        if !hours_before(new_start, at, cfg.min_notice_hours) {
            return Err(Refusal::invalid("wl_scheduling.problem.too_soon"));
        }
        if new_start - at > TimeDelta::days(cfg.horizon_days) {
            return Err(Refusal::invalid("wl_scheduling.problem.too_far"));
        }
    }
    let resources = resolve_resources(state, &service, &wanted).await?;
    check_eligible(state, &service, &resources, new_start).await?;
    let ids: Vec<i64> = resources.iter().map(|resource| resource.id).collect();
    let mut tx = state.db.begin().await?;
    let locked = slots::lock(&mut tx, &ids).await?;
    let exclude = Exclude {
        booking: Some(old.id),
    };
    if let Err(unfit) =
        slots::check_free(&mut tx, &service, &locked, new_start, end, at, exclude).await?
    {
        tx.rollback().await?;
        return Err(refusal_for(unfit));
    }
    let mut input = input_for(state, actor, at, &service);
    input.within_cutoff = hours_before(old.starts_at, at, service.reschedule_cutoff_hours);
    input.new_start = Some(new_start);
    input.new_end = Some(end);
    input.occupy = occupy_rows(&service, &locked, new_start, end);
    let to_request = service.requires_acceptance || old.status == BookingStatus::Requested;
    if to_request {
        input.hold_until = Some(at + TimeDelta::minutes(cfg.request_hold_minutes));
    }
    let name = if to_request {
        RESCHEDULE_REQUEST
    } else {
        RESCHEDULE
    };
    let outcome = old
        .transition(name)
        .by_system()
        .input(&input)
        .meta(actor.meta())
        .run_in(&mut tx)
        .await?;
    tx.commit().await?;
    outcome.emit(state).await?;
    let moved = Booking::find_or_404(&state.db, id).await?;
    events::rescheduled(state, &moved, &old, old_ids.clone(), ids).await?;
    events::slot_released(state, &service, old_ids, old.starts_at, old.ends_at).await?;
    Ok(moved)
}

/// Checks the party in (`check_in`); `bookings.checkin`.
pub async fn check_in(
    state: &AppState,
    id: i64,
    actor: Actor,
    at: DateTime,
) -> std::result::Result<Booking, Refusal> {
    staff_only(actor)?;
    let booking = Booking::find_or_404(&state.db, id).await?;
    let service = service_of(state, &booking).await?;
    let input = input_for(state, actor, at, &service);
    let done = run_simple(state, &booking, CHECK_IN, &input, actor, None).await?;
    events::checked_in(state, &done).await?;
    Ok(done)
}

/// Starts the service (`start`).
pub async fn start(
    state: &AppState,
    id: i64,
    actor: Actor,
    at: DateTime,
) -> std::result::Result<Booking, Refusal> {
    staff_only(actor)?;
    let booking = Booking::find_or_404(&state.db, id).await?;
    let service = service_of(state, &booking).await?;
    let input = input_for(state, actor, at, &service);
    run_simple(state, &booking, START, &input, actor, None).await
}

/// The service took place (`complete`): by staff, the resource, or the
/// system when a vertical record completes.
pub async fn complete(
    state: &AppState,
    id: i64,
    actor: Actor,
    at: DateTime,
) -> std::result::Result<Booking, Refusal> {
    staff_only(actor)?;
    let booking = Booking::find_or_404(&state.db, id).await?;
    let service = service_of(state, &booking).await?;
    let ids = booking.resource_ids(&state.db).await?;
    let input = input_for(state, actor, at, &service);
    let done = run_simple(state, &booking, COMPLETE, &input, actor, None).await?;
    events::completed(state, &done, &service, ids).await?;
    Ok(done)
}

/// The party did not come (`mark_no_show`): the slot is released.
pub async fn mark_no_show(
    state: &AppState,
    id: i64,
    actor: Actor,
    at: DateTime,
) -> std::result::Result<Booking, Refusal> {
    staff_only(actor)?;
    let booking = Booking::find_or_404(&state.db, id).await?;
    let service = service_of(state, &booking).await?;
    let ids = booking.resource_ids(&state.db).await?;
    let input = input_for(state, actor, at, &service);
    let done = run_simple(state, &booking, MARK_NO_SHOW, &input, actor, None).await?;
    events::no_show(state, &done, &service).await?;
    events::slot_released(state, &service, ids, booking.starts_at, booking.ends_at).await?;
    Ok(done)
}

/// What a cancellation says.
#[derive(Debug, Clone)]
pub struct CancelRequest {
    /// The reason code.
    pub reason_code: String,
    /// The reason text, if any.
    pub note: Option<String>,
}

impl CancelRequest {
    /// A cancellation with this reason code.
    pub fn new(reason_code: &str) -> Self {
        CancelRequest {
            reason_code: reason_code.to_owned(),
            note: None,
        }
    }

    /// With this reason text.
    pub fn note(mut self, note: &str) -> Self {
        self.note = Some(note.to_owned());
        self
    }
}

/// Cancels a booking (`cancel`) with a reason code. The slot is released and
/// offered to the waiting list.
pub async fn cancel(
    state: &AppState,
    id: i64,
    request: CancelRequest,
    actor: Actor,
    at: DateTime,
) -> std::result::Result<Booking, Refusal> {
    let booking = Booking::find_or_404(&state.db, id).await?;
    let service = service_of(state, &booking).await?;
    let ids = booking.resource_ids(&state.db).await?;
    let mut input = input_for(state, actor, at, &service);
    input.reason_code = Some(request.reason_code.clone());
    input.note = request.note.clone();
    let done = run_simple(
        state,
        &booking,
        CANCEL,
        &input,
        actor,
        Some(&request.reason_code),
    )
    .await?;
    events::slot_released(state, &service, ids, booking.starts_at, booking.ends_at).await?;
    Ok(done)
}
