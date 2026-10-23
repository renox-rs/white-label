//! The sales order pages (wl-sales §4.2), each behind its permission.
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /sales-orders` | `wl_sales.sales_orders` | `sales_orders.view` |
//! | `GET /sales-orders/{id}` | `wl_sales.sales_order` | `sales_orders.view` |
//! | `POST /sales-orders/{id}/transitions/{name}` | `wl_sales.sales_order_transitions.` | the transition's own |
//! | `GET /sales-orders?awaiting=1` | `wl_sales.sales_orders` | `sales_orders.view`: the orders waiting for their online payment |
//! | `POST /sales-orders` | `wl_sales.sales_order_create` | `sales_orders.create` (fields `external_number`, `promo_code`) |
//! | `POST /sales-orders/{id}/lines` | `wl_sales.sales_order_line` | `sales_orders.create` |
//! | `POST /sales-orders/{id}/submit` | `wl_sales.sales_order_submit` | `sales_orders.create` |
//! | `POST /sales-orders/{id}/reference` | `wl_sales.sales_order_reference` | `sales_orders.create` |
//! | `POST /sales-orders/{id}/accept` | `wl_sales.sales_order_accept` | `sales_orders.accept` |
//! | `POST /sales-orders/{id}/release` | `wl_sales.sales_order_release` | `credit.release` |
//! | `POST /sales-orders/{id}/cancel` | `wl_sales.sales_order_cancel` | `sales_orders.cancel` (fields `reason`, `line_<id>_outcome`, `line_<id>_fee`, `line_<id>_waive`): the whole order |
//! | `POST /sales-orders/{id}/cancel-line` | `wl_sales.sales_order_cancel_line` | `sales_orders.cancel` (fields `line_no`, `quantity`, `reason`, `outcome`, `fee`, `waive_fee`) |
//! | `POST /sales-orders/{id}/cancel-remaining` | `wl_sales.sales_order_cancel_remaining` | `sales_orders.cancel` (fields `reason`, `line_<id>_outcome`, `line_<id>_fee`, `line_<id>_waive`) |
//! | `POST /sales-orders/{id}/refund` | `wl_sales.sales_order_refund` | `returns.request` (fields `reason`, `category`, `party`, `ticket`, `line_<id>_refund`): a refund after completion with nothing returned (`SAL.ACL.06`) |
//! | `POST /sales-orders/{id}/amend` | `wl_sales.sales_order_amend` | `sales_orders.amend` (fields `revision`, `reason`, `line_<id>_quantity`, `_price`, `_discount`, `add_item`, `add_quantity`, `add_price`) |
//! | `GET /sales-orders/{id}/revisions/{revision}` | `wl_sales.sales_order_revision` | `sales_orders.view`: an earlier revision, read-only |
//! | `POST /sales-orders/{id}/resend-link` | `wl_sales.sales_order_resend_link` | `sales_orders.amend` |
//! | `POST /sales-orders/{id}/amend-back` | `wl_sales.sales_order_amend_back` | `sales_orders.amend` |
//!
//! `reject` runs through the kernel's generic transition route, with the reason
//! the form gives; `accept` has its own route because the credit check of an
//! order on account runs first; `cancel` needs the plan of its outcomes and
//! has the page's own form (BL-SAL-07). The system transitions
//! (`fulfil`, `confirm_payment`...) are not reachable from a page.

use std::collections::BTreeMap;

use renox::prelude::*;
use renox::state_machine::{Available, StateMachine};
use renox::validation::{Errors, ValidationError};
use serde::Serialize;
use wl_kernel::PartyId;
use wl_kernel::invoicing::VatCategory;

use crate::amend::{self, AmendInput, AmendOutcome};
use crate::cancel_lines::{self, CancelOutcome, WantedLine};
use crate::config::config_of;
use crate::notices::aed;
use crate::order_change::{OrderRevision, OrderSettlement, STATUS_EXPIRED};
use crate::pages::campaigns::parse_fils;
use crate::pages::counter_sales::parse_milli;
use crate::sale_line::{KIND_ITEM, NewLine, SaleLine, totals};
use crate::sales_order::{OrderPayment, SalesOrder, SalesOrderStatus};
use crate::sales_order_machine::{
    ACCEPT, ACCEPT_HOLD, RELEASE, RELEASE_APPROVED, RELEASE_PARTLY, RELEASE_PARTLY_APPROVED,
    RELEASE_PENDING, RELEASE_PENDING_APPROVED, SUBMIT, SUBMIT_FOR_ACCEPTANCE, SUBMIT_HOLD,
    SUBMIT_ONLINE,
};
use crate::sales_orders::{self, Imported, NewSalesOrder};
use crate::{permissions, texts};
use wl_kernel::approvals::ApprovalRequest;
use wl_kernel::sales::{LineOutcomeKind, fulfilment_progress};

/// The texts a template reads as `tx['orders.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The payment modes an order can be entered with.
const PAYMENT_MODES: &[&str] = &["on_account", "online", "cash_on_delivery", "cash", "card"];

/// The routes of the pages, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .get("/sales-orders", index)
                .name("wl_sales.sales_orders")
                .get("/sales-orders/{id}", show)
                .name("wl_sales.sales_order")
                .get("/sales-orders/{id}/revisions/{revision}", revision)
                .name("wl_sales.sales_order_revision")
                .group(
                    "/sales-orders",
                    "wl_sales.sales_order_transitions.",
                    wl_kernel::machines::routes::<SalesOrder>(),
                )
                .require_permission(permissions::SALES_ORDERS_VIEW),
        )
        .merge(
            Routes::new()
                .post("/sales-orders", create)
                .name("wl_sales.sales_order_create")
                .post("/sales-orders/{id}/lines", add_line)
                .name("wl_sales.sales_order_line")
                .post("/sales-orders/{id}/submit", submit)
                .name("wl_sales.sales_order_submit")
                .post("/sales-orders/{id}/reference", reference)
                .name("wl_sales.sales_order_reference")
                .require_permission(permissions::SALES_ORDERS_CREATE),
        )
        .merge(
            Routes::new()
                .post("/sales-orders/{id}/accept", accept)
                .name("wl_sales.sales_order_accept")
                .require_permission(permissions::SALES_ORDERS_ACCEPT),
        )
        .merge(
            Routes::new()
                .post("/sales-orders/{id}/release", release)
                .name("wl_sales.sales_order_release")
                .require_permission(permissions::CREDIT_RELEASE),
        )
        .merge(
            Routes::new()
                .post("/sales-orders/{id}/cancel", cancel_whole)
                .name("wl_sales.sales_order_cancel")
                .post("/sales-orders/{id}/cancel-line", cancel_line)
                .name("wl_sales.sales_order_cancel_line")
                .post("/sales-orders/{id}/cancel-remaining", cancel_remaining)
                .name("wl_sales.sales_order_cancel_remaining")
                .require_permission(permissions::SALES_ORDERS_CANCEL),
        )
        .merge(
            Routes::new()
                .post("/sales-orders/{id}/refund", refund_order)
                .name("wl_sales.sales_order_refund")
                .require_permission(permissions::RETURNS_REQUEST),
        )
        .merge(
            Routes::new()
                .post("/sales-orders/{id}/amend", amend_order)
                .name("wl_sales.sales_order_amend")
                .post("/sales-orders/{id}/resend-link", resend_link)
                .name("wl_sales.sales_order_resend_link")
                .post("/sales-orders/{id}/amend-back", amend_back)
                .name("wl_sales.sales_order_amend_back")
                .require_permission(permissions::SALES_ORDERS_AMEND),
        )
}

/// The package text `key` (without the `wl_sales.` prefix).
fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_sales.{key}"))
}

/// `fils` as `AED 4,000.00`.
fn money(fils: i64) -> String {
    format!("AED {}", aed(fils))
}

/// A 422 with `message` under `field`, keeping what was typed.
fn invalid<F: Serialize>(field: &str, message: String, form: &F) -> Error {
    let mut errors = Errors::new();
    errors.add(field, message);
    Error::Validation(ValidationError::new(errors).with_input(form))
}

/// The badge colour of a status.
fn status_kind(status: SalesOrderStatus) -> &'static str {
    use SalesOrderStatus::{
        Cancelled, Closed, Confirmed, CreditHold, Draft, Fulfilled, PartiallyFulfilled, Rejected,
    };
    match status {
        Confirmed | PartiallyFulfilled => "info",
        Fulfilled | Closed => "success",
        CreditHold => "warning",
        Rejected | Cancelled => "error",
        Draft | SalesOrderStatus::AwaitingPayment | SalesOrderStatus::PendingAcceptance => "info",
    }
}

/// One order as the list shows it.
#[derive(Serialize, Debug, Clone)]
pub struct OrderRow {
    /// The primary key.
    pub id: i64,
    /// The order number.
    pub number: String,
    /// The customer's party id.
    pub customer_id: i64,
    /// The customer's name.
    pub customer: String,
    /// Why the order is on credit hold, for a viewer of the credit figures.
    pub hold_reason: String,
    /// The channel.
    pub channel: String,
    /// The total, `AED 105.00`.
    pub total: String,
    /// The status in words.
    pub status_label: String,
    /// The badge colour.
    pub status_kind: &'static str,
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-sales/sales_orders.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    orders: Vec<OrderRow>,
    can_create: bool,
    awaiting: bool,
    awaiting_count: i64,
    /// Only the orders on credit hold are listed.
    held_only: bool,
    /// How many orders are on credit hold.
    held_count: i64,
    /// The viewer sees why an order is held.
    can_see_credit: bool,
}

/// `GET /sales-orders`: the latest orders, newest first. An order waiting for
/// its online payment is not in the list; `?awaiting=1` lists those instead
/// (BL-SAL-02: they are invisible to fulfilment until the payment is
/// confirmed). `?view=credit_hold` lists the orders on credit hold, oldest
/// hold first, for the accountant and the sales rep.
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Query(query): Query<BTreeMap<String, String>>,
) -> Result<View> {
    let db = &state.db;
    let awaiting = matches!(query.get("awaiting").map(String::as_str), Some("1"));
    let held_only = query.get("view").is_some_and(|v| v == "credit_hold");
    let can_see_credit = user.has_permission(permissions::CREDIT_VIEW);
    let mut rows = SalesOrder::query()
        .order_by_desc(SalesOrder::ID)
        .get(db)
        .await?;
    let awaiting_count = rows
        .iter()
        .filter(|o| o.status == SalesOrderStatus::AwaitingPayment)
        .count() as i64;
    let held_count = rows
        .iter()
        .filter(|o| o.status == SalesOrderStatus::CreditHold)
        .count() as i64;
    if held_only {
        rows.retain(|o| o.status == SalesOrderStatus::CreditHold);
        rows.reverse();
    } else {
        rows.retain(|o| (o.status == SalesOrderStatus::AwaitingPayment) == awaiting);
    }
    let mut orders = Vec::new();
    for order in rows.into_iter().take(50) {
        orders.push(OrderRow {
            id: order.id,
            number: order.number.clone(),
            customer_id: order.party_id,
            customer: super::party_name(&state, &lang, order.party_id).await,
            hold_reason: if can_see_credit && order.status == SalesOrderStatus::CreditHold {
                crate::pages::credit::hold_reason_text(&lang, order.hold_reason.as_deref())
            } else {
                String::new()
            },
            channel: order.channel.clone(),
            total: money(order.gross_total),
            status_label: text(&lang, &format!("orders.status.{}", order.status.as_str())),
            status_kind: status_kind(order.status),
        });
    }
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        orders,
        can_create: user.has_permission(permissions::SALES_ORDERS_CREATE),
        awaiting,
        awaiting_count,
        held_only,
        held_count,
        can_see_credit,
    }))
}

/// One line as the page shows it.
#[derive(Serialize, Debug, Clone)]
pub struct LineRow {
    /// The primary key.
    pub id: i64,
    /// The position of the line, from 1.
    pub line_no: i64,
    /// The description.
    pub description: String,
    /// The quantity in units.
    pub quantity: String,
    /// The quantity handed over, in units.
    pub fulfilled: String,
    /// The quantity cancelled, in units.
    pub cancelled: String,
    /// The quantity still open, in units.
    pub open: String,
    /// The quantity held until the payment link of an increase is paid.
    pub held: String,
    /// The price of one unit before VAT, `40.00`, as the amend form takes it.
    pub price: String,
    /// The discount before VAT, `0.00`.
    pub discount: String,
    /// Whether preparation of the line started.
    pub started: bool,
    /// What happened to the started goods of its cancelled quantity.
    pub outcome: String,
    /// The line with VAT, `AED 105.00`.
    pub gross: String,
}

/// The credit hold of an order as its page shows it. The reason and the
/// figures are for a holder of `credit.view`; everyone else sees only that
/// the order is on hold.
#[derive(Serialize, Debug, Clone)]
pub struct HoldView {
    /// The viewer may see the reason and the figures.
    pub detail: bool,
    /// Why, in words.
    pub reason: String,
    /// The state the order returns to, in words.
    pub returns_to: String,
    /// What the customer owes with this order counted.
    pub owes: String,
    /// The overdue balance.
    pub overdue: String,
    /// The limit checked against.
    pub limit: String,
    /// When it was put on hold, `18 Oct 2026`.
    pub held_on: String,
}

/// The release of a held order as its page offers it (`SAL.ACL.03`).
#[derive(Serialize, Debug, Clone, Default)]
pub struct ReleaseView {
    /// The viewer holds `credit.release` and the order is on hold.
    pub shown: bool,
    /// Why the button is disabled, if it is.
    pub blocked: String,
    /// What the sheet says, with the account hold warning when it applies.
    pub description: String,
    /// The customer account is on hold: the sheet offers to ask for the
    /// clearing in the same action.
    pub account_on_hold: bool,
    /// The reasons a clearing can give, as (value, words).
    pub kinds: Vec<(String, String)>,
}

/// The customer PO number of an order as its page shows it.
#[derive(Serialize, Debug, Clone, Default)]
pub struct PoView {
    /// The reference and call-off number in words; empty for none.
    pub reference: String,
    /// The reference as typed, for the edit sheet.
    pub customer_po: String,
    /// The call-off number as typed.
    pub call_off: String,
    /// The existing order that already holds the number: its id, number and
    /// status in words, while the draft cannot be submitted.
    pub clash_id: Option<i64>,
    /// The existing order's number.
    pub clash_number: String,
    /// Why *Submit* is disabled; empty when it is not.
    pub blocked: String,
}

/// One earlier revision as the order page lists it.
#[derive(Serialize, Debug, Clone)]
pub struct RevisionRow {
    /// The revision number.
    pub revision: i64,
    /// Why it was replaced.
    pub reason: String,
    /// Its total, `AED 4,000.00`.
    pub total: String,
    /// When it was replaced.
    pub replaced_on: String,
}

/// One money step of the changes as the order page lists it.
#[derive(Serialize, Debug, Clone)]
pub struct SettlementRow {
    /// The step in words.
    pub kind: String,
    /// The amount, `AED 60.00`.
    pub amount: String,
    /// Where it stands, in words.
    pub status: String,
    /// The public path of a payment link.
    pub link: String,
    /// When a link expires.
    pub due: String,
    /// The note, such as why a step failed.
    pub note: String,
}

/// One online payment handled for the order, as the page shows it.
#[derive(Serialize, Debug, Clone)]
pub struct PaymentRow {
    /// The row id.
    pub id: i64,
    /// The payment id in wl-payments.
    pub payment: i64,
    /// The amount, `AED 85.00`.
    pub amount: String,
    /// What was done with it, in words.
    pub outcome: String,
    /// The refund asked of wl-payments, if any.
    pub refund: String,
}

/// What the order page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-sales/sales_order.html")]
pub(crate) struct ShowPage {
    payment_label: String,
    /// How the order is paid, in words (`cash_on_delivery` is "Cash on delivery").
    payment_mode_label: String,
    payment_due: String,
    awaiting_hint: String,
    payments: Vec<PaymentRow>,
    tx: Texts,
    order: SalesOrder,
    title: String,
    status_label: String,
    status_kind: &'static str,
    net: String,
    vat: String,
    gross: String,
    lines: Vec<LineRow>,
    actions: Vec<Available>,
    member: Option<String>,
    hold: Option<HoldView>,
    po: PoView,
    release: ReleaseView,
    approvals: Vec<wl_kernel::approvals::ApprovalCard>,
    can_edit: bool,
    can_accept: bool,
    can_attach: bool,
    can_cancel_line: bool,
    can_amend: bool,
    amend_blocked: String,
    amend_hint: String,
    can_cancel_remaining: bool,
    can_cancel_order: bool,
    cancel_blocked: String,
    can_refund: bool,
    refund_blocked: String,
    refunded: String,
    category_options: Vec<[String; 2]>,
    party_options: Vec<[String; 2]>,
    remaining_blocked: String,
    can_resolve_link: bool,
    expired_hint: String,
    waiting_alert: String,
    outcome_options: Vec<[String; 2]>,
    outcome_default: String,
    revisions: Vec<RevisionRow>,
    settlements: Vec<SettlementRow>,
    redeem: crate::pages::redeem::RedeemView,
}

/// `1.5` for 1500 thousandths, `2` for 2000.
fn quantity_text(milli: i64) -> String {
    if milli % 1000 == 0 {
        (milli / 1000).to_string()
    } else {
        format!("{}.{:03}", milli / 1000, milli % 1000)
            .trim_end_matches('0')
            .to_owned()
    }
}

/// `GET /sales-orders/{id}`: the order, its lines and the transitions the
/// viewer may run.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(order): Found<SalesOrder>,
) -> Result<View> {
    let lines = SaleLine::of(&state.db, &order.subject()).await?;
    let t = totals(&lines);
    let item_ids: Vec<i64> = lines
        .iter()
        .filter(|l| l.kind == KIND_ITEM)
        .map(|l| l.id)
        .collect();
    let started = fulfilment_progress(&state)
        .started_lines(&state, order.order_id(), &item_ids)
        .await?;
    let rows = lines
        .iter()
        .map(|line| LineRow {
            id: line.id,
            line_no: line.line_no,
            description: line.description.clone(),
            quantity: quantity_text(line.quantity_milli),
            fulfilled: quantity_text(line.fulfilled_milli),
            cancelled: quantity_text(line.cancelled_milli),
            open: quantity_text(line.open_milli()),
            held: quantity_text(line.held_milli),
            price: aed(line.unit_price),
            discount: aed(line.discount),
            started: started.contains(&line.id),
            outcome: line
                .cancel_outcome
                .as_deref()
                .map(|o| text(&lang, &format!("cancelling.outcome.{o}")))
                .unwrap_or_default(),
            gross: money(line.gross()),
        })
        .collect();
    // Submitting takes the page's own button, which reads the lines; the
    // amendments and cancellations of lines take their own forms.
    let actions = order
        .available(&user, &state.db)
        .await?
        .into_iter()
        .filter(|a| {
            ![
                SUBMIT,
                SUBMIT_ONLINE,
                SUBMIT_FOR_ACCEPTANCE,
                SUBMIT_HOLD,
                ACCEPT,
                ACCEPT_HOLD,
                RELEASE,
                RELEASE_PARTLY,
                RELEASE_PENDING,
                RELEASE_APPROVED,
                RELEASE_PARTLY_APPROVED,
                RELEASE_PENDING_APPROVED,
            ]
            .contains(&a.name.as_str())
                && !crate::order_changes::is_change(&a.name)
        })
        .collect();
    let config = config_of(&state);
    let amendable_state = matches!(
        order.status,
        SalesOrderStatus::PendingAcceptance
            | SalesOrderStatus::CreditHold
            | SalesOrderStatus::Confirmed
            | SalesOrderStatus::PartiallyFulfilled
    );
    let holds_amend = user.has_permission(permissions::SALES_ORDERS_AMEND);
    let channel_ok = config.amendments_allowed(&order.channel);
    let waiting = ApprovalRequest::pending_about(&state.db, &order.subject()).await?;
    let revisions = OrderRevision::of_order(&state.db, order.id)
        .await?
        .into_iter()
        .map(|r| RevisionRow {
            revision: r.revision,
            reason: r.reason.clone(),
            total: money(r.gross_total),
            replaced_on: r
                .created_at
                .map(wl_kernel::documents::dubai_text)
                .unwrap_or_default(),
        })
        .collect();
    let steps = OrderSettlement::of_order(&state.db, order.id).await?;
    let link_expired = steps
        .iter()
        .any(|s| s.kind == crate::order_change::SETTLE_LINK && s.status == STATUS_EXPIRED);
    let settlements = steps
        .into_iter()
        .map(|s| SettlementRow {
            kind: text(&lang, &format!("settle.kind.{}", s.kind)),
            amount: money(s.amount),
            status: text(&lang, &format!("settle.status.{}", s.status)),
            link: s.link_path.clone().unwrap_or_default(),
            due: s
                .due_at
                .map(wl_kernel::documents::dubai_text)
                .unwrap_or_default(),
            note: s.note.clone().unwrap_or_default(),
        })
        .collect();
    let outcome_options: Vec<[String; 2]> = config
        .cancel_outcomes
        .iter()
        .filter(|o| LineOutcomeKind::parse(o).is_some())
        .map(|o| [o.clone(), text(&lang, &format!("cancelling.outcome.{o}"))])
        .collect();
    let outcome_default = outcome_options
        .first()
        .map(|o| o[0].clone())
        .unwrap_or_default();
    let handed_over = matches!(
        order.status,
        SalesOrderStatus::PartiallyFulfilled
            | SalesOrderStatus::Fulfilled
            | SalesOrderStatus::Closed
    );
    let member =
        crate::loyalty_account::label_of(&state, order.loyalty_account_id, &lang.locale).await?;
    let redeem = crate::pages::redeem::view_of(
        &state,
        &user,
        &lang,
        &order.subject(),
        order.is_draft(),
        order.loyalty_account_id.is_some(),
    )
    .await?;
    let attachable = !matches!(
        order.status,
        SalesOrderStatus::Fulfilled
            | SalesOrderStatus::Closed
            | SalesOrderStatus::Cancelled
            | SalesOrderStatus::Rejected
    );
    let hold = hold_view(&user, &lang, &order);
    let po = po_view(&state, &lang, &order).await?;
    let release = release_view(&state, &user, &lang, &order).await?;
    let approvals = if order.status == SalesOrderStatus::CreditHold {
        wl_kernel::approvals::cards_for(&state.db, &order.subject(), &user, &lang).await?
    } else {
        Vec::new()
    };
    let payments = OrderPayment::of_order(&state.db, order.id)
        .await?
        .into_iter()
        .map(|p| PaymentRow {
            id: p.id,
            payment: p.payment_id,
            amount: money(p.amount),
            outcome: text(&lang, &format!("orders.outcome.{}", p.outcome)),
            refund: p.refund_id.map(|r| r.to_string()).unwrap_or_default(),
        })
        .collect();
    let payment_due = order
        .payment_due_at
        .map(wl_kernel::documents::dubai_text)
        .unwrap_or_default();
    let awaiting_hint = if order.status == SalesOrderStatus::AwaitingPayment {
        texts::t_with(
            &lang,
            "wl_sales.orders.awaiting_hint",
            &[("due", &payment_due)],
        )
    } else {
        String::new()
    };
    Ok(View::typed(ShowPage {
        payment_mode_label: {
            let words = order.payment_mode.replace('_', " ");
            let mut chars = words.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        },
        payment_label: text(
            &lang,
            &format!("orders.payment_status.{}", order.payment_status.as_str()),
        ),
        payment_due,
        awaiting_hint,
        payments,
        tx: texts::all(&lang),
        hold,
        po,
        release,
        approvals,
        title: order.number.clone(),
        status_label: text(&lang, &format!("orders.status.{}", order.status.as_str())),
        status_kind: status_kind(order.status),
        net: money(t.net),
        vat: money(t.vat),
        gross: money(t.gross),
        lines: rows,
        actions,
        member,
        can_attach: attachable && user.has_permission(permissions::LOYALTY_ENROL),
        redeem,
        can_cancel_line: matches!(
            order.status,
            SalesOrderStatus::Confirmed | SalesOrderStatus::PartiallyFulfilled
        ) && user.has_permission(permissions::SALES_ORDERS_CANCEL),
        can_cancel_remaining: order.status == SalesOrderStatus::PartiallyFulfilled
            && user.has_permission(permissions::SALES_ORDERS_CANCEL),
        can_refund: handed_over && user.has_permission(permissions::RETURNS_REQUEST),
        refund_blocked: if !handed_over
            && user.has_permission(permissions::RETURNS_REQUEST)
            && matches!(order.status, SalesOrderStatus::Confirmed)
        {
            text(&lang, "returns.refund_blocked")
        } else {
            String::new()
        },
        refunded: money(order.refunded_amount),
        category_options: crate::pages::sales_returns::options(
            &lang,
            "returns.category",
            &config.complaint_categories,
        ),
        party_options: crate::pages::sales_returns::options(
            &lang,
            "returns.party",
            &config.responsible_parties,
        ),
        // `cancel` leaves the states before anything is handed over; once it
        // was, the page says why it is not offered.
        can_cancel_order: matches!(
            order.status,
            SalesOrderStatus::Draft
                | SalesOrderStatus::PendingAcceptance
                | SalesOrderStatus::CreditHold
                | SalesOrderStatus::Confirmed
        ) && user.has_permission(permissions::SALES_ORDERS_CANCEL),
        cancel_blocked: if order.status == SalesOrderStatus::PartiallyFulfilled
            && user.has_permission(permissions::SALES_ORDERS_CANCEL)
        {
            text(&lang, "cancelling.order_blocked")
        } else {
            String::new()
        },
        // Nothing handed over yet: the whole order is cancelled instead.
        remaining_blocked: if order.status == SalesOrderStatus::Confirmed
            && user.has_permission(permissions::SALES_ORDERS_CANCEL)
        {
            text(&lang, "cancelling.remaining_blocked")
        } else {
            String::new()
        },
        // `amend` is not offered on an aggregator order: say why instead.
        can_amend: amendable_state && holds_amend && channel_ok,
        amend_blocked: if amendable_state && holds_amend && !channel_ok {
            text(&lang, "amend.aggregator")
        } else {
            String::new()
        },
        amend_hint: text(&lang, "amend.use_cancel_line"),
        can_resolve_link: link_expired && holds_amend,
        expired_hint: if link_expired {
            text(&lang, "settle.expired_hint")
        } else {
            String::new()
        },
        waiting_alert: if waiting.is_empty() {
            String::new()
        } else {
            text(&lang, "changes.waiting_alert")
        },
        outcome_options,
        outcome_default,
        revisions,
        settlements,
        can_edit: order.is_draft() && user.has_permission(permissions::SALES_ORDERS_CREATE),
        can_accept: order.status == SalesOrderStatus::PendingAcceptance
            && user.has_permission(permissions::SALES_ORDERS_ACCEPT),
        order,
    }))
}

/// What the release button of `order` shows `user`: nothing for someone
/// without `credit.release`, a disabled button with the reason for the
/// order's creator or while a release waits for approval, else the sheet,
/// which warns when the customer account is on hold.
async fn release_view(
    state: &AppState,
    user: &AuthUser,
    lang: &Lang,
    order: &SalesOrder,
) -> Result<ReleaseView> {
    if order.status != SalesOrderStatus::CreditHold
        || !user.has_permission(permissions::CREDIT_RELEASE)
    {
        return Ok(ReleaseView::default());
    }
    let waiting = !wl_kernel::ApprovalRequest::pending_about(&state.db, &order.subject())
        .await?
        .iter()
        .all(|request| request.action != crate::policy::RELEASE_ACTION);
    let blocked = if order.created_by == Some(user.id) {
        text(lang, "credit.release_creator")
    } else if waiting {
        text(lang, "credit.release_waiting")
    } else {
        String::new()
    };
    let on_hold = match crate::customer_account::CustomerAccount::of_party(
        &state.db,
        order.party_id,
    )
    .await?
    {
        Some(account) => account.is_on_hold(&state.db).await?,
        None => false,
    };
    let mut description = text(lang, "credit.release_description");
    if on_hold {
        description = format!(
            "{} {}",
            text(lang, "credit.release_account_on_hold"),
            description
        );
    }
    let kinds = crate::account_hold_clearing::KINDS
        .iter()
        .map(|kind| {
            (
                (*kind).to_owned(),
                text(lang, &format!("credit.kind.{kind}")),
            )
        })
        .collect();
    Ok(ReleaseView {
        shown: true,
        blocked,
        description,
        account_on_hold: on_hold,
        kinds,
    })
}

/// `POST /sales-orders/{id}/release`: a credit releaser asks to release the
/// held order; another one approves it.
async fn release(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(order): Found<SalesOrder>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let get = |key: &str| form.get(key).map_or("", |v| v.trim());
    // The same action may ask to clear the account hold too; its reason is
    // checked before anything is requested.
    let clear = matches!(get("clear_hold"), "1" | "on");
    if clear {
        if !crate::account_hold_clearing::KINDS.contains(&get("kind")) {
            return Err(invalid("kind", text(&lang, "orders.errors.choice"), &form));
        }
        if get("kind") == crate::account_hold_clearing::KIND_OTHER && get("clear_note").is_empty() {
            return Err(invalid(
                "clear_note",
                text(&lang, "orders.errors.reason"),
                &form,
            ));
        }
    }
    let (_, done) = sales_orders::release(&state, &user, order.id, get("note")).await?;
    if clear
        && user.has_permission(permissions::CREDIT_RELEASE)
        && let Some(account) =
            crate::customer_account::CustomerAccount::of_party(&state.db, order.party_id).await?
    {
        for source in account.open_sources(&state.db).await? {
            // A source whose clearing already waits is left alone.
            match crate::account_hold::request_clearing(
                &state,
                user.id,
                account.id,
                source.id,
                get("kind"),
                get("clear_note"),
            )
            .await
            {
                Ok(_) | Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _)) => {}
                Err(error) => return Err(error),
            }
        }
    }
    let key = if done {
        "credit.released"
    } else {
        "credit.release_requested"
    };
    Ok((
        Toast::success(text(&lang, key)),
        Redirect::route("wl_sales.sales_order", &[&order.id])?,
    )
        .into_response())
}

/// The PO number of `order` for its page, and the order that clashes with it.
async fn po_view(state: &AppState, lang: &Lang, order: &SalesOrder) -> Result<PoView> {
    let customer_po = order.customer_po.clone().unwrap_or_default();
    let call_off = order.call_off.clone().unwrap_or_default();
    let reference = match (customer_po.is_empty(), call_off.is_empty()) {
        (true, _) => String::new(),
        (false, true) => customer_po.clone(),
        (false, false) => texts::t_with(
            lang,
            "wl_sales.po.with_call_off",
            &[("po", &customer_po), ("call_off", &call_off)],
        ),
    };
    let clash = crate::po_number::clash_of(state, order).await?;
    let (clash_id, clash_number, blocked) = match &clash {
        Some(other) => {
            let status = text(lang, &format!("orders.status.{}", other.status.as_str()));
            (
                Some(other.id),
                other.number.clone(),
                texts::t_with(
                    lang,
                    "wl_sales.po.submit_blocked",
                    &[("number", &other.number), ("status", &status)],
                ),
            )
        }
        None => (None, String::new(), String::new()),
    };
    Ok(PoView {
        reference,
        customer_po,
        call_off,
        clash_id,
        clash_number,
        blocked,
    })
}

/// `POST /sales-orders/{id}/reference`: changes the customer PO number and
/// call-off number of the draft order (`customer_po`, `call_off`).
async fn reference(
    State(state): State<AppState>,
    lang: Lang,
    Found(order): Found<SalesOrder>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let get = |key: &str| form.get(key).map_or("", |v| v.trim());
    crate::po_number::set_reference(&state, order.id, get("customer_po"), get("call_off")).await?;
    Ok((
        Toast::success(text(&lang, "po.saved")),
        Redirect::route("wl_sales.sales_order", &[&order.id])?,
    )
        .into_response())
}

/// The credit hold card of `order`, if it is on credit hold.
fn hold_view(user: &AuthUser, lang: &Lang, order: &SalesOrder) -> Option<HoldView> {
    if order.status != SalesOrderStatus::CreditHold {
        return None;
    }
    Some(HoldView {
        detail: user.has_permission(permissions::CREDIT_VIEW),
        reason: crate::pages::credit::hold_reason_text(lang, order.hold_reason.as_deref()),
        returns_to: text(
            lang,
            &format!(
                "orders.status.{}",
                order.held_from.as_deref().unwrap_or("confirmed")
            ),
        ),
        owes: money(order.hold_open_balance.unwrap_or_default()),
        overdue: money(order.hold_overdue.unwrap_or_default()),
        limit: order
            .hold_limit
            .map_or_else(|| text(lang, "credit.no_limit"), money),
        held_on: order
            .held_at
            .map(|at| at.format("%-d %b %Y").to_string())
            .unwrap_or_default(),
    })
}

/// `POST /sales-orders`: enters a draft order for the customer `party_id`.
async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let get = |key: &str| form.get(key).map_or("", |v| v.trim());
    let party: i64 = get("party_id")
        .parse()
        .ok()
        .filter(|p| *p > 0)
        .ok_or_else(|| invalid("party_id", text(&lang, "orders.errors.number"), &form))?;
    let mut new = NewSalesOrder::new(PartyId(party));
    if !get("channel").is_empty() {
        new = new.channel(&get("channel").to_lowercase());
    }
    if !get("payment_mode").is_empty() {
        if !PAYMENT_MODES.contains(&get("payment_mode")) {
            return Err(invalid(
                "payment_mode",
                text(&lang, "orders.errors.choice"),
                &form,
            ));
        }
        new = new.payment_mode(get("payment_mode"));
    }
    new = new
        .customer_po(get("customer_po"))
        .call_off(get("call_off"));
    if !get("external_number").is_empty() {
        new = new.external_number(get("external_number"));
    }
    if !get("promo_code").is_empty() {
        new = new.promo_code(get("promo_code"));
    }
    match sales_orders::import(&state, Some(user.id), new).await? {
        Imported::Created(order) => Ok((
            Toast::success(text(&lang, "orders.created")),
            Redirect::route("wl_sales.sales_order", &[&order.id])?,
        )
            .into_response()),
        // The channel sent that number before: say which order it is.
        Imported::Duplicate { id, number } => Err(invalid(
            "external_number",
            texts::t_with(
                &lang,
                "wl_sales.orders.problem.duplicate_external",
                &[("number", &number), ("id", &id)],
            ),
            &form,
        )),
    }
}

/// `POST /sales-orders/{id}/lines`: adds a line to the draft order.
async fn add_line(
    State(state): State<AppState>,
    lang: Lang,
    Found(order): Found<SalesOrder>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let get = |key: &str| form.get(key).map_or("", |v| v.trim());
    if get("item_code").is_empty() {
        return Err(invalid(
            "item_code",
            text(&lang, "orders.errors.item"),
            &form,
        ));
    }
    let quantity = match get("quantity") {
        "" => 1000,
        v => parse_milli(v)
            .filter(|q| *q > 0)
            .ok_or_else(|| invalid("quantity", text(&lang, "orders.errors.number"), &form))?,
    };
    let price = parse_fils(get("unit_price"))
        .filter(|p| *p >= 0)
        .ok_or_else(|| invalid("unit_price", text(&lang, "orders.errors.amount"), &form))?;
    let category = match get("vat") {
        "" | "standard" => VatCategory::Standard,
        "zero_rated" => VatCategory::ZeroRated,
        "exempt" => VatCategory::Exempt,
        _ => return Err(invalid("vat", text(&lang, "orders.errors.choice"), &form)),
    };
    let mut line = NewLine::new(get("item_code"), price)
        .quantity_milli(quantity)
        .vat_category(category);
    if !get("description").is_empty() {
        line = line.description(get("description"));
    }
    if matches!(get("loyalty_excluded"), "1" | "on") {
        line = line.loyalty_excluded();
    }
    sales_orders::add_line(&state, order.id, line).await?;
    Ok((
        Toast::success(text(&lang, "orders.line_added")),
        Redirect::route("wl_sales.sales_order", &[&order.id])?,
    )
        .into_response())
}

/// `POST /sales-orders/{id}/submit`: places the order.
async fn submit(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(order): Found<SalesOrder>,
) -> Result<Response> {
    sales_orders::submit(&state, &user, order.id).await?;
    Ok((
        Toast::success(text(&lang, "orders.submitted")),
        Redirect::route("wl_sales.sales_order", &[&order.id])?,
    )
        .into_response())
}

/// `POST /sales-orders/{id}/accept`: the seller accepts the order; an order
/// on account whose customer fails the credit check goes on credit hold.
async fn accept(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(order): Found<SalesOrder>,
) -> Result<Response> {
    let accepted = sales_orders::accept(&state, &user, order.id).await?;
    let key = if accepted.status == SalesOrderStatus::CreditHold {
        "orders.accepted_held"
    } else {
        "orders.accepted"
    };
    Ok((
        Toast::success(text(&lang, key)),
        Redirect::route("wl_sales.sales_order", &[&order.id])?,
    )
        .into_response())
}

/// The outcome the form named, `None` when it named none.
fn outcome_field(
    lang: &Lang,
    form: &BTreeMap<String, String>,
    key: &str,
) -> std::result::Result<Option<LineOutcomeKind>, Error> {
    let value = form.get(key).map_or("", |v| v.trim());
    if value.is_empty() {
        return Ok(None);
    }
    LineOutcomeKind::parse(value)
        .map(Some)
        .ok_or_else(|| invalid(key, text(lang, "orders.errors.choice"), form))
}

/// The fee the form gave in AED, `None` when it gave none.
fn fee_field(
    lang: &Lang,
    form: &BTreeMap<String, String>,
    key: &str,
) -> std::result::Result<Option<i64>, Error> {
    let value = form.get(key).map_or("", |v| v.trim());
    if value.is_empty() {
        return Ok(None);
    }
    parse_fils(value)
        .filter(|fee| *fee >= 0)
        .map(Some)
        .ok_or_else(|| invalid(key, text(lang, "orders.errors.amount"), form))
}

/// Whether a checkbox of the form is ticked.
fn ticked(form: &BTreeMap<String, String>, key: &str) -> bool {
    matches!(form.get(key).map(String::as_str), Some("1" | "on" | "true"))
}

/// The toast and redirect that answer a cancellation.
fn cancel_answer(lang: &Lang, order: i64, outcome: &CancelOutcome, done: &str) -> Result<Response> {
    let toast = match outcome {
        CancelOutcome::Done(_) => Toast::success(text(lang, done)),
        CancelOutcome::Waiting(_) => Toast::info(text(lang, "cancelling.waiting")),
    };
    Ok((toast, Redirect::route("wl_sales.sales_order", &[&order])?).into_response())
}

/// `POST /sales-orders/{id}/cancel`: cancels the whole order. Fields: `reason`
/// and, per line whose preparation started, `line_<id>_outcome`,
/// `line_<id>_fee` (AED) and `line_<id>_waive`. A started line, or a paid
/// order above the limit, waits for a manager.
async fn cancel_whole(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(order): Found<SalesOrder>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let reason = form.get("reason").map_or("", |v| v.trim());
    if reason.is_empty() {
        return Err(invalid(
            "reason",
            text(&lang, "orders.errors.reason"),
            &form,
        ));
    }
    let lines = SaleLine::of(&state.db, &order.subject()).await?;
    let mut choices = Vec::new();
    for line in lines.iter().filter(|l| l.kind == KIND_ITEM) {
        choices.push(WantedLine {
            line: line.id,
            outcome: outcome_field(&lang, &form, &format!("line_{}_outcome", line.id))?,
            fee: fee_field(&lang, &form, &format!("line_{}_fee", line.id))?,
            waive_fee: ticked(&form, &format!("line_{}_waive", line.id)),
            ..WantedLine::default()
        });
    }
    let outcome = cancel_lines::cancel_order(&state, &user, order.id, reason, choices).await?;
    cancel_answer(&lang, order.id, &outcome, "cancelling.order_done")
}

/// `POST /sales-orders/{id}/refund`: raises a *no goods* return from the
/// completed order and resolves it with a refund (`SAL.ACL.06`). Fields:
/// `reason`, `category`, `party`, `ticket` (optional) and, per line,
/// `line_<id>_refund` (AED with VAT). Above the limit a manager approves.
async fn refund_order(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(order): Found<SalesOrder>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let get = |key: &str| form.get(key).map_or("", |v| v.trim());
    if get("category").is_empty() {
        return Err(invalid(
            "category",
            text(&lang, "returns.problem.complaint"),
            &form,
        ));
    }
    if get("party").is_empty() {
        return Err(invalid(
            "party",
            text(&lang, "returns.problem.complaint"),
            &form,
        ));
    }
    let mut new = crate::sales_returns::NewReturn::no_goods(order.subject())
        .reason(get("reason"))
        .complaint(get("category"), get("party"));
    if !get("ticket").is_empty() {
        let ticket: i64 = get("ticket")
            .parse()
            .map_err(|_| invalid("ticket", text(&lang, "orders.errors.number"), &form))?;
        new = new.ticket(ticket);
    }
    let lines = SaleLine::of(&state.db, &order.subject()).await?;
    for line in lines.iter().filter(|l| l.kind == KIND_ITEM) {
        let key = format!("line_{}_refund", line.id);
        if get(&key).is_empty() {
            continue;
        }
        let amount = parse_fils(get(&key))
            .filter(|a| *a > 0)
            .ok_or_else(|| invalid(&key, text(&lang, "orders.errors.amount"), &form))?;
        new = new.refund_line(line.id, amount);
    }
    let (toast, to) =
        match crate::sales_returns::refund_after_completion(&state, &user, new).await? {
            crate::sales_returns::RefundOutcome::Resolved(done) => {
                (Toast::success(text(&lang, "returns.refund_done")), done.id)
            }
            crate::sales_returns::RefundOutcome::Waiting(created, _) => (
                Toast::info(text(&lang, "returns.refund_waiting")),
                created.id,
            ),
            crate::sales_returns::RefundOutcome::Requested(created) => (
                Toast::info(text(&lang, "returns.refund_recorded")),
                created.id,
            ),
        };
    Ok((toast, Redirect::route("wl_sales.sales_return", &[&to])?).into_response())
}

/// `POST /sales-orders/{id}/cancel-line`: cancels open quantity of one line.
/// Fields: `line_no` (the position of the line), `quantity` (units),
/// `reason`, and for a line whose preparation started `outcome`, `fee`
/// (AED) and `waive_fee`. A started line waits for a manager.
async fn cancel_line(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(order): Found<SalesOrder>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let get = |key: &str| form.get(key).map_or("", |v| v.trim());
    let line_no: i64 = get("line_no")
        .parse()
        .map_err(|_| invalid("line_no", text(&lang, "orders.errors.number"), &form))?;
    let quantity = parse_milli(get("quantity"))
        .filter(|q| *q > 0)
        .ok_or_else(|| invalid("quantity", text(&lang, "orders.errors.number"), &form))?;
    if get("reason").is_empty() {
        return Err(invalid(
            "reason",
            text(&lang, "orders.errors.reason"),
            &form,
        ));
    }
    let lines = SaleLine::of(&state.db, &order.subject()).await?;
    let line = lines
        .iter()
        .find(|l| l.line_no == line_no)
        .ok_or_else(|| invalid("line_no", text(&lang, "orders.problem.unknown_line"), &form))?;
    let choice = WantedLine {
        outcome: outcome_field(&lang, &form, "outcome")?,
        fee: fee_field(&lang, &form, "fee")?,
        waive_fee: ticked(&form, "waive_fee"),
        ..WantedLine::default()
    };
    let outcome = cancel_lines::cancel_line(
        &state,
        &user,
        order.id,
        line.id,
        quantity,
        get("reason"),
        Some(choice),
    )
    .await?;
    cancel_answer(&lang, order.id, &outcome, "cancelling.line_cancelled")
}

/// `POST /sales-orders/{id}/cancel-remaining`: cancels every open line of a
/// partly fulfilled order. Fields: `reason` and, per line,
/// `line_<id>_outcome`, `line_<id>_fee` (AED) and `line_<id>_waive`.
async fn cancel_remaining(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(order): Found<SalesOrder>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let reason = form.get("reason").map_or("", |v| v.trim());
    if reason.is_empty() {
        return Err(invalid(
            "reason",
            text(&lang, "orders.errors.reason"),
            &form,
        ));
    }
    let lines = SaleLine::of(&state.db, &order.subject()).await?;
    let mut choices = Vec::new();
    for line in lines.iter().filter(|l| l.kind == KIND_ITEM) {
        choices.push(WantedLine {
            line: line.id,
            outcome: outcome_field(&lang, &form, &format!("line_{}_outcome", line.id))?,
            fee: fee_field(&lang, &form, &format!("line_{}_fee", line.id))?,
            waive_fee: ticked(&form, &format!("line_{}_waive", line.id)),
            ..WantedLine::default()
        });
    }
    let outcome = cancel_lines::cancel_remaining(&state, &user, order.id, reason, choices).await?;
    cancel_answer(&lang, order.id, &outcome, "cancelling.remaining_done")
}

/// `POST /sales-orders/{id}/amend`: amends the order as a new revision.
/// Fields: `revision` (the one the form was opened on), `reason`, per item
/// line `line_<id>_quantity` (units), `line_<id>_price` and
/// `line_<id>_discount` (AED), and optionally a line to add: `add_item`,
/// `add_quantity`, `add_price`.
async fn amend_order(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(order): Found<SalesOrder>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let get = |key: &str| form.get(key).map_or("", |v| v.trim());
    // A blank reason is refused by the amendment itself (422).
    let mut input = AmendInput::new(get("reason")).seen_at(get("revision").parse().unwrap_or(0));
    let lines = SaleLine::of(&state.db, &order.subject()).await?;
    for line in lines.iter().filter(|l| l.kind == KIND_ITEM) {
        let quantity_key = format!("line_{}_quantity", line.id);
        let price_key = format!("line_{}_price", line.id);
        let discount_key = format!("line_{}_discount", line.id);
        let quantity = match get(&quantity_key) {
            "" => line.quantity_milli,
            v => parse_milli(v).ok_or_else(|| {
                invalid(&quantity_key, text(&lang, "orders.errors.number"), &form)
            })?,
        };
        let price = match get(&price_key) {
            "" => line.unit_price,
            v => parse_fils(v)
                .ok_or_else(|| invalid(&price_key, text(&lang, "orders.errors.amount"), &form))?,
        };
        let discount = match get(&discount_key) {
            "" => line.discount,
            v => parse_fils(v).ok_or_else(|| {
                invalid(&discount_key, text(&lang, "orders.errors.amount"), &form)
            })?,
        };
        input = input.line(line.id, quantity, price, discount);
    }
    if !get("add_item").is_empty() {
        let quantity = match get("add_quantity") {
            "" => 1000,
            v => parse_milli(v).filter(|q| *q > 0).ok_or_else(|| {
                invalid("add_quantity", text(&lang, "orders.errors.number"), &form)
            })?,
        };
        let price = parse_fils(get("add_price"))
            .filter(|p| *p >= 0)
            .ok_or_else(|| invalid("add_price", text(&lang, "orders.errors.amount"), &form))?;
        input = input.add(get("add_item"), quantity, price);
    }
    let toast = match amend::amend(&state, &user, order.id, input).await? {
        AmendOutcome::Amended(done) => Toast::success(texts::t_with(
            &lang,
            "wl_sales.amend.done",
            &[("revision", &done.revision)],
        )),
        AmendOutcome::Waiting(_) => Toast::info(text(&lang, "amend.waiting")),
    };
    Ok((
        toast,
        Redirect::route("wl_sales.sales_order", &[&order.id])?,
    )
        .into_response())
}

/// `POST /sales-orders/{id}/resend-link`: a new payment link for the increase
/// whose first link expired unpaid.
async fn resend_link(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(order): Found<SalesOrder>,
) -> Result<Response> {
    crate::settle::resend_link(&state, &user, order.id).await?;
    Ok((
        Toast::success(text(&lang, "settle.resent")),
        Redirect::route("wl_sales.sales_order", &[&order.id])?,
    )
        .into_response())
}

/// `POST /sales-orders/{id}/amend-back`: a new revision without the increase
/// whose payment link expired unpaid, with no money effect.
async fn amend_back(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(order): Found<SalesOrder>,
) -> Result<Response> {
    let toast = match crate::settle::amend_back(&state, &user, order.id).await? {
        AmendOutcome::Amended(_) => Toast::success(text(&lang, "settle.amended_back")),
        AmendOutcome::Waiting(_) => Toast::info(text(&lang, "amend.waiting")),
    };
    Ok((
        toast,
        Redirect::route("wl_sales.sales_order", &[&order.id])?,
    )
        .into_response())
}

/// One line of an earlier revision as its page shows it.
#[derive(Serialize, Debug, Clone)]
pub struct RevisionLineRow {
    /// The position.
    pub line_no: i64,
    /// The description.
    pub description: String,
    /// The quantity in units.
    pub quantity: String,
    /// The price of one unit before VAT.
    pub price: String,
    /// The discount before VAT.
    pub discount: String,
    /// The line with VAT.
    pub gross: String,
}

/// What the page of an earlier revision needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-sales/sales_order_revision.html")]
pub(crate) struct RevisionPage {
    tx: Texts,
    title: String,
    order_id: i64,
    revision: i64,
    reason: String,
    total: String,
    replaced_on: String,
    lines: Vec<RevisionLineRow>,
}

/// `GET /sales-orders/{id}/revisions/{revision}`: an earlier revision, kept
/// read-only (nothing on the page changes it).
async fn revision(
    State(state): State<AppState>,
    lang: Lang,
    Path((id, number)): Path<(i64, i64)>,
) -> Result<View> {
    let order = SalesOrder::find_or_404(&state.db, id).await?;
    let Some(kept) = OrderRevision::find_of(&state.db, order.id, number).await? else {
        return Err(Error::NotFound);
    };
    let lines = kept
        .lines()
        .into_iter()
        .map(|l| RevisionLineRow {
            line_no: l.line_no,
            description: l.description.clone(),
            quantity: quantity_text(l.quantity_milli),
            price: aed(l.unit_price),
            discount: aed(l.discount),
            gross: money(l.gross),
        })
        .collect();
    Ok(View::typed(RevisionPage {
        tx: texts::all(&lang),
        title: texts::t_with(
            &lang,
            "wl_sales.amend.revision_title",
            &[("number", &order.number), ("revision", &number)],
        ),
        order_id: order.id,
        revision: number,
        reason: kept.reason.clone(),
        total: money(kept.gross_total),
        replaced_on: kept
            .created_at
            .map(wl_kernel::documents::dubai_text)
            .unwrap_or_default(),
        lines,
    }))
}
