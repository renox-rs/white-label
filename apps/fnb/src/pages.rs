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
//! | `GET /kitchen` | `fnb.kitchen` | `kds.work` |
//! | `POST /tickets/{id}/start` | `fnb.ticket_start` | `kds.work` |
//! | `POST /tickets/{id}/bump` | `fnb.ticket_bump` | `kds.work` |
//! | `GET /customers/cod` | `fnb.cod_customers` | `customers.cod` |
//! | `POST /customers/{party}/cod` | `fnb.cod_on` | `customers.cod` |
//! | `GET /checkout/payment` | `fnb.checkout_payment` | signed in (the customer; staff give `party`) |
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
use wl_kernel::sales::order_desk;

use crate::cod::{self, CustomerFlag};
use crate::money::parse_dirhams;
use crate::order::{FnbOrder, FnbOrderStatus, kind, party};
use crate::permissions;
use crate::placing::{Details, refused};
use crate::refunds::{self, RefundAsk};
use crate::screen_texts::SCREEN_TEXTS;
use crate::texts::t;
use crate::ticket::{BUMP, KitchenTicket, START, TicketStatus};
use crate::{close, failed, fulfilment, placing};

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
];

/// The texts a template reads as `tx['title']`.
pub(crate) type Texts = BTreeMap<&'static str, String>;

const PREFIX: &str = "fnb.screen.";

/// Every text of the screens, by the name a template reads.
pub(crate) fn tx(lang: &Lang) -> Texts {
    SCREEN_TEXTS
        .iter()
        .filter_map(|(key, _)| Some((key.strip_prefix(PREFIX)?, t(lang, key))))
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
            failure: o
                .failure_reason
                .as_deref()
                .map(|reason| t(&lang, &format!("fnb.screen.reason.{reason}")))
                .unwrap_or_default(),
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
    parties: Vec<[String; 2]>,
    kinds: Vec<[String; 2]>,
    notes: Vec<String>,
    refund_left: i64,
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
