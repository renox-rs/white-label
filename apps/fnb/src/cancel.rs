//! Cancelling an order, before and after the outlet accepted it (06 F&B §7.1
//! `cancel`, `SAL.ACL.01`, `SAL.ACL.05`, `SAL.ACL.04`; BL-FNB-04).
//!
//! - **The customer** cancels a `placed` direct order from the order page
//!   ([`by_customer`]): the sales order is cancelled as the system with the
//!   payment not kept, so wl-sales asks for the full refund, and nothing ever
//!   reached the kitchen. Once the outlet accepted the order the cancel is
//!   refused ("order already accepted"); whichever of accept and cancel runs
//!   first wins, because both go through the sales order's own transition.
//! - **Staff** (`orders.cancel`, the cashier and the kitchen manager) cancel a
//!   `placed`, `accepted`, `preparing` or `ready` order with a reason and the
//!   responsible party: *business* (the kitchen), *aggregator* or *customer*
//!   ([`by_staff`]). The reason *entered in error* is always the business's.
//!   The cancel is the sales order desk's (`cancel_order`): a paid order above
//!   AED 50, or a line the kitchen started, waits for a manager who is not the
//!   requester, and each started line needs an outcome, *return to stock* or
//!   *scrap* (the food is posted as wastage by wl-inventory).
//! - **The order follows the sales order** ([`on_cancelled`]): on
//!   `SalesOrderCancelled` the system runs `cancel_system` with the reason,
//!   party and requester kept on the order when the cancel was asked, and the
//!   open kitchen tickets are voided ([`after_cancelled`]). A cancel that
//!   waits for a manager changes nothing until the yes.
//! - **An aggregator order** cancelled with the party *aggregator* issues no
//!   refund (the sales order of an aggregator is not prepaid: the aggregator
//!   holds the customer's money) and is flagged *compensation expected* for the
//!   payout reconciliation.
//! - **A ready order's delivery** is cancelled with it.
//! - **A ticket that starts** while a change waits for a manager withdraws the
//!   request ([`refuse_waiting`]): it was asked on a kitchen that had not
//!   started the line, so it is asked again.
//!
//! The customer's `FNB.RUL.01` auto-reject timer is not built yet (it belongs
//! to the order-acceptance issues); a customer cancel changes the order to
//! `cancelled`, so that timer finds nothing to reject.

use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::Subject;
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::delivery::{DeliveryId, requester};
use wl_kernel::sales::{
    CancelByVertical, CancelOrder, ChangeOutcome, LineOutcomeKind, OrderView,
    SALES_ORDER_SUBJECT_TYPE, SalesOrderCancelled, order_desk,
};

use crate::cod;
use crate::money::aed;
use crate::notices::tell_customer;
use crate::order::{FnbOrder, FnbOrderStatus, cancel_reason, party};
use crate::order_machine::{CANCEL_SYSTEM, OrderInput};
use crate::permissions;
use crate::placing::refused;
use crate::progress;
use crate::texts::english;
use crate::ticket::{KitchenTicket, TicketStatus, VOID};

/// The approval actions of the sales order (`wl-sales`) that a started ticket
/// withdraws: an amendment, a cancelled line, the remaining lines or the order.
const CHANGE_ACTIONS: [&str; 4] = [
    "sales.order_amend",
    "sales.order_cancel_line",
    "sales.order_cancel_remaining",
    "sales.order_cancel",
];

/// What a member of staff asks when cancelling an order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StaffCancel {
    /// The reason, one of [`cancel_reason::CHOICES`].
    pub reason_code: String,
    /// The responsible party, one of [`party::CANCEL`]; ignored for the
    /// reason *entered in error*, which is the business's.
    pub party: String,
    /// A note; required for the reason *other*.
    pub note: String,
    /// The outcome chosen for each started line: the sale line and
    /// `return_to_stock` or `scrap`.
    pub outcomes: Vec<(i64, String)>,
}

/// The reason in words, with the note after a colon.
pub fn reason_text(code: &str, note: &str) -> String {
    let label = english(&format!("fnb.cancel.reason.{code}"), &[]);
    if note.trim().is_empty() {
        label
    } else {
        format!("{label}: {}", note.trim())
    }
}

/// 422 unless the order can still be cancelled: before it is out of the door.
fn ensure_cancellable(order: &FnbOrder) -> Result {
    match order.status {
        FnbOrderStatus::Placed
        | FnbOrderStatus::Accepted
        | FnbOrderStatus::Preparing
        | FnbOrderStatus::Ready => Ok(()),
        FnbOrderStatus::Cancelled | FnbOrderStatus::Rejected => {
            Err(refused(english("fnb.cancel.already_cancelled", &[])))
        }
        _ => Err(refused(english("fnb.cancel.too_late", &[]))),
    }
}

/// Whether a request for a change of the order waits for a manager now.
pub async fn waiting(db: &Db, order: &FnbOrder) -> Result<bool> {
    let subject = Subject::new(SALES_ORDER_SUBJECT_TYPE, order.sales_order_id);
    Ok(ApprovalRequest::pending_about(db, &subject)
        .await?
        .iter()
        .any(|request| CHANGE_ACTIONS.contains(&request.action.as_str())))
}

/// The item lines of `view` still open whose kitchen ticket was started.
pub(crate) async fn started_lines<'a>(
    db: &Db,
    order: &FnbOrder,
    view: &'a OrderView,
) -> Result<Vec<&'a wl_kernel::sales::OrderLineView>> {
    let open: Vec<&wl_kernel::sales::OrderLineView> = view
        .lines
        .iter()
        .filter(|line| line.is_item && line.open_milli() > 0)
        .collect();
    let ids: Vec<i64> = open.iter().map(|line| line.id).collect();
    let started = progress::started(db, order, &ids).await?;
    Ok(open
        .into_iter()
        .filter(|line| started.contains(&line.id))
        .collect())
}

/// Keeps who asked, why and for whose account on the order, before the sales
/// order is asked: the order reads it when the cancellation happens.
async fn mark(
    state: &AppState,
    id: i64,
    by: Option<i64>,
    party_chosen: &str,
    code: &str,
    text: &str,
    by_customer: bool,
) -> Result {
    let party_value = Some(party_chosen.to_owned());
    let code_value = Some(code.to_owned());
    let text_value = Some(text.to_owned());
    FnbOrder::query()
        .where_(FnbOrder::ID.eq(id))
        .update(
            &state.db,
            &[
                ("cancel_party", &party_value),
                ("cancel_reason_code", &code_value),
                ("close_reason", &text_value),
                ("cancel_requested_by", &by),
                ("cancelled_by_customer", &by_customer),
            ],
        )
        .await?;
    Ok(())
}

/// Forgets a cancellation that was not made.
async fn unmark(state: &AppState, id: i64) -> Result {
    let nothing: Option<String> = None;
    let nobody: Option<i64> = None;
    FnbOrder::query()
        .where_(FnbOrder::ID.eq(id))
        .update(
            &state.db,
            &[
                ("cancel_party", &nothing),
                ("cancel_reason_code", &nothing),
                ("close_reason", &nothing),
                ("cancel_requested_by", &nobody),
                ("cancel_request_id", &nobody),
                ("cancelled_by_customer", &false),
            ],
        )
        .await?;
    Ok(())
}

/// The customer cancels their own `placed` order from the order page.
///
/// # Errors
///
/// `Forbidden` for a signed-in user who is not the order's customer; 422 with
/// "order already accepted" (or "already cancelled") when the outlet was
/// quicker, and the sales module's refusals.
pub async fn by_customer(state: &AppState, user: &AuthUser, id: i64) -> Result {
    let order = FnbOrder::find_or_404(&state.db, id).await?;
    let customer = cod::customer_of(state, user).await?;
    if customer.map(|party| party.0) != Some(order.party_id) {
        return Err(Error::NotFound);
    }
    match order.status {
        FnbOrderStatus::Placed => {}
        FnbOrderStatus::Cancelled | FnbOrderStatus::Rejected => {
            return Err(refused(english("fnb.cancel.already_cancelled", &[])));
        }
        _ => return Err(refused(english("fnb.cancel.already_accepted", &[]))),
    }
    let text = english("fnb.cancel.customer_reason", &[]);
    mark(
        state,
        id,
        Some(user.id),
        party::CUSTOMER,
        cancel_reason::CUSTOMER_REQUEST,
        &text,
        true,
    )
    .await?;
    // The payment is not kept: wl-sales asks for the full refund.
    let cancelled = order_desk(state)
        .cancel(
            state,
            CancelByVertical {
                order: order.sales_order(),
                reason: text,
                outcomes: Vec::new(),
                keep_payment: false,
                by: Some(user.id),
            },
        )
        .await;
    if let Err(error) = cancelled {
        unmark(state, id).await?;
        // The outlet accepted it between the page and the click.
        let now = FnbOrder::find_or_404(&state.db, id).await?;
        return Err(match now.status {
            FnbOrderStatus::Placed => error,
            FnbOrderStatus::Cancelled | FnbOrderStatus::Rejected => {
                refused(english("fnb.cancel.already_cancelled", &[]))
            }
            _ => refused(english("fnb.cancel.already_accepted", &[])),
        });
    }
    Ok(())
}

/// A member of staff (`orders.cancel`) cancels the order `id`. Returns whether
/// it is cancelled or waits for a manager's approval.
///
/// # Errors
///
/// `Forbidden` without `orders.cancel`; 422 for an order out of the door or
/// finished, an unknown reason or party, the party *aggregator* on a direct
/// order, a started line with no outcome, a request that already waits, and the
/// sales module's refusals.
pub async fn by_staff(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    ask: StaffCancel,
) -> Result<ChangeOutcome> {
    if !user.has_permission(permissions::ORDERS_CANCEL) {
        return Err(Error::Forbidden);
    }
    let order = FnbOrder::find_or_404(&state.db, id).await?;
    ensure_cancellable(&order)?;
    let code = ask.reason_code.trim().to_owned();
    if !cancel_reason::CHOICES.contains(&code.as_str()) {
        return Err(refused(english("fnb.cancel.reason_unknown", &[])));
    }
    if code == cancel_reason::OTHER && ask.note.trim().is_empty() {
        return Err(refused(english("fnb.cancel.note_needed", &[])));
    }
    if waiting(&state.db, &order).await? {
        return Err(refused(english("fnb.cancel.pending", &[])));
    }
    let Some(view) = order_desk(state).find(state, order.sales_order()).await? else {
        return Err(Error::NotFound);
    };
    // Entered in error is the business's, not editable.
    let chosen = if code == cancel_reason::ENTERED_IN_ERROR {
        party::BUSINESS.to_owned()
    } else {
        ask.party.trim().to_owned()
    };
    if !party::CANCEL.contains(&chosen.as_str()) {
        return Err(refused(english("fnb.cancel.party_unknown", &[])));
    }
    if chosen == party::AGGREGATOR && !view.aggregator {
        return Err(refused(english("fnb.cancel.not_aggregator", &[])));
    }
    // Each started line needs an outcome.
    let mut outcomes = Vec::new();
    for line in started_lines(&state.db, &order, &view).await? {
        let picked = ask
            .outcomes
            .iter()
            .find(|(sale_line, _)| *sale_line == line.id)
            .and_then(|(_, name)| LineOutcomeKind::parse(name))
            .filter(|kind| {
                matches!(kind, LineOutcomeKind::ReturnToStock | LineOutcomeKind::Scrap)
            });
        match picked {
            Some(kind) => outcomes.push((line.id, kind)),
            None => {
                return Err(refused(english(
                    "fnb.cancel.outcome_needed",
                    &[("item", &line.description)],
                )));
            }
        }
    }
    let text = reason_text(&code, &ask.note);
    mark(state, id, Some(user.id), &chosen, &code, &text, false).await?;
    let asked = order_desk(state)
        .cancel_order(
            state,
            user,
            CancelOrder {
                order: order.sales_order(),
                reason: text,
                outcomes,
            },
        )
        .await;
    match asked {
        Ok(ChangeOutcome::Done) => Ok(ChangeOutcome::Done),
        Ok(ChangeOutcome::Waiting { request }) => {
            let waiting_for = Some(request);
            FnbOrder::query()
                .where_(FnbOrder::ID.eq(id))
                .update(&state.db, &[("cancel_request_id", &waiting_for)])
                .await?;
            Ok(ChangeOutcome::Waiting { request })
        }
        Err(error) => {
            unmark(state, id).await?;
            Err(error)
        }
    }
}

/// The kitchen manager records that the aggregator cancelled a `ready` order
/// whose rider never came (`FNB.RUL.16`): the order is cancelled with the
/// party *aggregator*, every open line scrapped as wastage, *compensation
/// expected* set, and nobody refunded. The choice is the `SAL.ACL.05`
/// approval, so no second one is asked.
///
/// # Errors
///
/// `Forbidden` without `orders.manage` and `orders.cancel`; 422 unless the
/// order is an aggregator's and `ready`.
pub async fn by_aggregator(state: &AppState, user: &AuthUser, id: i64) -> Result {
    if !user.has_permission(permissions::ORDERS_MANAGE)
        || !user.has_permission(permissions::ORDERS_CANCEL)
    {
        return Err(Error::Forbidden);
    }
    let order = FnbOrder::find_or_404(&state.db, id).await?;
    crate::pickup::ensure_waiting_aggregator(state, &order).await?;
    let Some(view) = order_desk(state).find(state, order.sales_order()).await? else {
        return Err(Error::NotFound);
    };
    let outcomes: Vec<(i64, LineOutcomeKind)> = view
        .lines
        .iter()
        .filter(|line| line.is_item && line.open_milli() > 0)
        .map(|line| (line.id, LineOutcomeKind::Scrap))
        .collect();
    let text = english("fnb.pickup.cancel_reason", &[]);
    mark(
        state,
        id,
        Some(user.id),
        party::AGGREGATOR,
        cancel_reason::AGGREGATOR_CANCELLED,
        &text,
        false,
    )
    .await?;
    let cancelled = order_desk(state)
        .cancel(
            state,
            CancelByVertical {
                order: order.sales_order(),
                reason: text,
                outcomes,
                // The aggregator holds the customer's money: nobody is refunded.
                keep_payment: true,
                by: Some(user.id),
            },
        )
        .await;
    if let Err(error) = cancelled {
        unmark(state, id).await?;
        return Err(error);
    }
    Ok(())
}

/// `SalesOrderCancelled`: the order follows the sales order. The reason, the
/// party and who asked were kept on the order when the cancel was asked; a
/// cancellation nobody asked through here (the sales module's own screen, a
/// failed online payment) is the business's, with the sales order's reason.
pub(crate) async fn on_cancelled(state: &AppState, event: SalesOrderCancelled) -> Result {
    let Some(order) = FnbOrder::of_sales_order(&state.db, event.order.0).await? else {
        return Ok(());
    };
    if !matches!(
        order.status,
        FnbOrderStatus::Placed
            | FnbOrderStatus::Accepted
            | FnbOrderStatus::Preparing
            | FnbOrderStatus::Ready
    ) {
        // Closed by the failed delivery, or told twice.
        return Ok(());
    }
    let asked = order.cancel_party.is_some() && order.cancel_reason_code.is_some();
    let input = OrderInput {
        paid_amount: order.paid_amount,
        close_reason: Some(
            order
                .close_reason
                .clone()
                .filter(|_| asked)
                .unwrap_or_else(|| event.reason.clone()),
        ),
        cancel_party: Some(
            order
                .cancel_party
                .clone()
                .unwrap_or_else(|| party::BUSINESS.to_owned()),
        ),
        cancel_reason_code: Some(
            order
                .cancel_reason_code
                .clone()
                .unwrap_or_else(|| cancel_reason::OTHER.to_owned()),
        ),
        cancelled_by: order.cancel_requested_by,
        by_customer: order.cancelled_by_customer,
        wastage_posted: event
            .outcomes
            .iter()
            .any(|outcome| outcome.kind == LineOutcomeKind::Scrap),
        ..OrderInput::default()
    };
    let history = english(
        "fnb.cancel.history",
        &[
            ("number", &order.number),
            (
                "reason",
                &input.close_reason.clone().unwrap_or_default(),
            ),
        ],
    );
    let _outcome = order
        .transition(CANCEL_SYSTEM)
        .by_system()
        .reason(history)
        .input(&input)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(())
}

/// After `cancel_system` committed: the tickets still on a screen are voided
/// (a bumped one stays `done`: its food follows the line's outcome) and the
/// customer is told, with the refund when they paid.
///
/// # Errors
///
/// Database errors and the tickets' transition failures.
pub(crate) async fn after_cancelled(state: &AppState, id: i64) -> Result {
    let Some(order) = FnbOrder::find(&state.db, id).await? else {
        return Ok(());
    };
    for ticket in KitchenTicket::of_order(&state.db, order.id).await? {
        if matches!(ticket.status, TicketStatus::Queued | TicketStatus::InPrep) {
            let _outcome = ticket
                .transition(VOID)
                .by_system()
                .run(state)
                .await
                .map_err(TransitionError::into_error)?;
        }
    }
    // A ready order's delivery was asked for: it is taken back.
    if let Some(delivery) = order.delivery_id
        && let Err(error) = requester(state)
            .cancel(state, DeliveryId(delivery), &english("fnb.cancel.delivery_reason", &[]))
            .await
    {
        eprintln!(
            "fnb: the delivery of the cancelled order {} could not be cancelled: {error}",
            order.number
        );
    }
    let number = order.number.clone();
    let body = if order.refund_amount > 0 {
        english(
            "fnb.cancel.customer_body_refund",
            &[("number", &number), ("amount", &aed(order.refund_amount))],
        )
    } else {
        english("fnb.cancel.customer_body", &[("number", &number)])
    };
    tell_customer(
        state,
        &order,
        &english("fnb.cancel.customer_title", &[("number", &number)]),
        &body,
    )
    .await
}

/// A ticket of the order started: the changes that wait for a manager were
/// asked on a kitchen that had not started it, so their requests are
/// withdrawn and the requester asks again (`SAL.ACL.05`).
///
/// # Errors
///
/// Database errors and the approval module's.
pub(crate) async fn refuse_waiting(state: &AppState, order_id: i64) -> Result {
    let Some(order) = FnbOrder::find(&state.db, order_id).await? else {
        return Ok(());
    };
    let subject = Subject::new(SALES_ORDER_SUBJECT_TYPE, order.sales_order_id);
    let reason = english("fnb.cancel.withdrawn", &[("number", &order.number)]);
    let mut withdrew = false;
    for request in ApprovalRequest::pending_about(&state.db, &subject).await? {
        if CHANGE_ACTIONS.contains(&request.action.as_str()) {
            approvals::withdraw_because(state, request.id, &reason).await?;
            withdrew = true;
        }
    }
    if withdrew && order.cancel_request_id.is_some() {
        unmark(state, order.id).await?;
    }
    Ok(())
}
