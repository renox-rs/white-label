//! The automations of a booking's last hours (wl-scheduling §6 *Automations*):
//! `SCH.RUL.06` escalates a conflict that is still open, `SCH.RUL.01` sends
//! reminders but not for a conflicted booking, and `SCH.RUL.02` marks a
//! no-show but not for a conflicted booking (BL-SCH-06).
//!
//! The task [`TASK`] runs [`run`] every five minutes; each step takes the
//! moment as a parameter, so a test runs it at 13:00.
//!
//! Decisions:
//!
//! - **Escalation.** When a booking still has an open conflict `H` hours
//!   (2 by default) before its start, the branch manager is told and the party
//!   gets the change message with *accept*, *pick another time* and *cancel*
//!   links. A conflict raised inside `H` is escalated at once
//!   ([`crate::conflict::raise`] calls [`escalate`]). The rows of the
//!   booking's open conflicts carry `escalated_at`, so a repeated run sends
//!   nothing: one escalation per booking per conflict.
//! - **Branch manager.** The kernel has no `ApproverResolver` yet. The
//!   holders of the manager role the vertical named
//!   ([`WlScheduling::manager_role`](crate::WlScheduling::manager_role)) in the
//!   scope of the booking's location are told; when there are none, the users
//!   with `bookings.manage` in that scope; when there are none either, the
//!   company administrators.
//! - **Reminders** go at `N` hours before the start (24 and 2 by default). Of
//!   the reminders due at a run, the latest (smallest `N`) is the one that is
//!   sent; the earlier ones are marked skipped. A conflicted booking gets none:
//!   the reminder is marked *suppressed* and does not come later when the
//!   conflict clears, because it would be stale. The reminders of the times
//!   still to come are not affected. The message has the cancel link only: the
//!   confirm and reschedule links of the spec are not built.
//! - **No-show.** The sweep marks `booked` and `confirmed` bookings whose
//!   grace has passed and leaves a conflicted booking to staff; the machine
//!   refuses `mark_no_show` on a conflicted booking for every actor.

use renox::auth::permissions::{Scope, users_with_permission_in, users_with_role_in};
use renox::chrono::TimeDelta;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::scheduling::ConflictCause;
use wl_kernel::scopes::{self, COMPANY_ADMIN_ROLE};

use crate::booking::{Booking, BookingStatus};
use crate::config::config_of;
use crate::conflict::{BLOCK_REASON_CODE, Conflict, can_conflict, open_of};
use crate::notices::{self, ChangeEntry};
use crate::permissions;
use crate::prepayment;
use crate::refusal::Refusal;
use crate::resource::Resource;
use crate::service::{self, Actor};

/// The scheduled task and command.
pub const TASK: &str = "scheduling:rules";

/// The reason code of a change caused by a licence that lapses.
const INELIGIBLE_REASON_CODE: &str = "resource_ineligible";

/// A reminder handled for a booking at its start.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_scheduling_reminders", index(booking_id))]
pub struct Reminder {
    /// The primary key.
    pub id: i64,
    /// The booking.
    #[model(references = "wl_scheduling_bookings")]
    pub booking_id: i64,
    /// The hours before the start it is for.
    pub hours_before: i64,
    /// The start it was for.
    pub for_start: DateTime,
    /// `sent`, `suppressed` or `skipped`.
    pub status: String,
    /// When it was handled.
    pub handled_at: DateTime,
}

/// What one run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sent {
    /// Bookings whose open conflict was escalated.
    pub escalations: usize,
    /// Reminders sent.
    pub reminders: usize,
    /// Reminders not sent because the booking is in conflict.
    pub suppressed: usize,
    /// Bookings marked no-show.
    pub no_shows: usize,
    /// Unpaid bookings released at the end of their prepayment hold
    /// (`SCH.RUL.04`).
    pub released: usize,
}

/// The names of the resources of `booking`, comma separated.
async fn resource_names(db: &Db, booking: &Booking) -> Result<String> {
    let mut names = Vec::new();
    for id in booking.resource_ids(db).await? {
        if let Some(resource) = Resource::find(db, id).await? {
            names.push(resource.name);
        }
    }
    Ok(names.join(", "))
}

/// The scope of the location of `booking`, or of its first resource.
pub(crate) async fn scope_of(db: &Db, booking: &Booking) -> Result<Scope> {
    if let Some(location) = booking.location() {
        return scopes::scope_of(db, &location).await;
    }
    for id in booking.resource_ids(db).await? {
        if let Some(location) = Resource::find(db, id).await?.and_then(|r| r.location()) {
            return scopes::scope_of(db, &location).await;
        }
    }
    Ok(Scope::global())
}

/// The users `SCH.RUL.06` tells: the manager role in the booking's location,
/// else the holders of `bookings.manage` there, else the administrators.
pub async fn managers(state: &AppState, booking: &Booking) -> Result<Vec<i64>> {
    let scope = scope_of(&state.db, booking).await?;
    if let Some(role) = config_of(state).manager_role {
        let holders = users_with_role_in(&state.db, &role, &scope).await?;
        if !holders.is_empty() {
            return Ok(holders.iter().map(|user| user.id).collect());
        }
    }
    let staff = users_with_permission_in(&state.db, permissions::MANAGE, &scope).await?;
    if !staff.is_empty() {
        return Ok(staff.iter().map(|user| user.id).collect());
    }
    let admins = users_with_role_in(&state.db, COMPANY_ADMIN_ROLE, &scope).await?;
    Ok(admins.iter().map(|user| user.id).collect())
}

/// The front desk: the holders of `bookings.manage` in the booking's location.
pub(crate) async fn front_desk(state: &AppState, booking: &Booking) -> Result<Vec<i64>> {
    let scope = scope_of(&state.db, booking).await?;
    let staff = users_with_permission_in(&state.db, permissions::MANAGE, &scope).await?;
    Ok(staff.iter().map(|user| user.id).collect())
}

/// Escalates the open conflict of booking `booking_id` if it is due at `at`
/// (`H` hours before the start, or later) and was not escalated yet: tells the
/// branch manager and sends the party the change message. Returns whether it
/// did.
///
/// # Errors
///
/// A database error or the notifier's.
pub async fn escalate(state: &AppState, booking_id: i64, at: DateTime) -> Result<bool> {
    let Some(booking) = Booking::find(&state.db, booking_id).await? else {
        return Ok(false);
    };
    if !can_conflict(booking.status) {
        return Ok(false);
    }
    let hours = config_of(state).escalation_hours;
    if booking.starts_at - TimeDelta::hours(hours) > at {
        return Ok(false);
    }
    let rows: Vec<Conflict> = open_of(&state.db, booking.id)
        .await?
        .into_iter()
        .filter(|row| row.escalated_at.is_none())
        .collect();
    let Some(first) = rows.first() else {
        return Ok(false);
    };
    // Marked before the messages go, so that a run that dies half way does not
    // send them twice.
    for row in &rows {
        Conflict::query()
            .where_(Conflict::ID.eq(row.id))
            .update(&state.db, &[("escalated_at", &Some(at))])
            .await?;
    }
    let code = if first.cause == ConflictCause::Eligibility.as_str() {
        INELIGIBLE_REASON_CODE
    } else {
        BLOCK_REASON_CODE
    };
    let users = managers(state, &booking).await?;
    let names = resource_names(&state.db, &booking).await?;
    notices::escalated(state, &booking, &names, &users).await?;
    let entry = ChangeEntry {
        booking: booking.clone(),
        resource: None,
        slots: Vec::new(),
    };
    notices::changes(
        state,
        booking.party(),
        &[entry],
        notices::CHANGED,
        code,
        at,
    )
    .await?;
    Ok(true)
}

/// Escalates every open conflict that is due at `at`. Returns how many
/// bookings were escalated.
///
/// # Errors
///
/// A database error or the notifier's.
pub async fn escalate_due(state: &AppState, at: DateTime) -> Result<usize> {
    let mut booking_ids: Vec<i64> = Conflict::query()
        .where_(Conflict::CLEARED_AT.is_null())
        .where_(Conflict::ESCALATED_AT.is_null())
        .order_by(Conflict::BOOKING_ID)
        .get(&state.db)
        .await?
        .into_iter()
        .map(|row| row.booking_id)
        .collect();
    booking_ids.dedup();
    let mut escalated = 0;
    for id in booking_ids {
        if escalate(state, id, at).await? {
            escalated += 1;
        }
    }
    Ok(escalated)
}

/// Sends the reminders due at `at` (`SCH.RUL.01`). A conflicted booking gets
/// none. Returns `(sent, suppressed)`.
///
/// # Errors
///
/// A database error or the notifier's.
pub async fn remind(state: &AppState, at: DateTime) -> Result<(usize, usize)> {
    let cfg = config_of(state);
    let Some(longest) = cfg.reminder_hours.iter().copied().max() else {
        return Ok((0, 0));
    };
    let mut hours = cfg.reminder_hours.clone();
    hours.sort_unstable();
    hours.dedup();
    let mut bookings = Vec::new();
    for status in [BookingStatus::Booked, BookingStatus::Confirmed] {
        bookings.extend(
            Booking::query()
                .where_(Booking::STATUS.eq(status))
                .where_(Booking::STARTS_AT.gt(at))
                .where_(Booking::STARTS_AT.lte(at + TimeDelta::hours(longest)))
                .order_by(Booking::ID)
                .get(&state.db)
                .await?,
        );
    }
    let (mut sent, mut suppressed) = (0, 0);
    for booking in bookings {
        // A booking whose prepayment is awaited is not confirmed yet.
        if booking.payment_pending {
            continue;
        }
        let due: Vec<i64> = hours
            .iter()
            .copied()
            .filter(|n| booking.starts_at - TimeDelta::hours(*n) <= at)
            .collect();
        // The latest due reminder is the one that matters.
        let Some(latest) = due.first().copied() else {
            continue;
        };
        let handled: Vec<i64> = Reminder::query()
            .where_(Reminder::BOOKING_ID.eq(booking.id))
            .where_(Reminder::FOR_START.eq(booking.starts_at))
            .get(&state.db)
            .await?
            .into_iter()
            .map(|row| row.hours_before)
            .collect();
        for n in due.iter().copied().filter(|n| !handled.contains(n)) {
            let status = if n != latest {
                "skipped"
            } else if booking.has_conflict() {
                "suppressed"
            } else {
                "sent"
            };
            // Marked before the message goes: a run that dies does not send
            // it twice.
            Reminder {
                booking_id: booking.id,
                hours_before: n,
                for_start: booking.starts_at,
                status: status.to_owned(),
                handled_at: at,
                ..Reminder::default()
            }
            .save(&state.db)
            .await?;
            match status {
                "sent" => {
                    let names = resource_names(&state.db, &booking).await?;
                    notices::reminder(state, &booking, &names, at).await?;
                    sent += 1;
                }
                "suppressed" => suppressed += 1,
                _ => {}
            }
        }
    }
    Ok((sent, suppressed))
}

/// Marks as no-show, as the system, the `booked` and `confirmed` bookings
/// whose grace has passed at `at` (`SCH.RUL.02`), except those in conflict,
/// and tells the front desk. Returns how many were marked.
///
/// # Errors
///
/// A database error or the first refusal that is not "not allowed now".
pub async fn no_shows(state: &AppState, at: DateTime) -> Result<usize> {
    let grace = config_of(state).no_show_grace_minutes;
    let mut marked = 0;
    let mut failure: Option<Error> = None;
    for status in [BookingStatus::Booked, BookingStatus::Confirmed] {
        let due = Booking::query()
            .where_(Booking::STATUS.eq(status))
            .where_(Booking::STARTS_AT.lte(at - TimeDelta::minutes(grace)))
            .order_by(Booking::ID)
            .get(&state.db)
            .await?;
        for booking in due {
            if booking.has_conflict() {
                continue;
            }
            match service::mark_no_show(state, booking.id, Actor::system(), at).await {
                Ok(done) => {
                    marked += 1;
                    let names = resource_names(&state.db, &done).await?;
                    let users = front_desk(state, &done).await?;
                    notices::no_show(state, &done, &names, &users).await?;
                }
                Err(Refusal::Transition(_)) => {}
                Err(other) => {
                    // The no-show stands; the deposit or a message failed.
                    if failure.is_none() {
                        failure = Some(Error::Status(
                            StatusCode::UNPROCESSABLE_ENTITY,
                            other.plain(),
                        ));
                    }
                }
            }
        }
    }
    match failure {
        Some(error) => Err(error),
        None => Ok(marked),
    }
}

/// Runs the three automations at `at`.
///
/// # Errors
///
/// A database error or the notifier's.
pub async fn run(state: &AppState, at: DateTime) -> Result<Sent> {
    let escalations = escalate_due(state, at).await?;
    let (reminders, suppressed) = remind(state, at).await?;
    let no_shows = no_shows(state, at).await?;
    let released = prepayment::release_due(state, at).await?;
    Ok(Sent {
        escalations,
        reminders,
        suppressed,
        no_shows,
        released,
    })
}

/// Registers the five-minute task and its command.
pub(crate) fn schedule(app: &mut Registry) {
    app.schedule()
        .every_minutes(5, TASK, |state: AppState| async move {
            run(&state, renox::db::now()).await.map(|_| ())
        });
    app.command(
        TASK,
        "Escalate open booking conflicts, send reminders, mark no-shows and release unpaid holds (SCH.RUL.06, SCH.RUL.01, SCH.RUL.02, SCH.RUL.04)",
        |_args, state: AppState| async move {
            let sent = run(&state, renox::db::now()).await?;
            println!(
                "{} escalated, {} reminders sent, {} suppressed, {} no-shows, {} unpaid released.",
                sent.escalations, sent.reminders, sent.suppressed, sent.no_shows, sent.released
            );
            Ok(())
        },
    );
}
