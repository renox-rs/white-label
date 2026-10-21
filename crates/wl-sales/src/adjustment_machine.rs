//! The PointsAdjustment state machine (wl-sales §4.7, `SAL.ACL.07`;
//! BL-SAL-24). See [`crate::adjustment`] for the diagram.
//!
//! Every transition is `.system()`: [`crate::adjustments`] runs exactly one of
//! `apply` and `submit_over_limit` after reading the limit, and gives the
//! machine that [`AdjustInput`] so the guards agree. `submit_over_limit` asks
//! the kernel approval engine (`approvals::require_in`, action
//! `loyalty.adjustment_over_limit`, requester = `requested_by`); the kernel
//! runs `submit_over_limit_approved` on the last yes and `back_to_draft`
//! when the request ends without one. A listener on `ApprovalDecided` then
//! runs `reject` after a rejection.
//!
//! The approved run has no input (the kernel makes it), so what it writes is
//! read from the database inside the transaction: the account, its
//! programme's point value and validity, and today's date.

use renox::db::{Transaction, now};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::{Machine, StateMachine, Transition};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{self, Require, Required};
use wl_kernel::documents::today;
use wl_kernel::machines::SubjectMachine;

use crate::adjustment::{ADJUSTMENT_SUBJECT_TYPE, AdjustmentStatus, PointsAdjustment};
use crate::loyalty_account::{AccountStatus, LoyaltyAccount};
use crate::loyalty_points::{CAUSE_ADJUST, PointsKind, PointsMovement};
use crate::loyalty_programme::LoyaltyProgramme;
use crate::loyalty_rules::expiry;
use crate::policy;
use crate::redemption_machine::consume_lots;
use crate::texts::english;

/// `draft → applied`: at or below the limit.
pub const APPLY: &str = "apply";
/// `draft → pending_approval`: above the limit, or closing with points.
pub const SUBMIT_OVER_LIMIT: &str = "submit_over_limit";
/// `pending_approval → applied`: the kernel runs it on the last yes.
pub const SUBMIT_OVER_LIMIT_APPROVED: &str = "submit_over_limit_approved";
/// `pending_approval → draft`: the kernel runs it when the request ends
/// without a yes (reject, return, withdraw, expiry, failed final check).
pub const BACK_TO_DRAFT: &str = "back_to_draft";
/// `draft → rejected`: an approver rejected the request.
pub const REJECT: &str = "reject";

/// The stored reason of a rejection with no comment.
const REJECTED: &str = "rejected";

/// What `apply` and `submit_over_limit` take: the limit the guards check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdjustInput {
    /// The points above which an adjustment needs a manager.
    pub limit: i64,
}

/// A 422 with the English text `key`.
fn refused(key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key, &[]))
}

/// Writes the `adjust` movement of `adj` inside `tx`: a positive adjustment
/// is a new lot, a negative one is taken from the lots, oldest expiry first;
/// closing takes everything that is left and closes the account.
async fn write(tx: &mut Transaction, adj: &PointsAdjustment, approver: Option<i64>) -> Result {
    let Some(account) = LoyaltyAccount::find(&mut *tx, adj.account_id).await? else {
        return Err(refused("wl_sales.adjust.problem.no_account"));
    };
    let Some(programme) = LoyaltyProgramme::find(&mut *tx, account.programme_id).await? else {
        return Err(refused("wl_sales.adjust.problem.no_account"));
    };
    if account.status != AccountStatus::Active {
        return Err(refused("wl_sales.adjust.problem.closed"));
    }
    let points = if adj.closes_account {
        -account.points_balance.max(0)
    } else {
        adj.points
    };
    if points < 0 && -points > account.points_balance.max(0) {
        return Err(refused("wl_sales.adjust.problem.more_than_held"));
    }
    let subject = adj.subject();
    let template = PointsMovement {
        account_id: account.id,
        kind: PointsKind::Adjust,
        source_type: subject.subject_type.clone(),
        source_id: subject.subject_id,
        cause: CAUSE_ADJUST.to_owned(),
        reason: Some(adj.reason.clone()),
        approval_request_id: adj.approval_request_id,
        user_id: Some(adj.requested_by),
        ..PointsMovement::default()
    };
    let movement = if points > 0 {
        let mut lot = PointsMovement {
            points,
            value: points * programme.point_value,
            lot_expires_on: Some(expiry(today(), programme.validity_months)),
            ..template
        };
        lot.insert(&mut *tx).await?;
        LoyaltyAccount::query()
            .where_(LoyaltyAccount::ID.eq(account.id))
            .increment(&mut *tx, "points_balance", points)
            .await?;
        Some(lot.id)
    } else if points < 0 {
        consume_lots(tx, &template, -points, None, programme.point_value).await?
    } else {
        None
    };
    if adj.closes_account {
        let reason = Some(format!(
            "{} {}",
            english("wl_sales.adjust.closed_by_staff", &[]),
            adj.reason
        ));
        let closed_at = Some(now());
        LoyaltyAccount::query()
            .where_(LoyaltyAccount::ID.eq(account.id))
            .update(
                &mut *tx,
                &[
                    ("status", &AccountStatus::Closed),
                    ("closed_reason", &reason),
                    ("closed_at", &closed_at),
                    ("updated_at", &now()),
                ],
            )
            .await?;
    }
    PointsAdjustment::query()
        .where_(PointsAdjustment::ID.eq(adj.id))
        .update(
            &mut *tx,
            &[
                ("points", &points),
                ("movement_id", &movement),
                ("approved_by", &approver),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(())
}

/// Sets non-status columns of adjustment `id` outside the machine's guard and
/// stamps `updated_at`.
async fn set(
    tx: &mut Transaction,
    id: i64,
    columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn renox::db::ToDbValue + Sync))> = columns.to_vec();
    let stamp = now();
    all.push(("updated_at", &stamp));
    PointsAdjustment::query()
        .where_(PointsAdjustment::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}

impl StateMachine for PointsAdjustment {
    type State = AdjustmentStatus;
    const COLUMN: renox::db::Col<PointsAdjustment, AdjustmentStatus> = PointsAdjustment::STATUS;

    fn machine() -> Machine<Self> {
        use AdjustmentStatus::{Applied, Draft, PendingApproval, Rejected};
        Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(PendingApproval)
            .tone("warning")
            .state(Applied)
            .tone("success")
            .terminal()
            .state(Rejected)
            .tone("danger")
            .terminal()
            .transition(apply())
            .transition(submit_over_limit())
            .transition(
                Transition::new(SUBMIT_OVER_LIMIT_APPROVED)
                    .label("Approve")
                    .from([PendingApproval])
                    .to(Applied)
                    .system()
                    .effect(|a: &PointsAdjustment, ctx, tx| {
                        // The user who gave the last yes.
                        let approver = ctx.meta().get("actor_id").and_then(Value::as_i64);
                        let a = a.clone();
                        Box::pin(async move { write(tx, &a, approver).await })
                    }),
            )
            .transition(
                Transition::new(BACK_TO_DRAFT)
                    .label("Approval ended")
                    .from([PendingApproval])
                    .to(Draft)
                    .system()
                    .effect(|a: &PointsAdjustment, ctx, tx| {
                        // The rejection comment, or the kernel's text.
                        let reason = ctx.reason().map(|r| r.trim().to_owned());
                        let id = a.id;
                        Box::pin(async move { set(tx, id, &[("outcome_reason", &reason)]).await })
                    }),
            )
            .transition(
                Transition::new(REJECT)
                    .label("Reject")
                    .from([Draft])
                    .to(Rejected)
                    .system()
                    .effect(|a: &PointsAdjustment, ctx, tx| {
                        let reason = ctx
                            .reason()
                            .map(str::trim)
                            .filter(|r| !r.is_empty())
                            .map(str::to_owned)
                            .or_else(|| a.outcome_reason.clone())
                            .unwrap_or_else(|| REJECTED.to_owned());
                        let id = a.id;
                        Box::pin(
                            async move { set(tx, id, &[("outcome_reason", &Some(reason))]).await },
                        )
                    }),
            )
    }
}

/// The limit the run was given.
fn limit_of(ctx: &renox::state_machine::TransitionCtx) -> Option<i64> {
    ctx.input_as::<AdjustInput>().ok().map(|i| i.limit)
}

/// `apply`: at or below the limit, written on the spot.
fn apply() -> Transition<PointsAdjustment> {
    Transition::new(APPLY)
        .label("Apply")
        .from([AdjustmentStatus::Draft])
        .to(AdjustmentStatus::Applied)
        .system()
        .guard("within_limit", |a: &PointsAdjustment, ctx| {
            let verdict = match limit_of(ctx) {
                Some(limit) if !a.needs_manager(limit) => Ok(()),
                Some(_) => Err(english("wl_sales.adjust.problem.needs_manager", &[])),
                None => Err(english("wl_sales.adjust.problem.no_limit", &[])),
            };
            Box::pin(async move { verdict })
        })
        .effect(|a: &PointsAdjustment, _ctx, tx| {
            let a = a.clone();
            Box::pin(async move {
                set(tx, a.id, &[("outcome_reason", &None::<String>)]).await?;
                write(tx, &a, None).await
            })
        })
}

/// `submit_over_limit`: above the limit, an approval request.
fn submit_over_limit() -> Transition<PointsAdjustment> {
    Transition::new(SUBMIT_OVER_LIMIT)
        .label("Request approval")
        .from([AdjustmentStatus::Draft])
        .to(AdjustmentStatus::PendingApproval)
        .system()
        .guard("over_limit", |a: &PointsAdjustment, ctx| {
            let verdict = match limit_of(ctx) {
                Some(limit) if a.needs_manager(limit) => Ok(()),
                Some(_) => Err(english("wl_sales.adjust.problem.within_limit", &[])),
                None => Err(english("wl_sales.adjust.problem.no_limit", &[])),
            };
            Box::pin(async move { verdict })
        })
        .effect(|a: &PointsAdjustment, _ctx, tx| {
            let a = a.clone();
            Box::pin(async move {
                let require = Require::new(
                    policy::ADJUSTMENT_ACTION,
                    a.subject(),
                    SUBMIT_OVER_LIMIT,
                    a.requested_by,
                    "draft",
                )
                .payload(json!({
                    "account_id": a.account_id,
                    "points": a.points,
                    "closes_account": a.closes_account,
                }));
                match approvals::require_in(&mut *tx, &require).await? {
                    Required::Pending(request) => {
                        set(
                            tx,
                            a.id,
                            &[
                                ("approval_request_id", &Some(request.id)),
                                ("outcome_reason", &None::<String>),
                            ],
                        )
                        .await
                    }
                    // Without a policy a large adjustment would be applied
                    // unchecked; the transaction rolls back instead.
                    Required::Allowed => Err(refused("wl_sales.adjust.problem.no_policy")),
                }
            })
        })
}

/// The machine's registry entry: the waiting state `pending_approval`, the
/// action `loyalty.adjustment_over_limit` and the global scope.
pub fn subject_machine() -> SubjectMachine {
    SubjectMachine::of::<PointsAdjustment>(ADJUSTMENT_SUBJECT_TYPE)
        .waiting(AdjustmentStatus::PendingApproval)
        .action(SUBMIT_OVER_LIMIT, policy::ADJUSTMENT_ACTION)
        .scope(|_a: &PointsAdjustment| renox::auth::permissions::Scope::global())
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_transition_of_the_spec_is_declared_and_system() {
        let machine = PointsAdjustment::machine();
        let table = [
            (APPLY, "draft", "applied"),
            (SUBMIT_OVER_LIMIT, "draft", "pending_approval"),
            (SUBMIT_OVER_LIMIT_APPROVED, "pending_approval", "applied"),
            (BACK_TO_DRAFT, "pending_approval", "draft"),
            (REJECT, "draft", "rejected"),
        ];
        assert_eq!(machine.transitions().len(), table.len());
        for (name, from, to) in table {
            let t = machine.find(name).expect(name);
            assert!(t.is_system(), "{name}");
            assert_eq!(t.from_states(), [from], "{name}");
            assert_eq!(t.to_state(), Some(to), "{name}");
        }
        assert_eq!(machine.initial_states(), ["draft"]);
        assert_eq!(machine.find(APPLY).unwrap().guard_names(), ["within_limit"]);
        assert_eq!(
            machine.find(SUBMIT_OVER_LIMIT).unwrap().guard_names(),
            ["over_limit"]
        );
    }

    #[test]
    fn the_registry_entry_waits_in_pending_approval() {
        let m = subject_machine();
        assert_eq!(m.waiting_state(), Some("pending_approval"));
        assert_eq!(
            m.action_key(SUBMIT_OVER_LIMIT),
            "loyalty.adjustment_over_limit"
        );
    }
}
