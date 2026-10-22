//! The staff pages and actions on F&B orders, the kitchen board, the cash on
//! delivery list and the checkout's payment choice (06 F&B §7.1 *Order* and
//! *Kitchen ticket*, §7.2 `FNB.RUL.14`).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /orders` | `fnb.orders` | `orders.view` |
//! | `GET /orders/{id}` | `fnb.order` | `orders.view` |
//! | `POST /orders/{id}/accept` | `fnb.order_accept` | `orders.manage` (and `sales_orders.accept`) |
//! | `POST /orders/{id}/reject` | `fnb.order_reject` | `orders.manage` (and `sales_orders.accept`), field `reason` |
//! | `POST /orders/{id}/details` | `fnb.order_details` | `orders.manage` |
//! | `POST /orders/{id}/dispatch` | `fnb.order_dispatch` | `orders.manage` |
//! | `POST /orders/{id}/complete` | `fnb.order_complete` | `orders.manage` |
//! | `POST /orders/{id}/close` | `fnb.order_close` | `orders.manage` |
//! | `POST /orders/{id}/responsible` | `fnb.order_responsible` | `orders.manage`, fields `party`, `note` |
//! | `POST /orders/{id}/refund` | `fnb.order_refund` | `orders.refund_request`, fields `amount`, `reason` |
//! | `POST /orders/{id}/cancel` | `fnb.order_cancel` | `orders.cancel`, fields `reason`, `party`, `note`, `outcome_<line>` |
//! | `POST /orders/{id}/amend` | `fnb.order_amend` | `orders.amend`, fields `line`, `new_item`, `quantity`, `price`, `reason`, `customer_agreed`, `call_time` |
//! | `POST /orders/{id}/line-cancel` | `fnb.order_line_cancel` | `orders.cancel`, fields `line`, `reason`, `note`, `outcome` |
//! | `POST /orders/{id}/pickup/late` | `fnb.order_picked_up_late` | `orders.manage`, an aggregator's `ready` order |
//! | `POST /orders/{id}/pickup/cancel` | `fnb.order_aggregator_cancelled` | `orders.manage` and `orders.cancel`, an aggregator's `ready` order |
//! | `GET /my/orders/{id}` | `fnb.my_order` | the order's customer |
//! | `POST /my/orders/{id}/cancel` | `fnb.my_order_cancel` | the order's customer, while it is `placed` |
//! | `GET /kitchen` | `fnb.kitchen` | `kds.work` |
//! | `POST /tickets/{id}/start` | `fnb.ticket_start` | `kds.work` |
//! | `POST /tickets/{id}/bump` | `fnb.ticket_bump` | `kds.work` |
//! | `GET /customers/cod` | `fnb.cod_customers` | `customers.cod` |
//! | `POST /customers/{party}/cod` | `fnb.cod_on` | `customers.cod` |
//! | `GET /checkout/payment` | `fnb.checkout_payment` | signed in (the customer; staff give `party`) |
//!
//! The order page also posts to wl-delivery's own routes for a delivered
//! order the customer says never arrived: `POST /deliveries/{id}/open-review`
//! (`deliveries.manage`), `POST /deliveries/{id}/close-review`
//! (`deliveries.manage`) and `POST /deliveries/{id}/reverse`
//! (`deliveries.reverse`, a second holder approves on the delivery page).
//!
//! A successful action answers a redirect (303) with a toast. A refused
//! action answers 422 with the reason (403 without the permission). No page
//! offers `fail_delivery`, a reattempt or a manual failure: a failed delivery
//! of cooked food is the system's and gets one attempt.

use std::collections::BTreeMap;

use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use serde::Serialize;
use wl_kernel::PartyId;
use wl_kernel::sales::{ChangeOutcome, order_desk};

use crate::cancel::{self, StaffCancel};
use crate::changes::{self, AmendAsk, ChangeNote, ExpectedRefund, LineAsk};
use crate::cod::{self, CustomerFlag};
use crate::money::parse_dirhams;
use crate::order::{FnbOrder, FnbOrderStatus, cancel_reason, kind, party};
use crate::permissions;
use crate::placing::{Details, refused};
use crate::refunds::{self, RefundAsk};
use crate::texts::{every, t};
use crate::ticket::{BUMP, KitchenTicket, START, TicketStatus};
use crate::{close, failed, fulfilment, pickup, placing};

/// The templates, compiled in. An app replaces one with a file of the same
/// name under its views directory.
pub(crate) const VIEWS: &[(&str, &str)] = &[
    (
        "fnb/orders.html",
        include_str!("../views/fnb/orders.html"),
    ),
    ("fnb/order.html", include_str!("../views/fnb/order.html")),
    (
        "fnb/kitchen.html",
        include_str!("../views/fnb/kitchen.html"),
    ),
    (
        "fnb/cod_customers.html",
        include_str!("../views/fnb/cod_customers.html"),
    ),
    (
        "fnb/checkout_payment.html",
        include_str!("../views/fnb/checkout_payment.html"),
    ),
    (
        "fnb/my_order.html",
        include_str!("../views/fnb/my_order.html"),
    ),
];

/// The texts a template reads as `tx['title']`.
pub(crate) type Texts = BTreeMap<&'static str, String>;

const PREFIX: &str = "fnb.screen.";

/// wl-delivery's permission to put a delivery under review and close the
/// review (`WRK.ACL.10` is `deliveries.reverse`).
const DELIVERIES_MANAGE: &str = "deliveries.manage";

/// wl-delivery's permission to reverse a delivery: the kitchen manager, never
/// the rider.
const DELIVERIES_REVERSE: &str = "deliveries.reverse";

/// Every text of the screens, by the name a template reads.
pub(crate) fn tx(lang: &Lang) -> Texts {
    every().filter_map(|(key, _)| Some((key.strip_prefix(PREFIX)?, t(lang, key))))
        .collect()
}

/// The word of a status.
fn status_word(lang: &Lang, status: FnbOrderStatus) -> String {
    t(lang, &format!("fnb.screen.status.{}", status.as_str()))
}

/// The badge colour of a status.
fn status_kind(status: FnbOrderStatus) -> &'static str {
    use FnbOrderStatus::{
        Accepted, Cancelled, Completed, DeliveryFailed, Dispatched, Placed, Preparing, Ready,
        Rejected,
    };
    match status {
        Completed | Ready => "success",
        Accepted | Preparing | Dispatched => "info",
        DeliveryFailed => "warning",
        Cancelled => "error",
        Placed | Rejected => "info",
    }
}

/// The routes.
pub fn routes() -> Routes {
    Routes::new()
        .get("/orders", index)
        .name("fnb.orders")
        .get("/orders/{id}", show)
        .name("fnb.order")
        .post("/orders/{id}/accept", accept)
        .name("fnb.order_accept")
        .post("/orders/{id}/reject", reject)
        .name("fnb.order_reject")
        .post("/orders/{id}/details", details)
        .name("fnb.order_details")
        .post("/orders/{id}/dispatch", dispatch)
        .name("fnb.order_dispatch")
        .post("/orders/{id}/complete", complete)
        .name("fnb.order_complete")
        .post("/orders/{id}/close", close_failed)
        .name("fnb.order_close")
        .post("/orders/{id}/responsible", responsible)
        .name("fnb.order_responsible")
        .post("/orders/{id}/refund", refund)
        .name("fnb.order_refund")
        .post("/orders/{id}/cancel", cancel_order)
        .name("fnb.order_cancel")
        .post("/orders/{id}/amend", amend_order)
        .name("fnb.order_amend")
        .post("/orders/{id}/line-cancel", cancel_line)
        .name("fnb.order_line_cancel")
        .post("/orders/{id}/pickup/late", picked_up_late)
        .name("fnb.order_picked_up_late")
        .post("/orders/{id}/pickup/cancel", aggregator_cancelled)
        .name("fnb.order_aggregator_cancelled")
        .get("/my/orders/{id}", my_order)
        .name("fnb.my_order")
        .post("/my/orders/{id}/cancel", my_order_cancel)
        .name("fnb.my_order_cancel")
        .get("/kitchen", kitchen)
        .name("fnb.kitchen")
        .post("/tickets/{id}/start", ticket_start)
        .name("fnb.ticket_start")
        .post("/tickets/{id}/bump", ticket_bump)
        .name("fnb.ticket_bump")
        .get("/customers/cod", cod_customers)
        .name("fnb.cod_customers")
        .post("/customers/{party}/cod", cod_on)
        .name("fnb.cod_on")
        .get("/checkout/payment", checkout_payment)
        .name("fnb.checkout_payment")
        .require_auth()
}

fn answer(error: TransitionError) -> Error {
    error.into_error()
}

pub(crate) type Fields = Form<BTreeMap<String, String>>;

pub(crate) fn field(form: &BTreeMap<String, String>, name: &str) -> String {
    form.get(name).cloned().unwrap_or_default()
}

/// A redirect to the order's page with a toast.
fn back_to(lang: &Lang, key: &str, id: i64) -> Result<Response> {
    Ok((
        Toast::success(t(lang, key)),
        Redirect::route("fnb.order", &[&id])?,
    )
        .into_response())
}

/// One order of the list.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct Row {
    id: i64,
    number: String,
    recipient: String,
    area: String,
    channel: String,
    kind: String,
    payment: String,
    total: i64,
    status: String,
    status_kind: &'static str,
    failure: String,
    url: String,
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "fnb/orders.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    orders: Vec<Row>,
}

async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<Response> {
    if !user.has_permission(permissions::ORDERS_VIEW) {
        return Err(Error::Forbidden);
    }
    let orders = FnbOrder::newest(&state.db, 200)
        .await?
        .iter()
        .map(|o| Row {
            id: o.id,
            number: o.number.clone(),
            recipient: o.recipient_name.clone(),
            area: o.area.clone(),
            channel: o.channel.clone(),
            kind: t(&lang, &format!("fnb.screen.kind.{}", o.kind)),
            payment: t(&lang, &format!("fnb.screen.payment.{}", o.payment_mode)),
            total: o.total,
            status: status_word(&lang, o.status),
            status_kind: status_kind(o.status),
            failure: match (&o.failure_reason, &o.cancel_reason_code) {
                (Some(reason), _) => t(&lang, &format!("fnb.screen.reason.{reason}")),
                (None, Some(code)) if o.status == FnbOrderStatus::Cancelled => {
                    t(&lang, &format!("fnb.cancel.reason.{code}"))
                }
                _ => String::new(),
            },
            url: format!("/orders/{}", o.id),
        })
        .collect();
    Ok(View::typed(IndexPage {
        tx: tx(&lang),
        orders,
    })
    .into_response())
}

/// A line of the page.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct LineRow {
    description: String,
    quantity: String,
    gross: i64,
}

/// A ticket of the page.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct TicketRow {
    description: String,
    station: String,
    status: String,
}

/// A change of state of the page.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct HistoryRow {
    at: String,
    what: String,
    to: String,
    reason: String,
}

/// A refund asked for the order.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct RefundRow {
    amount: i64,
    reason: String,
    approved: bool,
    at: String,
}

/// What the order's page needs.
#[derive(Serialize, ViewData)]
#[view(template = "fnb/order.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    order: FnbOrder,
    status_label: String,
    status_kind: &'static str,
    kind_label: String,
    payment_label: String,
    lines: Vec<LineRow>,
    tickets: Vec<TicketRow>,
    history: Vec<HistoryRow>,
    refunds: Vec<RefundRow>,
    delivery_url: String,
    failure_label: String,
    delivery_reason_label: String,
    side_label: String,
    party_label: String,
    decision_label: String,
    can_accept: bool,
    can_details: bool,
    can_dispatch: bool,
    can_complete: bool,
    can_close: bool,
    can_party: bool,
    can_refund: bool,
    can_cancel: bool,
    parties: Vec<[String; 2]>,
    kinds: Vec<[String; 2]>,
    cancel_reasons: Vec<[String; 2]>,
    cancel_parties: Vec<[String; 2]>,
    outcomes: Vec<[String; 2]>,
    started: Vec<StartedRow>,
    cancel_reason_label: String,
    cancel_party_label: String,
    cancelled_by_label: String,
    can_amend: bool,
    can_cancel_line: bool,
    can_review: bool,
    can_reverse: bool,
    reverse_reasons: Vec<[String; 2]>,
    liabilities: Vec<[String; 2]>,
    refunded_after: i64,
    reversal_resolution_label: String,
    reversal_party_label: String,
    aggregator_waiting: bool,
    can_pickup_late: bool,
    can_pickup_cancel: bool,
    amend_lines: Vec<[String; 2]>,
    open_lines: Vec<[String; 2]>,
    menu: Vec<[String; 2]>,
    line_reasons: Vec<[String; 2]>,
    revision: i64,
    changes: Vec<ChangeRow>,
    expected: Vec<ExpectedRow>,
    notes: Vec<String>,
    refund_left: i64,
}

/// A change made to the order.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ChangeRow {
    at: String,
    what: String,
    by: String,
    reason: String,
    call: String,
    state: String,
}

/// A refund an aggregator is expected to make.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ExpectedRow {
    amount: i64,
    reason: String,
}

/// A started line of the cancel sheet: it needs an outcome.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct StartedRow {
    id: i64,
    description: String,
}

async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    if !user.has_permission(permissions::ORDERS_VIEW) {
        return Err(Error::Forbidden);
    }
    let order = FnbOrder::find_or_404(&state.db, id).await?;
    let manages = user.has_permission(permissions::ORDERS_MANAGE);
    use FnbOrderStatus::{Accepted, DeliveryFailed, Dispatched, Placed, Preparing, Ready};
    let mut notes = Vec::new();
    // Dispatch is offered to a manager of a ready delivery order; without a
    // rider on a run it says why it cannot be done yet.
    let ready_delivery = manages && order.status == Ready && order.is_delivery();
    let can_dispatch = ready_delivery && order.rider_user_id.is_some();
    if ready_delivery && !can_dispatch {
        notes.push(t(&lang, "fnb.dispatch.no_rider"));
    }
    if order.status == DeliveryFailed {
        notes.push(t(&lang, "fnb.screen.note_one_attempt"));
    }
    let refund_problem = if user.has_permission(permissions::ORDERS_REFUND_REQUEST)
        && order.closed_at.is_some()
    {
        refunds::problem(&state, &order).await?
    } else {
        None
    };
    let can_refund = user.has_permission(permissions::ORDERS_REFUND_REQUEST)
        && order.closed_at.is_some()
        && refund_problem.is_none();
    if let Some(problem) = &refund_problem {
        notes.push(problem.clone());
    }
    let view = order_desk(&state).find(&state, order.sales_order()).await?;
    let lines = match &view {
        Some(view) => view
            .lines
            .iter()
            .filter(|line| line.is_item)
            .map(|line| LineRow {
                description: line.description.clone(),
                quantity: format!("{}", line.quantity_milli / 1_000),
                gross: line.gross,
            })
            .collect(),
        None => Vec::new(),
    };
    // Cancel: a request that waits for a manager says so instead.
    let cancellable = matches!(order.status, Placed | Accepted | Preparing | Ready);
    let holds_cancel = user.has_permission(permissions::ORDERS_CANCEL);
    let cancel_waiting = holds_cancel && cancellable && cancel::waiting(&state.db, &order).await?;
    if cancel_waiting {
        notes.push(t(&lang, "fnb.cancel.pending"));
    }
    let can_cancel = holds_cancel && cancellable && !cancel_waiting;
    let aggregator = view.as_ref().is_some_and(|view| view.aggregator);
    let mut started = Vec::new();
    if can_cancel && let Some(view) = &view {
        for line in cancel::started_lines(&state.db, &order, view).await? {
            started.push(StartedRow {
                id: line.id,
                description: line.description.clone(),
            });
        }
    }
    // Amend and cancel a line (BL-FNB-05).
    let holds_amend = user.has_permission(permissions::ORDERS_AMEND);
    let all_started: Vec<i64> = match &view {
        Some(view) => cancel::started_lines(&state.db, &order, view)
            .await?
            .iter()
            .map(|line| line.id)
            .collect(),
        None => Vec::new(),
    };
    let tickets_of_order = KitchenTicket::of_order(&state.db, order.id).await?;
    let open_item_lines: Vec<&wl_kernel::sales::OrderLineView> = match &view {
        Some(view) => view
            .lines
            .iter()
            .filter(|line| line.is_item && line.open_milli() > 0)
            .collect(),
        None => Vec::new(),
    };
    let amend_lines: Vec<[String; 2]> = open_item_lines
        .iter()
        .filter(|line| !all_started.contains(&line.id))
        .map(|line| [line.id.to_string(), line.description.clone()])
        .collect();
    let can_amend =
        holds_amend && matches!(order.status, Placed | Accepted) && !aggregator && !amend_lines.is_empty();
    if holds_amend && matches!(order.status, Placed | Accepted) && aggregator {
        notes.push(t(&lang, "fnb.amend.aggregator"));
    }
    let open_lines: Vec<[String; 2]> = open_item_lines
        .iter()
        .filter(|line| {
            !tickets_of_order
                .iter()
                .any(|ticket| ticket.sale_line_id == line.id && ticket.status == crate::ticket::TicketStatus::Done)
        })
        .map(|line| [line.id.to_string(), line.description.clone()])
        .collect();
    let line_ready = holds_cancel && matches!(order.status, Accepted | Preparing);
    if line_ready && open_item_lines.len() < 2 {
        notes.push(t(&lang, "fnb.line.last"));
    }
    let can_cancel_line = line_ready && open_item_lines.len() >= 2 && !cancel_waiting;
    let menu: Vec<[String; 2]> = if can_amend {
        crate::catalogue::MenuItem::query()
            .order_by(crate::catalogue::MenuItem::NAME)
            .get(&state.db)
            .await?
            .into_iter()
            .map(|item| [item.code, item.name])
            .collect()
    } else {
        Vec::new()
    };
    let mut changes = Vec::new();
    for note in ChangeNote::of_order(&state.db, order.id).await? {
        let by = match note.requested_by {
            Some(id) => User::find(&state.db, id)
                .await?
                .map(|found| found.name)
                .unwrap_or_default(),
            None => String::new(),
        };
        changes.push(ChangeRow {
            at: note.created_at.map(|at| at.to_string()).unwrap_or_default(),
            what: t(&lang, &format!("fnb.screen.change.{}", note.kind)),
            by,
            reason: note.reason.clone(),
            call: note.call_time.clone().unwrap_or_default(),
            state: if note.status == changes::WAITING {
                t(&lang, "fnb.screen.waiting")
            } else {
                t(&lang, "fnb.screen.done")
            },
        });
    }
    let expected: Vec<ExpectedRow> = ExpectedRefund::of_order(&state.db, order.id)
        .await?
        .into_iter()
        .map(|row| ExpectedRow {
            amount: row.amount,
            reason: row.reason,
        })
        .collect();
    let revision = view.as_ref().map_or(1, |view| view.revision);
    let refunded_after = view.as_ref().map_or(0, |view| view.refunded_amount);
    // A delivered order the customer says never arrived: review it, and the
    // kitchen manager reverses the delivery (`WRK.ACL.10`).
    let delivered_order = order.status == FnbOrderStatus::Completed
        && order.is_delivery()
        && order.delivery_id.is_some();
    let can_review = delivered_order && user.has_permission(DELIVERIES_MANAGE);
    let can_reverse = delivered_order && user.has_permission(DELIVERIES_REVERSE);
    // An aggregator's order waiting at the pass: the kitchen manager records
    // what happened to its rider (`FNB.RUL.16`).
    let aggregator_waiting = aggregator && order.status == Ready;
    let cancelled_by_label = if order.cancelled_by_customer {
        t(&lang, "fnb.screen.cancelled_by_customer")
    } else {
        match order.cancel_requested_by {
            Some(id) if order.cancelled_at.is_some() => User::find(&state.db, id)
                .await?
                .map(|found| found.name)
                .unwrap_or_default(),
            _ => String::new(),
        }
    };
    let tickets = KitchenTicket::of_order(&state.db, order.id)
        .await?
        .iter()
        .map(|ticket| TicketRow {
            description: ticket.description.clone(),
            station: ticket.station.clone(),
            status: t(&lang, &format!("fnb.screen.ticket.{}", ticket.status.as_str())),
        })
        .collect();
    let history = renox::state_machine::history(&state.db, FnbOrder::TABLE, order.id)
        .await?
        .into_iter()
        .map(|row| HistoryRow {
            at: row.at.to_string(),
            what: row.transition,
            to: t(&lang, &format!("fnb.screen.status.{}", row.to_state)),
            reason: row.reason.unwrap_or_default(),
        })
        .collect();
    let asked: Vec<RefundRow> = RefundAsk::of_order(&state.db, order.id)
        .await?
        .iter()
        .map(|ask| RefundRow {
            amount: ask.amount,
            reason: ask.reason.clone(),
            approved: ask.approved_at.is_some(),
            at: ask.created_at.map(|at| at.to_string()).unwrap_or_default(),
        })
        .collect();
    let word = |prefix: &str, value: &Option<String>| {
        value
            .as_deref()
            .map(|v| t(&lang, &format!("fnb.screen.{prefix}.{v}")))
            .unwrap_or_default()
    };
    let page = ShowPage {
        tx: tx(&lang),
        status_label: status_word(&lang, order.status),
        status_kind: status_kind(order.status),
        kind_label: t(&lang, &format!("fnb.screen.kind.{}", order.kind)),
        payment_label: t(&lang, &format!("fnb.screen.payment.{}", order.payment_mode)),
        lines,
        tickets,
        history,
        refunds: asked,
        delivery_url: order
            .delivery_id
            .map(|id| format!("/deliveries/{id}"))
            .unwrap_or_default(),
        failure_label: word("reason", &order.failure_reason),
        delivery_reason_label: order.delivery_reason.clone().unwrap_or_default(),
        side_label: word("side", &order.failure_side),
        party_label: word("party", &order.responsible_party),
        decision_label: word("decision", &order.refund_decision),
        can_accept: manages && order.status == Placed,
        can_details: manages && matches!(order.status, Placed | Accepted | Preparing | Ready),
        can_dispatch,
        can_complete: manages && matches!(order.status, Ready | Dispatched),
        can_close: manages && order.status == DeliveryFailed,
        can_party: manages && order.failure_reason.is_some(),
        can_refund,
        can_cancel,
        cancel_reasons: cancel_reason::CHOICES
            .iter()
            .map(|c| [(*c).to_owned(), t(&lang, &format!("fnb.cancel.reason.{c}"))])
            .collect(),
        cancel_parties: party::CANCEL
            .iter()
            .filter(|p| aggregator || **p != party::AGGREGATOR)
            .map(|p| [(*p).to_owned(), t(&lang, &format!("fnb.screen.party.{p}"))])
            .collect(),
        outcomes: ["return_to_stock", "scrap"]
            .iter()
            .map(|o| [(*o).to_owned(), t(&lang, &format!("fnb.screen.outcome.{o}"))])
            .collect(),
        started,
        can_amend,
        can_cancel_line,
        can_review,
        can_reverse,
        reverse_reasons: ["marked_by_mistake", "recipient_denies", "wrong_recipient"]
            .iter()
            .map(|r| [(*r).to_owned(), t(&lang, &format!("fnb.screen.reverse_reason.{r}"))])
            .collect(),
        liabilities: ["driver", "business", "client", "none"]
            .iter()
            .map(|l| [(*l).to_owned(), t(&lang, &format!("fnb.screen.liability.{l}"))])
            .collect(),
        refunded_after,
        reversal_resolution_label: order
            .reversal_resolution
            .as_deref()
            .map(|how| t(&lang, &format!("fnb.screen.resolution.{how}")))
            .unwrap_or_default(),
        reversal_party_label: order
            .reversal_party
            .as_deref()
            .map(|who| t(&lang, &format!("fnb.screen.return_party.{who}")))
            .unwrap_or_default(),
        aggregator_waiting,
        can_pickup_late: manages && aggregator_waiting,
        can_pickup_cancel: manages && holds_cancel && aggregator_waiting,
        amend_lines,
        open_lines,
        menu,
        line_reasons: changes::LINE_REASON_CODES
            .iter()
            .map(|c| [(*c).to_owned(), t(&lang, &format!("fnb.line.reason.{c}"))])
            .collect(),
        revision,
        changes,
        expected,
        cancel_reason_label: order
            .cancel_reason_code
            .as_deref()
            .map(|code| t(&lang, &format!("fnb.cancel.reason.{code}")))
            .unwrap_or_default(),
        cancel_party_label: word("party", &order.cancel_party),
        cancelled_by_label,
        parties: party::ALL
            .iter()
            .map(|p| [(*p).to_owned(), t(&lang, &format!("fnb.screen.party.{p}"))])
            .collect(),
        kinds: [kind::DELIVERY, kind::PICKUP]
            .iter()
            .map(|k| [(*k).to_owned(), t(&lang, &format!("fnb.screen.kind.{k}"))])
            .collect(),
        notes,
        refund_left: refunds::refundable(&state.db, &order).await?,
        order,
    };
    Ok(View::typed(page).into_response())
}

/// `POST /orders/{id}/accept`: the sales order is accepted as the signed-in
/// user, and the order follows it.
async fn accept(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    if !user.has_permission(permissions::ORDERS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let order = FnbOrder::find_or_404(&state.db, id).await?;
    if order.status != FnbOrderStatus::Placed {
        return Err(refused(t(&lang, "fnb.accept.not_placed")));
    }
    order_desk(&state)
        .accept(&state, &user, order.sales_order())
        .await?;
    back_to(&lang, "fnb.accepted", id)
}

/// `POST /orders/{id}/reject`: field `reason`.
async fn reject(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    if !user.has_permission(permissions::ORDERS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let order = FnbOrder::find_or_404(&state.db, id).await?;
    if order.status != FnbOrderStatus::Placed {
        return Err(refused(t(&lang, "fnb.accept.not_placed")));
    }
    let reason = field(&form, "reason");
    if reason.trim().is_empty() {
        return Err(refused(t(&lang, "fnb.reject.reason_needed")));
    }
    order_desk(&state)
        .reject(&state, &user, order.sales_order(), reason.trim())
        .await?;
    back_to(&lang, "fnb.rejected", id)
}

/// `POST /orders/{id}/details`: fields `kind`, `recipient_name`,
/// `recipient_phone`, `address`, `area`.
async fn details(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    if !user.has_permission(permissions::ORDERS_MANAGE) {
        return Err(Error::Forbidden);
    }
    placing::set_details(
        &state,
        id,
        &Details {
            kind: field(&form, "kind"),
            recipient_name: field(&form, "recipient_name"),
            recipient_phone: field(&form, "recipient_phone"),
            address: field(&form, "address"),
            area: field(&form, "area"),
        },
    )
    .await?;
    back_to(&lang, "fnb.details_saved", id)
}

/// `POST /orders/{id}/dispatch`.
async fn dispatch(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    fulfilment::dispatch(&state, &user, id).await.map_err(answer)?;
    back_to(&lang, "fnb.dispatched", id)
}

/// `POST /orders/{id}/complete`.
async fn complete(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    fulfilment::complete(&state, &user, id).await.map_err(answer)?;
    back_to(&lang, "fnb.completed", id)
}

/// `POST /orders/{id}/close`: closes a failed delivery by hand, with the
/// proposed decision.
async fn close_failed(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    close::close_by_user(&state, &user, id).await.map_err(answer)?;
    back_to(&lang, "fnb.closed", id)
}

/// `POST /orders/{id}/responsible`: fields `party`, `note`.
async fn responsible(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    failed::change_party(&state, &user, id, &field(&form, "party"), &field(&form, "note"))
        .await?;
    back_to(&lang, "fnb.party_changed", id)
}

/// `POST /orders/{id}/refund`: fields `amount` (dirhams) and `reason`.
async fn refund(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let Some(amount) = parse_dirhams(&field(&form, "amount")) else {
        return Err(refused(t(&lang, "fnb.refund.amount")));
    };
    refunds::request(&state, &user, id, amount, &field(&form, "reason")).await?;
    back_to(&lang, "fnb.refund_asked", id)
}

/// `POST /orders/{id}/cancel`: fields `reason`, `party`, `note` and
/// `outcome_<line>` for each line the kitchen started.
async fn cancel_order(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let outcomes = form
        .iter()
        .filter_map(|(name, value)| {
            let line = name.strip_prefix("outcome_")?.parse::<i64>().ok()?;
            Some((line, value.clone()))
        })
        .collect();
    let ask = StaffCancel {
        reason_code: field(&form, "reason"),
        party: field(&form, "party"),
        note: field(&form, "note"),
        outcomes,
    };
    match cancel::by_staff(&state, &user, id, ask).await? {
        ChangeOutcome::Done => back_to(&lang, "fnb.order_cancelled", id),
        ChangeOutcome::Waiting { .. } => Ok((
            Toast::info(t(&lang, "fnb.order_cancel_waiting")),
            Redirect::route("fnb.order", &[&id])?,
        )
            .into_response()),
    }
}

/// `POST /orders/{id}/amend`: fields `line`, `new_item`, `quantity`, `price`,
/// `reason`, `customer_agreed`, `call_time`.
async fn amend_order(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let ask = AmendAsk {
        line: field(&form, "line").trim().parse().unwrap_or(0),
        new_item: field(&form, "new_item"),
        quantity: field(&form, "quantity"),
        price: field(&form, "price"),
        reason: field(&form, "reason"),
        customer_agreed: matches!(field(&form, "customer_agreed").as_str(), "on" | "1" | "true"),
        call_time: field(&form, "call_time"),
    };
    match changes::amend(&state, &user, id, ask).await? {
        ChangeOutcome::Done => back_to(&lang, "fnb.order_amended", id),
        ChangeOutcome::Waiting { .. } => Ok((
            Toast::info(t(&lang, "fnb.order_amend_waiting")),
            Redirect::route("fnb.order", &[&id])?,
        )
            .into_response()),
    }
}

/// `POST /orders/{id}/pickup/late`: the aggregator's rider collected the
/// `ready` order late; it is completed.
async fn picked_up_late(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    pickup::picked_up_late(&state, &user, id).await?;
    back_to(&lang, "fnb.picked_up_late", id)
}

/// `POST /orders/{id}/pickup/cancel`: the aggregator cancelled the `ready`
/// order; it is cancelled with the aggregator responsible, the food as
/// wastage, compensation expected and no refund.
async fn aggregator_cancelled(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    cancel::by_aggregator(&state, &user, id).await?;
    back_to(&lang, "fnb.aggregator_cancelled", id)
}

/// `POST /orders/{id}/line-cancel`: fields `line`, `reason`, `note`, `outcome`
/// (when the kitchen started the line).
async fn cancel_line(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let ask = LineAsk {
        line: field(&form, "line").trim().parse().unwrap_or(0),
        reason_code: field(&form, "reason"),
        note: field(&form, "note"),
        outcome: field(&form, "outcome"),
    };
    match changes::cancel_line(&state, &user, id, ask).await? {
        ChangeOutcome::Done => back_to(&lang, "fnb.line_cancelled", id),
        ChangeOutcome::Waiting { .. } => Ok((
            Toast::info(t(&lang, "fnb.line_cancel_waiting")),
            Redirect::route("fnb.order", &[&id])?,
        )
            .into_response()),
    }
}

/// What the customer's page of an order needs.
#[derive(Serialize, ViewData)]
#[view(template = "fnb/my_order.html")]
pub(crate) struct MyOrderPage {
    tx: Texts,
    order: FnbOrder,
    status_label: String,
    status_kind: &'static str,
    lines: Vec<LineRow>,
    can_cancel: bool,
    accepted: bool,
}

/// `GET /my/orders/{id}`: the signed-in customer's own order, with Cancel
/// while it is `placed`; once the outlet accepts it the page says so.
async fn my_order(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    let order = FnbOrder::find_or_404(&state.db, id).await?;
    let customer = cod::customer_of(&state, &user).await?;
    if customer.map(|party| party.0) != Some(order.party_id) {
        return Err(Error::NotFound);
    }
    let lines = match order_desk(&state).find(&state, order.sales_order()).await? {
        Some(view) => view
            .lines
            .iter()
            .filter(|line| line.is_item)
            .map(|line| LineRow {
                description: line.description.clone(),
                quantity: format!("{}", line.quantity_milli / 1_000),
                gross: line.gross,
            })
            .collect(),
        None => Vec::new(),
    };
    Ok(View::typed(MyOrderPage {
        tx: tx(&lang),
        status_label: status_word(&lang, order.status),
        status_kind: status_kind(order.status),
        lines,
        can_cancel: order.status == FnbOrderStatus::Placed,
        accepted: matches!(
            order.status,
            FnbOrderStatus::Accepted | FnbOrderStatus::Preparing | FnbOrderStatus::Ready
        ),
        order,
    })
    .into_response())
}

/// `POST /my/orders/{id}/cancel`: the customer cancels a `placed` order. An
/// order the outlet accepted meanwhile answers 422 "order already accepted"
/// and the page is drawn again.
async fn my_order_cancel(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    cancel::by_customer(&state, &user, id).await?;
    Ok((
        Toast::success(t(&lang, "fnb.my_order_cancelled")),
        Redirect::route("fnb.my_order", &[&id])?,
    )
        .into_response())
}

/// A ticket of the kitchen board.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct BoardRow {
    id: i64,
    order: String,
    order_url: String,
    description: String,
    quantity: String,
    station: String,
    status: String,
    can_start: bool,
}

/// What the kitchen board needs.
#[derive(Serialize, ViewData)]
#[view(template = "fnb/kitchen.html")]
pub(crate) struct KitchenPage {
    tx: Texts,
    tickets: Vec<BoardRow>,
}

async fn kitchen(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<Response> {
    if !user.has_permission(permissions::KDS_WORK) {
        return Err(Error::Forbidden);
    }
    let mut tickets = Vec::new();
    for ticket in KitchenTicket::open(&state.db).await? {
        let order = FnbOrder::find(&state.db, ticket.order_id).await?;
        tickets.push(BoardRow {
            id: ticket.id,
            order: order.as_ref().map(|o| o.number.clone()).unwrap_or_default(),
            // The kitchen screen shows no customer details (`FNB.ACL.01`).
            order_url: if user.has_permission(permissions::ORDERS_VIEW) {
                format!("/orders/{}", ticket.order_id)
            } else {
                String::new()
            },
            description: ticket.description.clone(),
            quantity: format!("{}", ticket.quantity_milli / 1_000),
            station: ticket.station.clone(),
            status: t(&lang, &format!("fnb.screen.ticket.{}", ticket.status.as_str())),
            can_start: ticket.status == TicketStatus::Queued,
        });
    }
    Ok(View::typed(KitchenPage {
        tx: tx(&lang),
        tickets,
    })
    .into_response())
}

/// Runs the ticket transition `name` as the signed-in user and goes back to
/// the board.
async fn ticket_step(
    state: &AppState,
    user: &AuthUser,
    lang: &Lang,
    id: i64,
    name: &str,
    toast: &str,
) -> Result<Response> {
    let ticket = KitchenTicket::find_or_404(&state.db, id).await?;
    let _outcome = ticket
        .transition(name)
        .by(user)
        .run(state)
        .await
        .map_err(answer)?;
    Ok((Toast::success(t(lang, toast)), Redirect::route("fnb.kitchen", &[])?).into_response())
}

/// `POST /tickets/{id}/start`.
async fn ticket_start(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    ticket_step(&state, &user, &lang, id, START, "fnb.ticket_started").await
}

/// `POST /tickets/{id}/bump`.
async fn ticket_bump(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    ticket_step(&state, &user, &lang, id, BUMP, "fnb.ticket_bumped").await
}

/// A customer whose cash on delivery is off.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct CodRow {
    party: i64,
    customer: String,
    off_at: String,
    order: String,
    order_url: String,
}

/// What the list of customers without cash on delivery needs.
#[derive(Serialize, ViewData)]
#[view(template = "fnb/cod_customers.html")]
pub(crate) struct CodPage {
    tx: Texts,
    customers: Vec<CodRow>,
}

async fn cod_customers(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
) -> Result<Response> {
    if !user.has_permission(permissions::CUSTOMERS_COD) {
        return Err(Error::Forbidden);
    }
    let directory = wl_kernel::parties::directory(&state);
    let mut customers = Vec::new();
    for flag in CustomerFlag::turned_off(&state.db).await? {
        let name = directory
            .find(&state.db, PartyId(flag.party_id))
            .await?
            .map(|summary| summary.display_name)
            .unwrap_or_default();
        let order = match flag.cod_off_order_id {
            Some(id) => FnbOrder::find(&state.db, id).await?,
            None => None,
        };
        customers.push(CodRow {
            party: flag.party_id,
            customer: name,
            off_at: flag.cod_off_at.map(|at| at.to_string()).unwrap_or_default(),
            order: order.as_ref().map(|o| o.number.clone()).unwrap_or_default(),
            order_url: order.map(|o| format!("/orders/{}", o.id)).unwrap_or_default(),
        });
    }
    Ok(View::typed(CodPage {
        tx: tx(&lang),
        customers,
    })
    .into_response())
}

/// `POST /customers/{party}/cod`: turns cash on delivery back on.
async fn cod_on(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(party): Path<i64>,
) -> Result<Response> {
    cod::turn_on(&state, &user, PartyId(party)).await?;
    Ok((
        Toast::success(t(&lang, "fnb.cod_on")),
        Redirect::route("fnb.cod_customers", &[])?,
    )
        .into_response())
}

/// What the checkout's payment choice needs.
#[derive(Serialize, ViewData)]
#[view(template = "fnb/checkout_payment.html")]
pub(crate) struct CheckoutPage {
    tx: Texts,
    online: bool,
    cash_on_delivery: bool,
}

/// `GET /checkout/payment`: what the signed-in customer can pay with. With
/// cash on delivery off (`FNB.RUL.14`) only online payment is offered, with the
/// line "Online payment only for this order". Staff taking a phone order give
/// the customer as `?party=`.
async fn checkout_payment(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Query(query): Query<BTreeMap<String, String>>,
) -> Result<Response> {
    let customer = match query.get("party").and_then(|p| p.parse::<i64>().ok()) {
        Some(party) if user.has_permission(permissions::ORDERS_VIEW) => Some(PartyId(party)),
        _ => cod::customer_of(&state, &user).await?,
    };
    let options = match customer {
        Some(party) => cod::payment_options(&state.db, party).await?,
        None => cod::PaymentOptions {
            online: true,
            cash_on_delivery: true,
        },
    };
    Ok(View::typed(CheckoutPage {
        tx: tx(&lang),
        online: options.online,
        cash_on_delivery: options.cash_on_delivery,
    })
    .into_response())
}
