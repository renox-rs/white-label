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
}

/// Records a contract.
///
/// # Errors
///
/// A 422 for a copay outside 0 to 10,000 basis points, a negative
/// deductible, or an end before the start.
pub async fn create(state: &AppState, by: Option<i64>, new: NewContract) -> Result<PayerContract> {
    if !(0..=10_000).contains(&new.copay_bp)
        || new.deductible < 0
        || new.valid_to.is_some_and(|to| to < new.valid_from)
    {
        return Err(refused(english("clinics.contract.problem.terms", &[])));
    }
    let mut contract = PayerContract {
        payer_party_id: new.payer.0,
        plan: new.plan,
        agreement_id: new.agreement,
        copay_bp: new.copay_bp,
        deductible: new.deductible,
        valid_from: new.valid_from,
        valid_to: new.valid_to,
        created_by: by,
        ..PayerContract::default()
    };
    contract.save(&state.db).await?;
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
        let new = NewContract::new(PartyId(5), day).plan(" Gold ").copay_bp(2_000);
        assert_eq!(new.plan.as_deref(), Some("Gold"));
        assert_eq!(new.copay_bp, 2_000);
        assert_eq!(new.deductible, 0);
        assert_eq!(NewContract::new(PartyId(5), day).plan("  ").plan, None);
    }
}
