//! The payer contracts (05 `CLN.INS.01`; base for charge capture): the
//! billing coordinator keeps them; charge capture reads the one of a policy.
//!
//! A contract is found for a policy on a day: the payer's, valid that day,
//! the one of the policy's plan before the one for every plan, the latest
//! start first.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::PartyId;

use crate::contract::{PayerContract, PayerTariff};
use crate::db::refused;
use crate::policy::InsurancePolicy;
use crate::texts::english;

/// A contract to record.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct NewContract {
    /// The payer.
    pub payer: PartyId,
    /// The plan it is for; `None` for every plan.
    pub plan: Option<String>,
    /// The payer agreement in wl-invoicing.
    pub agreement: Option<i64>,
    /// The copay in basis points.
    pub copay_bp: i64,
    /// The deductible of a visit in fils.
    pub deductible: i64,
    /// The first day.
    pub valid_from: NaiveDate,
    /// The last day.
    pub valid_to: Option<NaiveDate>,
    /// Days after the visit a first claim is accepted.
    pub first_submission_days: i64,
    /// Days after a remittance a resubmission is accepted.
    pub resubmission_days: i64,
    /// How many resubmissions of one claim the payer accepts.
    pub resubmission_limit: i64,
    /// Whether a first claim after the window is accepted with a late reason.
    pub accepts_late_claims: bool,
    /// N of `CLN.RUL.03`: the days after the visit the billing coordinator
    /// is told of a claim still in `draft` or `ready`; `None` takes
    /// [`DEFAULT_REMINDER_DAYS`], or one day less than a short window.
    pub reminder_days: Option<i64>,
}

impl NewContract {
    /// A contract of `payer` from `valid_from`: no copay, no deductible, for
    /// every plan.
    pub fn new(payer: PartyId, valid_from: NaiveDate) -> Self {
        NewContract {
            payer,
            plan: None,
            agreement: None,
            copay_bp: 0,
            deductible: 0,
            valid_from,
            valid_to: None,
            first_submission_days: DEFAULT_FIRST_SUBMISSION_DAYS,
            resubmission_days: DEFAULT_RESUBMISSION_DAYS,
            resubmission_limit: DEFAULT_RESUBMISSION_LIMIT,
            accepts_late_claims: false,
            reminder_days: None,
        }
    }

    /// For the plan `plan` only.
    pub fn plan(mut self, plan: &str) -> Self {
        self.plan = Some(plan.trim().to_owned()).filter(|p| !p.is_empty());
        self
    }

    /// Billed under the payer agreement `id` of wl-invoicing.
    pub fn agreement(mut self, id: i64) -> Self {
        self.agreement = Some(id);
        self
    }

    /// With the patient bearing `bp` basis points of the covered amount.
    pub fn copay_bp(mut self, bp: i64) -> Self {
        self.copay_bp = bp;
        self
    }

    /// With a deductible of `fils` a visit.
    pub fn deductible(mut self, fils: i64) -> Self {
        self.deductible = fils;
        self
    }

    /// Ending on `day`.
    pub fn valid_to(mut self, day: NaiveDate) -> Self {
        self.valid_to = Some(day);
        self
    }

    /// With a first-submission window of `first` days after the visit, a
    /// resubmission window of `resubmission` days after the remittance and
    /// at most `limit` resubmissions of a claim.
    pub fn windows(mut self, first: i64, resubmission: i64, limit: i64) -> Self {
        self.first_submission_days = first;
        self.resubmission_days = resubmission;
        self.resubmission_limit = limit;
        self
    }

    /// The payer accepts a first claim after its window, with a late reason.
    pub fn accepts_late_claims(mut self) -> Self {
        self.accepts_late_claims = true;
        self
    }

    /// The billing coordinator is told of a claim still in `draft` or
    /// `ready` `days` days after the visit (`CLN.RUL.03`, N); it must be
    /// below the first-submission window. 0 is allowed, for a window of one
    /// day.
    pub fn reminder_days(mut self, days: i64) -> Self {
        self.reminder_days = Some(days);
        self
    }

    /// N as it will be recorded: the one given, else 7 days or, for a window
    /// that is not longer than that, one day less than the window.
    pub fn reminder(&self) -> i64 {
        self.reminder_days
            .unwrap_or_else(|| DEFAULT_REMINDER_DAYS.min(self.first_submission_days - 1))
    }
}

/// Decision: the window of a contract that does not give one (spec: the
/// first-submission window is required, a full contract setting comes with
/// `CLN.INS.01`): 30 days to send a claim, 30 days to resubmit, twice.
pub const DEFAULT_FIRST_SUBMISSION_DAYS: i64 = 30;
/// See [`DEFAULT_FIRST_SUBMISSION_DAYS`].
pub const DEFAULT_RESUBMISSION_DAYS: i64 = 30;
/// See [`DEFAULT_FIRST_SUBMISSION_DAYS`].
pub const DEFAULT_RESUBMISSION_LIMIT: i64 = 2;
/// N of `CLN.RUL.03`: the days after a visit the billing coordinator is told
/// of a claim that is still a draft or ready.
pub const DEFAULT_REMINDER_DAYS: i64 = 7;
/// D of `CLN.RUL.03`: the days before the first-submission window ends the
/// clinic manager gets a task.
pub const TASK_DAYS_BEFORE: i64 = 2;

/// The terms of a contract the way [`create`] and [`update`] take them.
fn check_terms(new: &NewContract) -> Result {
    if !(0..=10_000).contains(&new.copay_bp)
        || new.deductible < 0
        || new.first_submission_days < 1
        || new.resubmission_days < 0
        || new.resubmission_limit < 0
        || new.valid_to.is_some_and(|to| to < new.valid_from)
    {
        return Err(refused(english("clinics.contract.problem.terms", &[])));
    }
    let reminder = new.reminder();
    if reminder < 0 || reminder >= new.first_submission_days {
        return Err(refused(english("clinics.contract.problem.reminder", &[])));
    }
    Ok(())
}

/// Records a contract.
///
/// # Errors
///
/// A 422 for a copay outside 0 to 10,000 basis points, a negative
/// deductible, an end before the start, a first-submission window below one
/// day, or a resubmission window or limit that is negative; a reminder N that
/// is negative or not below the first-submission window (`CLN.RUL.03`: "N
/// must be below the submission window").
pub async fn create(state: &AppState, by: Option<i64>, new: NewContract) -> Result<PayerContract> {
    check_terms(&new)?;
    let reminder = new.reminder();
    let mut contract = PayerContract {
        payer_party_id: new.payer.0,
        plan: new.plan,
        agreement_id: new.agreement,
        copay_bp: new.copay_bp,
        deductible: new.deductible,
        first_submission_days: new.first_submission_days,
        resubmission_days: new.resubmission_days,
        resubmission_limit: new.resubmission_limit,
        accepts_late_claims: new.accepts_late_claims,
        reminder_days: reminder,
        valid_from: new.valid_from,
        valid_to: new.valid_to,
        created_by: by,
        ..PayerContract::default()
    };
    contract.save(&state.db).await?;
    Ok(contract)
}

/// Changes the terms of contract `id` to those of `new`: the agreement, the
/// copay, the deductible, the last day, the windows, the late-claims switch
/// and the reminder days. The payer, the plan and the first day name the
/// contract and cannot change (a different one is a new contract); a charge
/// already split keeps the terms it was split under, and the claims and
/// reminders read the windows as they stand. Each change is audited with the
/// terms before and after.
///
/// # Errors
///
/// A 404 for an unknown contract; a 422 for the payer, the plan or the first
/// day changed, and for the terms [`create`] refuses.
pub async fn update(
    state: &AppState,
    by: Option<i64>,
    id: i64,
    new: NewContract,
) -> Result<PayerContract> {
    let mut contract = PayerContract::find_or_404(&state.db, id).await?;
    if new.payer.0 != contract.payer_party_id
        || new.plan != contract.plan
        || new.valid_from != contract.valid_from
    {
        return Err(refused(english("clinics.contract.problem.identity", &[])));
    }
    check_terms(&new)?;
    let before = renox::serde_json::json!({
        "agreement": contract.agreement_id,
        "copay_bp": contract.copay_bp,
        "deductible": contract.deductible,
        "valid_to": contract.valid_to,
        "first_submission_days": contract.first_submission_days,
        "resubmission_days": contract.resubmission_days,
        "resubmission_limit": contract.resubmission_limit,
        "accepts_late_claims": contract.accepts_late_claims,
        "reminder_days": contract.reminder_days,
    });
    contract.agreement_id = new.agreement;
    contract.copay_bp = new.copay_bp;
    contract.deductible = new.deductible;
    contract.valid_to = new.valid_to;
    contract.first_submission_days = new.first_submission_days;
    contract.resubmission_days = new.resubmission_days;
    contract.resubmission_limit = new.resubmission_limit;
    contract.accepts_late_claims = new.accepts_late_claims;
    contract.reminder_days = new.reminder();
    contract.save(&state.db).await?;
    let mut entry = renox::audit::Entry::new("clinics.contract_updated")
        .subject("clinics.payer_contract", contract.id)
        .data(renox::serde_json::json!({ "before": before }));
    if let Some(user) = by {
        entry = entry.user(user);
    }
    renox::audit::record(&state.db, entry).await?;
    Ok(contract)
}

/// Sets what contract `contract` says of service `code`: the payer's price
/// of one unit (`None` uses the self-pay price list) and whether the service
/// is covered. A second call for the same service replaces the first.
///
/// # Errors
///
/// A 404 for an unknown contract; a 422 for a blank code or a negative price.
pub async fn set_tariff(
    state: &AppState,
    contract: i64,
    code: &str,
    price: Option<i64>,
    covered: bool,
) -> Result<PayerTariff> {
    PayerContract::find_or_404(&state.db, contract).await?;
    let code = code.trim();
    if code.is_empty() || price.is_some_and(|p| p < 0) {
        return Err(refused(english("clinics.contract.problem.tariff", &[])));
    }
    let mut row = match PayerTariff::of(&state.db, contract, code).await? {
        Some(found) => found,
        None => PayerTariff {
            contract_id: contract,
            service_code: code.to_owned(),
            ..PayerTariff::default()
        },
    };
    row.price = price;
    row.covered = covered;
    row.save(&state.db).await?;
    Ok(row)
}

/// The contract that applies to `policy` on `day`: the payer's, valid that
/// day; the one of the policy's plan before the one for every plan; the
/// latest start first.
///
/// # Errors
///
/// The database's errors.
pub async fn for_policy(
    db: &Db,
    policy: &InsurancePolicy,
    day: NaiveDate,
) -> Result<Option<PayerContract>> {
    let mut found: Vec<PayerContract> = PayerContract::query()
        .where_(PayerContract::PAYER_PARTY_ID.eq(policy.payer_party_id))
        .order_by_desc(PayerContract::VALID_FROM)
        .get(db)
        .await?
        .into_iter()
        .filter(|contract| contract.applies_on(day))
        .filter(|contract| {
            contract
                .plan
                .as_deref()
                .is_none_or(|plan| plan == policy.plan)
        })
        .collect();
    // Stable: the latest start stays first within each group.
    found.sort_by_key(|contract| contract.plan.is_none());
    Ok(found.into_iter().next())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_contract_has_no_plan_copay_or_deductible() {
        let day: NaiveDate = "2026-11-01".parse().unwrap();
        let new = NewContract::new(PartyId(5), day)
            .plan(" Gold ")
            .copay_bp(2_000);
        assert_eq!(new.plan.as_deref(), Some("Gold"));
        assert_eq!(new.copay_bp, 2_000);
        assert_eq!(new.deductible, 0);
        assert_eq!(NewContract::new(PartyId(5), day).plan("  ").plan, None);
        let windows = NewContract::new(PartyId(5), day).windows(15, 20, 1);
        assert_eq!(
            (
                windows.first_submission_days,
                windows.resubmission_days,
                windows.resubmission_limit
            ),
            (15, 20, 1)
        );
        assert!(!windows.accepts_late_claims);
    }

    #[test]
    fn n_defaults_to_seven_days_or_below_a_short_window() {
        let day: NaiveDate = "2026-11-01".parse().unwrap();
        let new = NewContract::new(PartyId(5), day);
        assert_eq!(new.reminder(), 7);
        assert_eq!(new.clone().windows(1, 30, 2).reminder(), 0);
        assert_eq!(new.clone().windows(5, 30, 2).reminder(), 4);
        assert_eq!(new.windows(30, 30, 2).reminder_days(3).reminder(), 3);
    }
}
