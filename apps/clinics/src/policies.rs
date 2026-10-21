//! The services of the insurance policy and its eligibility results (05
//! §4.2 step 1, `CLN.INS.02`). Who may call them is decided by the pages
//! (`insurance_policies.manage`); a service refuses only what is wrong.

use renox::chrono::NaiveDate;
use renox::db::{DateTime, now};
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::documents::dubai_date;

use crate::db::refused;
use crate::policy::{EligibilityCheck, InsurancePolicy};
use crate::texts::english;

/// A policy to record.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct NewPolicy {
    /// The patient.
    pub patient: PartyId,
    /// The insurer or TPA.
    pub payer: PartyId,
    /// The plan.
    pub plan: String,
    /// The member id on the card.
    pub member_id: String,
    /// The first day of cover.
    pub valid_from: NaiveDate,
    /// The last day of cover, if it has one.
    pub valid_to: Option<NaiveDate>,
    /// The last day the card is valid, if it has an expiry.
    pub card_valid_to: Option<NaiveDate>,
}

impl NewPolicy {
    /// A policy of `patient` with `payer`, from `valid_from`, open ended and
    /// with a card that does not expire.
    pub fn new(
        patient: PartyId,
        payer: PartyId,
        plan: &str,
        member_id: &str,
        valid_from: NaiveDate,
    ) -> Self {
        NewPolicy {
            patient,
            payer,
            plan: plan.to_owned(),
            member_id: member_id.to_owned(),
            valid_from,
            valid_to: None,
            card_valid_to: None,
        }
    }

    /// The last day of cover.
    pub fn valid_to(mut self, day: NaiveDate) -> Self {
        self.valid_to = Some(day);
        self
    }

    /// The last day the card is valid.
    pub fn card_valid_to(mut self, day: NaiveDate) -> Self {
        self.card_valid_to = Some(day);
        self
    }
}

/// Records a policy.
///
/// # Errors
///
/// A 422 when the plan or the member id is empty, or the policy ends before
/// it starts; the database's errors.
pub async fn create(state: &AppState, by: Option<i64>, new: NewPolicy) -> Result<InsurancePolicy> {
    if new.plan.trim().is_empty() {
        return Err(refused(english("clinics.policies.problem.plan", &[])));
    }
    if new.member_id.trim().is_empty() {
        return Err(refused(english("clinics.policies.problem.member_id", &[])));
    }
    if new.valid_to.is_some_and(|to| to < new.valid_from) {
        return Err(refused(english("clinics.policies.problem.dates", &[])));
    }
    let mut policy = InsurancePolicy {
        patient_party_id: new.patient.0,
        payer_party_id: new.payer.0,
        plan: new.plan.trim().to_owned(),
        member_id: new.member_id.trim().to_owned(),
        valid_from: new.valid_from,
        valid_to: new.valid_to,
        card_valid_to: new.card_valid_to,
        created_by: by,
        ..InsurancePolicy::default()
    };
    policy.save(&state.db).await?;
    Ok(policy)
}

/// An updated card replaces the old one on the same policy: its new
/// *valid to* date (the image comes with the portal upload).
///
/// # Errors
///
/// A 404 for an unknown policy; the database's errors.
pub async fn update_card(
    state: &AppState,
    id: i64,
    card_valid_to: Option<NaiveDate>,
) -> Result<InsurancePolicy> {
    InsurancePolicy::find_or_404(&state.db, id).await?;
    InsurancePolicy::query()
        .where_(InsurancePolicy::ID.eq(id))
        .update(
            &state.db,
            &[("card_valid_to", &card_valid_to), ("updated_at", &now())],
        )
        .await?;
    InsurancePolicy::find_or_404(&state.db, id).await
}

/// Records the result of asking the payer whether `policy` covers the visit
/// today (`at` in Asia/Dubai), positive or negative, with the payer's
/// reference and a note.
///
/// # Errors
///
/// A 404 for an unknown policy; the database's errors.
pub async fn record_eligibility(
    state: &AppState,
    by: Option<i64>,
    policy: i64,
    positive: bool,
    reference: Option<&str>,
    note: Option<&str>,
    at: DateTime,
) -> Result<EligibilityCheck> {
    InsurancePolicy::find_or_404(&state.db, policy).await?;
    let clean = |text: Option<&str>| {
        text.map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
    };
    let mut check = EligibilityCheck {
        policy_id: policy,
        checked_on: dubai_date(at),
        checked_at: at,
        positive,
        reference: clean(reference),
        note: clean(note),
        recorded_by: by,
        ..EligibilityCheck::default()
    };
    check.save(&state.db).await?;
    Ok(check)
}
