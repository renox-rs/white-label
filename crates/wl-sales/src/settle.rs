//! The money of a change to a placed order (wl-sales §4.2 *Amendment of a
//! prepaid order*, rows `amend`, `cancel_line` and `cancel_remaining`, §10
//! *Amended and cancelled orders*; BL-SAL-06, and the refund, credit note and
//! fee of BL-SAL-08, BL-SAL-09). It runs after the change committed (and, when
//! a manager had to approve, after the approval), so nothing is refunded for
//! an amendment that was not approved.
//!
//! **An amendment** (the difference is the new total less the old one, in
//! fils, one link or one refund, never both):
//! - a *draft* invoice is updated to the new lines; an *issued* invoice is
//!   never edited: a line that went down is credited by a credit note
//!   (`Invoicer::credit_partly`), a line that went up is billed on an extra
//!   invoice drafted for the added value;
//! - a *prepaid* order (payment mode `online`, payment received): an increase
//!   sends the customer a payment link for the difference (it expires after
//!   the online payment window on the website and the portal, else
//!   [`SalesConfig::amend_link_hours`](crate::config::SalesConfig)), the added
//!   quantity stays reserved but is held back from fulfilment
//!   ([`SaleLine::held_milli`]) until the link is paid; a decrease is refunded
//!   through wl-payments with the reason *order amended*.
//! - the link expires unpaid: the order owner gets a task (the outlet manager
//!   for website and portal orders, the sales rep role otherwise): *resend the
//!   link* ([`resend_link`]) or *amend the order back* ([`amend_back`]: a new
//!   revision without the increase, no money effect). The revision stays until
//!   then.
//!
//! **A cancellation** of a line or of the remaining lines: the prepaid share
//! of the cancelled quantity less the fees is refunded once; an issued
//! invoice is credited for the cancelled value (a draft one is updated; an
//! order that becomes `fulfilled` without an invoice gets its invoice
//! drafted); a fee is billed on a fee invoice with the configured VAT
//! category (open item A-04), whether it is kept from the prepayment or not.
//!
//! Decisions:
//! - The position of an order line on its invoice is its line number.
//! - The refund's source is the order (`RefundSource::Order`); wl-payments
//!   applies its own approval (`FIN.ACL.04`).
//! - A step the other module refuses leaves a settlement row `failed` with the
//!   reason, and the change stays; a person acts from the order page.
//! - A fee is an amount with VAT; the fee line takes the net that gives that
//!   amount at the configured category, so the document can differ from the
//!   amount by a fil.

use renox::chrono::TimeDelta;
use renox::db::{DateTime, now};
use renox::prelude::*;
use wl_kernel::invoicing::{
    CreditLine, DraftLine, InvoiceDraft, InvoiceId, InvoiceStatus, VatCategory, invoicer,
};
use wl_kernel::payments::{
    PAYMENT_LINK_SUBJECT_TYPE, PaymentLinkRequest, RefundRequest, RefundSource, payment_links,
    payment_recorder,
};
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::sales::{FulfilledLine, SALES_ORDER_SUBJECT_TYPE, SalesOrderLinesReleased};
use wl_kernel::{COMPANY_ADMIN_ROLE, LegalEntity, Subject};

use crate::amend::{AmendDetail, AmendInput, AmendOutcome, amend};
use crate::cancel_lines::CancelDetail;
use crate::config::config_of;
use crate::order_change::{
    OrderChange, OrderRevision, OrderSettlement, SETTLE_CREDIT_NOTE, SETTLE_DRAFT_UPDATED,
    SETTLE_EXTRA_INVOICE, SETTLE_FEE_INVOICE, SETTLE_LINK, SETTLE_REFUND, STATUS_DONE,
    STATUS_EXPIRED, STATUS_FAILED, STATUS_OPEN, STATUS_PAID, STATUS_RESENT, STATUS_REQUESTED,
    STATUS_REVERTED,
};
use crate::order_notices;
use crate::sale_line::{KIND_ITEM, SaleLine, ratio_half_up};
use crate::sales_order::{OrderPaymentStatus, PAYMENT_ONLINE, SalesOrder, SalesOrderStatus};
use crate::texts::english;

/// The scheduled task, and the command, that expire the payment links of
/// amendments whose time has passed.
pub const TASK: &str = "sales:amendment-link-window";

/// A 422 with the English text `key`.
fn refused(key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key, &[]))
}

/// Whether the order was paid online and the payment is in.
pub fn is_prepaid(order: &SalesOrder) -> bool {
    order.payment_mode == PAYMENT_ONLINE && order.payment_status == OrderPaymentStatus::Received
}

/// The legal entity to bill or refund for: the order's, else the default.
async fn entity_of(state: &AppState, order: &SalesOrder) -> Result<Option<i64>> {
    match order.legal_entity_id {
        Some(entity) => Ok(Some(entity)),
        None => Ok(LegalEntity::default_entity(&state.db)
            .await
            .ok()
            .map(|entity| entity.id)),
    }
}

/// Stores a settlement row.
async fn store(
    state: &AppState,
    order: &SalesOrder,
    change: Option<i64>,
    kind: &str,
    amount: i64,
    status: &str,
) -> Result<OrderSettlement> {
    OrderSettlement::create(
        &state.db,
        OrderSettlement {
            order_id: order.id,
            change_id: change,
            revision: order.revision,
            kind: kind.to_owned(),
            amount,
            status: status.to_owned(),
            ..OrderSettlement::default()
        },
    )
    .await
}

/// Stores a failed step with the reason.
async fn failed(
    state: &AppState,
    order: &SalesOrder,
    change: Option<i64>,
    kind: &str,
    amount: i64,
    why: &str,
) -> Result<OrderSettlement> {
    eprintln!("sales: order {}: {kind} of {amount} failed: {why}", order.number);
    OrderSettlement::create(
        &state.db,
        OrderSettlement {
            order_id: order.id,
            change_id: change,
            revision: order.revision,
            kind: kind.to_owned(),
            amount,
            status: STATUS_FAILED.to_owned(),
            note: Some(why.to_owned()),
            ..OrderSettlement::default()
        },
    )
    .await
}

/// The invoice draft of the order's lines as they stand: what is not
/// cancelled, in order.
fn invoice_draft(order: &SalesOrder, lines: &[SaleLine]) -> InvoiceDraft {
    let mut draft = InvoiceDraft::new(order.party(), order.subject()).currency(&order.currency);
    if let Some(entity) = order.legal_entity_id {
        draft = draft.legal_entity(entity);
    }
    for line in lines {
        if let Some(billed) = line.billable_draft_line() {
            draft = draft.line(billed.origin(order.subject()));
        }
    }
    draft
}

/// The net of `quantity_milli` of an item at `unit_price` less `discount`.
fn net_of(
    quantity_milli: i64,
    unit_price: i64,
    discount: i64,
    category: VatCategory,
) -> i64 {
    DraftLine::new("", unit_price, category)
        .quantity_milli(quantity_milli)
        .discount(discount)
        .net()
}

/// `net` fils before VAT with the VAT of `category`.
fn with_vat(net: i64, category: VatCategory) -> i64 {
    ratio_half_up(net, 10_000 + category.rate_bp(), 10_000)
}

/// Settles the money of the amendment `change` of `order` (`detail`).
///
/// # Errors
///
/// A database error; a refusal by the payments or invoicing module is stored
/// on a settlement row instead.
pub async fn amendment(
    state: &AppState,
    order: &SalesOrder,
    change: &OrderChange,
    detail: &AmendDetail,
) -> Result {
    let delta = detail.value_after - detail.value_before;
    let lines = SaleLine::of(&state.db, &order.subject()).await?;
    amendment_invoice(state, order, change, detail, &lines).await?;
    if is_prepaid(order) {
        if delta > 0 {
            open_link(state, order, Some(change.id), delta).await?;
        } else if delta < 0 {
            refund(
                state,
                order,
                Some(change.id),
                -delta,
                english("wl_sales.settle.reason_amended", &[]),
                change.changed_by,
            )
            .await?;
        }
    }
    Ok(())
}

/// The invoice side of an amendment: a draft is updated, an issued invoice is
/// credited for what went down and an extra invoice bills what went up.
async fn amendment_invoice(
    state: &AppState,
    order: &SalesOrder,
    change: &OrderChange,
    detail: &AmendDetail,
    lines: &[SaleLine],
) -> Result {
    let Some(invoice) = order.invoice_id else {
        return Ok(());
    };
    let invoices = invoicer(state);
    let id = InvoiceId(invoice);
    let Some(summary) = invoices.find(&state.db, id).await? else {
        return Ok(());
    };
    if summary.status == InvoiceStatus::Draft {
        let done = invoices
            .update_draft(&state.db, id, invoice_draft(order, lines))
            .await?;
        let status = if done { STATUS_DONE } else { STATUS_FAILED };
        let row = store(state, order, Some(change.id), SETTLE_DRAFT_UPDATED, 0, status).await?;
        OrderSettlement::query()
            .where_(OrderSettlement::ID.eq(row.id))
            .update(&state.db, &[("reference_id", &Some(invoice))])
            .await?;
        return Ok(());
    }
    let earlier = OrderRevision::find_of(&state.db, order.id, detail.revision_before).await?;
    let kept = earlier.map(|r| r.lines()).unwrap_or_default();
    let mut credits = Vec::new();
    let mut credited_gross = 0;
    let mut extra: Vec<DraftLine> = Vec::new();
    let mut extra_gross = 0;
    for line in lines.iter().filter(|l| l.kind == KIND_ITEM) {
        let old = kept
            .iter()
            .find(|k| k.line == line.id)
            .map_or(0, |k| {
                net_of(k.quantity_milli, k.unit_price, k.discount, line.vat_category)
            });
        let new = line.net();
        if new < old {
            credits.push(CreditLine::new(line.line_no, old - new));
            credited_gross += with_vat(old - new, line.vat_category);
        } else if new > old {
            extra.push(
                DraftLine::new(
                    &english(
                        "wl_sales.settle.extra_line",
                        &[("description", &line.description), ("revision", &order.revision)],
                    ),
                    new - old,
                    line.vat_category,
                )
                .origin(order.subject()),
            );
            extra_gross += with_vat(new - old, line.vat_category);
        }
    }
    if !credits.is_empty() {
        match invoices
            .credit_partly(
                state,
                id,
                credits,
                english("wl_sales.settle.reason_amended", &[]),
                change.changed_by,
            )
            .await
        {
            Ok(note) => {
                let row = store(
                    state,
                    order,
                    Some(change.id),
                    SETTLE_CREDIT_NOTE,
                    credited_gross,
                    STATUS_REQUESTED,
                )
                .await?;
                OrderSettlement::query()
                    .where_(OrderSettlement::ID.eq(row.id))
                    .update(&state.db, &[("reference_id", &Some(note.0))])
                    .await?;
            }
            Err(error) => {
                failed(
                    state,
                    order,
                    Some(change.id),
                    SETTLE_CREDIT_NOTE,
                    credited_gross,
                    &error.to_string(),
                )
                .await?;
            }
        }
    }
    if !extra.is_empty() {
        let mut draft =
            InvoiceDraft::new(order.party(), order.subject()).currency(&order.currency);
        if let Some(entity) = order.legal_entity_id {
            draft = draft.legal_entity(entity);
        }
        for line in extra {
            draft = draft.line(line);
        }
        match invoices.draft(&state.db, draft).await {
            Ok(created) => {
                let row = store(
                    state,
                    order,
                    Some(change.id),
                    SETTLE_EXTRA_INVOICE,
                    extra_gross,
                    STATUS_REQUESTED,
                )
                .await?;
                OrderSettlement::query()
                    .where_(OrderSettlement::ID.eq(row.id))
                    .update(&state.db, &[("reference_id", &created.map(|i| i.0))])
                    .await?;
            }
            Err(error) => {
                failed(
                    state,
                    order,
                    Some(change.id),
                    SETTLE_EXTRA_INVOICE,
                    extra_gross,
                    &error.to_string(),
                )
                .await?;
            }
        }
    }
    Ok(())
}

/// Asks wl-payments for the refund of `amount` fils from the order, once.
pub(crate) async fn refund(
    state: &AppState,
    order: &SalesOrder,
    change: Option<i64>,
    amount: i64,
    reason: String,
    by: Option<i64>,
) -> Result {
    let Some(entity) = entity_of(state, order).await? else {
        failed(state, order, change, SETTLE_REFUND, amount, "no legal entity").await?;
        return Ok(());
    };
    let request = RefundRequest {
        source: RefundSource::Order(order.subject()),
        party: order.party(),
        legal_entity_id: entity,
        amount,
        currency: order.currency.clone(),
        method: None,
        gateway_txn: None,
        reason,
    };
    match payment_recorder(state).request_refund(state, request, by).await {
        Ok(id) => {
            let row = store(state, order, change, SETTLE_REFUND, amount, STATUS_REQUESTED).await?;
            OrderSettlement::query()
                .where_(OrderSettlement::ID.eq(row.id))
                .update(&state.db, &[("reference_id", &Some(id.0))])
                .await?;
        }
        Err(error) => {
            failed(state, order, change, SETTLE_REFUND, amount, &error.to_string()).await?;
        }
    }
    Ok(())
}

/// When a payment link of an amendment of `order` expires: the online payment
/// window on the website and the portal, else the hours of the setting.
fn link_expiry(state: &AppState, order: &SalesOrder) -> DateTime {
    let config = config_of(state);
    if config.order_acceptance_channels.contains(&order.channel) {
        now() + TimeDelta::minutes(config.online_payment_window_minutes)
    } else {
        now() + TimeDelta::hours(config.amend_link_hours)
    }
}

/// Asks wl-payments for a payment link of `amount` fils for the increase of
/// `order` and stores it; the customer is told. A refusal or no gateway
/// leaves a failed row and the owner's task. Returns the row.
async fn open_link(
    state: &AppState,
    order: &SalesOrder,
    change: Option<i64>,
    amount: i64,
) -> Result<OrderSettlement> {
    let due = link_expiry(state, order);
    let Some(entity) = entity_of(state, order).await? else {
        let row = failed(state, order, change, SETTLE_LINK, amount, "no legal entity").await?;
        expired_task(state, order, &row).await?;
        return Ok(row);
    };
    let request = PaymentLinkRequest {
        party: order.party(),
        legal_entity_id: entity,
        amount,
        currency: order.currency.clone(),
        prepaid: order.subject(),
        expires_at: due,
        created_by: order.created_by,
    };
    match payment_links(state).create(state, request).await {
        Ok(Some(link)) => {
            let row = OrderSettlement::create(
                &state.db,
                OrderSettlement {
                    order_id: order.id,
                    change_id: change,
                    revision: order.revision,
                    kind: SETTLE_LINK.to_owned(),
                    amount,
                    status: STATUS_OPEN.to_owned(),
                    reference_id: Some(link.link.subject_id),
                    link_path: Some(link.path.clone()),
                    due_at: Some(due),
                    ..OrderSettlement::default()
                },
            )
            .await?;
            order_notices::amend_link(state, order, amount, &link.path).await?;
            Ok(row)
        }
        Ok(None) => {
            let row = failed(
                state,
                order,
                change,
                SETTLE_LINK,
                amount,
                &english("wl_sales.settle.no_gateway", &[]),
            )
            .await?;
            expired_task(state, order, &row).await?;
            Ok(row)
        }
        Err(error) => {
            let row =
                failed(state, order, change, SETTLE_LINK, amount, &error.to_string()).await?;
            expired_task(state, order, &row).await?;
            Ok(row)
        }
    }
}

/// The task for the owner of the order: its payment link of the amendment
/// expired (or could not be made). The owner is the outlet manager for the
/// website and portal orders, the sales rep role otherwise.
async fn expired_task(state: &AppState, order: &SalesOrder, link: &OrderSettlement) -> Result {
    let config = config_of(state);
    let configured = if config.order_acceptance_channels.contains(&order.channel) {
        config.order_manager_role.as_str()
    } else {
        config.sales_rep_role.as_str()
    };
    let role = if configured.is_empty() {
        COMPANY_ADMIN_ROLE
    } else {
        configured
    };
    let amount = format!("AED {}", crate::notices::aed(link.amount));
    let (title, body) = {
        let params: [(&str, &dyn std::fmt::Display); 3] = [
            ("number", &order.number),
            ("amount", &amount),
            ("revision", &link.revision),
        ];
        (
            english("wl_sales.settle.expired_task_title", &params),
            english("wl_sales.settle.expired_task_body", &params),
        )
    };
    let key = format!("sales.amend_link:{}", link.id);
    let task = RoleTask::new(order.subject(), role, &title, &key).description(&body);
    role_tasks::open(state, task).await?;
    Ok(())
}

/// A payment of `amount` fils for `order` arrived: when it is the payment of
/// an open (or just expired) link of an amendment, the link is paid and the
/// held quantity is released to fulfilment. `false` when no such link
/// exists, so the caller treats the payment as it did before.
///
/// # Errors
///
/// A database error, or an event listener's.
pub(crate) async fn on_link_paid(state: &AppState, order: &SalesOrder, amount: i64) -> Result<bool> {
    let mut found = None;
    for status in [STATUS_OPEN, STATUS_EXPIRED] {
        found = OrderSettlement::links(&state.db, order.id, status)
            .await?
            .into_iter()
            .find(|link| link.amount == amount);
        if found.is_some() {
            break;
        }
    }
    let Some(link) = found else {
        return Ok(false);
    };
    OrderSettlement::set_status(&state.db, link.id, STATUS_PAID).await?;
    let lines = SaleLine::of(&state.db, &order.subject()).await?;
    let released: Vec<FulfilledLine> = lines
        .iter()
        .filter(|l| l.held_milli > 0)
        .map(|l| FulfilledLine {
            line: l.id,
            quantity_milli: l.held_milli,
        })
        .collect();
    SaleLine::query()
        .where_(SaleLine::SUBJECT_TYPE.eq(SALES_ORDER_SUBJECT_TYPE.to_owned()))
        .where_(SaleLine::SUBJECT_ID.eq(order.id))
        .update(&state.db, &[("held_milli", &0_i64), ("updated_at", &now())])
        .await?;
    if !released.is_empty() {
        state
            .emit(SalesOrderLinesReleased {
                order: order.order_id(),
                party: order.party(),
                lines: released,
            })
            .await?;
    }
    Ok(true)
}

/// Expires every open payment link of an amendment whose time has passed at
/// `at`: the link is withdrawn and the owner gets the task *payment link
/// expired*. The revision stays and the added quantity stays held. Returns
/// how many expired; one failing does not stop the others.
///
/// # Errors
///
/// A database error reading the links.
pub async fn lapse_links(state: &AppState, at: DateTime) -> Result<usize> {
    let due = OrderSettlement::query()
        .where_(OrderSettlement::KIND.eq(SETTLE_LINK))
        .where_(OrderSettlement::STATUS.eq(STATUS_OPEN))
        .where_(OrderSettlement::DUE_AT.lte(at))
        .order_by(OrderSettlement::ID)
        .get(&state.db)
        .await?;
    let mut expired = 0;
    for link in due {
        match expire(state, &link).await {
            Ok(()) => expired += 1,
            Err(error) => eprintln!("{TASK}: link {} failed: {error}", link.id),
        }
    }
    Ok(expired)
}

/// Expires one link.
async fn expire(state: &AppState, link: &OrderSettlement) -> Result {
    OrderSettlement::set_status(&state.db, link.id, STATUS_EXPIRED).await?;
    if let Some(id) = link.reference_id {
        let subject = Subject::new(PAYMENT_LINK_SUBJECT_TYPE, id);
        // The link only stops further attempts; failing to close it changes nothing.
        let _closed = payment_links(state).cancel(state, &subject).await;
    }
    let order = SalesOrder::find_or_404(&state.db, link.order_id).await?;
    expired_task(state, &order, link).await
}

/// The latest expired link of an amendment of `order`, if any.
async fn expired_link(state: &AppState, order: &SalesOrder) -> Result<OrderSettlement> {
    OrderSettlement::links(&state.db, order.id, STATUS_EXPIRED)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| refused("wl_sales.settle.problem.no_expired_link"))
}

/// *Resend the link*: the owner of `order_id` asks a new payment link for the
/// same amount after the first expired unpaid. The old link is marked resent.
///
/// # Errors
///
/// 403 without `sales_orders.amend`; 422 when no expired link waits or the
/// order is no longer open.
pub async fn resend_link(state: &AppState, user: &AuthUser, order_id: i64) -> Result<OrderSettlement> {
    if !user.has_permission(crate::permissions::SALES_ORDERS_AMEND) {
        return Err(Error::Forbidden);
    }
    let order = SalesOrder::find_or_404(&state.db, order_id).await?;
    let old = expired_link(state, &order).await?;
    if matches!(
        order.status,
        SalesOrderStatus::Cancelled
            | SalesOrderStatus::Rejected
            | SalesOrderStatus::Closed
            | SalesOrderStatus::Fulfilled
    ) {
        return Err(refused("wl_sales.settle.problem.order_closed"));
    }
    let fresh = open_link(state, &order, old.change_id, old.amount).await?;
    if fresh.status == STATUS_OPEN {
        OrderSettlement::set_status(&state.db, old.id, STATUS_RESENT).await?;
    }
    Ok(fresh)
}

/// *Amend back*: the owner of `order_id` removes an increase whose payment
/// link expired unpaid. A new revision takes the lines of the revision before
/// the increase; nothing is charged or refunded.
///
/// # Errors
///
/// 403 without `sales_orders.amend`; 422 when no expired link waits, the order
/// was amended again since, or the amendment's own guards refuse.
pub async fn amend_back(state: &AppState, user: &AuthUser, order_id: i64) -> Result<AmendOutcome> {
    if !user.has_permission(crate::permissions::SALES_ORDERS_AMEND) {
        return Err(Error::Forbidden);
    }
    let order = SalesOrder::find_or_404(&state.db, order_id).await?;
    let link = expired_link(state, &order).await?;
    if link.revision != order.revision {
        return Err(refused("wl_sales.amend.problem.moved"));
    }
    let Some(previous) = OrderRevision::find_of(&state.db, order.id, order.revision - 1).await?
    else {
        return Err(refused("wl_sales.settle.problem.no_expired_link"));
    };
    let kept = previous.lines();
    let lines = SaleLine::of(&state.db, &order.subject()).await?;
    let mut input = AmendInput::new(&english("wl_sales.settle.amend_back_reason", &[]))
        .seen_at(order.revision);
    for line in lines.iter().filter(|l| l.kind == KIND_ITEM) {
        let (quantity, price, discount) = kept.iter().find(|k| k.line == line.id).map_or(
            (
                line.fulfilled_milli + line.cancelled_milli,
                line.unit_price,
                line.discount,
            ),
            |k| (k.quantity_milli, k.unit_price, k.discount),
        );
        if quantity != line.quantity_milli
            || price != line.unit_price
            || discount != line.discount
        {
            input = input.line(line.id, quantity, price, discount);
        }
    }
    input.no_money = true;
    let outcome = amend(state, user, order_id, input).await?;
    if matches!(outcome, AmendOutcome::Amended(_)) {
        OrderSettlement::set_status(&state.db, link.id, STATUS_REVERTED).await?;
    }
    Ok(outcome)
}

/// Settles the money of the cancellation `change` of `order` (`detail`): the
/// invoice, the refund of the prepaid share less the fees, and the fee
/// document.
///
/// # Errors
///
/// A database error; a refusal by the payments or invoicing module is stored
/// on a settlement row instead.
pub async fn cancellation(
    state: &AppState,
    order: &SalesOrder,
    change: &OrderChange,
    detail: &CancelDetail,
) -> Result {
    let lines = SaleLine::of(&state.db, &order.subject()).await?;
    if order.invoice_id.is_some() {
        cancellation_invoice(state, order, change, detail, &lines).await?;
    } else if detail.complete {
        // The order is complete without a hand-over of the cancelled part:
        // its invoice is drafted for what was fulfilled.
        crate::sales_orders::draft_invoice(state, order.id).await?;
    }
    let shares: i64 = detail.lines.iter().map(|l| l.share).sum();
    let fees: i64 = detail.lines.iter().map(|l| l.fee).sum();
    if is_prepaid(order) && shares - fees > 0 && !detail.keep_payment {
        let why = if detail.whole {
            "wl_sales.settle.reason_order_cancelled"
        } else {
            "wl_sales.settle.reason_cancelled"
        };
        refund(
            state,
            order,
            Some(change.id),
            shares - fees,
            english(why, &[]),
            change.changed_by,
        )
        .await?;
    }
    if fees > 0 {
        fee_document(state, order, change, fees).await?;
    }
    Ok(())
}

/// The invoice side of a cancellation: a draft is updated, an issued invoice
/// is credited for the cancelled value.
async fn cancellation_invoice(
    state: &AppState,
    order: &SalesOrder,
    change: &OrderChange,
    detail: &CancelDetail,
    lines: &[SaleLine],
) -> Result {
    let Some(invoice) = order.invoice_id else {
        return Ok(());
    };
    let invoices = invoicer(state);
    let id = InvoiceId(invoice);
    let Some(summary) = invoices.find(&state.db, id).await? else {
        return Ok(());
    };
    if summary.status == InvoiceStatus::Draft {
        let done = invoices
            .update_draft(&state.db, id, invoice_draft(order, lines))
            .await?;
        let status = if done { STATUS_DONE } else { STATUS_FAILED };
        let row = store(state, order, Some(change.id), SETTLE_DRAFT_UPDATED, 0, status).await?;
        OrderSettlement::query()
            .where_(OrderSettlement::ID.eq(row.id))
            .update(&state.db, &[("reference_id", &Some(invoice))])
            .await?;
        return Ok(());
    }
    let mut credits = Vec::new();
    let mut gross = 0;
    for cancelled in &detail.lines {
        let Some(line) = lines.iter().find(|l| l.id == cancelled.line) else {
            continue;
        };
        let net = ratio_half_up(line.net(), cancelled.quantity_milli, line.quantity_milli);
        if net > 0 {
            credits.push(CreditLine::new(line.line_no, net));
            gross += with_vat(net, line.vat_category);
        }
    }
    if credits.is_empty() {
        return Ok(());
    }
    match invoices
        .credit_partly(
            state,
            id,
            credits,
            english("wl_sales.settle.reason_cancelled", &[]),
            change.changed_by,
        )
        .await
    {
        Ok(note) => {
            let row = store(
                state,
                order,
                Some(change.id),
                SETTLE_CREDIT_NOTE,
                gross,
                STATUS_REQUESTED,
            )
            .await?;
            OrderSettlement::query()
                .where_(OrderSettlement::ID.eq(row.id))
                .update(&state.db, &[("reference_id", &Some(note.0))])
                .await?;
        }
        Err(error) => {
            failed(
                state,
                order,
                Some(change.id),
                SETTLE_CREDIT_NOTE,
                gross,
                &error.to_string(),
            )
            .await?;
        }
    }
    Ok(())
}

/// Drafts the invoice of the cancellation fee `fee` fils (with VAT): one line
/// at the configured VAT category (open item A-04).
async fn fee_document(
    state: &AppState,
    order: &SalesOrder,
    change: &OrderChange,
    fee: i64,
) -> Result {
    let category = config_of(state).cancellation_fee_category();
    let net = ratio_half_up(fee, 10_000, 10_000 + category.rate_bp());
    let line = DraftLine::new(
        &english("wl_sales.settle.fee_line", &[("number", &order.number)]),
        net,
        category,
    )
    .origin(order.subject());
    let mut draft = InvoiceDraft::new(order.party(), order.subject()).currency(&order.currency);
    if let Some(entity) = order.legal_entity_id {
        draft = draft.legal_entity(entity);
    }
    draft = draft.line(line);
    match invoicer(state).draft(&state.db, draft).await {
        Ok(created) => {
            let row = store(
                state,
                order,
                Some(change.id),
                SETTLE_FEE_INVOICE,
                fee,
                STATUS_REQUESTED,
            )
            .await?;
            OrderSettlement::query()
                .where_(OrderSettlement::ID.eq(row.id))
                .update(&state.db, &[("reference_id", &created.map(|i| i.0))])
                .await?;
        }
        Err(error) => {
            failed(
                state,
                order,
                Some(change.id),
                SETTLE_FEE_INVOICE,
                fee,
                &error.to_string(),
            )
            .await?;
        }
    }
    Ok(())
}

/// Registers the minute job and the command that expire the payment links.
pub(crate) fn register(app: &mut Registry) {
    app.model::<OrderSettlement>();
    app.model::<OrderChange>();
    app.model::<OrderRevision>();
    app.schedule()
        .every_minute(TASK, |state: AppState| async move {
            lapse_links(&state, now()).await.map(|_| ())
        });
    app.command(
        TASK,
        "Expire the payment links of amended orders whose time has passed",
        |_args, state: AppState| async move {
            let expired = lapse_links(&state, now()).await?;
            println!("{expired} payment links expired.");
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vat_is_added_to_a_net_and_a_fee_is_split_into_net_and_vat() {
        assert_eq!(with_vat(38_095, VatCategory::Standard), 40_000);
        assert_eq!(with_vat(40_000, VatCategory::ZeroRated), 40_000);
        assert_eq!(ratio_half_up(800, 10_000, 10_500), 762);
    }

    #[test]
    fn only_an_online_order_with_its_payment_in_is_prepaid() {
        let mut order = SalesOrder {
            payment_mode: PAYMENT_ONLINE.to_owned(),
            payment_status: OrderPaymentStatus::Received,
            ..SalesOrder::default()
        };
        assert!(is_prepaid(&order));
        order.payment_status = OrderPaymentStatus::Pending;
        assert!(!is_prepaid(&order));
        order.payment_mode = "on_account".to_owned();
        order.payment_status = OrderPaymentStatus::Received;
        assert!(!is_prepaid(&order));
    }
}
