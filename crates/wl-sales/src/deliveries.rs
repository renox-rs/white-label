//! The order follows its deliveries (wl-sales §4.2 rows `fulfil` and
//! `reverse_fulfilment`, the paragraph *Fulfilment from deliveries*, §5.2
//! step 4, §6 consumed events; BL-SAL-13, review items X-11, X-14, 07-N4-a).
//!
//! | Event | Source | What the order does |
//! |---|---|---|
//! | `DispatchDelivered` | an order | `fulfil` for the quantities the customer **accepted** (per sale line, summed over the dispatch lines); once per dispatch |
//! | `DeliveryDelivered` | an order (no dispatch note) | the same, with the delivery as the hand-over |
//! | `DispatchReturned`, `DeliveryReturned` | an order | no transition: the lines show *not dispatched* again, the reservation request is kept, the sales rep and the customer are told; the lines are delivered again (a new dispatch) or cancelled (`cancel_line`) |
//! | `DispatchReversed`, `DeliveryReversed` | an order | `reverse_fulfilment` of what that hand-over held (`reverse_fulfilment_held` on a credit hold); a **closed** order stays closed and the accountant and the sales rep get a task; an issued invoice is not changed and the accountant gets a task; a draft invoice is brought up to date |
//! | `DispatchLost`, `DeliveryLost` | an order | nothing is fulfilled; from a delivered dispatch (or a delivery under review) `reverse_fulfilment` runs first (cause `lost`); the sales rep gets a task; no replacement or credit is made |
//!
//! Decisions:
//! - **One hand-over, one fulfilment.** An order delivered through a dispatch
//!   note gets `DeliveryDelivered`, `DeliveryReversed` and so on from
//!   wl-delivery as well as the `Dispatch*` events; the delivery events name
//!   the *dispatch* as their source, and the order follows the dispatch
//!   note's events only, ignoring those. A delivery requested for the order
//!   itself is followed by its own events, the delivery being the hand-over.
//! - A refused quantity stays open on its line and nothing is cancelled for
//!   it. A dispatch line with no sale line is not on the order and is left
//!   out.
//! - **The fulfilment progress** of a line (not dispatched, picked, out for
//!   delivery, partly delivered, delivered, lost) is worked out from the
//!   dispatches of the order each time it is shown ([`crate::progress`]); it
//!   is not a state.
//! - An order that cannot take the fulfilment (on credit hold, cancelled,
//!   closed, a quantity above what is open) does not fail the delivery: the
//!   sales rep gets a task that names the delivery and says why nothing was
//!   fulfilled. A `credit_hold` order does take a *reversal*.
//! - The task goes to the customer's sales rep (its account), else to the
//!   user who entered the order, else to the holder of the sales rep role
//!   (the company admin when nobody holds it); the accountant's task goes to
//!   the accountant role.

use std::collections::BTreeMap;

use renox::prelude::*;
use renox::serde_json::json;
use wl_kernel::Subject;
use wl_kernel::delivery::{
    DeliveryDelivered, DeliveryId, DeliveryLost, DeliveryReturned, DeliveryReversed,
};
use wl_kernel::inventory::{
    DISPATCH_SUBJECT_TYPE, DispatchDelivered, DispatchLost, DispatchReturned, DispatchReversed,
    DispatchedLine, dispatch_directory,
};
use wl_kernel::notify::{self, NotifyRequest};
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::sales::{FulfilOrder, FulfilledLine, ReverseFulfilment, SALES_ORDER_SUBJECT_TYPE};
use wl_kernel::tickets::{Assignee, OpenTicket, TASK_CATEGORY, task_creator};

use crate::config::config_of;
use crate::customer_account::CustomerAccount;
use crate::delivery_invoice::issued_invoice;
use crate::notices::aed;
use crate::sales_order::{FulfilmentRecord, SalesOrder, SalesOrderStatus};
use crate::sales_orders::{fulfil, reverse_fulfilment};
use crate::texts::english;

/// The subject type of a sale line, as a dispatch line or a delivery line
/// names the line it fulfils.
pub const SALE_LINE_SUBJECT_TYPE: &str = "sales.sale_line";

/// The customer's delivery came back: the sales rep and the customer are told.
pub const DELIVERY_RETURNED: &str = "sales.delivery_returned";

/// The sale line named by a source line reference.
fn sale_line_of(line: &Option<Subject>) -> Option<i64> {
    line.as_ref()
        .filter(|line| line.subject_type == SALE_LINE_SUBJECT_TYPE)
        .map(|line| line.subject_id)
}

/// The order a hand-over is for, when the source is a sales order.
async fn order_of(state: &AppState, source: &Subject) -> Result<Option<SalesOrder>> {
    if source.subject_type != SALES_ORDER_SUBJECT_TYPE {
        return Ok(None);
    }
    SalesOrder::find(&state.db, source.subject_id).await
}

/// The accepted quantities of a dispatch, per sale line.
fn accepted(lines: &[DispatchedLine]) -> Vec<FulfilledLine> {
    let mut per_line: BTreeMap<i64, i64> = BTreeMap::new();
    for line in lines {
        if let Some(id) = sale_line_of(&line.source_line)
            && line.delivered_milli > 0
        {
            *per_line.entry(id).or_default() += line.delivered_milli;
        }
    }
    per_line
        .into_iter()
        .map(|(line, quantity_milli)| FulfilledLine {
            line,
            quantity_milli,
        })
        .collect()
}

/// The user who owns the follow-up of `order`: the rep of the customer's
/// account, else the user who entered the order, else a holder of the sales
/// rep role, else the company admin.
async fn rep_of(state: &AppState, order: &SalesOrder) -> Result<Option<i64>> {
    if let Some(account) = CustomerAccount::of_party(&state.db, order.party_id).await?
        && let Some(rep) = account.sales_rep_id
    {
        return Ok(Some(rep));
    }
    if let Some(entered) = order.created_by {
        return Ok(Some(entered));
    }
    role_tasks::assignee(&state.db, &config_of(state).sales_rep_role).await
}

/// Opens the task `key` for the sales rep of `order`, once while it is open.
async fn rep_task(
    state: &AppState,
    order: &SalesOrder,
    key: &str,
    title: &str,
    body: &str,
) -> Result {
    let creator = task_creator(state);
    if creator.find_open(state, key).await?.is_some() {
        return Ok(());
    }
    let mut ticket = OpenTicket::new(order.subject(), TASK_CATEGORY, title)
        .description(body)
        .source_key(key);
    if let Some(rep) = rep_of(state, order).await? {
        ticket = ticket.assign_to(Assignee::user(rep));
    }
    creator.open(state, ticket).await?;
    Ok(())
}

/// Opens the task `key` for the accountant role, once while it is open.
async fn accountant_task(
    state: &AppState,
    order: &SalesOrder,
    key: &str,
    title: &str,
    body: &str,
) -> Result {
    let config = config_of(state);
    let role = if config.accountant_role.is_empty() {
        wl_kernel::COMPANY_ADMIN_ROLE.to_owned()
    } else {
        config.accountant_role
    };
    role_tasks::open(
        state,
        RoleTask::new(order.subject(), &role, title, key).description(body),
    )
    .await?;
    Ok(())
}

/// The title and the body of the task texts `{key}_title` and `{key}_body`
/// of `wl_sales.deliveries`, with `params` filled in. Built apart so the
/// borrowed parameters are gone before the task is awaited.
fn words(key: &str, params: &[(&str, &str)]) -> (String, String) {
    let params: Vec<(&str, &dyn std::fmt::Display)> = params
        .iter()
        .map(|(name, value)| (*name, value as &dyn std::fmt::Display))
        .collect();
    (
        english(&format!("wl_sales.deliveries.{key}_title"), &params),
        english(&format!("wl_sales.deliveries.{key}_body"), &params),
    )
}

/// What the hand-over is called in a text: the number of the dispatch note
/// when the directory knows it, else `delivery 5`.
async fn handover_text(state: &AppState, source: &Subject) -> Result<String> {
    if source.subject_type == DISPATCH_SUBJECT_TYPE
        && let Some(facts) = dispatch_directory(state).facts(state, source).await?
        && !facts.number.is_empty()
    {
        return Ok(facts.number);
    }
    Ok(format!("{} {}", source.subject_type, source.subject_id))
}

/// Fulfils `order` with `lines` from the hand-over `source`; a refusal
/// becomes a task for the sales rep.
async fn fulfil_from(
    state: &AppState,
    order: SalesOrder,
    lines: Vec<FulfilledLine>,
    source: Subject,
) -> Result {
    if lines.is_empty() {
        return Ok(());
    }
    let number = order.number.clone();
    let why = match order.status {
        SalesOrderStatus::Confirmed | SalesOrderStatus::PartiallyFulfilled => None,
        other => Some(english(
            &format!("wl_sales.deliveries.status.{}", other.as_str()),
            &[],
        )),
    };
    let why = match why {
        Some(why) => Some(why),
        None => match fulfil(
            state,
            FulfilOrder {
                order: order.order_id(),
                lines,
                source: source.clone(),
            },
        )
        .await
        {
            Ok(_) => None,
            Err(Error::Status(status, message)) if status == StatusCode::UNPROCESSABLE_ENTITY => {
                Some(message)
            }
            Err(error) => return Err(error),
        },
    };
    if let Some(why) = why {
        let hand_over = handover_text(state, &source).await?;
        let (title, body) = words(
            "not_fulfilled",
            &[("number", &number), ("delivery", &hand_over), ("why", &why)],
        );
        rep_task(
            state,
            &order,
            &format!("sales.delivery_not_fulfilled:{}:{}", order.id, source),
            &title,
            &body,
        )
        .await?;
    }
    Ok(())
}

/// Takes back what the hand-over `source` fulfilled on `order`.
async fn reverse_from(state: &AppState, order: SalesOrder, source: Subject, cause: &str) -> Result {
    let Some(record) = FulfilmentRecord::of_source(&state.db, order.id, &source).await? else {
        return Ok(());
    };
    let held = record.held();
    if held.is_empty() {
        return Ok(());
    }
    let number = order.number.clone();
    let hand_over = handover_text(state, &source).await?;
    match order.status {
        SalesOrderStatus::PartiallyFulfilled
        | SalesOrderStatus::Fulfilled
        | SalesOrderStatus::CreditHold => {
            reverse_fulfilment(
                state,
                ReverseFulfilment {
                    order: order.order_id(),
                    lines: held,
                    source: source.clone(),
                    cause: cause.to_owned(),
                },
            )
            .await?;
            let order = SalesOrder::find_or_404(&state.db, order.id).await?;
            if let Some(invoice) = issued_invoice(state, &order).await? {
                let invoice = invoice.to_string();
                let (title, body) = words(
                    "invoice",
                    &[
                        ("number", &number),
                        ("delivery", &hand_over),
                        ("invoice", &invoice),
                    ],
                );
                accountant_task(
                    state,
                    &order,
                    &format!("sales.reversal_invoice:{}:{}", order.id, source),
                    &title,
                    &body,
                )
                .await?;
            }
        }
        SalesOrderStatus::Closed => {
            let key = format!("sales.reversal_closed:{}:{}", order.id, source);
            let (title, body) = words("closed", &[("number", &number), ("delivery", &hand_over)]);
            accountant_task(state, &order, &key, &title, &body).await?;
            rep_task(state, &order, &format!("{key}:rep"), &title, &body).await?;
        }
        _ => {}
    }
    Ok(())
}

/// Tells the sales rep that the delivery of `order` is lost.
async fn lost_task(state: &AppState, order: &SalesOrder, source: &Subject) -> Result {
    let number = order.number.clone();
    let hand_over = handover_text(state, source).await?;
    let (title, body) = words("lost", &[("number", &number), ("delivery", &hand_over)]);
    rep_task(
        state,
        order,
        &format!("sales.delivery_lost:{}:{}", order.id, source),
        &title,
        &body,
    )
    .await
}

/// Tells the sales rep and the customer that the goods of `order` came back.
async fn returned_notice(state: &AppState, order: &SalesOrder, reason: &str) -> Result {
    let mut request = NotifyRequest::new(DELIVERY_RETURNED, order.subject())
        .to_party(order.party())
        .subject_label(&order.number)
        .data(json!({
            "number": order.number,
            "total": aed(order.gross_total),
            "reason": reason,
        }));
    if let Some(rep) = rep_of(state, order).await? {
        request = request.to_user(rep);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// The hand-over a delivery is, when no dispatch note stands for it.
fn delivery_source(delivery: DeliveryId) -> Subject {
    delivery.subject()
}

/// Registers the listeners.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: DispatchDelivered, state: AppState| async move {
        match order_of(&state, &event.source).await? {
            Some(order) => fulfil_from(&state, order, accepted(&event.lines), event.dispatch).await,
            None => Ok(()),
        }
    });
    app.listen(|event: DeliveryDelivered, state: AppState| async move {
        // A dispatch note fulfils the order through `DispatchDelivered`.
        let Some(order) = order_of(&state, &event.source).await? else {
            return Ok(());
        };
        let lines = event
            .lines
            .iter()
            .filter_map(|line| {
                Some(FulfilledLine {
                    line: sale_line_of(&line.line)?,
                    quantity_milli: line.delivered_milli,
                })
            })
            .filter(|line| line.quantity_milli > 0)
            .collect();
        fulfil_from(&state, order, lines, delivery_source(event.delivery)).await
    });
    app.listen(|event: DispatchReturned, state: AppState| async move {
        match order_of(&state, &event.source).await? {
            Some(order) => returned_notice(&state, &order, &event.reason).await,
            None => Ok(()),
        }
    });
    app.listen(|event: DeliveryReturned, state: AppState| async move {
        match order_of(&state, &event.source).await? {
            Some(order) => returned_notice(&state, &order, &event.reason).await,
            None => Ok(()),
        }
    });
    app.listen(|event: DispatchReversed, state: AppState| async move {
        match order_of(&state, &event.source).await? {
            Some(order) => reverse_from(&state, order, event.dispatch, "reversed").await,
            None => Ok(()),
        }
    });
    app.listen(|event: DeliveryReversed, state: AppState| async move {
        match order_of(&state, &event.source).await? {
            Some(order) => {
                reverse_from(&state, order, delivery_source(event.delivery), "reversed").await
            }
            None => Ok(()),
        }
    });
    app.listen(|event: DispatchLost, state: AppState| async move {
        let Some(order) = order_of(&state, &event.source).await? else {
            return Ok(());
        };
        if event.was_delivered {
            reverse_from(&state, order.clone(), event.dispatch.clone(), "lost").await?;
        }
        let order = SalesOrder::find_or_404(&state.db, order.id).await?;
        lost_task(&state, &order, &event.dispatch).await
    });
    app.listen(|event: DeliveryLost, state: AppState| async move {
        let Some(order) = order_of(&state, &event.source).await? else {
            return Ok(());
        };
        let source = delivery_source(event.delivery);
        if event.was_under_review {
            reverse_from(&state, order.clone(), source.clone(), "lost").await?;
        }
        let order = SalesOrder::find_or_404(&state.db, order.id).await?;
        lost_task(&state, &order, &source).await
    });
}
