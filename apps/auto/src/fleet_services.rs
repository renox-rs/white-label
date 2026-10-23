//! The service plans of a fleet contract and the bookings they create (08
//! §4.5 steps 1 and 2, `AUT.FLT.01`, `WRK.PPM.01`, `AUT.RUL.04`, `SCH.BKG.01`).
//!
//! A plan says that a vehicle under contract is serviced every so many
//! kilometres and/or months. [`schedule_all`] sets the first due day and
//! reading when the contract is activated; [`request_bookings`] books the
//! services that fall due through the kernel's `BookingActions`
//! (wl-scheduling) for the first free slot, for the fleet, with the vehicle as
//! the booking's subject, so the check-in opens the job card
//! ([`crate::bookings`]); [`on_delivered`] moves the plan on when the visit is
//! delivered.
//!
//! Decisions:
//!
//! - A service is due [`LEAD_DAYS`] days before its due day (`AUT.RUL.04`,
//!   N = 7) or [`KM_LEAD`] kilometres before its due reading.
//! - Bookings are requested for a contract that is `active` only: a suspended
//!   contract (BL-AUT-13) and one in its renewal window book nothing new; no
//!   booking is asked for a day after the contract's last day of service
//!   (`end_date`, or the effective date of a termination scheduled,
//!   BL-AUT-12).
//! - The plan keeps the booking it asked for until the visit is delivered; a
//!   cancelled booking frees the plan, which asks again at the next run.
//! - The fleet contact is told of a booking made (`AUT.RUL.04`) by the
//!   notification types of BL-AUT-18; nothing is sent here (see `DEFERRED.md`).

use renox::chrono::NaiveDate;
use renox::db::now;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::calendar::BusinessCalendar;
use wl_kernel::documents::{dubai_date, today};
use wl_kernel::scheduling::{BookingCancelled, BookingId, ServiceBooking, booking_actions};

use crate::db::refused;
use crate::fleet_contract::{FleetContract, FleetStatus};
use crate::fleet_service::FleetService;
use crate::job_card::JobCard;
use crate::permissions;
use crate::reasons::FLEET_SUSPENDED;
use crate::texts::english;
use crate::vehicle::Vehicle;

/// Days before the due day that the booking is requested (`AUT.RUL.04`).
pub const LEAD_DAYS: i64 = 7;

/// Kilometres before the due reading that the booking is requested.
pub const KM_LEAD: i64 = 500;

/// The hour (Asia/Dubai) that a booking is asked for on its day.
pub const BOOKING_HOUR: u32 = 9;

/// A service plan to add.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPlan {
    /// The vehicle, under the contract.
    pub vehicle_id: i64,
    /// The code of the service type that is booked.
    pub service_code: String,
    /// The mileage between two services.
    pub interval_km: Option<i64>,
    /// The months between two services.
    pub interval_months: Option<i64>,
    /// The day of the last service, when there was one.
    pub last_service_date: Option<NaiveDate>,
    /// The odometer reading of the last service.
    pub last_service_km: Option<i64>,
}

impl NewPlan {
    /// A plan to book `service_code` for `vehicle_id`.
    pub fn new(vehicle_id: i64, service_code: &str) -> Self {
        NewPlan {
            vehicle_id,
            service_code: service_code.to_owned(),
            interval_km: None,
            interval_months: None,
            last_service_date: None,
            last_service_km: None,
        }
    }

    /// Every `km` kilometres.
    #[must_use]
    pub fn every_km(mut self, km: i64) -> Self {
        self.interval_km = Some(km);
        self
    }

    /// Every `months` months.
    #[must_use]
    pub fn every_months(mut self, months: i64) -> Self {
        self.interval_months = Some(months);
        self
    }

    /// The last service was on `date` at `km`.
    #[must_use]
    pub fn last_service(mut self, date: NaiveDate, km: i64) -> Self {
        self.last_service_date = Some(date);
        self.last_service_km = Some(km);
        self
    }
}

/// Adds a service plan to contract `contract_id` (`AUT.FLT.01`).
///
/// # Errors
///
/// A 403 without `fleet_contracts.manage`; a 422 for a contract that ended, a
/// vehicle that is not under it, a blank service code, no interval or one that
/// is not above zero, or a last service in the future.
pub async fn add_plan(
    state: &AppState,
    by: &AuthUser,
    contract_id: i64,
    new: NewPlan,
) -> Result<FleetService> {
    if !by.has_permission(permissions::FLEET_CONTRACTS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let contract = FleetContract::find_or_404(&state.db, contract_id).await?;
    if contract.status == FleetStatus::Ended {
        return Err(refused(english("auto.fleet.problem.ended", &[])));
    }
    let vehicle = Vehicle::find(&state.db, new.vehicle_id)
        .await?
        .filter(|vehicle| vehicle.fleet_contract_id == Some(contract_id))
        .ok_or_else(|| refused(english("auto.fleet.problem.not_on_contract", &[])))?;
    let code = new.service_code.trim().to_lowercase();
    if code.is_empty() {
        return Err(refused(english("auto.fleet.problem.service_code", &[])));
    }
    if new.interval_km.is_none() && new.interval_months.is_none() {
        return Err(refused(english("auto.fleet.problem.interval", &[])));
    }
    if new.interval_km.is_some_and(|km| km <= 0)
        || new.interval_months.is_some_and(|months| months <= 0)
        || new.last_service_km.is_some_and(|km| km < 0)
    {
        return Err(refused(english("auto.fleet.problem.interval", &[])));
    }
    if new.last_service_date.is_some_and(|date| date > today()) {
        return Err(refused(english("auto.fleet.problem.last_service", &[])));
    }
    let mut plan = FleetService {
        contract_id,
        vehicle_id: vehicle.id,
        service_code: code,
        interval_km: new.interval_km,
        interval_months: new.interval_months,
        last_service_date: new.last_service_date,
        last_service_km: new.last_service_km,
        active: true,
        created_by: Some(by.id),
        ..FleetService::default()
    };
    let (date, km) = plan.next_due(contract.start_date);
    plan.next_due_date = date;
    plan.next_due_km = km;
    plan.save(&state.db).await?;
    FleetService::find_or_404(&state.db, plan.id).await
}

/// Stops service plan `plan_id`.
///
/// # Errors
///
/// A 403 without `fleet_contracts.manage`.
pub async fn remove_plan(state: &AppState, by: &AuthUser, plan_id: i64) -> Result<FleetService> {
    if !by.has_permission(permissions::FLEET_CONTRACTS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let mut plan = FleetService::find_or_404(&state.db, plan_id).await?;
    plan.active = false;
    plan.save(&state.db).await?;
    FleetService::find_or_404(&state.db, plan_id).await
}

/// How many plans of the contract are in use.
///
/// # Errors
///
/// The database's errors.
pub async fn count_active(db: &Db, contract_id: i64) -> Result<i64> {
    let count = FleetService::query()
        .where_(FleetService::CONTRACT_ID.eq(contract_id))
        .where_(FleetService::ACTIVE.eq(true))
        .count(db)
        .await?;
    Ok(i64::try_from(count).unwrap_or(i64::MAX))
}

/// The plans of the contract, by vehicle.
///
/// # Errors
///
/// The database's errors.
pub async fn plans_of(db: &Db, contract_id: i64) -> Result<Vec<FleetService>> {
    FleetService::query()
        .where_(FleetService::CONTRACT_ID.eq(contract_id))
        .order_by(FleetService::VEHICLE_ID)
        .order_by(FleetService::ID)
        .get(db)
        .await
}

/// The bookings already made for the contract's plans, kept when the contract
/// is suspended and listed for the advisor to confirm or cancel (BL-AUT-13).
///
/// # Errors
///
/// The database's errors.
pub async fn bookings_to_confirm(db: &Db, contract_id: i64) -> Result<Vec<FleetService>> {
    FleetService::query()
        .where_(FleetService::CONTRACT_ID.eq(contract_id))
        .where_(FleetService::ACTIVE.eq(true))
        .where_(FleetService::BOOKING_ID.is_not_null())
        .order_by(FleetService::REQUESTED_FOR_DATE)
        .get(db)
        .await
}

/// Sets the first due day and reading of the plans that have none, from the
/// contract's first day (`WRK.PPM.01`): done when the contract is activated.
///
/// # Errors
///
/// The database's errors.
pub async fn schedule_all(state: &AppState, contract: &FleetContract) -> Result {
    for mut plan in plans_of(&state.db, contract.id).await? {
        if !plan.active || (plan.next_due_date.is_some() || plan.next_due_km.is_some()) {
            continue;
        }
        let (date, km) = plan.next_due(contract.start_date);
        plan.next_due_date = date;
        plan.next_due_km = km;
        plan.save(&state.db).await?;
    }
    Ok(())
}

/// What a run of [`request_bookings`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BookingRun {
    /// The plans a booking was made for, with the booking.
    pub booked: Vec<(i64, BookingId)>,
    /// The plans whose booking was refused, with why.
    pub failed: Vec<(i64, String)>,
}

/// Books the services of `contract` that fall due on `day` (`AUT.RUL.04`): for
/// the first free slot from their due day, for the fleet, with the vehicle as
/// the subject. Only a contract that is `active` books.
///
/// # Errors
///
/// The database's errors; a booking that scheduling refuses is reported in
/// [`BookingRun::failed`] and tried again at the next run.
pub async fn request_bookings(
    state: &AppState,
    contract: &FleetContract,
    day: NaiveDate,
) -> Result<BookingRun> {
    let mut run = BookingRun::default();
    if contract.status != FleetStatus::Active {
        return Ok(run);
    }
    let last_day = match (contract.termination_scheduled, contract.effective_date) {
        (true, Some(effective)) => effective.min(contract.end_date),
        _ => contract.end_date,
    };
    for mut plan in plans_of(&state.db, contract.id).await? {
        if !plan.active || plan.booking_id.is_some() {
            continue;
        }
        let Some(vehicle) = Vehicle::find(&state.db, plan.vehicle_id).await? else {
            continue;
        };
        if !plan.is_due(day, LEAD_DAYS, vehicle.odometer_km, KM_LEAD) {
            continue;
        }
        let wanted = plan.next_due_date.unwrap_or(day).max(day);
        if wanted > last_day {
            continue;
        }
        let Some(local) = wanted.and_hms_opt(BOOKING_HOUR, 0, 0) else {
            continue;
        };
        let from = BusinessCalendar::default().from_local(local);
        let request =
            ServiceBooking::new(PartyId(contract.fleet_party_id), &plan.service_code, from)
                .for_subject(vehicle.subject())
                .note(&english(
                    "auto.fleet.booking_note",
                    &[("contract", &contract.number())],
                ));
        match booking_actions(state).book_service(state, request).await {
            Ok(booking) => {
                // The day the booking is for: the first free slot may be a
                // later day than the one asked for.
                let day = booking_actions(state)
                    .starts_at(state, booking)
                    .await
                    .ok()
                    .flatten()
                    .map_or(wanted, dubai_date);
                plan.booking_id = Some(booking.0);
                plan.requested_for_date = Some(day);
                plan.requested_at = Some(now());
                plan.booking_confirmed_at = None;
                plan.save(&state.db).await?;
                run.booked.push((plan.id, booking));
            }
            Err(error) => run.failed.push((plan.id, error.to_string())),
        }
    }
    Ok(run)
}

/// The visit of a job card was delivered: the plan that booked it moves on
/// (last service today at the odometer reading of the delivery, the next due
/// day and reading from the intervals). A job card that did not come from a
/// plan's booking moves no plan.
///
/// # Errors
///
/// The database's errors.
pub async fn on_delivered(state: &AppState, card: &JobCard) -> Result {
    let Some(booking) = card.booking_id else {
        return Ok(());
    };
    let plans = FleetService::query()
        .where_(FleetService::BOOKING_ID.eq(Some(booking)))
        .get(&state.db)
        .await?;
    for mut plan in plans {
        let km = match card.odometer_out {
            Some(km) => km,
            None => Vehicle::find(&state.db, plan.vehicle_id)
                .await?
                .map_or(card.odometer_in, |vehicle| vehicle.odometer_km),
        };
        plan.last_service_date = Some(today());
        plan.last_service_km = Some(km);
        let (date, next_km) = plan.next_due(today());
        plan.next_due_date = date;
        plan.next_due_km = next_km;
        plan.booking_id = None;
        plan.requested_for_date = None;
        plan.booking_confirmed_at = None;
        plan.save(&state.db).await?;
    }
    Ok(())
}

/// A booking was cancelled: the plan that asked for it is free to ask again.
///
/// # Errors
///
/// The database's errors.
pub async fn on_booking_cancelled(state: &AppState, event: &BookingCancelled) -> Result {
    let none: Option<i64> = None;
    let no_day: Option<NaiveDate> = None;
    let no_time: Option<renox::db::DateTime> = None;
    FleetService::query()
        .where_(FleetService::BOOKING_ID.eq(Some(event.booking.0)))
        .update(
            &state.db,
            &[
                ("booking_id", &none),
                ("requested_for_date", &no_day),
                ("booking_confirmed_at", &no_time),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(())
}

/// Cancels, as the system with `reason_code`, the bookings the contract's
/// plans asked for and that did not start yet; answers how many were
/// cancelled (BL-AUT-12, BL-AUT-13).
///
/// # Errors
///
/// The scheduling contract's errors.
pub async fn cancel_open_bookings(
    state: &AppState,
    contract_id: i64,
    reason_code: &str,
) -> Result<usize> {
    cancel_bookings_after(state, contract_id, reason_code, None).await
}

/// [`cancel_open_bookings`] limited to the bookings dated after `after`: a
/// booking on the last day of service itself stays (BL-AUT-12, BL-AUT-13).
///
/// # Errors
///
/// The scheduling contract's errors.
pub async fn cancel_bookings_after(
    state: &AppState,
    contract_id: i64,
    reason_code: &str,
    after: Option<NaiveDate>,
) -> Result<usize> {
    let mut cancelled = 0;
    for plan in bookings_to_confirm(&state.db, contract_id).await? {
        let Some(booking) = plan.booking_id else {
            continue;
        };
        if let Some(day) = after
            && plan.requested_for_date.is_some_and(|date| date <= day)
        {
            continue;
        }
        if booking_actions(state)
            .cancel_booking(state, BookingId(booking), reason_code)
            .await?
        {
            cancelled += 1;
        }
    }
    Ok(cancelled)
}

/// Stops every service plan of the contract: the future schedules of a
/// contract that ended are cancelled (BL-AUT-12, BL-AUT-13).
///
/// # Errors
///
/// The database's errors.
pub async fn stop_all_plans(db: &Db, contract_id: i64) -> Result {
    FleetService::query()
        .where_(FleetService::CONTRACT_ID.eq(contract_id))
        .where_(FleetService::ACTIVE.eq(true))
        .update(db, &[("active", &false), ("updated_at", &now())])
        .await?;
    Ok(())
}

/// The advisor confirms the booking of plan `plan_id`, kept while the contract
/// is suspended (BL-AUT-13).
///
/// # Errors
///
/// A 403 without `job_cards.advise` or `fleet_contracts.manage`; a 422 for a
/// plan with no booking waiting.
pub async fn confirm_booking(state: &AppState, by: &AuthUser, plan_id: i64) -> Result<FleetService> {
    if !may_decide_booking(by) {
        return Err(Error::Forbidden);
    }
    let mut plan = FleetService::find_or_404(&state.db, plan_id).await?;
    if plan.booking_id.is_none() || !plan.active {
        return Err(refused(english("auto.fleet.problem.no_booking", &[])));
    }
    plan.booking_confirmed_at = Some(now());
    plan.save(&state.db).await?;
    FleetService::find_or_404(&state.db, plan_id).await
}

/// The advisor cancels the booking of plan `plan_id`, kept while the contract
/// is suspended (BL-AUT-13): the booking is cancelled for staff with the reason
/// `fleet_suspended` and the plan is free to ask again once the contract is
/// resumed.
///
/// # Errors
///
/// A 403 without `job_cards.advise` or `fleet_contracts.manage`; a 422 for a
/// plan with no booking waiting, or one that cannot be cancelled any more.
pub async fn cancel_booking(state: &AppState, by: &AuthUser, plan_id: i64) -> Result<FleetService> {
    if !may_decide_booking(by) {
        return Err(Error::Forbidden);
    }
    let plan = FleetService::find_or_404(&state.db, plan_id).await?;
    let Some(booking) = plan.booking_id.filter(|_| plan.active) else {
        return Err(refused(english("auto.fleet.problem.no_booking", &[])));
    };
    let done = booking_actions(state)
        .cancel_booking_by_staff(state, BookingId(booking), FLEET_SUSPENDED, by.id)
        .await?;
    if !done {
        return Err(refused(english("auto.fleet.problem.booking_kept", &[])));
    }
    FleetService::find_or_404(&state.db, plan_id).await
}

/// Whether `user` decides the bookings kept for a suspended contract: the
/// service advisor, or the fleet account manager.
fn may_decide_booking(user: &AuthUser) -> bool {
    user.has_permission(permissions::JOB_CARDS_ADVISE)
        || user.has_permission(permissions::FLEET_CONTRACTS_MANAGE)
}

/// Registers the listener that frees a plan when its booking is cancelled.
pub(crate) fn register_listeners(app: &mut Registry) {
    app.listen(|event: BookingCancelled, state: AppState| async move {
        on_booking_cancelled(&state, &event).await
    });
}
