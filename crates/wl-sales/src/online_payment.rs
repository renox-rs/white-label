//! The online payment of a sales order (wl-sales §4.2 *Online payment*, §5.3
//! steps 1-3, §6 `SAL.RUL.06`, §7 *Online payment window*; BL-SAL-02,
//! BL-SAL-03, BL-SAL-04).
//!
//! An order paid online (payment mode `online`) is placed in
//! `awaiting_payment` by `submit` and is not shown to the outlet, kitchen,
//! dispatch or acceptance lists
//! ([`SalesOrder::visible_to_fulfilment`](crate::sales_order::SalesOrder::visible_to_fulfilment));
//! no stock is reserved and no ticket exists. `submit` asks `wl-payments` for
//! a payment link of the order's total that expires with the payment window
//! (15 minutes by default), and stores the link on the order.
//!
//! - **`PaymentReceived`** for the full total of an order in `awaiting_payment`
//!   runs `confirm_payment` (to `pending_acceptance` or `confirmed`, with no
//!   credit check: that check is not built yet, BL-SAL-10), once per payment
//!   id.
//! - **`PaymentFailed`**, or the job [`lapse_due`] when the window has passed,
//!   whichever comes first, runs `payment_failed` (cancelled, reason *payment
//!   failed*, the promo code use given back, `SalesOrderCancelled`); the
//!   customer is told to try again with a new order. The other one finds the
//!   order cancelled and does nothing.
//! - **`SAL.RUL.06`**: a `PaymentReceived` that cannot confirm the order (it is
//!   `cancelled` or `rejected`, it is already fully paid, or it is in
//!   `awaiting_payment` and the amount is not the total) is refunded in full
//!   through `wl-payments` (reason *late payment*), once per payment id; the
//!   customer and the manager role are told; the order never changes state.
//!
//! Decisions:
//!
//! - **Which order.** `PaymentReceived` does not name the order, so the
//!   module asks `PaymentRecorder::payments_for(order)` for the online orders
//!   of the paying party, newest first (as `wl-scheduling` does for
//!   bookings). `PaymentFailed` names it in `prepaid`.
//! - **One claim per payment.** A row per payment ([`OrderPayment`], unique on
//!   the payment id) is made before anything is done; a second delivery of the
//!   same event finds it and does nothing, so one payment confirms or is
//!   refunded once.
//! - **Already fully paid** is the sum of the payments that confirmed the
//!   order before this one reaching the order total.
//! - **A refund `wl-payments` refuses** (for example its own late-confirmation
//!   refund of the same money exists) leaves the row `refund_refused`; the
//!   manager is still told, and the module does not retry.
//! - **The manager** is the role of the setting `order_manager_role` (the
//!   company admin when empty), told through a role task.
//! - **No payment link** (the payments module off, no gateway, a refusal): the
//!   order still waits and times out; nothing else is lost.
//! - **Paid at submit.** An online order whose total is already covered by
//!   received payments made for it skips `awaiting_payment`.
//! - The kernel's single `PrepaidSubjects` slot is not provided by this
//!   module (`wl-scheduling` provides it); `wl-payments` therefore treats an
//!   order as active, and this module's rule is the one that refunds.

use renox::db::{DateTime, now};
use renox::prelude::*;
use wl_kernel::payments::{
    PAYMENT_LINK_SUBJECT_TYPE, PaymentFailed, PaymentLinkRequest, PaymentReceived, PaymentSummary,
    RefundRequest, RefundSource, payment_links, payment_recorder,
};
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::sales::SALES_ORDER_SUBJECT_TYPE;
use wl_kernel::{COMPANY_ADMIN_ROLE, LegalEntity, Subject};

use crate::config::config_of;
use crate::notices;
use crate::sales_order::{
    OrderPayment, OrderPaymentStatus, PAYMENT_CLAIMED, PAYMENT_CONFIRMED, PAYMENT_ONLINE,
    PAYMENT_REFUND_REFUSED, PAYMENT_REFUND_REQUESTED, SalesOrder, SalesOrderStatus,
};
use crate::sales_orders;
use crate::texts::english;

/// The scheduled task, and the command, that cancel the orders whose payment
/// window has passed.
pub const TASK: &str = "sales:online-payment-window";

/// The payments received for `order`, from `wl-payments`.
async fn payments_of(state: &AppState, order: &SalesOrder) -> Result<Vec<PaymentSummary>> {
    payment_recorder(state)
        .payments_for(&state.db, &order.subject())
        .await
}

/// Makes the claim of `payment_id` for `order_id`; `None` when the payment was
/// claimed before (a second delivery of the same event).
async fn claim(
    state: &AppState,
    order_id: i64,
    payment_id: i64,
    amount: i64,
) -> Result<Option<OrderPayment>> {
    let row = OrderPayment {
        order_id,
        payment_id,
        amount,
        outcome: PAYMENT_CLAIMED.to_owned(),
        ..OrderPayment::default()
    };
    match OrderPayment::create(&state.db, row).await {
        Ok(created) => Ok(Some(created)),
        Err(error) if error.is_unique_violation() => Ok(None),
        Err(error) => Err(error),
    }
}

/// Records what was done with the claim `id`.
async fn settle(state: &AppState, id: i64, outcome: &str, refund: Option<i64>) -> Result {
    OrderPayment::query()
        .where_(OrderPayment::ID.eq(id))
        .update(
            &state.db,
            &[
                ("outcome", &outcome.to_owned()),
                ("refund_id", &refund),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(())
}

/// Whether received payments made for the draft `order` already cover
/// `gross`, so it skips `awaiting_payment`.
pub(crate) async fn paid_before_submit(
    state: &AppState,
    order: &SalesOrder,
    gross: i64,
) -> Result<bool> {
    if gross <= 0 {
        return Ok(false);
    }
    let paid: i64 = payments_of(state, order)
        .await?
        .iter()
        .filter(|p| p.status.is_received())
        .map(|p| p.amount)
        .sum();
    Ok(paid >= gross)
}

/// Marks the order `id`, submitted already paid, as paid and keeps the
/// payments that paid it as confirming payments.
pub(crate) async fn mark_prepaid(state: &AppState, id: i64) -> Result {
    let order = SalesOrder::find_or_404(&state.db, id).await?;
    for payment in payments_of(state, &order)
        .await?
        .iter()
        .filter(|p| p.status.is_received())
    {
        if let Some(row) = claim(state, id, payment.id.0, payment.amount).await? {
            settle(state, row.id, PAYMENT_CONFIRMED, None).await?;
        }
    }
    SalesOrder::query()
        .where_(SalesOrder::ID.eq(id))
        .update(
            &state.db,
            &[(
                "payment_status",
                &OrderPaymentStatus::Received.as_str().to_owned(),
            )],
        )
        .await?;
    Ok(())
}

/// Asks `wl-payments` for the payment link of the order `id` that waits for
/// its payment, expiring with the payment window, and stores it on the order.
/// A refusal or no gateway leaves the order waiting; it times out.
pub(crate) async fn open_link(state: &AppState, id: i64) -> Result {
    let order = SalesOrder::find_or_404(&state.db, id).await?;
    let Some(due) = order.payment_due_at else {
        return Ok(());
    };
    let entity = match order.legal_entity_id {
        Some(entity) => entity,
        None => match LegalEntity::default_entity(&state.db).await {
            Ok(entity) => entity.id,
            // No legal entity to bill: no link; the order times out.
            Err(_) => return Ok(()),
        },
    };
    let request = PaymentLinkRequest {
        party: order.party(),
        legal_entity_id: entity,
        amount: order.gross_total,
        currency: order.currency.clone(),
        prepaid: order.subject(),
        expires_at: due,
        created_by: order.created_by,
    };
    match payment_links(state).create(state, request).await {
        Ok(Some(link)) => {
            let link_id = Some(link.link.subject_id);
            let path = Some(link.path);
            SalesOrder::query()
                .where_(SalesOrder::ID.eq(id))
                .update(
                    &state.db,
                    &[("payment_link_id", &link_id), ("payment_link", &path)],
                )
                .await?;
        }
        Ok(None) => {}
        Err(refused) => {
            eprintln!(
                "sales: no payment link for order {} was made: {refused}",
                order.number
            );
        }
    }
    Ok(())
}

/// The online order the payment of `event` was made for, with the payment as
/// `wl-payments` knows it.
async fn order_of(
    state: &AppState,
    event: &PaymentReceived,
) -> Result<Option<(SalesOrder, PaymentSummary)>> {
    let candidates = SalesOrder::query()
        .where_(SalesOrder::PARTY_ID.eq(event.party.0))
        .where_(SalesOrder::PAYMENT_MODE.eq(PAYMENT_ONLINE))
        .order_by_desc(SalesOrder::ID)
        .get(&state.db)
        .await?;
    for order in candidates.into_iter().take(100) {
        let paid = payments_of(state, &order).await?;
        if let Some(payment) = paid.into_iter().find(|p| p.id == event.payment) {
            return Ok(Some((order, payment)));
        }
    }
    Ok(None)
}

/// `PaymentReceived`: confirms the order the payment was made for, or applies
/// `SAL.RUL.06` and refunds the payment. A payment that is not for an order
/// of this module changes nothing.
///
/// # Errors
///
/// A database error, or the notifier's.
pub async fn on_payment_received(state: &AppState, event: &PaymentReceived) -> Result {
    let Some((order, summary)) = order_of(state, event).await? else {
        return Ok(());
    };
    let Some(row) = claim(state, order.id, event.payment.0, event.amount).await? else {
        // The same payment was delivered twice.
        return Ok(());
    };
    // The payment of the link of an amendment (BL-SAL-06) is not the payment
    // of the order: it releases the quantity held back until it was paid.
    if crate::settle::on_link_paid(state, &order, event.amount).await? {
        return settle(state, row.id, PAYMENT_CONFIRMED, None).await;
    }
    let before: i64 = OrderPayment::of_order(&state.db, order.id)
        .await?
        .iter()
        .filter(|p| p.id != row.id && p.outcome == PAYMENT_CONFIRMED)
        .map(|p| p.amount)
        .sum();
    let already_paid = order.gross_total > 0 && before >= order.gross_total;
    let usable = order.status == SalesOrderStatus::AwaitingPayment
        && !already_paid
        && event.amount == order.gross_total;
    if usable {
        match sales_orders::confirm_payment(state, order.id, event.amount).await {
            Ok(_) => return settle(state, row.id, PAYMENT_CONFIRMED, None).await,
            Err(error) => {
                let now_in = SalesOrder::find_or_404(&state.db, order.id).await?;
                if now_in.status == SalesOrderStatus::AwaitingPayment {
                    // Not a race: give the claim back so the delivery can retry.
                    OrderPayment::query()
                        .where_(OrderPayment::ID.eq(row.id))
                        .delete(&state.db)
                        .await?;
                    return Err(error);
                }
            }
        }
    }
    refund_late(state, &order, &row, event, &summary).await
}

/// `SAL.RUL.06`: asks the full refund of the unusable payment, once, and tells
/// the customer and the manager. The order is not touched.
async fn refund_late(
    state: &AppState,
    order: &SalesOrder,
    claim: &OrderPayment,
    event: &PaymentReceived,
    payment: &PaymentSummary,
) -> Result {
    let request = RefundRequest {
        source: RefundSource::Payment(event.payment),
        party: event.party,
        legal_entity_id: payment.legal_entity_id,
        amount: event.amount,
        currency: payment.currency.clone(),
        method: None,
        gateway_txn: None,
        reason: english("wl_sales.orders.late_payment_reason", &[]),
    };
    let refunded = match payment_recorder(state)
        .request_refund(state, request, None)
        .await
    {
        Ok(refund) => {
            settle(state, claim.id, PAYMENT_REFUND_REQUESTED, Some(refund.0)).await?;
            true
        }
        Err(refused) => {
            settle(state, claim.id, PAYMENT_REFUND_REFUSED, None).await?;
            eprintln!(
                "sales: the refund of payment {} for order {} was refused: {refused}",
                event.payment, order.number
            );
            false
        }
    };
    notices::order_late_payment(state, order, event.amount).await?;
    manager_task(state, order, event, refunded).await
}

/// The task for the manager role: a payment was refunded as late (or could not
/// be refunded by the module and needs a person).
async fn manager_task(
    state: &AppState,
    order: &SalesOrder,
    event: &PaymentReceived,
    refunded: bool,
) -> Result {
    let amount = format!("AED {}", notices::aed(event.amount));
    let (title, description) = {
        let params: [(&str, &dyn std::fmt::Display); 3] = [
            ("number", &order.number),
            ("payment", &event.payment),
            ("amount", &amount),
        ];
        let body = if refunded {
            "wl_sales.orders.late_task_body"
        } else {
            "wl_sales.orders.late_task_unrefunded"
        };
        (
            english("wl_sales.orders.late_task_title", &params),
            english(body, &params),
        )
    };
    let config = config_of(state);
    let role = if config.order_manager_role.is_empty() {
        COMPANY_ADMIN_ROLE
    } else {
        config.order_manager_role.as_str()
    };
    let key = format!("sales.late_payment:{}", event.payment);
    let task = RoleTask::new(order.subject(), role, &title, &key).description(&description);
    role_tasks::open(state, task).await?;
    Ok(())
}

/// Cancels the order `id` as *payment failed* when it still waits for its
/// payment, withdraws its link and tells the customer. `false` when the order
/// was not waiting (the other trigger came first).
async fn fail_order(state: &AppState, id: i64) -> Result<bool> {
    let Some(order) = SalesOrder::find(&state.db, id).await? else {
        return Ok(false);
    };
    if order.status != SalesOrderStatus::AwaitingPayment {
        return Ok(false);
    }
    let failed = sales_orders::payment_failed(state, id).await?;
    if let Some(link) = failed.payment_link_id {
        let subject = Subject::new(PAYMENT_LINK_SUBJECT_TYPE, link);
        // The link only stops further attempts; a failure to close it changes
        // nothing here.
        let _closed = payment_links(state).cancel(state, &subject).await;
    }
    notices::order_payment_failed(state, &failed).await?;
    Ok(true)
}

/// `PaymentFailed` for a payment made in advance for a sales order: the order
/// is cancelled as *payment failed*.
///
/// # Errors
///
/// A database error, or the notifier's.
pub async fn on_payment_failed(state: &AppState, event: &PaymentFailed) -> Result {
    let Some(prepaid) = &event.prepaid else {
        return Ok(());
    };
    if prepaid.subject_type != SALES_ORDER_SUBJECT_TYPE {
        return Ok(());
    }
    fail_order(state, prepaid.subject_id).await.map(|_| ())
}

/// Cancels every order that waits for its payment past its window at `at`;
/// returns how many. Each order is handled on its own, so a failure on one is
/// printed and the run goes on.
pub async fn lapse_due(state: &AppState, at: DateTime) -> Result<usize> {
    let due = SalesOrder::query()
        .where_(SalesOrder::STATUS.eq(SalesOrderStatus::AwaitingPayment))
        .where_(SalesOrder::PAYMENT_DUE_AT.lte(at))
        .order_by(SalesOrder::ID)
        .get(&state.db)
        .await?;
    let mut lapsed = 0;
    for order in due {
        match fail_order(state, order.id).await {
            Ok(true) => lapsed += 1,
            Ok(false) => {}
            Err(error) => eprintln!("{TASK}: order {} failed: {error}", order.number),
        }
    }
    Ok(lapsed)
}

/// Registers the listeners of `PaymentReceived` and `PaymentFailed`, the
/// minute job and its command.
pub(crate) fn register(app: &mut Registry) {
    app.model::<OrderPayment>();
    app.listen(|event: PaymentReceived, state: AppState| async move {
        on_payment_received(&state, &event).await
    });
    app.listen(|event: PaymentFailed, state: AppState| async move {
        on_payment_failed(&state, &event).await
    });
    app.schedule()
        .every_minute(TASK, |state: AppState| async move {
            lapse_due(&state, now()).await.map(|_| ())
        });
    app.command(
        TASK,
        "Cancel the orders whose online payment window has passed",
        |_args, state: AppState| async move {
            let lapsed = lapse_due(&state, now()).await?;
            println!("{lapsed} orders cancelled as payment failed.");
            Ok(())
        },
    );
}
