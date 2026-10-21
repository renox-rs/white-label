//! The 03 Logistics rules the host runs (03 §7.2; BL-LOG-25): `LOG.RUL.10`,
//! which grounds a vehicle whose documents expired, and the compliance-critical
//! marks of 03.
//!
//! **Compliance-critical rules** (§7.2, owner alert `PLT.RUL.04` to the company
//! admin and the client's owner role): `LOG.RUL.10`, `DOC.RUL.03` for the
//! blocking kinds of [`logistics::defaults::KINDS`] and `WRK.RUL.15` (the
//! midday break). A run of one that fails after its last attempt, or a nightly
//! one that does not run, raises the alert (wl-kernel §5.15).
//!
//! **`LOG.RUL.10`** runs daily at 00:30 Asia/Dubai as a kernel rule run for
//! the day: every vehicle on a `planned` or `dispatched` run is asked for its
//! blocking documents (Mulkiya, insurance, permit) on that day, and one that
//! has expired is grounded ([`logistics::grounding::ground`]): the fleet
//! managers and the dispatchers are told. Dispatch itself is refused by the
//! document guard.
//!
//! **Approvals** (`WRK.ACL.10`): the module also puts the approval of a
//! delivery's reversal and of a manual `mark_lost` with the operations manager
//! and the company admin.

use std::collections::BTreeSet;

use logistics::defaults::{self, KINDS, VEHICLE_SUBJECT};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_delivery::{Run, RunStatus, VehicleType};
use wl_kernel::Subject;
use wl_kernel::approvals::PolicyDefault;
use wl_kernel::automation::{RuleRunSpec, RunOutcome, run_rule};
use wl_kernel::compliance::{CriticalRule, occurrence_key, schedule_subject};
use wl_kernel::documents::{InvalidReason, Validity, guard, today};

use super::roles::{ADMIN, OPERATIONS_MANAGER};

/// The name of the scheduled task of `LOG.RUL.10`.
pub const GROUND_TASK: &str = "logistics-vehicle-documents";

/// Marks the rules of 03 compliance-critical for a `logistics` client;
/// `LOG.RUL.10` is also watched for a missed nightly run.
pub fn register_critical(client_app: &str) {
    if client_app != super::logistics_seed::LOGISTICS {
        return;
    }
    CriticalRule::new(defaults::GROUND_RULE)
        .daily_at(defaults::GROUND_RULE_AT)
        .register();
    for rule in defaults::CRITICAL_RULES {
        if rule != defaults::GROUND_RULE {
            CriticalRule::new(rule).register();
        }
    }
}

/// The ids of the vehicles on the open runs.
async fn vehicles_in_use(db: &Db) -> Result<BTreeSet<i64>> {
    let runs = Run::query()
        .where_(Run::STATUS.is_in([RunStatus::Planned, RunStatus::Dispatched]))
        .get(db)
        .await?;
    Ok(runs
        .into_iter()
        .filter(|run| run.vehicle_type != VehicleType::OnFoot)
        .filter_map(|run| run.vehicle_asset_id)
        .collect())
}

/// Grounds the vehicles on open runs whose blocking documents had expired on
/// `day`. Returns how many vehicle documents grounded a vehicle now.
async fn ground_expired(state: &AppState, day: NaiveDate) -> Result<usize> {
    let kinds: Vec<&str> = KINDS
        .iter()
        .filter(|kind| kind.is_legal_block() && kind.subjects == [VEHICLE_SUBJECT])
        .map(|kind| kind.key)
        .collect();
    let mut grounded = 0;
    for vehicle in vehicles_in_use(&state.db).await? {
        let subject = Subject::new(VEHICLE_SUBJECT, vehicle);
        for kind in &kinds {
            let validity = guard(state).is_valid(&state.db, &subject, kind, day).await?;
            if let Validity::Invalid {
                reason: InvalidReason::Expired,
                expired_on: Some(expired),
                ..
            } = validity
                && logistics::grounding::ground(state, vehicle, kind, expired, day).await?
            {
                grounded += 1;
            }
        }
    }
    Ok(grounded)
}

/// `LOG.RUL.10` for `day` (an Asia/Dubai date) as a rule run, once for the
/// day: a failing run is retried with the kernel's backoff and, after the last
/// attempt, alerts the owner.
///
/// # Errors
///
/// Database errors of the run itself.
pub async fn run_ground_rule(state: &AppState, day: NaiveDate) -> Result<RunOutcome> {
    let spec = RuleRunSpec::new(
        defaults::GROUND_RULE,
        schedule_subject(),
        occurrence_key(defaults::GROUND_RULE, day),
    )
    .compliance_critical();
    run_rule(state, spec, move |state| async move {
        ground_expired(&state, day).await.map(|_| ())
    })
    .await
}

/// The module that schedules `LOG.RUL.10` on a logistics client.
pub struct LogisticsRules;

impl Module for LogisticsRules {
    fn name(&self) -> &'static str {
        "logistics_rules"
    }

    fn register(&self, app: &mut Registry) {
        // `WRK.ACL.10`: a reversal of a delivery and a manual `mark_lost` are
        // approved by the operations manager or the company admin, never the
        // requester (a dispatcher asks) and never the run's driver. This runs
        // after wl-delivery's own register, whose default (any other holder of
        // `deliveries.reverse`) it replaces.
        PolicyDefault::new(wl_delivery::reverse::ACTION)
            .step([OPERATIONS_MANAGER, ADMIN])
            .expiry_hours(Some(wl_delivery::reverse::EXPIRY_HOURS))
            .register();
        PolicyDefault::new(wl_delivery::lost::ACTION)
            .step([OPERATIONS_MANAGER, ADMIN])
            .expiry_hours(Some(wl_delivery::lost::EXPIRY_HOURS))
            .register();
        app.schedule()
            .daily_at(defaults::GROUND_RULE_AT, GROUND_TASK, |state: AppState| async move {
                run_ground_rule(&state, today()).await.map(|_| ())
            })
            .timezone("Asia/Dubai");
    }
}
