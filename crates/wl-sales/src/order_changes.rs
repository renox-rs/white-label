//! What follows a change of a placed order once it committed (BL-SAL-05,
//! BL-SAL-06, BL-SAL-08, BL-SAL-09), and the wiring the changes share:
//!
//! - [`announce_pending`] reads the change records the transition wrote in its
//!   transaction ([`OrderChange`]) and, once for each, emits the events
//!   (`SalesOrderAmended`, one `SalesOrderLineCancelled` per cancelled line),
//!   re-runs the credit check, settles the money ([`crate::settle`]) and
//!   tells the customer. A change the kernel ran at the last approval and one a
//!   user ran go the same way, because both end in a transition of the order.
//! - [`request_or_run`] is the entry of every change that may need a manager:
//!   it asks the kernel for an approval request (action key, payload and
//!   requester; the requester cannot approve) or runs the transition.
//! - [`register`] registers the order with the kernel's approval engine (the
//!   twins `<transition>_approved` are run through it), the default policies,
//!   the final re-check and the link from "Needs your action" to the order.

use renox::prelude::*;
use renox::state_machine::StateMachine;
use serde::Serialize;
use wl_kernel::approvals::{self, ApprovalRequest, Require, Required};
use wl_kernel::sales::{
    LineOutcome, SALES_ORDER_SUBJECT_TYPE, SalesOrderAmended, SalesOrderCancelled,
    SalesOrderLineCancelled,
};

use crate::amend::{self, AmendDetail};
use crate::cancel_lines::{self, CancelDetail};
use crate::order_change::{KIND_AMEND, KIND_CANCEL, KIND_CANCEL_LINE, KIND_CANCEL_REMAINING, OrderChange};
use crate::order_notices;
use crate::sales_order::SalesOrder;
use crate::texts::english;

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// Whether the transition `name` is one of the changes of this module
/// (a user's or the kernel's `_approved` twin).
pub(crate) fn is_change(name: &str) -> bool {
    let base = name.strip_suffix(wl_kernel::machines::APPROVED_SUFFIX).unwrap_or(name);
    amend::TRANSITIONS.iter().any(|(n, _)| *n == base)
        || [
            cancel_lines::CANCEL_LINE,
            cancel_lines::CANCEL_LINE_PARTLY,
            cancel_lines::CANCEL_LINE_LAST,
            cancel_lines::CANCEL_REMAINING,
            cancel_lines::CANCEL_ORDER,
        ]
        .contains(&base)
}

/// Asks for the approval of the change `name` of `order` (action key
/// `action`, payload `input`) when `needs_approval`, else runs the transition
/// as `user` with `reason`. Returns the request when the change waits for a
/// manager.
///
/// # Errors
///
/// 422 when a request for the order and action already waits, or when
/// `needs_approval` and no policy exists for the action (the change must not
/// run unchecked); the transition's own refusals otherwise.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn request_or_run<I: Serialize>(
    state: &AppState,
    user: &AuthUser,
    order: &SalesOrder,
    action: &str,
    name: &str,
    input: &I,
    needs_approval: bool,
    reason: &str,
) -> Result<Option<Box<ApprovalRequest>>> {
    let waiting = ApprovalRequest::pending_about(&state.db, &order.subject()).await?;
    if waiting.iter().any(|request| request.action == action) {
        return Err(refused(english("wl_sales.changes.pending", &[])));
    }
    if needs_approval {
        let payload = renox::serde_json::to_value(input)
            .map_err(|e| Error::Internal(renox::anyhow::Error::new(e)))?;
        let mut tx = state.db.begin().await?;
        let require = Require::new(action, order.subject(), name, user.id, order.status.as_str())
            .payload(payload);
        return match approvals::require_in(&mut tx, &require).await? {
            Required::Pending(request) => {
                tx.commit().await?;
                approvals::announce_request(state, &request).await?;
                Ok(Some(request))
            }
            Required::Allowed => {
                tx.rollback().await?;
                Err(refused(english("wl_sales.changes.no_policy", &[])))
            }
        };
    }
    let _ = order
        .transition(name)
        .by(user)
        .reason(reason.to_owned())
        .input(input)
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    Ok(None)
}

/// Announces the changes of `order_id` whose transition committed: each is
/// claimed first, so it is announced once. A failure of the money or of a
/// notice is printed and does not undo the change (a person acts on the
/// settlement rows that failed).
///
/// # Errors
///
/// A database error, or an event listener's.
pub async fn announce_pending(state: &AppState, order_id: i64) -> Result {
    for change in OrderChange::unannounced(&state.db, order_id).await? {
        if !OrderChange::claim(&state.db, change.id).await? {
            continue;
        }
        match change.kind.as_str() {
            KIND_AMEND => amended(state, &change).await?,
            KIND_CANCEL_LINE | KIND_CANCEL_REMAINING => cancelled(state, &change).await?,
            KIND_CANCEL => order_cancelled(state, &change).await?,
            _ => {}
        }
    }
    Ok(())
}

/// An amendment: the event, the credit check, the money and the revised
/// confirmation.
async fn amended(state: &AppState, change: &OrderChange) -> Result {
    let Some(detail) = change.detail_as::<AmendDetail>() else {
        return Ok(());
    };
    let order = SalesOrder::find_or_404(&state.db, change.order_id).await?;
    state
        .emit(SalesOrderAmended {
            order: order.order_id(),
            party: order.party(),
            revision: detail.revision_after,
            lines: detail.lines.clone(),
            value_before: detail.value_before,
            value_after: detail.value_after,
            reason: change.reason.clone(),
        })
        .await?;
    if let Err(error) = crate::credit::check_after_amend(state, order.id, detail.value_before).await
    {
        eprintln!("sales: credit check of order {} failed: {error}", order.number);
    }
    let order = SalesOrder::find_or_404(&state.db, change.order_id).await?;
    if !detail.no_money
        && let Err(error) = crate::settle::amendment(state, &order, change, &detail).await
    {
        eprintln!("sales: the money of the amendment of order {} failed: {error}", order.number);
    }
    order_notices::order_revised(state, &order, &detail).await
}

/// A cancelled line or the remaining lines: one event per line, then the
/// money, then the invoice when the order is complete.
async fn cancelled(state: &AppState, change: &OrderChange) -> Result {
    let Some(detail) = change.detail_as::<CancelDetail>() else {
        return Ok(());
    };
    let order = SalesOrder::find_or_404(&state.db, change.order_id).await?;
    for line in &detail.lines {
        state
            .emit(SalesOrderLineCancelled {
                order: order.order_id(),
                party: order.party(),
                line: line.line,
                quantity_milli: line.quantity_milli,
                reason: change.reason.clone(),
                outcome: line.outcome,
                fee: line.fee,
                fee_waived: line.fee_waived,
            })
            .await?;
    }
    if let Err(error) = crate::settle::cancellation(state, &order, change, &detail).await {
        eprintln!("sales: the money of the cancellation of order {} failed: {error}", order.number);
    }
    order_notices::lines_cancelled(state, &order, &detail).await
}

/// A whole order cancelled (BL-SAL-07): the event with the outcome and fee of
/// each started line (stock and the vertical act on it), then the money: one
/// refund of what was paid less the fees, and the fee document.
async fn order_cancelled(state: &AppState, change: &OrderChange) -> Result {
    let Some(detail) = change.detail_as::<CancelDetail>() else {
        return Ok(());
    };
    let order = SalesOrder::find_or_404(&state.db, change.order_id).await?;
    state
        .emit(SalesOrderCancelled {
            order: order.order_id(),
            party: order.party(),
            reason: change.reason.clone(),
            outcomes: outcomes_of(&detail),
        })
        .await?;
    if let Err(error) = crate::settle::cancellation(state, &order, change, &detail).await {
        eprintln!("sales: the money of the cancellation of order {} failed: {error}", order.number);
    }
    Ok(())
}

/// The outcomes of a cancellation as the order event carries them
/// (BL-SAL-07): the started lines only.
pub fn outcomes_of(detail: &CancelDetail) -> Vec<LineOutcome> {
    detail
        .lines
        .iter()
        .filter_map(|line| {
            line.outcome.map(|kind| LineOutcome {
                line: line.line,
                kind,
                fee: line.fee,
                fee_waived: line.fee_waived,
            })
        })
        .collect()
}

/// The final check of a request about an order: the module's actions are
/// re-read on the order as it is; any other request goes through.
async fn recheck(
    tx: &mut renox::db::Transaction,
    request: &ApprovalRequest,
) -> Result<Option<String>> {
    match request.action.as_str() {
        amend::ACTION => amend::recheck(tx, request).await,
        cancel_lines::ACTION_LINE | cancel_lines::ACTION_REMAINING | cancel_lines::ACTION_ORDER => {
            cancel_lines::recheck(tx, request).await
        }
        _ => Ok(None),
    }
}

/// Registers the order with the kernel's approval engine, the policies of the
/// changes and their final check, and the link to the order from the
/// approvals inbox.
pub(crate) fn register(_app: &mut Registry) {
    // The kernel runs `<transition>_approved` on the order through this entry.
    wl_kernel::SubjectMachine::of::<SalesOrder>(SALES_ORDER_SUBJECT_TYPE)
        .scope(|_order: &SalesOrder| renox::auth::permissions::Scope::global())
        .register();
    for policy in crate::policy::order_change_policies() {
        policy.register();
    }
    approvals::register_final_check(SALES_ORDER_SUBJECT_TYPE, |tx, request| {
        Box::pin(recheck(tx, request))
    });
    approvals::register_subject_link(SALES_ORDER_SUBJECT_TYPE, |id| format!("/sales-orders/{id}"));
    order_notices::register_types();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cancel_lines::CancelledLine;
    use wl_kernel::sales::LineOutcomeKind;

    #[test]
    fn the_changes_and_their_twins_are_told_apart_from_the_other_transitions() {
        assert!(is_change("amend"));
        assert!(is_change("amend_on_hold_approved"));
        assert!(is_change("cancel_line_last"));
        assert!(is_change("cancel_remaining_approved"));
        assert!(is_change("cancel"));
        assert!(is_change("cancel_approved"));
        assert!(!is_change("fulfil"));
        assert!(!is_change("hold"));
    }

    #[test]
    fn only_the_started_lines_carry_an_outcome_into_the_event() {
        let detail = CancelDetail {
            lines: vec![
                CancelledLine {
                    line: 1,
                    outcome: Some(LineOutcomeKind::Scrap),
                    fee: 800,
                    started: true,
                    ..CancelledLine::default()
                },
                CancelledLine {
                    line: 2,
                    ..CancelledLine::default()
                },
            ],
            complete: false,
            whole: false,
        };
        let outcomes = outcomes_of(&detail);
        assert_eq!(outcomes.len(), 1);
        assert_eq!((outcomes[0].line, outcomes[0].fee), (1, 800));
    }
}
