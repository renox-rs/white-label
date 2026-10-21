//! The prior authorisation: a payer's approval for named services and a
//! number of units, valid to a date (05 §6 *Prior authorisation*,
//! `CLN.INS.03`; base for BL-CLN-02 and BL-CLN-03).
//!
//! The status moves only through the machine in
//! [`crate::authorisation_machine`]; [`crate::authorisations`] holds the
//! services. This file has the records:
//!
//! - [`PriorAuthorisation`]: the patient's policy, the services it covers, the
//!   units requested, approved and consumed, the payer's reference, the last
//!   day it is valid, the reason it was denied.
//! - [`PriorAuthService`]: the payer's list of services that need an
//!   authorisation. Decision: the payer contract (`CLN.INS.01`) is not built;
//!   its prior-auth list is this table, one row per payer and service code,
//!   kept by [`crate::authorisations::require_for`].
//! - [`AuthorisationHold`]: the units a checked-in visit counts against an
//!   authorisation until the visit is completed or cancelled (05 §4.2: units
//!   left at check-in = approved − consumed − units held by the other visits).
//!   Nothing is consumed at check-in; consumption is charge capture's.

use renox::chrono::NaiveDate;
use renox::db::{DateTime, Json};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

/// The subject type of a prior authorisation.
pub const AUTHORISATION_SUBJECT_TYPE: &str = "clinics.prior_authorisation";

/// Where a prior authorisation stands (05 §7.1 *Prior authorisation*).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AuthStatus {
    /// Being prepared.
    #[default]
    Draft,
    /// Sent to the payer.
    Requested,
    /// Approved for the requested services and units.
    Approved,
    /// Approved with fewer units.
    PartiallyApproved,
    /// Refused (terminal).
    Denied,
    /// All approved units consumed (terminal).
    Used,
    /// The validity ended (terminal).
    Expired,
    /// No longer needed (terminal).
    Cancelled,
}

impl AuthStatus {
    /// Whether the authorisation can cover a visit: `approved` or
    /// `partially_approved`.
    pub fn is_usable(self) -> bool {
        matches!(self, AuthStatus::Approved | AuthStatus::PartiallyApproved)
    }
}

/// A prior authorisation.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "clinics_prior_authorisations",
    state_machine,
    index(patient_party_id),
    index(status)
)]
pub struct PriorAuthorisation {
    /// The primary key.
    pub id: i64,
    /// The policy it is for.
    #[model(references = "clinics_insurance_policies")]
    pub policy_id: i64,
    /// The patient, a party.
    pub patient_party_id: i64,
    /// The payer, a party (the policy's).
    pub payer_party_id: i64,
    /// The codes of the services it covers.
    pub services: Json<Vec<String>>,
    /// The units asked for.
    pub units_requested: i64,
    /// The units approved; 0 until it is.
    pub units_approved: i64,
    /// The units charge capture consumed.
    pub units_consumed: i64,
    /// The payer's reference.
    pub reference: Option<String>,
    /// The last day it is valid (Asia/Dubai); it is valid all that day.
    pub valid_until: Option<NaiveDate>,
    /// Why the payer refused it.
    pub denial_reason: Option<String>,
    /// The day it was sent to the payer.
    pub requested_on: Option<NaiveDate>,
    /// The day the payer answered.
    pub decided_on: Option<NaiveDate>,
    /// The user who recorded the answer.
    pub decided_by: Option<i64>,
    /// The user who prepared it.
    pub created_by: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: AuthStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl PriorAuthorisation {
    /// The authorisation as a [`Subject`]: `clinics.prior_authorisation:7`.
    pub fn subject(&self) -> Subject {
        Subject::new(AUTHORISATION_SUBJECT_TYPE, self.id)
    }

    /// The number shown to staff: `PA-0007`.
    pub fn number(&self) -> String {
        format!("PA-{:04}", self.id)
    }

    /// Whether it covers the service `code`.
    pub fn covers(&self, code: &str) -> bool {
        self.services.0.iter().any(|service| service == code)
    }

    /// The units approved and not yet consumed.
    pub fn units_left(&self) -> i64 {
        (self.units_approved - self.units_consumed).max(0)
    }

    /// Whether it can cover a visit on `day`: usable, and `day` is not after
    /// the last day it is valid (the last day counts, Asia/Dubai).
    pub fn is_valid_on(&self, day: NaiveDate) -> bool {
        self.status.is_usable() && self.valid_until.is_some_and(|until| until >= day)
    }

    /// The usable authorisations of a patient with a payer, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn usable_of(
        db: &Db,
        patient: i64,
        payer: i64,
    ) -> Result<Vec<PriorAuthorisation>> {
        let all = PriorAuthorisation::query()
            .where_(PriorAuthorisation::PATIENT_PARTY_ID.eq(patient))
            .where_(PriorAuthorisation::PAYER_PARTY_ID.eq(payer))
            .order_by(PriorAuthorisation::ID)
            .get(db)
            .await?;
        Ok(all.into_iter().filter(|a| a.status.is_usable()).collect())
    }

    /// Every authorisation of a patient, newest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_patient(db: &Db, patient: i64) -> Result<Vec<PriorAuthorisation>> {
        PriorAuthorisation::query()
            .where_(PriorAuthorisation::PATIENT_PARTY_ID.eq(patient))
            .order_by_desc(PriorAuthorisation::ID)
            .get(db)
            .await
    }
}

/// A service a payer wants authorised before it is delivered.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "clinics_prior_auth_services", unique(payer_party_id, service_code))]
pub struct PriorAuthService {
    /// The primary key.
    pub id: i64,
    /// The payer, a party.
    pub payer_party_id: i64,
    /// The service code.
    pub service_code: String,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl PriorAuthService {
    /// Whether the payer's list has the service `code`.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn needed(db: &Db, payer: i64, code: &str) -> Result<bool> {
        Ok(PriorAuthService::query()
            .where_(PriorAuthService::PAYER_PARTY_ID.eq(payer))
            .where_(PriorAuthService::SERVICE_CODE.eq(code))
            .first(db)
            .await?
            .is_some())
    }

    /// The payer's list, by code.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_payer(db: &Db, payer: i64) -> Result<Vec<PriorAuthService>> {
        PriorAuthService::query()
            .where_(PriorAuthService::PAYER_PARTY_ID.eq(payer))
            .order_by(PriorAuthService::SERVICE_CODE)
            .get(db)
            .await
    }
}

/// Units of an authorisation a checked-in visit counts for one service.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "clinics_authorisation_holds",
    index(authorisation_id),
    index(visit_id)
)]
pub struct AuthorisationHold {
    /// The primary key.
    pub id: i64,
    /// The authorisation the units count against.
    #[model(references = "clinics_prior_authorisations")]
    pub authorisation_id: i64,
    /// The checked-in visit.
    #[model(references = "clinics_visits")]
    pub visit_id: i64,
    /// The visit's booking.
    pub booking_id: i64,
    /// The service the units are for.
    pub service_code: String,
    /// The units held.
    pub units: i64,
    /// The day the visit was completed or cancelled and the units were
    /// given back (Asia/Dubai).
    pub released_on: Option<NaiveDate>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl AuthorisationHold {
    /// The units of authorisation `id` held by visits still open, leaving out
    /// the visit of booking `except_booking`.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn held(db: &Db, id: i64, except_booking: Option<i64>) -> Result<i64> {
        let holds = AuthorisationHold::query()
            .where_(AuthorisationHold::AUTHORISATION_ID.eq(id))
            .where_(AuthorisationHold::RELEASED_ON.is_null())
            .get(db)
            .await?;
        Ok(holds
            .iter()
            .filter(|hold| Some(hold.booking_id) != except_booking)
            .map(|hold| hold.units)
            .sum())
    }

    /// The holds of the visit `visit`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_visit(db: &Db, visit: i64) -> Result<Vec<AuthorisationHold>> {
        AuthorisationHold::query()
            .where_(AuthorisationHold::VISIT_ID.eq(visit))
            .order_by(AuthorisationHold::ID)
            .get(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> NaiveDate {
        text.parse().expect("a date")
    }

    fn approved(units: i64, consumed: i64, until: &str) -> PriorAuthorisation {
        PriorAuthorisation {
            status: AuthStatus::Approved,
            units_approved: units,
            units_consumed: consumed,
            valid_until: Some(day(until)),
            services: Json(vec!["laser".to_owned()]),
            ..PriorAuthorisation::default()
        }
    }

    #[test]
    fn the_last_day_of_the_validity_counts() {
        let auth = approved(3, 0, "2026-11-10");
        assert!(auth.is_valid_on(day("2026-11-10")));
        assert!(!auth.is_valid_on(day("2026-11-11")));
    }

    #[test]
    fn only_an_approved_authorisation_is_usable() {
        let mut auth = approved(3, 0, "2026-11-10");
        for (status, usable) in [
            (AuthStatus::Draft, false),
            (AuthStatus::Requested, false),
            (AuthStatus::Approved, true),
            (AuthStatus::PartiallyApproved, true),
            (AuthStatus::Denied, false),
            (AuthStatus::Used, false),
            (AuthStatus::Expired, false),
            (AuthStatus::Cancelled, false),
        ] {
            auth.status = status;
            assert_eq!(auth.is_valid_on(day("2026-11-01")), usable, "{status:?}");
        }
    }

    #[test]
    fn the_units_left_are_approved_less_consumed() {
        assert_eq!(approved(3, 1, "2026-11-10").units_left(), 2);
        assert_eq!(approved(3, 3, "2026-11-10").units_left(), 0);
        assert!(approved(3, 0, "2026-11-10").covers("laser"));
        assert!(!approved(3, 0, "2026-11-10").covers("peel"));
    }
}
