//! The prior authorisation at check-in (05 §4.2 *When it goes wrong* row
//! "Prior authorisation missing, expired or used up before the visit", §7.1
//! *Appointment* `check_in`, `CLN.ACL.15`; BL-CLN-02).
//!
//! A service that needs a prior authorisation (the payer's list,
//! [`crate::authorisation::PriorAuthService`]) goes ahead on an insured visit
//! only with a valid authorisation that has units left on the visit day, or as
//! self-pay with a signed consent, or the appointment is rescheduled while a
//! new authorisation is requested.
//!
//! [`plan`] answers it **per service**: for each service of the visit that
//! needs one, which authorisations give the units and how many units are left
//! to go self-pay. Units left of an authorisation = approved − consumed −
//! the units held by the visits of other bookings that are checked in and not
//! yet completed or cancelled ([`crate::authorisation::AuthorisationHold`]);
//! nothing is consumed at check-in. A covered consultation keeps its place in
//! the payer split while the procedure beside it goes self-pay with a consent
//! that lists only the procedure ([`sign_consent`]).
//!
//! Decisions:
//!
//! - *The services of a visit* are the ones the receptionist entered with
//!   their units ([`crate::coverage::set_services`]); none entered means the
//!   booked service with one unit. Charge capture (not built) will add the
//!   services done and consume the units.
//! - *Several authorisations.* The units come from the usable authorisations
//!   that cover the service, the one that ends first before the others, so a
//!   service can be shared between two when one is not enough.
//! - *A self-pay outcome for the whole visit* needs no authorisation: the
//!   plan is empty.
//! - *The holds are taken at the check-in* ([`hold_units`], from the move of
//!   the pending data to the visit) with the plan of that moment, and given
//!   back when the visit is completed or cancelled ([`release_holds`]). Two
//!   check-ins at the same instant could both see the last unit; the second
//!   to commit is rare enough that the hold is not locked.

use std::collections::BTreeMap;

use renox::db::now;
use renox::prelude::*;
use wl_kernel::documents::today;

use crate::appointment::Appointment;
use crate::authorisation::{AuthorisationHold, PriorAuthService};
use crate::authorisations::covering;
use crate::consent::{self, SelfPayConsent};
use crate::coverage::{CoverageOutcome, PendingCheckIn, VisitService};
use crate::db::refused;
use crate::policy::InsurancePolicy;
use crate::texts::english;
use crate::visit::Visit;

/// What a visit needs of one service that needs a prior authorisation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Need {
    /// The service code.
    pub code: String,
    /// The units the visit needs.
    pub units: i64,
    /// Where the units come from: `(authorisation id, units)`.
    pub authorised: Vec<(i64, i64)>,
    /// The units no authorisation covers: they go self-pay with consent.
    pub self_pay: i64,
}

impl Need {
    /// The units an authorisation covers.
    pub fn covered(&self) -> i64 {
        self.authorised.iter().map(|(_, units)| units).sum()
    }
}

/// The services of the visit: the ones entered, else the booked service with
/// one unit.
pub fn services_of(
    appointment: &Appointment,
    pending: Option<&PendingCheckIn>,
) -> Vec<VisitService> {
    match pending {
        Some(p) if !p.services.0.is_empty() => p.services.0.clone(),
        _ => vec![VisitService::new(&appointment.service_code, 1)],
    }
}

/// The plan of the check-in of `appointment` on its visit day: one [`Need`]
/// for each service that needs a prior authorisation. Empty for an
/// appointment that is not insured, or whose outcome is self-pay.
///
/// # Errors
///
/// The database's errors.
pub async fn plan(
    state: &AppState,
    appointment: &Appointment,
    pending: Option<&PendingCheckIn>,
) -> Result<Vec<Need>> {
    let Some(policy_id) = pending
        .and_then(|p| p.coverage_policy_id)
        .or(appointment.policy_id)
    else {
        return Ok(Vec::new());
    };
    if pending.is_some_and(|p| p.outcome == Some(CoverageOutcome::SelfPay)) {
        return Ok(Vec::new());
    }
    let policy = InsurancePolicy::find_or_404(&state.db, policy_id).await?;
    let mut taken: BTreeMap<i64, i64> = BTreeMap::new();
    let mut needs = Vec::new();
    for service in services_of(appointment, pending) {
        if !PriorAuthService::needed(&state.db, policy.payer_party_id, &service.code).await? {
            continue;
        }
        let mut found = covering(
            &state.db,
            policy.patient_party_id,
            policy.payer_party_id,
            &service.code,
            appointment.visit_date,
            None,
        )
        .await?;
        found.sort_by_key(|a| (a.valid_until, a.id));
        let mut remaining = service.units;
        let mut authorised = Vec::new();
        for authorisation in found {
            if remaining == 0 {
                break;
            }
            let held =
                AuthorisationHold::held(&state.db, authorisation.id, Some(appointment.booking_id))
                    .await?;
            let used = taken.get(&authorisation.id).copied().unwrap_or(0);
            let available = (authorisation.units_left() - held - used).max(0);
            let units = remaining.min(available);
            if units > 0 {
                authorised.push((authorisation.id, units));
                *taken.entry(authorisation.id).or_insert(0) += units;
                remaining -= units;
            }
        }
        needs.push(Need {
            code: service.code,
            units: service.units,
            authorised,
            self_pay: remaining,
        });
    }
    Ok(needs)
}

/// The services no authorisation covers in full.
pub fn shortfalls(needs: &[Need]) -> Vec<&Need> {
    needs.iter().filter(|need| need.self_pay > 0).collect()
}

/// Whether the consent in force of `booking` lists every service in
/// `short` for at least the units that go self-pay: a consent for one unit
/// does not let three through.
///
/// # Errors
///
/// The database's errors.
pub async fn consented(state: &AppState, booking: i64, short: &[&Need]) -> Result<bool> {
    let current = SelfPayConsent::current(&state.db, booking).await?;
    Ok(current.is_some_and(|c| {
        short
            .iter()
            .all(|need| c.units_of(&need.code) >= need.self_pay)
    }))
}

/// Whether `consent` lists every one of `services` for at least its units:
/// what a self-pay visit needs before the patient is checked in, for the
/// booked service and for the ones entered with
/// [`crate::coverage::set_services`].
pub fn covers_all(consent: &SelfPayConsent, services: &[VisitService]) -> bool {
    services
        .iter()
        .all(|service| consent.units_of(&service.code) >= service.units)
}

/// The patient signs the self-pay consent for the services the prior
/// authorisation does not cover: it lists only those services (the rest stays
/// in the payer split) and does not change the coverage outcome.
///
/// # Errors
///
/// A 422 when every service is covered, or a service has no self-pay price or
/// the signer's name is blank.
pub async fn sign_consent(
    state: &AppState,
    by: Option<i64>,
    appointment: &Appointment,
    pending: &PendingCheckIn,
    signer_name: &str,
    ip: Option<&str>,
    at: renox::db::DateTime,
) -> Result<SelfPayConsent> {
    let needs = plan(state, appointment, Some(pending)).await?;
    // The units that go self-pay, not the units of the visit: the patient
    // signs the price of what they will be billed.
    let wanted: Vec<(String, i64)> = shortfalls(&needs)
        .iter()
        .map(|n| (n.code.clone(), n.self_pay))
        .collect();
    if wanted.is_empty() {
        return Err(refused(english(
            "clinics.authorisation.problem.nothing_to_sign",
            &[],
        )));
    }
    consent::sign_lines(state, by, appointment, signer_name, ip, &wanted, at).await
}

/// The visit `visit` was checked in with `pending`: the units of each
/// authorisation the plan takes are held against it until the visit is
/// completed or cancelled. Returns the holds.
///
/// # Errors
///
/// The database's errors.
pub async fn hold_units(
    state: &AppState,
    visit: &Visit,
    pending: &PendingCheckIn,
) -> Result<Vec<AuthorisationHold>> {
    let Some(appointment) = Appointment::of_booking(&state.db, visit.booking_id).await? else {
        return Ok(Vec::new());
    };
    let mut holds = Vec::new();
    for need in plan(state, &appointment, Some(pending)).await? {
        for (authorisation, units) in &need.authorised {
            let mut hold = AuthorisationHold {
                authorisation_id: *authorisation,
                visit_id: visit.id,
                booking_id: visit.booking_id,
                service_code: need.code.clone(),
                units: *units,
                ..AuthorisationHold::default()
            };
            hold.save(&state.db).await?;
            holds.push(hold);
        }
    }
    Ok(holds)
}

/// The visit ended: the units it held are given back. Returns how many holds
/// were released.
///
/// # Errors
///
/// The database's errors.
pub async fn release_holds(state: &AppState, visit: i64) -> Result<u64> {
    AuthorisationHold::query()
        .where_(AuthorisationHold::VISIT_ID.eq(visit))
        .where_(AuthorisationHold::RELEASED_ON.is_null())
        .update(
            &state.db,
            &[("released_on", &Some(today())), ("updated_at", &now())],
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_need_counts_the_units_the_authorisations_cover() {
        let need = Need {
            code: "laser".into(),
            units: 3,
            authorised: vec![(1, 2)],
            self_pay: 1,
        };
        assert_eq!(need.covered(), 2);
        assert_eq!(shortfalls(std::slice::from_ref(&need)).len(), 1);
        let covered = Need {
            self_pay: 0,
            ..need
        };
        assert!(shortfalls(&[covered]).is_empty());
    }

    #[test]
    fn no_services_entered_means_the_booked_service_once() {
        let appointment = Appointment {
            service_code: "consultation".into(),
            ..Appointment::default()
        };
        assert_eq!(
            services_of(&appointment, None),
            [VisitService::new("consultation", 1)]
        );
        let pending = PendingCheckIn {
            services: renox::db::Json(vec![VisitService::new("laser", 2)]),
            ..PendingCheckIn::default()
        };
        assert_eq!(
            services_of(&appointment, Some(&pending)),
            [VisitService::new("laser", 2)]
        );
    }
}
