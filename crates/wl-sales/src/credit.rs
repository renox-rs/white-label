//! The credit check an amendment re-runs and the hold it can lead to
//! (wl-sales §4.2 rows `amend` and `hold`, `SAL.RUL.01`; BL-SAL-05, shared
//! with BL-SAL-10).
//!
//! The check runs only for an order on account and only when the new total is
//! above the old one. The exposure is the customer's exposure without this
//! order (the kernel's [`CreditExposure`](wl_kernel::sales::CreditExposure))
//! plus the order at its new total; it is checked against the customer's limit
//! and the group's combined limit. Fails: the order goes to `credit_hold`
//! through the system transition [`HOLD`], which stores the state before the
//! hold in `held_from` (BL-SAL-10 releases to it), and the accountant and the
//! sales rep are told (`SAL.RUL.01`). Nothing provides credit limits until
//! BL-SAL-10: the default has none, so the check passes.

use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition};
use wl_kernel::sales::credit_exposure;

use crate::order_notices;
use crate::sales_order::{PAYMENT_ON_ACCOUNT, SalesOrder, SalesOrderStatus};
use crate::texts::english;

/// `pending_acceptance | confirmed | partially_fulfilled → credit_hold`.
pub const HOLD: &str = "hold";

/// The transition: the order is blocked by the credit check.
pub(crate) fn hold() -> Transition<SalesOrder> {
    Transition::new(HOLD)
        .label("Put on credit hold")
        .tone("warning")
        .system()
        .from([
            SalesOrderStatus::PendingAcceptance,
            SalesOrderStatus::Confirmed,
            SalesOrderStatus::PartiallyFulfilled,
        ])
        .to(SalesOrderStatus::CreditHold)
        .effect(|order: &SalesOrder, _ctx, tx| {
            let id = order.id;
            let before = Some(order.status.as_str().to_owned());
            Box::pin(async move {
                SalesOrder::query()
                    .where_(SalesOrder::ID.eq(id))
                    .update(&mut *tx, &[("held_from", &before)])
                    .await?;
                Ok(())
            })
        })
}

/// The machine with the hold added.
pub(crate) fn add_to(machine: Machine<SalesOrder>) -> Machine<SalesOrder> {
    machine.transition(hold())
}

/// Whether `exposure` (without the order) plus the order at `total` is over
/// `limit`.
pub fn over(exposure: i64, total: i64, limit: Option<i64>) -> bool {
    limit.is_some_and(|limit| exposure + total > limit)
}

/// Re-runs the credit check of the amended order `order_id` whose total was
/// `old_gross` fils; puts it on hold when the check fails. Returns whether the
/// order is now on hold because of this check.
///
/// # Errors
///
/// A database error, or the notifier's.
pub async fn check_after_amend(state: &AppState, order_id: i64, old_gross: i64) -> Result<bool> {
    let order = SalesOrder::find_or_404(&state.db, order_id).await?;
    if order.payment_mode != PAYMENT_ON_ACCOUNT || order.gross_total <= old_gross {
        return Ok(false);
    }
    let subject = order.subject();
    let Some(position) = credit_exposure(state)
        .position(state, order.party(), &order.currency, &subject)
        .await?
    else {
        return Ok(false);
    };
    let own = over(position.exposure, order.gross_total, position.limit);
    let group = match position.group_exposure {
        Some(exposure) => over(exposure, order.gross_total, position.group_limit),
        None => false,
    };
    if !(own || group) {
        return Ok(false);
    }
    if order.status == SalesOrderStatus::CreditHold {
        // Amended while on hold: it stays on hold, the release is still needed.
        return Ok(false);
    }
    let reason = english("wl_sales.credit.hold_reason", &[]);
    let _ = order
        .transition(HOLD)
        .by_system()
        .reason(reason)
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    let held = SalesOrder::find_or_404(&state.db, order_id).await?;
    order_notices::credit_hold(state, &held, position.exposure + held.gross_total).await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_exposure_is_the_rest_plus_the_order_at_its_new_total() {
        // 36,000 without the order + 4,800 = 40,800.
        assert!(!over(3_600_000, 480_000, Some(5_000_000)));
        assert!(over(3_600_000, 480_000, Some(4_050_000)));
        assert!(!over(3_600_000, 480_000, None));
        assert!(!over(3_600_000, 480_000, Some(4_080_000)));
    }

    #[test]
    fn hold_is_a_system_transition_into_credit_hold() {
        let machine = SalesOrder::machine();
        let t = machine.find(HOLD).expect(HOLD);
        assert!(t.is_system());
        assert_eq!(t.to_state(), Some("credit_hold"));
        assert_eq!(
            t.from_states(),
            ["pending_acceptance", "confirmed", "partially_fulfilled"]
        );
    }
}
