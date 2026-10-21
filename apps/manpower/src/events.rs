//! The events the vertical emits (09 Manpower §7.1). Other modules and the
//! issues that build on the base listen to them; each is emitted once, after
//! the commit of the transition it names.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::workforce::EmployeeId;
use wl_kernel::{PartyId, Subject};

/// A client contract became `active` (`activate`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractActivated {
    /// The contract, `manpower.client_contract:4`.
    pub contract: Subject,
    /// The client company.
    pub client: PartyId,
    /// The last day of the contract.
    pub end_date: NaiveDate,
}

impl Event for ContractActivated {}

/// A deployment became `active` (`activate`): the worker is on site.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentActivated {
    /// The deployment, `manpower.deployment:4`.
    pub deployment: Subject,
    /// The worker.
    pub employee: EmployeeId,
    /// The client company.
    pub client: PartyId,
    /// The site, `manpower.site:3`.
    pub site: Subject,
    /// The request it fills, if any.
    pub request: Option<Subject>,
    /// The role.
    pub role: String,
}

impl Event for DeploymentActivated {}

/// A client contract was renewed (`renew`, BL-MPW-05).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractRenewed {
    /// The contract, `manpower.client_contract:4`.
    pub contract: Subject,
    /// The client company.
    pub client: PartyId,
    /// The end date before the renewal.
    pub previous_end_date: NaiveDate,
    /// The new end date.
    pub end_date: NaiveDate,
}

impl Event for ContractRenewed {}

/// A client contract `end`ed (BL-MPW-06).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractEnded {
    /// The contract, `manpower.client_contract:4`.
    pub contract: Subject,
    /// The client company.
    pub client: PartyId,
    /// The last day of the contract.
    pub end_date: NaiveDate,
    /// The workers back on the bench: `active` in HR and on no other site.
    pub bench: Vec<EmployeeId>,
}

impl Event for ContractEnded {}

/// A deployment `end`ed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentEnded {
    /// The deployment.
    pub deployment: Subject,
    /// The worker, back on the bench when still `active` in HR.
    pub employee: EmployeeId,
    /// The client company.
    pub client: PartyId,
    /// Why: one of [`crate::deployment::reasons`].
    pub reason: String,
    /// The last day on site.
    pub last_day: NaiveDate,
}

impl Event for DeploymentEnded {}

/// A deployment `suspend`ed (BL-MPW-11): the worker may not work at the site
/// for now. Emitted once per suspension, after the commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentSuspended {
    /// The deployment.
    pub deployment: Subject,
    /// The worker.
    pub employee: EmployeeId,
    /// The client company.
    pub client: PartyId,
    /// The first day suspended.
    pub from: NaiveDate,
    /// Why, as people read it: *work permit*, *site access withdrawn*.
    pub reasons: Vec<String>,
}

impl Event for DeploymentSuspended {}

/// A suspended deployment `resume`d (BL-MPW-12): the worker is back on site.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentResumed {
    /// The deployment.
    pub deployment: Subject,
    /// The worker.
    pub employee: EmployeeId,
    /// The client company.
    pub client: PartyId,
    /// The first day back.
    pub on: NaiveDate,
}

impl Event for DeploymentResumed {}

/// A cover request was opened for a deployment (`MPW.DEP.02`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverRequested {
    /// The cover request, `manpower.request:4`.
    pub request: Subject,
    /// The deployment covered.
    pub deployment: Subject,
    /// The client company.
    pub client: PartyId,
    /// The first day of cover.
    pub from: NaiveDate,
    /// The last day; none for an open-ended cover.
    pub to: Option<NaiveDate>,
    /// `leave`, `suspension`, `rejection` or `manual`.
    pub cause: String,
}

impl Event for CoverRequested {}

/// A planned deployment was `cancel`led.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentCancelled {
    /// The deployment.
    pub deployment: Subject,
    /// The worker.
    pub employee: EmployeeId,
    /// The client company.
    pub client: PartyId,
    /// Why.
    pub reason: String,
}

impl Event for DeploymentCancelled {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_events_round_trip_through_json() {
        let event = ContractActivated {
            contract: Subject::new("manpower.client_contract", 4),
            client: PartyId(9),
            end_date: NaiveDate::from_ymd_opt(2027, 5, 31).unwrap(),
        };
        let json = renox::serde_json::to_string(&event).unwrap();
        assert_eq!(
            renox::serde_json::from_str::<ContractActivated>(&json).unwrap(),
            event
        );
        let ended = DeploymentEnded {
            deployment: Subject::new("manpower.deployment", 2),
            employee: EmployeeId(7),
            client: PartyId(9),
            reason: "end_date".into(),
            last_day: NaiveDate::from_ymd_opt(2027, 5, 31).unwrap(),
        };
        let json = renox::serde_json::to_string(&ended).unwrap();
        assert_eq!(
            renox::serde_json::from_str::<DeploymentEnded>(&json).unwrap(),
            ended
        );
    }
}
