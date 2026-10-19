//! The Broadcast state machine (wl-notify §4.3, #61 A4).
//!
//! ```text
//! [*] --> draft --request--> scheduled --start--> sending --complete--> completed
//!         draft --request_over_limit--> pending_approval
//!         pending_approval --request_over_limit_approved--> scheduled
//!         pending_approval --reject | expire--> cancelled
//!         pending_approval | scheduled --cancel--> cancelled
//! ```
//!
//! A row is inserted as `draft` and moved by `request` or
//! `request_over_limit` in the same transaction, so the machine needs a
//! state to leave. `request` (estimate at most the limit) and
//! `request_over_limit` (above it) are system transitions: the limit comes
//! in their input ([`RequestInput`]). The effect of `request_over_limit`
//! asks the kernel approval engine (`approvals::require_in`, action
//! [`ACTION`]) with the requester as the user who asked and stores the
//! request id; the caller announces the request after the commit. The kernel
//! runs `request_over_limit_approved` on the last yes, which stores the
//! approver. `reject` (a rejected request, reason = the approver's comment),
//! `expire` (run by #63) and the system twin `cancel_system` (a withdrawn
//! request, an ended requester) are for the system; `cancel` is for a user
//! who is the requester or holds `notifications.marketing.send`. `start`
//! and `complete` are run by the broadcast run (#61 part 5); `complete`
//! stores the counts it is given ([`CompleteInput`]).
//!
//! The kernel ends a request without a yes by running `back_to_draft` on
//! a broadcast still in `pending_approval` (its waiting state); here that
//! cancels it, so `reject` and `expire` are for a direct run.
//!
//! The status changes only through this machine: `Broadcast` is
//! `#[model(state_machine)]`. The machine is registered with the kernel as
//! the subject type `notify.broadcast`, with the waiting state
//! `pending_approval`.

use renox::db::Json;
use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{self, Require, Required};
use wl_kernel::broadcast::BROADCAST_SUBJECT_TYPE;
use wl_kernel::machines::SubjectMachine;

use crate::broadcast::{Broadcast, BroadcastResults, BroadcastStatus, ExcludedCounts};
use crate::permissions;
use crate::texts::english;

/// The subject type of a broadcast in the kernel registry.
pub const SUBJECT_TYPE: &str = BROADCAST_SUBJECT_TYPE;

/// The approval policies' action key of a broadcast above the limit.
pub const ACTION: &str = "notifications.broadcast_approval";

/// `draft → scheduled`: the estimate is within the limit.
pub const REQUEST: &str = "request";
/// `draft → pending_approval`: the estimate is above the limit.
pub const REQUEST_OVER_LIMIT: &str = "request_over_limit";
/// `pending_approval → scheduled`: the kernel runs it on the last yes.
pub const REQUEST_OVER_LIMIT_APPROVED: &str = "request_over_limit_approved";
/// `pending_approval → cancelled`: the kernel runs it when the approval ends
/// without a yes (reject, return, withdraw, expiry, failed final check),
/// because `pending_approval` is the waiting state and the request was made
/// from `draft`; the reason is the approver's comment or the kernel's text.
pub const BACK_TO_DRAFT: &str = "back_to_draft";
/// `pending_approval → cancelled`: an approver rejected the request.
pub const REJECT: &str = "reject";
/// `scheduled → sending`: the send time came.
pub const START: &str = "start";
/// `sending → completed`: every recipient has a message or an exclusion.
pub const COMPLETE: &str = "complete";
/// `pending_approval | scheduled → cancelled`: by a user.
pub const CANCEL: &str = "cancel";
/// The system twin of [`CANCEL`]: a withdrawn request, an ended requester.
pub const CANCEL_SYSTEM: &str = "cancel_system";
/// `pending_approval → cancelled`: the approval ran out (run by #63).
pub const EXPIRE: &str = "expire";

/// The stored reason of an expired approval.
pub const APPROVAL_EXPIRED: &str = "approval_expired";
/// The stored reason of a cancel with none given.
pub const CANCELLED: &str = "cancelled";
/// The stored reason of a reject with no comment.
pub const REJECTED: &str = "rejected";

/// The input of `request` and `request_over_limit`: the estimate limit the
/// caller read from the settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestInput {
    /// A broadcast whose estimate is above this needs approval.
    pub limit: i64,
}

/// The input of `complete`: what the run counted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompleteInput {
    /// The recipients resolved when it started.
    pub recipients_resolved: i64,
    /// The members left out, by reason.
    pub excluded: ExcludedCounts,
    /// The results of the messages.
    pub results: BroadcastResults,
}

impl StateMachine for Broadcast {
    type State = BroadcastStatus;
    const COLUMN: renox::db::Col<Broadcast, BroadcastStatus> = Broadcast::STATUS;

    fn machine() -> Machine<Self> {
        use BroadcastStatus::{Cancelled, Completed, Draft, PendingApproval, Scheduled, Sending};
        let machine = Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(PendingApproval)
            .tone("warning")
            .state(Scheduled)
            .tone("info")
            .state(Sending)
            .tone("info")
            .state(Completed)
            .tone("success")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal();
        [
            request(),
            request_over_limit(),
            Transition::new(REQUEST_OVER_LIMIT_APPROVED)
                .label("Approve")
                .from([PendingApproval])
                .to(Scheduled)
                .system()
                .effect(|b: &Broadcast, ctx, tx| {
                    // The user who gave the last yes.
                    let approver = ctx.meta().get("actor_id").and_then(Value::as_i64);
                    let id = b.id;
                    Box::pin(async move { set(&mut *tx, id, &[("approved_by", &approver)]).await })
                }),
            system(
                BACK_TO_DRAFT,
                "Approval ended",
                [PendingApproval],
                Cancelled,
                REJECTED,
            ),
            system(REJECT, "Reject", [PendingApproval], Cancelled, REJECTED),
            system(
                EXPIRE,
                "Expire",
                [PendingApproval],
                Cancelled,
                APPROVAL_EXPIRED,
            ),
            system(
                CANCEL_SYSTEM,
                "Cancel",
                [PendingApproval, Scheduled],
                Cancelled,
                CANCELLED,
            ),
            user_cancel(),
            Transition::new(START)
                .label("Start")
                .from([Scheduled])
                .to(Sending)
                .system()
                .effect(|b: &Broadcast, _ctx, tx| {
                    let id = b.id;
                    Box::pin(async move {
                        set(&mut *tx, id, &[("started_at", &Some(renox::db::now()))]).await
                    })
                }),
            complete(),
        ]
        .into_iter()
        .fold(machine, Machine::transition)
    }
}

/// Sets non-status columns of broadcast `id`, outside the machine's guard,
/// and stamps `updated_at`.
async fn set(
    tx: &mut renox::db::Transaction,
    id: i64,
    columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn renox::db::ToDbValue + Sync))> = columns.to_vec();
    let now = renox::db::now();
    all.push(("updated_at", &now));
    Broadcast::query()
        .where_(Broadcast::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}

/// A 422 with `message`.
fn refused(message: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.to_owned())
}

/// The limit a run was given.
fn limit_of(ctx: &TransitionCtx) -> Option<i64> {
    ctx.input_as::<RequestInput>().ok().map(|i| i.limit)
}

/// The `request` transition: within the limit, no approval.
fn request() -> Transition<Broadcast> {
    Transition::new(REQUEST)
        .label("Request")
        .from([BroadcastStatus::Draft])
        .to(BroadcastStatus::Scheduled)
        .system()
        .guard("within_limit", |b: &Broadcast, ctx| {
            let estimate = b.estimate;
            let limit = limit_of(ctx);
            Box::pin(async move {
                match limit {
                    Some(limit) if estimate <= limit => Ok(()),
                    Some(_) => Err(english("wl_notify.broadcast.problem.over_limit")),
                    None => Err(english("wl_notify.broadcast.problem.no_limit")),
                }
            })
        })
}

/// The `request_over_limit` transition: above the limit, an approval request.
fn request_over_limit() -> Transition<Broadcast> {
    Transition::new(REQUEST_OVER_LIMIT)
        .label("Request approval")
        .from([BroadcastStatus::Draft])
        .to(BroadcastStatus::PendingApproval)
        .system()
        .guard("over_limit", |b: &Broadcast, ctx| {
            let estimate = b.estimate;
            let limit = limit_of(ctx);
            Box::pin(async move {
                match limit {
                    Some(limit) if estimate > limit => Ok(()),
                    Some(_) => Err(english("wl_notify.broadcast.problem.within_limit")),
                    None => Err(english("wl_notify.broadcast.problem.no_limit")),
                }
            })
        })
        .effect(|b: &Broadcast, _ctx, tx| {
            let b = b.clone();
            Box::pin(async move {
                let require = Require::new(
                    ACTION,
                    b.broadcast_id().subject(),
                    REQUEST_OVER_LIMIT,
                    b.requested_by,
                    "draft",
                )
                .payload(json!({
                    "estimate": b.estimate,
                    "channel": b.channel.to_string(),
                    "segment_id": b.segment_id,
                }));
                match approvals::require_in(&mut *tx, &require).await? {
                    Required::Pending(request) => {
                        set(
                            &mut *tx,
                            b.id,
                            &[("approval_request_id", &Some(request.id))],
                        )
                        .await
                    }
                    // Without a policy a broadcast above the limit would go
                    // out unapproved; the transaction rolls back instead.
                    Required::Allowed => Err(refused(&english(
                        "wl_notify.broadcast.problem.no_approval_policy",
                    ))),
                }
            })
        })
}

/// A system transition `name` into `to` that stores its reason: the one
/// given, else `default`.
fn system<const N: usize>(
    name: &str,
    label: &str,
    from: [BroadcastStatus; N],
    to: BroadcastStatus,
    default: &'static str,
) -> Transition<Broadcast> {
    Transition::new(name)
        .label(label)
        .from(from)
        .to(to)
        .system()
        .effect(store_reason(default))
}

/// An effect that stores the run's reason (trimmed), else `default`, as the
/// cancel reason.
fn store_reason(
    default: &'static str,
) -> impl for<'a> Fn(
    &'a Broadcast,
    &'a TransitionCtx,
    &'a mut renox::db::Transaction,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result> + Send + 'a>>
+ Send
+ Sync
+ 'static {
    move |b, ctx, tx| {
        let id = b.id;
        let reason = ctx
            .reason()
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .unwrap_or(default)
            .to_owned();
        Box::pin(async move { set(&mut *tx, id, &[("cancel_reason", &Some(reason))]).await })
    }
}

/// `cancel`: the requester or a holder of `notifications.marketing.send`.
fn user_cancel() -> Transition<Broadcast> {
    Transition::new(CANCEL)
        .label("Cancel")
        .tone("danger")
        .from([BroadcastStatus::PendingApproval, BroadcastStatus::Scheduled])
        .to(BroadcastStatus::Cancelled)
        .guard("requester_or_holder", |b: &Broadcast, ctx| {
            let allowed = ctx.actor().is_some_and(|user| {
                user.id == b.requested_by || user.has_permission(permissions::MARKETING_SEND)
            });
            Box::pin(async move {
                if allowed {
                    Ok(())
                } else {
                    Err(english("wl_notify.broadcast.problem.cancel_not_allowed"))
                }
            })
        })
        .effect(store_reason(CANCELLED))
}

/// `complete`: stores the counts the run gives.
fn complete() -> Transition<Broadcast> {
    Transition::new(COMPLETE)
        .label("Complete")
        .from([BroadcastStatus::Sending])
        .to(BroadcastStatus::Completed)
        .system()
        .effect(|b: &Broadcast, ctx, tx| {
            let id = b.id;
            let input = ctx.input_as::<CompleteInput>().unwrap_or_default();
            Box::pin(async move {
                set(
                    &mut *tx,
                    id,
                    &[
                        ("recipients_resolved", &Some(input.recipients_resolved)),
                        ("excluded", &Json(input.excluded)),
                        ("results", &Json(input.results)),
                        ("completed_at", &Some(renox::db::now())),
                    ],
                )
                .await
            })
        })
}

/// The machine's registry entry: the waiting state `pending_approval`.
pub fn subject_machine() -> SubjectMachine {
    SubjectMachine::of::<Broadcast>(SUBJECT_TYPE)
        .waiting(BroadcastStatus::PendingApproval)
        .build()
}

/// Registers the machine with the kernel and `rnx state:diagram`.
pub(crate) fn register() {
    subject_machine().register();
    renox::state_machine::register::<Broadcast>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_transition_of_the_spec_is_declared() {
        let machine = Broadcast::machine();
        let table = [
            (REQUEST, &["draft"][..], "scheduled", true),
            (REQUEST_OVER_LIMIT, &["draft"], "pending_approval", true),
            (
                REQUEST_OVER_LIMIT_APPROVED,
                &["pending_approval"],
                "scheduled",
                true,
            ),
            (BACK_TO_DRAFT, &["pending_approval"], "cancelled", true),
            (REJECT, &["pending_approval"], "cancelled", true),
            (EXPIRE, &["pending_approval"], "cancelled", true),
            (
                CANCEL_SYSTEM,
                &["pending_approval", "scheduled"],
                "cancelled",
                true,
            ),
            (
                CANCEL,
                &["pending_approval", "scheduled"],
                "cancelled",
                false,
            ),
            (START, &["scheduled"], "sending", true),
            (COMPLETE, &["sending"], "completed", true),
        ];
        assert_eq!(machine.transitions().len(), table.len());
        for (name, from, to, system) in table {
            let transition = machine.find(name).expect(name);
            assert_eq!(transition.from_states(), from, "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
            assert_eq!(transition.is_system(), system, "{name}");
        }
    }

    #[test]
    fn a_broadcast_starts_a_draft_and_completed_cancelled_are_terminal() {
        let machine = Broadcast::machine();
        assert_eq!(machine.initial_states(), ["draft"]);
        let terminal: Vec<_> = machine
            .states()
            .iter()
            .filter(|s| s.terminal)
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(terminal, ["completed", "cancelled"]);
        assert_eq!(machine.states().len(), BroadcastStatus::ALL.len());
    }

    #[test]
    fn the_machine_is_registered_with_the_waiting_state_and_the_approved_twin() {
        let machine = subject_machine();
        assert_eq!(machine.subject_type(), "notify.broadcast");
        assert_eq!(machine.waiting_state(), Some("pending_approval"));
        assert!(machine.transition(REQUEST_OVER_LIMIT_APPROVED).is_some());
    }

    fn ctx(limit: Option<i64>) -> TransitionCtx {
        let ctx = TransitionCtx::new(None);
        match limit {
            Some(limit) => ctx.with_input(json!({ "limit": limit })),
            None => ctx,
        }
    }

    async fn refusal(name: &str, estimate: i64, limit: Option<i64>) -> Option<String> {
        let broadcast = Broadcast {
            estimate,
            ..Default::default()
        };
        match Broadcast::machine()
            .find(name)
            .unwrap()
            .check_guards(&broadcast, &ctx(limit))
            .await
        {
            Ok(()) => None,
            Err(e) => Some(e.to_string()),
        }
    }

    #[renox::test]
    async fn the_estimate_picks_request_or_request_over_limit() {
        assert_eq!(refusal(REQUEST, 200, Some(200)).await, None);
        assert!(refusal(REQUEST, 201, Some(200)).await.is_some());
        assert_eq!(refusal(REQUEST_OVER_LIMIT, 201, Some(200)).await, None);
        assert!(refusal(REQUEST_OVER_LIMIT, 200, Some(200)).await.is_some());
        assert!(refusal(REQUEST, 1, None).await.is_some());
        assert!(refusal(REQUEST_OVER_LIMIT, 1, None).await.is_some());
    }

    #[renox::test]
    async fn a_user_cancel_without_a_user_is_refused() {
        let broadcast = Broadcast::default();
        let refused = Broadcast::machine()
            .find(CANCEL)
            .unwrap()
            .check_guards(&broadcast, &ctx(None))
            .await;
        assert!(refused.is_err());
    }
}
