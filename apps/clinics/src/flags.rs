//! The flag on an appointment: a mark the front desk sees on the day list and
//! on the appointment at check-in (05 §7.2 `CLN.RUL.07`, `CLN.RUL.09`,
//! `CLN.RUL.10`; base for BL-CLN-03 and BL-CLN-04).
//!
//! A flag has a [`FlagCause`] (the appointment *needs authorisation*, or the
//! patient's *insurance card* has to be renewed), the `origin` that raised it
//! (`booked`, `policy_attached`, `rescheduled`, `denied`, `used`, `expired`,
//! `look_ahead`, `card`), the authorisation that caused it, and a text for
//! staff. There is at most one **open** flag per appointment and cause:
//! raising it again changes nothing ([`raise`]). A flag is never deleted;
//! [`clear`] stamps the day and the reason it no longer holds (the
//! appointment is covered, was rescheduled, cancelled or checked in).

use renox::chrono::NaiveDate;
use renox::db::{DateTime, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;

use crate::appointment::Appointment;

/// Why an appointment is flagged.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FlagCause {
    /// A service of the appointment needs a prior authorisation that is
    /// missing, denied, used up or ends before the visit (`CLN.RUL.07`,
    /// `CLN.RUL.09`).
    #[default]
    NeedsAuthorisation,
    /// The policy ends before the visit or the card on file has expired
    /// (`CLN.RUL.10`).
    InsuranceCard,
}

impl FlagCause {
    /// The stored name.
    pub fn key(self) -> &'static str {
        match self {
            FlagCause::NeedsAuthorisation => "needs_authorisation",
            FlagCause::InsuranceCard => "insurance_card",
        }
    }
}

/// A flag on an appointment.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "clinics_appointment_flags", index(booking_id), index(cause))]
pub struct AppointmentFlag {
    /// The primary key.
    pub id: i64,
    /// The flagged booking.
    pub booking_id: i64,
    /// The clinic's record of the booking.
    #[model(references = "clinics_appointments")]
    pub appointment_id: i64,
    /// Why it is flagged.
    pub cause: FlagCause,
    /// What raised it: `booked`, `policy_attached`, `rescheduled`, `denied`,
    /// `used`, `expired`, `look_ahead` or `card`.
    pub origin: String,
    /// The prior authorisation that caused it, if one did.
    pub authorisation_id: Option<i64>,
    /// A note for staff.
    pub detail: Option<String>,
    /// The day it was raised (Asia/Dubai).
    pub raised_on: NaiveDate,
    /// The day it no longer held.
    pub cleared_on: Option<NaiveDate>,
    /// Why: `covered`, `rescheduled`, `cancelled`, `checked_in`,
    /// `card_accepted` …
    pub clear_reason: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl AppointmentFlag {
    /// Whether it still holds.
    pub fn is_open(&self) -> bool {
        self.cleared_on.is_none()
    }

    /// The open flags of booking `booking`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn open_of(db: &Db, booking: i64) -> Result<Vec<AppointmentFlag>> {
        AppointmentFlag::query()
            .where_(AppointmentFlag::BOOKING_ID.eq(booking))
            .where_(AppointmentFlag::CLEARED_ON.is_null())
            .order_by(AppointmentFlag::ID)
            .get(db)
            .await
    }

    /// The open flag of `cause` on booking `booking`, if there is one.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn open_cause(
        db: &Db,
        booking: i64,
        cause: FlagCause,
    ) -> Result<Option<AppointmentFlag>> {
        AppointmentFlag::query()
            .where_(AppointmentFlag::BOOKING_ID.eq(booking))
            .where_(AppointmentFlag::CAUSE.eq(cause))
            .where_(AppointmentFlag::CLEARED_ON.is_null())
            .first(db)
            .await
    }

    /// Every flag of booking `booking`, oldest first (what was raised and
    /// cleared).
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_booking(db: &Db, booking: i64) -> Result<Vec<AppointmentFlag>> {
        AppointmentFlag::query()
            .where_(AppointmentFlag::BOOKING_ID.eq(booking))
            .order_by(AppointmentFlag::ID)
            .get(db)
            .await
    }
}

/// Flags `appointment` for `cause`. A flag of that cause that is open already
/// is left as it is: the answer is `(flag, false)`.
///
/// # Errors
///
/// The database's errors.
pub async fn raise(
    state: &AppState,
    appointment: &Appointment,
    cause: FlagCause,
    origin: &str,
    authorisation: Option<i64>,
    detail: Option<&str>,
) -> Result<(AppointmentFlag, bool)> {
    if let Some(open) =
        AppointmentFlag::open_cause(&state.db, appointment.booking_id, cause).await?
    {
        return Ok((open, false));
    }
    let mut flag = AppointmentFlag {
        booking_id: appointment.booking_id,
        appointment_id: appointment.id,
        cause,
        origin: origin.to_owned(),
        authorisation_id: authorisation,
        detail: detail.map(str::to_owned),
        raised_on: today(),
        ..AppointmentFlag::default()
    };
    flag.save(&state.db).await?;
    Ok((flag, true))
}

/// Clears the open flag of `cause` on booking `booking`, with `reason`.
/// Returns the flag that was cleared, `None` when none was open.
///
/// # Errors
///
/// The database's errors.
pub async fn clear(
    state: &AppState,
    booking: i64,
    cause: FlagCause,
    reason: &str,
) -> Result<Option<AppointmentFlag>> {
    let Some(open) = AppointmentFlag::open_cause(&state.db, booking, cause).await? else {
        return Ok(None);
    };
    AppointmentFlag::query()
        .where_(AppointmentFlag::ID.eq(open.id))
        .update(
            &state.db,
            &[
                ("cleared_on", &Some(today())),
                ("clear_reason", &Some(reason.to_owned())),
                ("updated_at", &now()),
            ],
        )
        .await?;
    AppointmentFlag::find(&state.db, open.id).await
}

/// Clears every open flag of booking `booking` with `reason`.
///
/// # Errors
///
/// The database's errors.
pub async fn clear_all(state: &AppState, booking: i64, reason: &str) -> Result<usize> {
    let mut cleared = 0;
    for cause in [FlagCause::NeedsAuthorisation, FlagCause::InsuranceCard] {
        if clear(state, booking, cause, reason).await?.is_some() {
            cleared += 1;
        }
    }
    Ok(cleared)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flag_is_open_until_it_is_cleared() {
        let mut flag = AppointmentFlag::default();
        assert!(flag.is_open());
        flag.cleared_on = NaiveDate::from_ymd_opt(2026, 11, 20);
        assert!(!flag.is_open());
    }

    #[test]
    fn the_causes_are_stored_by_name() {
        assert_eq!(FlagCause::NeedsAuthorisation.key(), "needs_authorisation");
        assert_eq!(FlagCause::InsuranceCard.key(), "insurance_card");
        assert_eq!(FlagCause::InsuranceCard.as_str(), "insurance_card");
    }
}
