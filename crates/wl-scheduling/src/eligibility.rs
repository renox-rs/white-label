//! Whether the resource of a booking holds what the service needs
//! (wl-scheduling §5.6, `SCH.ACL.04`, `SCH.RUL.05`; BL-SCH-07, BL-SCH-08).
//!
//! A service type lists the qualification kinds its person must hold
//! (`required_qualifications`, asked with `qualification_valid` of `wl-hr`
//! through the kernel's [`Qualifications`](wl_kernel::qualifications::Qualifications))
//! and the document kinds its resource must hold (`required_documents`, asked
//! with `document_valid` of `wl-documents` through the kernel's
//! [`DocumentGuard`](wl_kernel::documents::DocumentGuard)). [`failures`] answers
//! for one resource at one moment; `book`, `accept`, `reschedule`, `reassign`
//! (the booking's start), `check_in` and `start` (now) refuse a resource that
//! fails, whatever the vertical's own
//! [`ResourceEligibility`](wl_kernel::scheduling::ResourceEligibility) says.
//!
//! `SCH.RUL.05` ([`look_ahead`], [`flag_planned`], [`clear_renewed`]) looks at
//! the bookings that were made before the item lapsed:
//!
//! - the nightly run (02:00 Asia/Dubai, [`TASK`]) checks the bookings that
//!   start within `D` days (30 by default) at **their start**, never at today's
//!   date, flags each failing one with the conflict cause `eligibility` (one
//!   [`BookingConflict`](wl_kernel::scheduling::BookingConflict) per resource and
//!   item) and opens a task for the front desk to reassign or reschedule it;
//! - `DocumentExpiring`, `DocumentExpired`, `QualificationExpiring` and
//!   `QualificationExpired` do not search again: they take the `affected`
//!   (or `blocked`) records the event carries, apply the same window and the
//!   same check, so that the night and the event flag the same bookings once;
//! - a renewal (`DocumentAccepted`, or the nightly run) clears the conflict of
//!   the bookings that now pass at their start and resolves the task. A booking
//!   with a `block` conflict as well stays flagged for the block.
//!
//! Decisions:
//!
//! - **Item.** The conflict's `item` is the kind (`practitioner_licence`).
//!   The same kind on the same booking is flagged once.
//! - **Who is asked.** A qualification applies to `person` resources, as
//!   `qualification_valid(employee, kind, date, role, scope)` with the
//!   resource's `role` and location; a person without an employee fails. A
//!   document applies to the resource's employee, else its asset
//!   ([`ASSET_SUBJECT_TYPE`]); a room has no document.
//! - **Verified, not trusted.** An event only names the bookings to look at;
//!   each is checked again at its start, so a renewal that is already
//!   accepted flags nothing.
//! - **Providers.** The module registers the kernel's `BlockSubjects` and
//!   `FutureWork` for `booking.book` and `booking.check_in` ([`register`]):
//!   the subjects of a booking are its resources' employees and assets, and
//!   the planned work of a subject is its open bookings dated from a day.
//! - **Task.** The front desk is the holders of `bookings.manage` in the
//!   booking's location; the task goes to the first of them (unassigned when
//!   there is none) and is found again by its source key
//!   `scheduling.eligibility:<booking>:<item>`.
//! - **Party.** Nothing is said to the party when the conflict is raised; the
//!   change message of the reassign, or the escalation of `SCH.RUL.06`, never
//!   names the reason.

use std::collections::BTreeMap;

use renox::chrono::{NaiveDate, TimeDelta};
use renox::db::DateTime;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::documents::{
    self, AffectedGroup, BlockEntry, BlockSubjects, BlockedGroup, DocumentAccepted,
    DocumentExpired, DocumentExpiring, DocumentFuture, FutureWork, InvalidReason, PlannedRecord,
    Validity, dubai_date, dubai_text,
};
use wl_kernel::qualifications::qualifications;
use wl_kernel::scheduling::{BOOKING_SUBJECT_TYPE, ConflictCause};
use wl_kernel::tickets::{Assignee, OpenTicket, TASK_CATEGORY, task_creator};
use wl_kernel::workforce::{
    AffectedWork, EMPLOYEE_SUBJECT_TYPE, EmployeeId, QualificationExpired, QualificationExpiring,
};

use crate::booking::{Booking, BookingResource, BookingStatus};
use crate::config::config_of;
use crate::conflict::{Conflict, can_conflict, open_of, raise, set_flag};
use crate::resource::{Resource, ResourceType, ServiceType};
use crate::rules;
use crate::texts::english;

/// The scheduled task and command of the nightly look-ahead.
pub const TASK: &str = "scheduling:eligibility";

/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "02:00";

/// The time zone of [`TASK`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// The subject type of the asset behind a bay or a piece of equipment
/// (`wl-inventory`), whose documents a service may require.
pub const ASSET_SUBJECT_TYPE: &str = "inventory.asset";

/// The transitions of a booking whose resources are guarded (the
/// `<entity>.<transition>` the document kinds list in `blocks`).
pub const GUARDED_BOOK: &str = "booking.book";

/// The guarded `check_in`.
pub const GUARDED_CHECK_IN: &str = "booking.check_in";

/// How a required item fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// It was valid through this date.
    Expired(NaiveDate),
    /// There is none.
    Missing,
    /// There is one, but not for this date, role or branch (or a renewal
    /// still waits for review).
    NotValid,
}

/// One required item a resource does not hold at a moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// The resource.
    pub resource_id: i64,
    /// The kind of qualification or document, e.g. `practitioner_licence`.
    pub kind: String,
    /// The date it was asked for: the booking's start, or today.
    pub on: NaiveDate,
    /// How it fails.
    pub problem: Problem,
}

impl Failure {
    /// The kind as staff read it: `practitioner licence`.
    pub fn item(&self) -> String {
        self.kind.replace('_', " ")
    }

    /// What staff read: "practitioner licence expired on 20 Nov 2026". Never
    /// shown to the party.
    pub fn text(&self) -> String {
        let item = self.item();
        match &self.problem {
            Problem::Expired(date) => english(
                "wl_scheduling.eligibility.expired",
                &[("item", &item), ("date", &date_text(*date))],
            ),
            Problem::Missing => english("wl_scheduling.eligibility.missing", &[("item", &item)]),
            Problem::NotValid => english(
                "wl_scheduling.eligibility.not_valid",
                &[("item", &item), ("date", &date_text(self.on))],
            ),
        }
    }
}

/// `20 Nov 2026`.
fn date_text(date: NaiveDate) -> String {
    date.format("%d %b %Y").to_string()
}

/// The subject whose documents the resource must hold: its employee, else
/// its asset.
fn document_subject(resource: &Resource) -> Option<Subject> {
    match (resource.employee_id, resource.asset_id) {
        (Some(employee), _) => Some(EmployeeId(employee).subject()),
        (None, Some(asset)) => Some(Subject::new(ASSET_SUBJECT_TYPE, asset)),
        (None, None) => None,
    }
}

/// What `resource` lacks of what `service` requires, on the Dubai day of
/// `at`: the qualification kinds with `qualification_valid(employee, kind,
/// date, role, scope)` and the document kinds with `document_valid(subject,
/// kind, date)`. Empty when the service requires nothing.
///
/// # Errors
///
/// The guards' errors.
pub async fn failures(
    state: &AppState,
    service: &ServiceType,
    resource: &Resource,
    at: DateTime,
) -> Result<Vec<Failure>> {
    let on = dubai_date(at);
    let mut found = Vec::new();
    let wanted = service.qualification_kinds();
    if !wanted.is_empty() && resource.resource_type == ResourceType::Person {
        let guard = qualifications(state);
        let scope = resource.location();
        for kind in wanted {
            let problem = match resource.employee_id {
                None => Some(Problem::Missing),
                Some(employee) => {
                    let status = guard
                        .qualification_valid(
                            state,
                            EmployeeId(employee),
                            &kind,
                            on,
                            resource.role.as_deref(),
                            scope.as_ref(),
                        )
                        .await?;
                    if status.valid {
                        None
                    } else {
                        // The licence row's `valid_to` is for display; the
                        // document is what expired.
                        let expired = match status.valid_to {
                            Some(date) if date < on => Some(date),
                            _ => match documents::guard(state)
                                .is_valid(&state.db, &EmployeeId(employee).subject(), &kind, on)
                                .await?
                            {
                                Validity::Invalid {
                                    reason: InvalidReason::Expired,
                                    expired_on,
                                    ..
                                } => expired_on,
                                _ => None,
                            },
                        };
                        Some(match expired {
                            Some(date) => Problem::Expired(date),
                            None if status.number.is_none() => Problem::Missing,
                            None => Problem::NotValid,
                        })
                    }
                }
            };
            if let Some(problem) = problem {
                found.push(Failure {
                    resource_id: resource.id,
                    kind,
                    on,
                    problem,
                });
            }
        }
    }
    let wanted = service.document_kinds();
    if let Some(subject) = document_subject(resource).filter(|_| !wanted.is_empty()) {
        let guard = documents::guard(state);
        for kind in wanted {
            let validity = guard.is_valid(&state.db, &subject, &kind, on).await?;
            if let Validity::Invalid {
                reason, expired_on, ..
            } = validity
            {
                let problem = match (reason, expired_on) {
                    (InvalidReason::Expired, Some(date)) => Problem::Expired(date),
                    (InvalidReason::Missing, _) => Problem::Missing,
                    _ => Problem::NotValid,
                };
                found.push(Failure {
                    resource_id: resource.id,
                    kind,
                    on,
                    problem,
                });
            }
        }
    }
    Ok(found)
}

/// The failures of the resources `booking` holds, at its start; only those of
/// `kind` when it is given.
async fn booking_failures(
    state: &AppState,
    booking: &Booking,
    kind: Option<&str>,
) -> Result<Vec<Failure>> {
    let Some(service) = ServiceType::find(&state.db, booking.service_type_id).await? else {
        return Ok(Vec::new());
    };
    let mut found = Vec::new();
    for id in booking.resource_ids(&state.db).await? {
        if let Some(resource) = Resource::find(&state.db, id).await? {
            found.extend(failures(state, &service, &resource, booking.starts_at).await?);
        }
    }
    found.retain(|failure| kind.is_none_or(|kind| failure.kind == kind));
    Ok(found)
}

/// The task's source key.
fn task_key(booking: &Booking, item: &str) -> String {
    format!("scheduling.eligibility:{}:{}", booking.id, item)
}

/// Opens the front-desk task of a booking flagged for `item`.
async fn open_task(state: &AppState, booking: &Booking, item: &str) -> Result {
    let creator = task_creator(state);
    let key = task_key(booking, item);
    if creator.find_open(state, &key).await?.is_some() {
        return Ok(());
    }
    let when = dubai_text(booking.starts_at);
    let label = item.replace('_', " ");
    let title = english(
        "wl_scheduling.eligibility.task_title",
        &[("number", &booking.number), ("item", &label)],
    );
    let description = english(
        "wl_scheduling.eligibility.task_body",
        &[
            ("number", &booking.number),
            ("when", &when),
            ("item", &label),
        ],
    );
    let mut ticket = OpenTicket::new(booking.subject(), TASK_CATEGORY, &title)
        .description(&description)
        .due_at(booking.starts_at)
        .source_key(&key);
    if let Some(user) = rules::front_desk(state, booking).await?.first() {
        ticket = ticket.assign_to(Assignee::user(*user));
    }
    creator.open(state, ticket).await?;
    Ok(())
}

/// Resolves the front-desk task of `booking` for `item`, if one is open.
async fn resolve_task(state: &AppState, booking: &Booking, item: &str) -> Result {
    let creator = task_creator(state);
    if let Some(found) = creator.find_open(state, &task_key(booking, item)).await? {
        let note = english("wl_scheduling.eligibility.task_resolved", &[]);
        creator.resolve_from_source(state, found.id, &note).await?;
    }
    Ok(())
}

/// Flags the bookings among `bookings` that start inside the window of `D`
/// days after `at` and whose resource fails at their start (only for `kind`
/// when given), opens a task for each newly flagged one and returns how many
/// were flagged. A booking already flagged for the item is left as it is.
///
/// # Errors
///
/// A database error, the guards' or the listeners'.
pub async fn flag_bookings(
    state: &AppState,
    bookings: Vec<Booking>,
    kind: Option<&str>,
    at: DateTime,
) -> Result<usize> {
    let horizon = at + TimeDelta::days(config_of(state).look_ahead_days);
    let mut groups: BTreeMap<(i64, String), Vec<Booking>> = BTreeMap::new();
    for booking in bookings {
        if !can_conflict(booking.status) || booking.starts_at <= at || booking.starts_at > horizon {
            continue;
        }
        for failure in booking_failures(state, &booking, kind).await? {
            groups
                .entry((failure.resource_id, failure.kind))
                .or_default()
                .push(booking.clone());
        }
    }
    let mut flagged = 0;
    for ((resource_id, item), group) in groups {
        let newly = raise(
            state,
            resource_id,
            ConflictCause::Eligibility,
            None,
            Some(item.as_str()),
            &group,
            at,
        )
        .await?;
        for booking in &newly {
            open_task(state, booking, &item).await?;
            flagged += 1;
        }
    }
    Ok(flagged)
}

/// Flags the bookings named by the records of an event (`affected`,
/// `blocked`): the records that are bookings, looked up and checked again.
async fn flag_records(
    state: &AppState,
    records: impl IntoIterator<Item = Subject>,
    kind: &str,
    at: DateTime,
) -> Result<usize> {
    let mut bookings = Vec::new();
    for record in records {
        if record.subject_type == BOOKING_SUBJECT_TYPE
            && let Some(booking) = Booking::find(&state.db, record.subject_id).await?
        {
            bookings.push(booking);
        }
    }
    flag_bookings(state, bookings, Some(kind), at).await
}

/// `DocumentExpiring`: flags the bookings of its `affected` list.
///
/// # Errors
///
/// As [`flag_bookings`].
pub async fn on_document_expiring(
    state: &AppState,
    event: &DocumentExpiring,
    at: DateTime,
) -> Result<usize> {
    flag_records(state, planned_of(&event.affected), &event.kind, at).await
}

/// `DocumentExpired`: flags the bookings planned after the expiry date.
///
/// # Errors
///
/// As [`flag_bookings`].
pub async fn on_document_expired(
    state: &AppState,
    event: &DocumentExpired,
    at: DateTime,
) -> Result<usize> {
    flag_records(state, blocked_planned_of(&event.blocked), &event.kind, at).await
}

/// `QualificationExpiring` of `wl-hr`: flags the bookings it names.
///
/// # Errors
///
/// As [`flag_bookings`].
pub async fn on_qualification_expiring(
    state: &AppState,
    event: &QualificationExpiring,
    at: DateTime,
) -> Result<usize> {
    flag_records(state, work_of(&event.affected), &event.kind, at).await
}

/// `QualificationExpired` of `wl-hr`: flags the bookings it names.
///
/// # Errors
///
/// As [`flag_bookings`].
pub async fn on_qualification_expired(
    state: &AppState,
    event: &QualificationExpired,
    at: DateTime,
) -> Result<usize> {
    flag_records(state, work_of(&event.affected), &event.kind, at).await
}

/// The records of the booking groups of `groups`.
fn planned_of(groups: &[AffectedGroup]) -> Vec<Subject> {
    groups
        .iter()
        .filter(|group| group.guarded.starts_with("booking."))
        .flat_map(|group| group.records.iter().map(|record| record.record.clone()))
        .collect()
}

/// The planned records of the booking groups of `groups`.
fn blocked_planned_of(groups: &[BlockedGroup]) -> Vec<Subject> {
    groups
        .iter()
        .filter(|group| group.guarded.starts_with("booking."))
        .flat_map(|group| group.planned.iter().map(|record| record.record.clone()))
        .collect()
}

/// The planned (not running) records of `work`.
fn work_of(work: &[AffectedWork]) -> Vec<Subject> {
    work.iter()
        .filter(|item| item.guarded.starts_with("booking.") && !item.open)
        .map(|item| item.record.clone())
        .collect()
}

/// The open conflicts of the cause `eligibility`, optionally of one `kind`.
async fn open_eligibility(db: &Db, kind: Option<&str>) -> Result<Vec<Conflict>> {
    let rows = Conflict::query()
        .where_(Conflict::CAUSE.eq(ConflictCause::Eligibility.as_str()))
        .where_(Conflict::CLEARED_AT.is_null())
        .order_by(Conflict::ID)
        .get(db)
        .await?;
    Ok(rows
        .into_iter()
        .filter(|row| kind.is_none_or(|kind| row.item.as_deref() == Some(kind)))
        .collect())
}

/// Ends the open eligibility conflicts (of `kind` when given) whose item the
/// booking's resource now holds at the booking's start, and resolves their
/// task. The booking stays as it is; its flag drops when no eligibility
/// conflict is left. Returns how many were cleared.
///
/// # Errors
///
/// A database error or the guards'.
pub async fn clear_renewed(state: &AppState, kind: Option<&str>, at: DateTime) -> Result<usize> {
    let mut cleared = 0;
    for row in open_eligibility(&state.db, kind).await? {
        let Some(booking) = Booking::find(&state.db, row.booking_id).await? else {
            continue;
        };
        if !can_conflict(booking.status) {
            continue;
        }
        let item = row.item.clone().unwrap_or_default();
        let still = booking_failures(state, &booking, Some(&item))
            .await?
            .into_iter()
            .any(|failure| failure.resource_id == row.resource_id);
        if still {
            continue;
        }
        Conflict::query()
            .where_(Conflict::ID.eq(row.id))
            .update(
                &state.db,
                &[
                    ("cleared_at", &Some(at)),
                    ("cleared_by", &Some("renewed".to_owned())),
                ],
            )
            .await?;
        let left = open_of(&state.db, booking.id)
            .await?
            .iter()
            .any(|other| other.cause == ConflictCause::Eligibility.as_str());
        if !left {
            set_flag(&state.db, booking.id, ConflictCause::Eligibility, false).await?;
        }
        resolve_task(state, &booking, &item).await?;
        cleared += 1;
    }
    Ok(cleared)
}

/// What one nightly run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LookAhead {
    /// Bookings newly flagged.
    pub flagged: usize,
    /// Conflicts cleared because the item was renewed.
    pub cleared: usize,
}

/// `SCH.RUL.05` at `at`: checks the `requested`, `booked` and `confirmed`
/// bookings that start within the look-ahead at their start, flags those that
/// fail, and clears the conflicts whose item is held again.
///
/// # Errors
///
/// A database error, the guards' or the listeners'.
pub async fn look_ahead(state: &AppState, at: DateTime) -> Result<LookAhead> {
    let horizon = at + TimeDelta::days(config_of(state).look_ahead_days);
    let mut bookings = Vec::new();
    for status in [
        BookingStatus::Requested,
        BookingStatus::Booked,
        BookingStatus::Confirmed,
    ] {
        bookings.extend(
            Booking::query()
                .where_(Booking::STATUS.eq(status))
                .where_(Booking::STARTS_AT.gt(at))
                .where_(Booking::STARTS_AT.lte(horizon))
                .order_by(Booking::STARTS_AT)
                .get(&state.db)
                .await?,
        );
    }
    let flagged = flag_bookings(state, bookings, None, at).await?;
    let cleared = clear_renewed(state, None, at).await?;
    Ok(LookAhead { flagged, cleared })
}

/// The bookings in `statuses` held by the resources behind `subject` (their
/// employee or asset) whose service requires `kind`, from the Dubai day
/// `from` on, by start.
async fn bookings_of(
    state: &AppState,
    subject: &Subject,
    kind: &str,
    from: NaiveDate,
    statuses: &[BookingStatus],
) -> Result<Vec<Booking>> {
    let resources = if subject.subject_type == EMPLOYEE_SUBJECT_TYPE {
        Resource::query()
            .where_(Resource::EMPLOYEE_ID.eq(subject.subject_id))
            .get(&state.db)
            .await?
    } else if subject.subject_type == ASSET_SUBJECT_TYPE {
        Resource::query()
            .where_(Resource::ASSET_ID.eq(subject.subject_id))
            .get(&state.db)
            .await?
    } else {
        Vec::new()
    };
    let ids: Vec<i64> = resources.iter().map(|resource| resource.id).collect();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut booking_ids: Vec<i64> = BookingResource::query()
        .where_(BookingResource::RESOURCE_ID.is_in(ids))
        .where_(BookingResource::RELEASED.eq(false))
        .get(&state.db)
        .await?
        .into_iter()
        .map(|row| row.booking_id)
        .collect();
    booking_ids.sort_unstable();
    booking_ids.dedup();
    if booking_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut found = Vec::new();
    for booking in Booking::query()
        .where_(Booking::ID.is_in(booking_ids))
        .order_by(Booking::STARTS_AT)
        .get(&state.db)
        .await?
    {
        if !statuses.contains(&booking.status) || dubai_date(booking.starts_at) < from {
            continue;
        }
        let Some(service) = ServiceType::find(&state.db, booking.service_type_id).await? else {
            continue;
        };
        if service.qualification_kinds().iter().any(|k| k == kind)
            || service.document_kinds().iter().any(|k| k == kind)
        {
            found.push(booking);
        }
    }
    Ok(found)
}

/// The record a booking is in a [`PlannedRecord`].
fn planned_record(booking: &Booking) -> PlannedRecord {
    PlannedRecord {
        record: booking.subject(),
        date: dubai_date(booking.starts_at),
        label: format!("{} {}", booking.number, dubai_text(booking.starts_at)),
        scope: None,
    }
}

/// The `FutureWork` provider of the bookings (wl-documents §5.6): the open
/// bookings of a resource dated from a day, and those already running.
struct BookingWork;

impl FutureWork for BookingWork {
    fn planned_from<'a>(
        &'a self,
        state: &'a AppState,
        entry: &'a BlockEntry,
        subject: &'a Subject,
        from: NaiveDate,
    ) -> DocumentFuture<'a, Vec<PlannedRecord>> {
        Box::pin(async move {
            let found = bookings_of(
                state,
                subject,
                &entry.kind,
                from,
                &[
                    BookingStatus::Requested,
                    BookingStatus::Booked,
                    BookingStatus::Confirmed,
                ],
            )
            .await?;
            Ok(found.iter().map(planned_record).collect())
        })
    }

    fn open_for<'a>(
        &'a self,
        state: &'a AppState,
        entry: &'a BlockEntry,
        subject: &'a Subject,
    ) -> DocumentFuture<'a, Vec<PlannedRecord>> {
        Box::pin(async move {
            let found = bookings_of(
                state,
                subject,
                &entry.kind,
                NaiveDate::MIN,
                &[BookingStatus::CheckedIn, BookingStatus::InProgress],
            )
            .await?;
            Ok(found.iter().map(planned_record).collect())
        })
    }
}

/// The `BlockSubjects` provider of the bookings: the employees and assets
/// behind a booking's resources, for a kind its service requires.
struct BookingSubjects {
    /// Whether the transition books a slot (`booking.book`).
    bookable: bool,
}

impl BlockSubjects for BookingSubjects {
    fn subjects<'a>(
        &'a self,
        state: &'a AppState,
        entry: &'a BlockEntry,
        record_id: i64,
    ) -> DocumentFuture<'a, Vec<Subject>> {
        Box::pin(async move {
            let Some(booking) = Booking::find(&state.db, record_id).await? else {
                return Ok(Vec::new());
            };
            let Some(service) = ServiceType::find(&state.db, booking.service_type_id).await?
            else {
                return Ok(Vec::new());
            };
            let required = service.qualification_kinds().contains(&entry.kind)
                || service.document_kinds().contains(&entry.kind);
            if !required {
                return Ok(Vec::new());
            }
            let mut found = Vec::new();
            for id in booking.resource_ids(&state.db).await? {
                if let Some(resource) = Resource::find(&state.db, id).await?
                    && let Some(subject) = document_subject(&resource)
                    && !found.contains(&subject)
                {
                    found.push(subject);
                }
            }
            Ok(found)
        })
    }

    fn bookable(&self) -> bool {
        self.bookable
    }

    fn label<'a>(
        &'a self,
        state: &'a AppState,
        subject: &'a Subject,
    ) -> DocumentFuture<'a, Option<String>> {
        Box::pin(async move {
            let resource = if subject.subject_type == EMPLOYEE_SUBJECT_TYPE {
                Resource::query()
                    .where_(Resource::EMPLOYEE_ID.eq(subject.subject_id))
                    .first(&state.db)
                    .await?
            } else {
                Resource::query()
                    .where_(Resource::ASSET_ID.eq(subject.subject_id))
                    .first(&state.db)
                    .await?
            };
            Ok(resource.map(|resource| resource.name))
        })
    }
}

/// Registers the providers, the listeners of the expiry and renewal events
/// and the nightly task.
pub(crate) fn register(app: &mut Registry) {
    documents::register_block_subjects(GUARDED_BOOK, BookingSubjects { bookable: true });
    documents::register_block_subjects(GUARDED_CHECK_IN, BookingSubjects { bookable: false });
    documents::register_future_work(GUARDED_BOOK, BookingWork);
    documents::register_future_work(GUARDED_CHECK_IN, BookingWork);
    app.listen(|event: DocumentExpiring, state: AppState| async move {
        on_document_expiring(&state, &event, renox::db::now())
            .await
            .map(|_| ())
    });
    app.listen(|event: DocumentExpired, state: AppState| async move {
        on_document_expired(&state, &event, renox::db::now())
            .await
            .map(|_| ())
    });
    app.listen(|event: QualificationExpiring, state: AppState| async move {
        on_qualification_expiring(&state, &event, renox::db::now())
            .await
            .map(|_| ())
    });
    app.listen(|event: QualificationExpired, state: AppState| async move {
        on_qualification_expired(&state, &event, renox::db::now())
            .await
            .map(|_| ())
    });
    app.listen(|event: DocumentAccepted, state: AppState| async move {
        clear_renewed(&state, Some(&event.kind), renox::db::now())
            .await
            .map(|_| ())
    });
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            look_ahead(&state, renox::db::now()).await.map(|_| ())
        })
        .timezone(TASK_ZONE);
    app.command(
        TASK,
        "Flag the bookings whose resource will not hold a required licence or document at their start (SCH.RUL.05)",
        |_args, state: AppState| async move {
            let done = look_ahead(&state, renox::db::now()).await?;
            println!(
                "{} bookings flagged, {} conflicts cleared.",
                done.flagged, done.cleared
            );
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failure_names_the_item_and_the_date() {
        let failure = Failure {
            resource_id: 1,
            kind: "practitioner_licence".to_owned(),
            on: "2026-11-25".parse().unwrap(),
            problem: Problem::Expired("2026-11-20".parse().unwrap()),
        };
        assert_eq!(
            failure.text(),
            "practitioner licence expired on 20 Nov 2026"
        );
        let missing = Failure {
            problem: Problem::Missing,
            ..failure.clone()
        };
        assert_eq!(missing.text(), "practitioner licence is missing");
        let other = Failure {
            problem: Problem::NotValid,
            ..failure
        };
        assert_eq!(other.text(), "practitioner licence is not valid on 25 Nov 2026");
    }
}
