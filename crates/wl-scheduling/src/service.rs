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
use renox::state_machine::{StateMachine, TransitionError, TransitionRecord};
use wl_kernel::documents::dubai_date;
use wl_kernel::scheduling::{
    CancelStage, CancelledBy, CheckInAnswer, Eligibility, EligibilityQuery, Initiator, WorkDone,
    check_in_guard, complete_guard, resource_eligibility, work_done_status,
};
use wl_kernel::{PartyId, Subject};

use crate::booking::{
    BOOKING_SERIES, Booking, BookingResource, BookingStatus, PARTY_REQUEST, default_initiator,
};
use crate::booking_machine::{
    ACCEPT, ActorKind, BookingInput, CANCEL, CHECK_IN, COMPLETE, CONFIRM, DECLINE, MARK_NO_SHOW,
    Occupy, REASSIGN, REASSIGN_CONFIRMED, REASSIGN_REQUESTED, RESCHEDULE, RESCHEDULE_REQUEST,
    START,
};
use crate::config::config_of;
use crate::conflict;
use crate::deposit;
use crate::eligibility;
use crate::events;
use crate::notices;
use crate::prepayment;
use crate::refusal::Refusal;
use crate::resource::{Resource, ServiceResource, ServiceType};
use crate::slots::{self, Exclude, Unfit};
use crate::texts::english;

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

async fn service_of(
    state: &AppState,
    booking: &Booking,
) -> std::result::Result<ServiceType, Refusal> {
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

/// Asks whether every resource may deliver the service at `at`: the booking's
/// start, or now at check-in (`SCH.ACL.04`). First what the service type
/// requires of it ([`eligibility::failures`]: the qualification kinds with
/// `qualification_valid`, the document kinds with `document_valid`), then the
/// vertical's [`ResourceEligibility`](wl_kernel::scheduling::ResourceEligibility)
/// (the default says yes). There is no override.
pub(crate) async fn check_eligible(
    state: &AppState,
    service: &ServiceType,
    resources: &[Resource],
    at: DateTime,
) -> std::result::Result<(), Refusal> {
    check_eligible_with(state, service, resources, at, None).await
}

/// [`check_eligible`] whose refusal ends with the hint of the text `hint`
/// (`reassign first` at check-in, `cancel with resource_ineligible and rebook`
/// at start).
pub(crate) async fn check_eligible_with(
    state: &AppState,
    service: &ServiceType,
    resources: &[Resource],
    at: DateTime,
    hint: Option<&str>,
) -> std::result::Result<(), Refusal> {
    let strategy = resource_eligibility(state);
    for resource in resources {
        if let Some(first) = eligibility::failures(state, service, resource, at)
            .await?
            .first()
        {
            return Err(Refusal::Ineligible(with_hint(first.text(), hint)));
        }
        let query = EligibilityQuery {
            resource_id: resource.id,
            employee_id: resource.employee_id,
            service_type: service.code.clone(),
            location: resource.location(),
            at,
        };
        if let Eligibility::Refused(text) = strategy.check(state, query).await? {
            return Err(Refusal::Ineligible(with_hint(text, hint)));
        }
    }
    Ok(())
}

/// `text; hint`.
fn with_hint(text: String, hint: Option<&str>) -> String {
    match hint {
        Some(key) => format!("{text}; {}", english(key, &[])),
        None => text,
    }
}

/// What a party may be told of an ineligible resource: nothing about why.
fn neutral_for(actor: Actor, refusal: Refusal) -> Refusal {
    match refusal {
        Refusal::Ineligible(_) if actor.kind == ActorKind::Party => {
            Refusal::invalid("wl_scheduling.problem.not_available")
        }
        other => other,
    }
}

/// What the refused booker is told: the slot is closed, blocked or taken.
pub(crate) fn refusal_for(unfit: Unfit) -> Refusal {
    match unfit {
        Unfit::Taken => Refusal::Taken {
            nearest: Vec::new(),
            waiting_list: false,
        },
        other => Refusal::Unavailable(other),
    }
}

/// What the booker who lost the slot is told (§3 *No double-booking*): the
/// slot was just taken, here are the nearest free ones, or join the waiting
/// list. A closed or blocked resource is told as it is.
async fn refusal_after(
    state: &AppState,
    service: &ServiceType,
    resources: &[Resource],
    start: DateTime,
    at: DateTime,
    unfit: Unfit,
) -> std::result::Result<Refusal, Refusal> {
    Ok(match unfit {
        Unfit::Taken => Refusal::Taken {
            nearest: slots::nearest(state, service, resources, start, at, slots::SHOWN).await?,
            waiting_list: true,
        },
        other => Refusal::Unavailable(other),
    })
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
    create(state, new, actor, at, Kind::Book, Exclude::default()).await
}

/// A request for a service that needs staff acceptance (`request`). It holds
/// the slot for the request hold time.
pub async fn request(
    state: &AppState,
    new: NewBooking,
    actor: Actor,
    at: DateTime,
) -> std::result::Result<Booking, Refusal> {
    create(state, new, actor, at, Kind::Request, Exclude::default()).await
}

/// A party takes the slot a waiting-list entry was offered (§4.2 `accept`):
/// the offer's own hold does not stop its own booking.
pub(crate) async fn book_from_offer(
    state: &AppState,
    new: NewBooking,
    entry_id: i64,
    actor: Actor,
    at: DateTime,
) -> std::result::Result<Booking, Refusal> {
    let exclude = Exclude {
        entry: Some(entry_id),
        ..Exclude::default()
    };
    create(state, new, actor, at, Kind::Book, exclude).await
}

async fn create(
    state: &AppState,
    new: NewBooking,
    actor: Actor,
    at: DateTime,
    kind: Kind,
    exclude: Exclude,
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
    check_eligible(state, &service, &resources, start)
        .await
        .map_err(|refusal| neutral_for(actor, refusal))?;
    let number = BOOKING_SERIES
        .next_allowing_gaps(&state.db, None, None, dubai_date(at))
        .await?;
    let ids: Vec<i64> = resources.iter().map(|resource| resource.id).collect();
    let mut tx = state.db.begin_immediate().await?;
    let locked = slots::lock(&mut tx, &ids).await?;
    if let Err(unfit) =
        slots::check_free(&mut tx, &service, &locked, start, end, at, exclude).await?
    {
        tx.rollback().await?;
        return Err(refusal_after(state, &service, &locked, start, at, unfit).await?);
    }
    let (status, reason, hold_until) = match kind {
        Kind::Book => (BookingStatus::Booked, "book", None),
        Kind::Request => (
            BookingStatus::Requested,
            "request",
            Some(at + TimeDelta::minutes(cfg.request_hold_minutes)),
        ),
    };
    // A party who books by themselves pays the prepayment first: the booking
    // holds its slot until the payment link expires (`SCH.RUL.04`).
    let due = if actor.kind == ActorKind::Party {
        prepayment::amount_due(&service)
    } else {
        0
    };
    let hold_until = if due > 0 {
        Some(prepayment::hold_until(at, &cfg, hold_until))
    } else {
        hold_until
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
        payment_pending: due > 0,
        created_by: actor.user,
        deposit_amount: service.deposit_amount,
        ..Booking::default()
    };
    // A booking starts in `booked` or `requested`, both initial states of the
    // machine: its history names the way it began (`book`, `request`) where
    // `insert_in_state` would write the framework's `insert`.
    booking.insert(&mut tx).await?;
    let mut began = TransitionRecord::new(Booking::TABLE, booking.id, status.to_string(), reason);
    if let Some(user) = actor.user {
        began = began.actor(user);
    }
    began.record(&mut tx).await?;
    insert_rows(
        &mut tx,
        booking.id,
        &occupy_rows(&service, &locked, start, end),
    )
    .await?;
    tx.commit().await?;
    let mut booking = Booking::find_or_404(&state.db, booking.id).await?;
    if due > 0 {
        booking = prepayment::open_link(state, &booking, due, at).await?;
    }
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
    let mut tx = state.db.begin_immediate().await?;
    let locked = slots::lock(&mut tx, &ids).await?;
    let exclude = Exclude {
        booking: Some(booking.id),
        ..Exclude::default()
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
        return Err(refusal_after(state, &service, &locked, booking.starts_at, at, unfit).await?);
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
    events::cancelled(state, &done, cancelled_by(actor)).await?;
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
    reschedule_with(state, id, new_start, new_resources, actor, at, false).await
}

/// The party picks another time from a message about a change the business
/// made (*pick another time*, `reassign`, `BookingConflict`): the booking
/// moves with the same resources, and neither the reschedule cut-off nor the
/// minimum notice of a self-booking applies, as the party did not cause the
/// change. The slot is checked again under the lock and the conflict clears.
pub async fn reschedule_from_change(
    state: &AppState,
    id: i64,
    new_start: DateTime,
    at: DateTime,
) -> std::result::Result<Booking, Refusal> {
    reschedule_with(state, id, new_start, None, Actor::party(None), at, true).await
}

async fn reschedule_with(
    state: &AppState,
    id: i64,
    new_start: DateTime,
    new_resources: Option<Vec<i64>>,
    actor: Actor,
    at: DateTime,
    change_link: bool,
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
        if !change_link && !hours_before(new_start, at, cfg.min_notice_hours) {
            return Err(Refusal::invalid("wl_scheduling.problem.too_soon"));
        }
        if new_start - at > TimeDelta::days(cfg.horizon_days) {
            return Err(Refusal::invalid("wl_scheduling.problem.too_far"));
        }
    }
    let resources = resolve_resources(state, &service, &wanted).await?;
    check_eligible(state, &service, &resources, new_start)
        .await
        .map_err(|refusal| neutral_for(actor, refusal))?;
    let ids: Vec<i64> = resources.iter().map(|resource| resource.id).collect();
    let mut tx = state.db.begin_immediate().await?;
    let locked = slots::lock(&mut tx, &ids).await?;
    let exclude = Exclude {
        booking: Some(old.id),
        ..Exclude::default()
    };
    if let Err(unfit) =
        slots::check_free(&mut tx, &service, &locked, new_start, end, at, exclude).await?
    {
        tx.rollback().await?;
        return Err(refusal_after(state, &service, &locked, new_start, at, unfit).await?);
    }
    let mut input = input_for(state, actor, at, &service);
    input.within_cutoff =
        change_link || hours_before(old.starts_at, at, service.reschedule_cutoff_hours);
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
    conflict::close_all(&state.db, id, "reschedule", at).await?;
    let moved = Booking::find_or_404(&state.db, id).await?;
    events::rescheduled(state, &moved, &old, old_ids.clone(), ids).await?;
    events::slot_released(state, &service, old_ids, old.starts_at, old.ends_at).await?;
    Ok(moved)
}

/// Asks again whether the booking's resources may deliver the service at `at`
/// (check-in and start, `SCH.ACL.04`): a licence may have lapsed since the
/// booking was made. `hint` says what staff do next.
async fn recheck_eligible(
    state: &AppState,
    booking: &Booking,
    service: &ServiceType,
    at: DateTime,
    hint: &str,
) -> std::result::Result<(), Refusal> {
    let ids = booking.resource_ids(&state.db).await?;
    let mut resources = Vec::new();
    for id in ids {
        if let Some(resource) = Resource::find(&state.db, id).await? {
            resources.push(resource);
        }
    }
    check_eligible_with(state, service, &resources, at, Some(hint)).await
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
    // The resource must be eligible now; otherwise reassign first.
    recheck_eligible(
        state,
        &booking,
        &service,
        at,
        "wl_scheduling.eligibility.hint_reassign",
    )
    .await?;
    // The vertical may want more before the party is in (05: the coverage
    // outcome of an insured visit, `CLN.ACL.15`).
    if let CheckInAnswer::Refused(text) = check_in_guard(state)
        .check(state, booking.booking_id(), at)
        .await?
    {
        return Err(Refusal::Invalid(text));
    }
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
    // From `checked_in` there is no reassign: cancel and rebook.
    recheck_eligible(
        state,
        &booking,
        &service,
        at,
        "wl_scheduling.eligibility.hint_rebook",
    )
    .await?;
    let input = input_for(state, actor, at, &service);
    let done = run_simple(state, &booking, START, &input, actor, None).await?;
    events::started(state, &done).await?;
    Ok(done)
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
    // The vertical may want more before the service is over (05: every
    // lot-tracked consumable of the visit has its lot, `STK.LOT.01`).
    if booking.status.is_after_check_in()
        && let CheckInAnswer::Refused(text) = complete_guard(state)
            .check(state, booking.booking_id(), at)
            .await?
    {
        return Err(Refusal::Invalid(text));
    }
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
    let marked = run_simple(state, &booking, MARK_NO_SHOW, &input, actor, None).await?;
    // The deposit is forfeited (`SCH.ACL.05`).
    let settled = deposit::settle_no_show(state, &marked).await;
    let done = settled.as_ref().map_or(marked, Clone::clone);
    events::no_show(state, &done, &service).await?;
    events::slot_released(state, &service, ids, booking.starts_at, booking.ends_at).await?;
    settled
}

/// What a cancellation says.
#[derive(Debug, Clone)]
pub struct CancelRequest {
    /// The reason code. A party cancelling from the portal or an ordinary
    /// link gives none (the code is `party_request`); from a message about a
    /// change the business made it is the code of that change.
    pub reason_code: String,
    /// The reason text, if any. Required to record the other side than the
    /// code's default.
    pub note: Option<String>,
    /// Staff record this side instead of the code's default (needs a note).
    pub initiated_by: Option<Initiator>,
    /// The party cancels from a message about a change the business made
    /// (a reassign, a conflict): it is recorded as initiated by `business`
    /// and the cancellation cut-off does not apply.
    pub change_link: bool,
}

impl CancelRequest {
    /// A cancellation with this reason code.
    pub fn new(reason_code: &str) -> Self {
        CancelRequest {
            reason_code: reason_code.to_owned(),
            note: None,
            initiated_by: None,
            change_link: false,
        }
    }

    /// The party's own cancellation from the portal or an ordinary link:
    /// `party_request`, initiated by the party.
    pub fn by_party() -> Self {
        CancelRequest::new(PARTY_REQUEST)
    }

    /// A party cancelling from a message about the change `reason_code`
    /// (e.g. `resource_unavailable`): initiated by the business, also after
    /// the cut-off.
    pub fn from_change_message(reason_code: &str) -> Self {
        CancelRequest {
            change_link: true,
            ..CancelRequest::new(reason_code)
        }
    }

    /// With this reason text.
    pub fn note(mut self, note: &str) -> Self {
        self.note = Some(note.to_owned());
        self
    }

    /// Records `initiated_by` instead of the code's default (needs a note).
    pub fn initiated_by(mut self, initiated_by: Initiator) -> Self {
        self.initiated_by = Some(initiated_by);
        self
    }
}

fn cancelled_by(actor: Actor) -> CancelledBy {
    match actor.kind {
        ActorKind::Party => CancelledBy::Party,
        ActorKind::Staff => CancelledBy::Staff,
        ActorKind::System => CancelledBy::System,
    }
}

/// Whether a cancellation at `at` is *late* (§4.1): less than the deposit
/// forfeit window before the start; for a service type without a deposit,
/// less than the cancellation cut-off.
pub fn is_late(service: &ServiceType, start: DateTime, at: DateTime) -> bool {
    let window = if service.has_deposit() {
        service.deposit_forfeit_hours
    } else {
        service.cancel_cutoff_hours
    };
    !hours_before(start, at, window)
}

/// Cancels a booking (`cancel`) with a reason code (§4.1). The slot is
/// released and offered to the waiting list; the reason, who initiated it,
/// the stage and *late* are recorded and sent with `BookingCancelled`.
///
/// - A party's cancellation records `party_request`, initiated by the party,
///   and only before the cancellation cut-off, never after check-in. From a
///   change message ([`CancelRequest::from_change_message`]) it records the
///   code of the change, initiated by the business, at any time before the
///   check-in.
/// - Staff give the code; the side is the code's default unless they record
///   the other with a note.
pub async fn cancel(
    state: &AppState,
    id: i64,
    request: CancelRequest,
    actor: Actor,
    at: DateTime,
) -> std::result::Result<Booking, Refusal> {
    let cfg = config_of(state);
    let booking = Booking::find_or_404(&state.db, id).await?;
    let service = service_of(state, &booking).await?;
    let ids = booking.resource_ids(&state.db).await?;
    let mut input = input_for(state, actor, at, &service);
    let (code, initiated_by) = match actor.kind {
        ActorKind::Party if request.change_link => {
            (request.reason_code.clone(), Some(Initiator::Business))
        }
        ActorKind::Party => (PARTY_REQUEST.to_owned(), Some(Initiator::Party)),
        _ => {
            let default = default_initiator(&request.reason_code, &cfg.extra_reason_codes);
            (
                request.reason_code.clone(),
                request.initiated_by.or(default),
            )
        }
    };
    input.default_initiated_by = default_initiator(&code, &cfg.extra_reason_codes);
    input.reason_code = Some(code.clone());
    input.initiated_by = initiated_by;
    input.note = request.note.clone();
    input.change_link = request.change_link && actor.kind == ActorKind::Party;
    input.within_cutoff = hours_before(booking.starts_at, at, service.cancel_cutoff_hours);
    input.late = is_late(&service, booking.starts_at, at);
    input.stage = Some(match booking.status {
        BookingStatus::CheckedIn => CancelStage::CheckedIn,
        BookingStatus::InProgress => CancelStage::InProgress,
        _ => CancelStage::BeforeStart,
    });
    // After the check-in the vertical's record says what was done; while it
    // is pending the machine refuses (`SCH.ACL.06`).
    if booking.status.is_after_check_in() {
        let work = work_done_status(state)
            .status(state, booking.booking_id(), booking.work_done())
            .await?;
        match work {
            WorkDone::Pending => input.work_done_pending = true,
            WorkDone::Nothing => {}
            WorkDone::Captured { amount } => input.work_done_amount = amount.max(0),
        }
    }
    let cancelled = run_simple(state, &booking, CANCEL, &input, actor, Some(&code)).await?;
    conflict::close_all(&state.db, id, "cancel", at).await?;
    // The deposit follows `SCH.ACL.05`. A refund `wl-payments` refuses does
    // not undo the cancellation: the events still go out and the refusal is
    // returned after them.
    let settled = deposit::settle_cancelled(state, &cancelled, &service, actor.user).await;
    let done = settled.as_ref().map_or(cancelled, Clone::clone);
    events::cancelled(state, &done, cancelled_by(actor)).await?;
    // After check-in only the time that is left is freed.
    let from = if booking.status.is_after_check_in() {
        booking.starts_at.max(at)
    } else {
        booking.starts_at
    };
    events::slot_released(state, &service, ids, from, booking.ends_at).await?;
    settled
}

/// Staff move a booking to another resource at the same time (`reassign`):
/// `to` takes the place of `from`, the booking keeps its state, `from`'s slot
/// is released (`SlotReleased`), the conflict is cleared and the party is told
/// who it is with now, with *accept*, *pick another time* and *cancel* links.
/// The reason code (`reason_code`, one of the cancellation codes, for example
/// `resource_unavailable`) goes to the event and the history, never to the
/// party. A party who cancels from the message does so as the business.
///
/// The new resource is locked like a booking (`SCH.BKG.01`) and must be open,
/// free for the whole time and eligible at the booking's start.
///
/// # Errors
///
/// A party as actor; a booking that is not `requested`, `booked` or
/// `confirmed`; `from` not on the booking or `to` already on it; a resource
/// that cannot deliver the service; a busy or ineligible `to`; an unknown
/// reason code.
pub async fn reassign(
    state: &AppState,
    id: i64,
    from: i64,
    to: i64,
    reason_code: &str,
    actor: Actor,
    at: DateTime,
) -> std::result::Result<Booking, Refusal> {
    let (moved, who) = reassign_quietly(state, id, from, to, reason_code, actor, at).await?;
    notices::reassigned(state, &moved, &who, reason_code, at).await?;
    Ok(moved)
}

/// [`reassign`] without the message to the party: the booking, and the name of
/// the resource that took over. A bulk action tells each party once, for all
/// their bookings ([`crate::conflict::bulk_resolve`]).
pub(crate) async fn reassign_quietly(
    state: &AppState,
    id: i64,
    from: i64,
    to: i64,
    reason_code: &str,
    actor: Actor,
    at: DateTime,
) -> std::result::Result<(Booking, String), Refusal> {
    staff_only(actor)?;
    let cfg = config_of(state);
    let booking = Booking::find_or_404(&state.db, id).await?;
    let service = service_of(state, &booking).await?;
    let name = match booking.status {
        BookingStatus::Requested => REASSIGN_REQUESTED,
        BookingStatus::Booked => REASSIGN,
        BookingStatus::Confirmed => REASSIGN_CONFIRMED,
        other => {
            return Err(Refusal::Transition(TransitionError::NotFrom(
                other.as_str().to_owned(),
            )));
        }
    };
    let old_ids = booking.resource_ids(&state.db).await?;
    if !old_ids.contains(&from) {
        return Err(Refusal::invalid("wl_scheduling.problem.not_on_booking"));
    }
    if to == from || old_ids.contains(&to) {
        return Err(Refusal::invalid("wl_scheduling.problem.already_on_booking"));
    }
    let mut new_ids: Vec<i64> = old_ids.iter().copied().filter(|r| *r != from).collect();
    new_ids.push(to);
    new_ids.sort_unstable();
    let resources = resolve_resources(state, &service, &new_ids).await?;
    let incoming: Vec<Resource> = resources
        .iter()
        .filter(|resource| resource.id == to)
        .cloned()
        .collect();
    check_eligible(state, &service, &incoming, booking.starts_at).await?;
    let mut tx = state.db.begin_immediate().await?;
    let locked = slots::lock(&mut tx, &[to]).await?;
    let exclude = Exclude {
        booking: Some(booking.id),
        ..Exclude::default()
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
        return Err(refusal_after(state, &service, &locked, booking.starts_at, at, unfit).await?);
    }
    let mut input = input_for(state, actor, at, &service);
    input.reason_code = Some(reason_code.to_owned());
    input.default_initiated_by = default_initiator(reason_code, &cfg.extra_reason_codes);
    input.reassign_from = Some(from);
    input.occupy = occupy_rows(&service, &locked, booking.starts_at, booking.ends_at);
    let outcome = booking
        .transition(name)
        .by_system()
        .input(&input)
        .meta(actor.meta())
        .reason(reason_code)
        .run_in(&mut tx)
        .await?;
    tx.commit().await?;
    outcome.emit(state).await?;
    conflict::close_all(&state.db, id, "reassign", at).await?;
    let moved = Booking::find_or_404(&state.db, id).await?;
    events::reassigned(state, &moved, old_ids, new_ids, reason_code).await?;
    events::slot_released(
        state,
        &service,
        vec![from],
        booking.starts_at,
        booking.ends_at,
    )
    .await?;
    let who = incoming
        .first()
        .map(|resource| resource.name.clone())
        .unwrap_or_default();
    Ok((moved, who))
}

/// Links the record a vertical opened at check-in (a visit, a job card) to
/// the booking, so that `BookingCancelled` names it and
/// [`WorkDoneStatus`](wl_kernel::scheduling::WorkDoneStatus) is asked about
/// it.
pub async fn link_work_done(db: &Db, id: i64, record: &Subject) -> Result {
    Booking::query()
        .where_(Booking::ID.eq(id))
        .update(
            db,
            &[
                ("work_done_type", &Some(record.subject_type.clone())),
                ("work_done_id", &Some(record.subject_id)),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

/// The resources for a booking whose booker named none (the portal): the
/// first resource, by id, of the one type the service needs that is free at
/// `start`; when none is free, the first one, so that the refusal tells the
/// booker what is near. A service that needs several resources has no such
/// default.
pub async fn pick_resources(
    state: &AppState,
    service: &ServiceType,
    start: DateTime,
    at: DateTime,
) -> std::result::Result<Vec<i64>, Refusal> {
    let required = service.required();
    let [kind] = required.as_slice() else {
        return Err(Refusal::invalid("wl_scheduling.problem.choose_resource"));
    };
    let candidates: Vec<Resource> = crate::resource::eligible_resources(&state.db, service.id)
        .await?
        .into_iter()
        .filter(|resource| resource.resource_type == *kind)
        .collect();
    let Some(first) = candidates.first() else {
        return Err(Refusal::invalid("wl_scheduling.problem.no_resource"));
    };
    let end = start + TimeDelta::minutes(service.duration_minutes);
    let mut tx = state.db.begin().await?;
    let mut chosen = first.id;
    for candidate in &candidates {
        let free = slots::check_free(
            &mut tx,
            service,
            std::slice::from_ref(candidate),
            start,
            end,
            at,
            Exclude::default(),
        )
        .await?;
        if free.is_ok() {
            chosen = candidate.id;
            break;
        }
    }
    tx.rollback().await?;
    Ok(vec![chosen])
}
