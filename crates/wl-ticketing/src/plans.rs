//! Maintenance plans (wl-ticketing §3, §4.3), as far as service contracts
//! need them: work that recurs for a covered subject.
//!
//! `activate` of a contract creates one plan per covered subject and category
//! ([`MaintenancePlans::create_in`]); `WRK.RUL.18` ends them when the contract
//! ends ([`end_of_contract`]). Pausing and resuming a plan, the trigger and
//! the creation of planned visits (`WRK.PPM.01`, `WRK.RUL.03`, `WRK.RUL.04`)
//! come with the planned maintenance issues; an open generated ticket stays
//! open when its plan ends.
//!
//! ```text
//! [*] --> active --end | auto_end (system)--> ended (terminal)
//! ```

use renox::db::{DateTime, Transaction};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

/// `end`: the plan is no longer needed.
pub const END: &str = "end";
/// `end`, run by the system (the contract ended).
pub const AUTO_END: &str = "auto_end";

use crate::permissions::MAINTENANCE_PLANS_MANAGE as MANAGE;

/// Where a plan is in its life (wl-ticketing §4.3). Stored as `active` or
/// `ended` (`paused` comes with the planned maintenance issues).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PlanStatus {
    /// Generating work when due.
    #[default]
    Active,
    /// No longer needed (asset sold, contract ended).
    Ended,
}

/// A maintenance plan.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_ticketing_maintenance_plans",
    state_machine,
    index(contract_id)
)]
pub struct MaintenancePlan {
    /// The primary key.
    pub id: i64,
    /// The service contract that created it, if one did.
    #[model(references = "wl_ticketing_service_contracts")]
    pub contract_id: Option<i64>,
    /// The type of the subject the work is for.
    pub subject_type: String,
    /// The id of the subject.
    pub subject_id: i64,
    /// The category of the tickets it creates.
    #[model(references = "wl_ticketing_categories")]
    pub category_id: i64,
    /// When it ended.
    pub ended_at: Option<DateTime>,
    /// The lifecycle state; changed only by a transition.
    pub status: PlanStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl MaintenancePlan {
    /// The subject the work is for.
    pub fn subject(&self) -> Subject {
        Subject::new(&self.subject_type, self.subject_id)
    }
}

/// One plan to create, as `activate` works it out from the coverage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanSeed {
    /// The type of the subject.
    pub subject_type: String,
    /// The id of the subject.
    pub subject_id: i64,
    /// The category of the tickets.
    pub category_id: i64,
}

/// The service for plans, as the spec's `MaintenancePlans` names it.
#[derive(Debug, Clone, Copy, Default)]
pub struct MaintenancePlans;

impl MaintenancePlans {
    /// Creates an `active` plan of `contract` for `seed` inside `tx`.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn create_in(
        tx: &mut Transaction,
        contract: i64,
        seed: &PlanSeed,
    ) -> Result<MaintenancePlan> {
        let mut plan = MaintenancePlan {
            contract_id: Some(contract),
            subject_type: seed.subject_type.clone(),
            subject_id: seed.subject_id,
            category_id: seed.category_id,
            status: PlanStatus::Active,
            ..MaintenancePlan::default()
        };
        plan.insert(&mut *tx).await?;
        Ok(plan)
    }

    /// The plans of `contract`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_contract(db: &Db, contract: i64) -> Result<Vec<MaintenancePlan>> {
        MaintenancePlan::query()
            .where_(MaintenancePlan::CONTRACT_ID.eq(contract))
            .order_by(MaintenancePlan::ID)
            .get(db)
            .await
    }
}

/// Ends the plan as `user`.
///
/// # Errors
///
/// `Forbidden` without `maintenance_plans.manage`; `NotFrom` unless it is
/// `active`.
pub async fn end(
    state: &AppState,
    user: &AuthUser,
    plan: &MaintenancePlan,
) -> std::result::Result<(), TransitionError> {
    let _outcome = plan.transition(END).by(user).run(state).await?;
    Ok(())
}

/// Ends the active plans of `contract` as the system; returns how many.
///
/// # Errors
///
/// The database's errors.
pub async fn end_of_contract(state: &AppState, contract: i64) -> Result<usize> {
    let mut ended = 0;
    for plan in MaintenancePlans::of_contract(&state.db, contract).await? {
        if plan.status != PlanStatus::Active {
            continue;
        }
        match plan.transition(AUTO_END).by_system().run(state).await {
            Ok(_) => ended += 1,
            // Ended meanwhile by someone else.
            Err(TransitionError::NotFrom(_) | TransitionError::Conflict) => {}
            Err(other) => return Err(other.into_error()),
        }
    }
    Ok(ended)
}

/// Stores `ended_at` for both ends.
fn ending(transition: Transition<MaintenancePlan>) -> Transition<MaintenancePlan> {
    transition
        .from([PlanStatus::Active])
        .to(PlanStatus::Ended)
        .effect(|plan: &MaintenancePlan, _ctx, tx| {
            let id = plan.id;
            Box::pin(async move {
                let at = Some(renox::db::now());
                MaintenancePlan::query()
                    .where_(MaintenancePlan::ID.eq(id))
                    .update(&mut *tx, &[("ended_at", &at)])
                    .await?;
                Ok(())
            })
        })
}

impl StateMachine for MaintenancePlan {
    type State = PlanStatus;
    const COLUMN: renox::db::Col<MaintenancePlan, PlanStatus> = MaintenancePlan::STATUS;

    fn machine() -> Machine<Self> {
        Machine::new()
            .initial([PlanStatus::Active])
            .state(PlanStatus::Active)
            .tone("success")
            .state(PlanStatus::Ended)
            .tone("neutral")
            .terminal()
            .transition(ending(
                Transition::new(END)
                    .label("End")
                    .tone("danger")
                    .permission(MANAGE),
            ))
            .transition(ending(
                Transition::new(AUTO_END).label("End (system)").system(),
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plan_is_active_until_it_ends() {
        let machine = MaintenancePlan::machine();
        assert_eq!(machine.initial_states(), ["active"]);
        let end = machine.find("end").unwrap();
        assert_eq!(end.from_states(), ["active"]);
        assert_eq!(end.to_state(), Some("ended"));
        assert!(!end.is_system());
        assert!(machine.find("auto_end").unwrap().is_system());
        assert!(machine.states().iter().any(|s| s.name == "ended" && s.terminal));
    }

    #[test]
    fn a_plan_is_about_a_subject() {
        let plan = MaintenancePlan {
            subject_type: "property.building_system".into(),
            subject_id: 4,
            ..MaintenancePlan::default()
        };
        assert_eq!(plan.subject(), Subject::new("property.building_system", 4));
    }
}
