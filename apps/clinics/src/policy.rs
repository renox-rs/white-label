//! The patient's insurance policy and the eligibility results recorded for it
//! (05 §6 *Insurance policy*, `CLN.INS.02`).
//!
//! A policy is the patient's cover with a payer: plan, member id, the dates
//! it is valid, and the date the card is valid to. It is built to be
//! extended: prior authorisations (`CLN.INS.03`), the payer contract and the
//! patient-share rules (`CLN.INS.01`, `CLN.ENC.02`) hang on `policy_id`.
//!
//! **Expired** means the policy's *valid to* date, or the card's expiry, is
//! before the visit date in Asia/Dubai; a policy valid to the visit date is
//! valid all day ([`InsurancePolicy::is_valid_on`]).
//!
//! Decision: the card image is not kept by the base. It needs
//! `patients.sensitive.view` (`CLN.ACL.12`) and comes with the portal upload
//! (BL-CLN-04); an updated card is recorded as its new valid-to date
//! ([`crate::policies::update_card`]).

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::{PartyId, Subject};

/// The subject type of an insurance policy.
pub const POLICY_SUBJECT_TYPE: &str = "clinics.insurance_policy";

/// A patient's insurance policy.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "clinics_insurance_policies", index(patient_party_id))]
pub struct InsurancePolicy {
    /// The primary key.
    pub id: i64,
    /// The patient, a party.
    pub patient_party_id: i64,
    /// The insurer or TPA, a party.
    pub payer_party_id: i64,
    /// The plan.
    pub plan: String,
    /// The member id on the card.
    pub member_id: String,
    /// The first day of cover.
    pub valid_from: NaiveDate,
    /// The last day of cover; `None` while open ended.
    pub valid_to: Option<NaiveDate>,
    /// The last day the card is valid; `None` when it has no expiry.
    pub card_valid_to: Option<NaiveDate>,
    /// The user who recorded it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl InsurancePolicy {
    /// The policy as a [`Subject`]: `clinics.insurance_policy:7`.
    pub fn subject(&self) -> Subject {
        Subject::new(POLICY_SUBJECT_TYPE, self.id)
    }

    /// The patient as a [`PartyId`].
    pub fn patient(&self) -> PartyId {
        PartyId(self.patient_party_id)
    }

    /// The payer as a [`PartyId`].
    pub fn payer(&self) -> PartyId {
        PartyId(self.payer_party_id)
    }

    /// Whether the policy's dates cover `day`: it has started and its
    /// *valid to* is not before `day`.
    pub fn covers(&self, day: NaiveDate) -> bool {
        self.valid_from <= day && self.valid_to.is_none_or(|to| to >= day)
    }

    /// Whether the card is valid on `day`.
    pub fn card_valid_on(&self, day: NaiveDate) -> bool {
        self.card_valid_to.is_none_or(|to| to >= day)
    }

    /// Whether the policy and its card are both valid on `day`.
    pub fn is_valid_on(&self, day: NaiveDate) -> bool {
        self.covers(day) && self.card_valid_on(day)
    }
}

/// An eligibility result: the payer was asked, through its portal or by
/// phone, whether the policy covers the visit today (05 §4.2 step 1).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "clinics_eligibility_checks", index(policy_id))]
pub struct EligibilityCheck {
    /// The primary key.
    pub id: i64,
    /// The policy asked about.
    #[model(references = "clinics_insurance_policies")]
    pub policy_id: i64,
    /// The day it was asked (Asia/Dubai).
    pub checked_on: NaiveDate,
    /// When it was recorded.
    pub checked_at: DateTime,
    /// The answer: positive or negative.
    pub positive: bool,
    /// The payer's reference for the answer, if it gave one.
    pub reference: Option<String>,
    /// A note of the receptionist.
    pub note: Option<String>,
    /// The user who recorded it.
    pub recorded_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl EligibilityCheck {
    /// The latest result recorded for `policy` on `day`, if any.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn latest_on(
        db: &Db,
        policy: i64,
        day: NaiveDate,
    ) -> Result<Option<EligibilityCheck>> {
        EligibilityCheck::query()
            .where_(EligibilityCheck::POLICY_ID.eq(policy))
            .where_(EligibilityCheck::CHECKED_ON.eq(day))
            .order_by_desc(EligibilityCheck::ID)
            .first(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> NaiveDate {
        text.parse().expect("a date")
    }

    fn policy(valid_to: Option<&str>, card_to: Option<&str>) -> InsurancePolicy {
        InsurancePolicy {
            valid_from: day("2026-01-01"),
            valid_to: valid_to.map(day),
            card_valid_to: card_to.map(day),
            ..InsurancePolicy::default()
        }
    }

    #[test]
    fn a_policy_valid_to_the_visit_day_is_valid_all_day() {
        let p = policy(Some("2026-11-10"), None);
        assert!(p.is_valid_on(day("2026-11-10")));
        assert!(!p.is_valid_on(day("2026-11-11")));
        assert!(p.covers(day("2026-01-01")));
        assert!(!p.covers(day("2025-12-31")));
    }

    #[test]
    fn an_expired_card_fails_a_policy_that_is_still_in_force() {
        let p = policy(Some("2027-03-31"), Some("2026-11-09"));
        assert!(p.covers(day("2026-11-10")));
        assert!(!p.card_valid_on(day("2026-11-10")));
        assert!(!p.is_valid_on(day("2026-11-10")));
        assert!(policy(None, None).is_valid_on(day("2030-01-01")));
    }
}
