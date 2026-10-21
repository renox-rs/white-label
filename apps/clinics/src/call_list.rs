//! The call list for an SMS or WhatsApp outage (05 §8 *When a connection is
//! down*, wl-kernel §5.14, `PLT.RUL.05`; BL-CLN-15).
//!
//! While the SMS or the WhatsApp connection is `down`, reminders cannot go:
//! the front desk phones the patients instead. The list is the clinic's
//! appointments, `booked` or `confirmed` (not cancelled, not yet checked in),
//! that start in the 24 hours after the moment the connection went `down`
//! and have not started yet, grouped by branch, each with a *called* tick.
//!
//! Decisions:
//!
//! - An [`Appointment`] exists only for a booking that was booked (a
//!   `requested` one gets its row when staff accept it), so "booked or
//!   confirmed" is "has a row, is not cancelled and has no visit yet".
//! - The moment the connection went down is the time of its status change
//!   to `down`; the tick belongs to that moment, so a new outage starts a
//!   new list with no ticks.
//! - The list shows the patient's name, mobile number and the start, never
//!   the service (the receptionist sees demographics, not the reason,
//!   `CLN.ACL.02`).
//! - With both connections down, the earlier moment is the start.
//! - Decision: the 24 hours are counted from the moment the connection went
//!   down, and from `now` once the outage has lasted longer (05 gives the
//!   start only): reminders keep failing while it lasts, so the list must not
//!   stop at the first day.

use renox::chrono::Duration;
use renox::db::{DateTime, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;
use wl_kernel::integrations::{ConnectionStatus, IntegrationConnection, SMS, WHATSAPP};
use wl_kernel::notify::NotifyChannel;
use wl_kernel::parties::directory;
use wl_kernel::scopes::{label_of, scope_of};

use crate::appointment::Appointment;
use crate::db::refused;
use crate::permissions;
use crate::texts::english;
use crate::visit::Visit;

/// The screen the SMS and WhatsApp connections register, so that it shows the
/// banner while one is down.
pub const SCREEN: &str = "clinics.call_list";

/// The hours ahead of the moment the connection went down that the list
/// covers.
pub const HOURS_AHEAD: i64 = 24;

/// An appointment ticked as called during one outage.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "clinics_call_ticks", unique(booking_id, outage_since))]
pub struct CallTick {
    /// The primary key.
    pub id: i64,
    /// The booking called about.
    pub booking_id: i64,
    /// The moment the connection went down: the outage the tick belongs to.
    pub outage_since: DateTime,
    /// The user who called.
    pub called_by: Option<i64>,
    /// When the tick was made.
    pub called_at: DateTime,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// One row of the list.
#[derive(Debug, Clone)]
pub struct Entry {
    /// The appointment.
    pub appointment: Appointment,
    /// The branch's name.
    pub branch: String,
    /// The patient's name.
    pub patient: String,
    /// The patient's mobile number, if the directory has one.
    pub phone: Option<String>,
    /// Who ticked it and when, once called.
    pub called: Option<CallTick>,
}

/// The moment the SMS or the WhatsApp connection went down, if one is down
/// now: the earlier of the two.
///
/// # Errors
///
/// The database's errors.
pub async fn down_since(db: &Db) -> Result<Option<DateTime>> {
    let mut since: Option<DateTime> = None;
    for key in [SMS, WHATSAPP] {
        let Some(connection) = IntegrationConnection::by_key(db, key).await? else {
            continue;
        };
        if connection.status != ConnectionStatus::Down {
            continue;
        }
        let at = connection
            .status_changed_at
            .or(connection.outage_since)
            .unwrap_or_else(now);
        since = Some(since.map_or(at, |s| s.min(at)));
    }
    Ok(since)
}

/// The appointments to call now, with the branch of each, in the order of
/// branch then start. Empty when neither connection is down. `user` sees the
/// branches where it holds `appointments.call`.
///
/// # Errors
///
/// The database's errors.
pub async fn entries(state: &AppState, user: &AuthUser, lang: &str) -> Result<Vec<Entry>> {
    let Some(since) = down_since(&state.db).await? else {
        return Ok(Vec::new());
    };
    // The window is the next 24 hours from the moment the connection went
    // down; while the outage lasts it moves on with the clock, so an
    // appointment booked on day two of a long outage is listed too.
    let from = since.max(now());
    let until = from + Duration::hours(HOURS_AHEAD);
    let appointments = Appointment::query()
        .where_(Appointment::STARTS_AT.gte(from))
        .where_(Appointment::STARTS_AT.lte(until))
        .order_by(Appointment::STARTS_AT)
        .get(&state.db)
        .await?;
    let ticks = CallTick::query()
        .where_(CallTick::OUTAGE_SINCE.eq(since))
        .get(&state.db)
        .await?;
    // One query for the visits of all the bookings, not one per row.
    let visited: Vec<i64> = if appointments.is_empty() {
        Vec::new()
    } else {
        Visit::query()
            .where_(Visit::BOOKING_ID.is_in(appointments.iter().map(|a| a.booking_id)))
            .get(&state.db)
            .await?
            .into_iter()
            .map(|visit| visit.booking_id)
            .collect()
    };
    let mut rows = Vec::new();
    for appointment in appointments {
        if appointment.is_cancelled() || visited.contains(&appointment.booking_id) {
            continue;
        }
        let scope = scope_of(&state.db, &appointment.subject()).await?;
        if !user.has_permission_in(permissions::APPOINTMENTS_CALL, &scope) {
            continue;
        }
        let patient_party = appointment
            .patient_party_id
            .unwrap_or(appointment.booker_party_id);
        let directory = directory(state);
        let patient = directory
            .display_name(&state.db, PartyId(patient_party), lang)
            .await?
            .unwrap_or_else(|| format!("#{patient_party}"));
        let phone = directory
            .contacts(&state.db, PartyId(patient_party))
            .await?
            .and_then(|contacts| {
                contacts
                    .point_for(NotifyChannel::Sms)
                    .map(|point| point.address.clone())
            });
        let called = ticks
            .iter()
            .find(|t| t.booking_id == appointment.booking_id)
            .cloned();
        rows.push(Entry {
            branch: label_of(&state.db, &scope).await?,
            patient,
            phone,
            called,
            appointment,
        });
    }
    rows.sort_by(|a, b| {
        (&a.branch, a.appointment.starts_at).cmp(&(&b.branch, b.appointment.starts_at))
    });
    Ok(rows)
}

/// Ticks the appointment of booking `booking` as called, during the outage
/// that is on now. Ticking twice changes nothing.
///
/// # Errors
///
/// `Forbidden` without `appointments.call` in the appointment's branch; 422
/// when no connection is down or the appointment is not on the list; the
/// database's errors.
pub async fn tick(state: &AppState, by: &AuthUser, booking: i64) -> Result<CallTick> {
    let appointment = Appointment::of_booking(&state.db, booking)
        .await?
        .ok_or(Error::NotFound)?;
    let scope = scope_of(&state.db, &appointment.subject()).await?;
    if !by.has_permission_in(permissions::APPOINTMENTS_CALL, &scope) {
        return Err(Error::Forbidden);
    }
    let Some(since) = down_since(&state.db).await? else {
        return Err(refused(english("clinics.call_list.problem.none_down", &[])));
    };
    let listed = entries(state, by, "en")
        .await?
        .iter()
        .any(|entry| entry.appointment.booking_id == booking);
    if !listed {
        return Err(refused(english(
            "clinics.call_list.problem.not_listed",
            &[],
        )));
    }
    if let Some(done) = CallTick::query()
        .where_(CallTick::BOOKING_ID.eq(booking))
        .where_(CallTick::OUTAGE_SINCE.eq(since))
        .first(&state.db)
        .await?
    {
        return Ok(done);
    }
    let made = CallTick::create(
        &state.db,
        CallTick {
            booking_id: booking,
            outage_since: since,
            called_by: Some(by.id),
            called_at: now(),
            ..CallTick::default()
        },
    )
    .await;
    match made {
        // A double click: the other request ticked it first.
        Err(error) if error.is_unique_violation() => CallTick::query()
            .where_(CallTick::BOOKING_ID.eq(booking))
            .where_(CallTick::OUTAGE_SINCE.eq(since))
            .first(&state.db)
            .await?
            .ok_or(error),
        other => other,
    }
}
