//! The sales order pages (wl-sales §4.2), each behind its permission.
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /sales-orders` | `wl_sales.sales_orders` | `sales_orders.view` |
//! | `GET /sales-orders/{id}` | `wl_sales.sales_order` | `sales_orders.view` |
//! | `POST /sales-orders/{id}/transitions/{name}` | `wl_sales.sales_order_transitions.` | the transition's own |
//! | `POST /sales-orders` | `wl_sales.sales_order_create` | `sales_orders.create` |
//! | `POST /sales-orders/{id}/lines` | `wl_sales.sales_order_line` | `sales_orders.create` |
//! | `POST /sales-orders/{id}/submit` | `wl_sales.sales_order_submit` | `sales_orders.create` |
//! | `POST /sales-orders/{id}/cancel-line` | `wl_sales.sales_order_cancel_line` | `sales_orders.cancel` |
//! | `POST /sales-orders/{id}/accept` | `wl_sales.sales_order_accept` | `sales_orders.accept` |
//! | `POST /sales-orders/{id}/release` | `wl_sales.sales_order_release` | `credit.release` |
//!
//! `reject` and `cancel` run through the kernel's generic transition route,
//! with the reason the form gives; `accept` has its own route because the
//! credit check of an order on account runs first. The system transitions
//! (`fulfil`, `confirm_payment`...) are not reachable from a page.

use std::collections::BTreeMap;

use renox::prelude::*;
use renox::state_machine::{Available, StateMachine};
use renox::validation::{Errors, ValidationError};
use serde::Serialize;
use wl_kernel::PartyId;
use wl_kernel::invoicing::VatCategory;

use crate::notices::aed;
use crate::pages::campaigns::parse_fils;
use crate::pages::counter_sales::parse_milli;
use crate::sale_line::{NewLine, SaleLine, totals};
use crate::sales_order::{SalesOrder, SalesOrderStatus};
use crate::sales_order_machine::{
    ACCEPT, ACCEPT_HOLD, CANCEL_LINE, CANCEL_LINE_PARTLY, RELEASE, RELEASE_APPROVED,
    RELEASE_PARTLY, RELEASE_PARTLY_APPROVED, SUBMIT, SUBMIT_FOR_ACCEPTANCE, SUBMIT_HOLD,
    SUBMIT_ONLINE,
};
use crate::sales_orders::{self, NewSalesOrder};
use crate::{permissions, texts};

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
                .post("/sales-orders/{id}/cancel-line", cancel_line)
                .name("wl_sales.sales_order_cancel_line")
                .require_permission(permissions::SALES_ORDERS_CANCEL),
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
    pub customer: i64,
    /// The customer's name.
    pub customer_name: String,
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
    /// Only the orders on credit hold are listed.
    held_only: bool,
    /// How many orders are on credit hold.
    held_count: i64,
    /// The viewer sees why an order is held.
    can_see_credit: bool,
}

/// `GET /sales-orders`: the latest orders, newest first; `?view=credit_hold`
/// lists the orders on credit hold, oldest hold first, for the accountant and
/// the sales rep.
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Query(query): Query<BTreeMap<String, String>>,
) -> Result<View> {
    let held_only = query.get("view").is_some_and(|v| v == "credit_hold");
    let can_see_credit = user.has_permission(permissions::CREDIT_VIEW);
    let mut find = SalesOrder::query();
    find = if held_only {
        find.where_(SalesOrder::STATUS.eq(SalesOrderStatus::CreditHold))
            .order_by(SalesOrder::ID)
    } else {
        find.order_by_desc(SalesOrder::ID)
    };
    let mut rows = Vec::new();
    for order in find.get(&state.db).await?.into_iter().take(50) {
        rows.push(OrderRow {
            id: order.id,
            number: order.number.clone(),
            customer: order.party_id,
            customer_name: crate::pages::party_name(&state, &lang, order.party_id).await,
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
    let held_count = SalesOrder::query()
        .where_(SalesOrder::STATUS.eq(SalesOrderStatus::CreditHold))
        .count(&state.db)
        .await?;
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        orders: rows,
        can_create: user.has_permission(permissions::SALES_ORDERS_CREATE),
        held_only,
        held_count: i64::try_from(held_count).unwrap_or(i64::MAX),
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
}

/// What the order page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-sales/sales_order.html")]
pub(crate) struct ShowPage {
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
    release: ReleaseView,
    approvals: Vec<wl_kernel::approvals::ApprovalCard>,
    can_edit: bool,
    can_accept: bool,
    can_attach: bool,
    can_cancel_line: bool,
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
    let rows = lines
        .iter()
        .map(|line| LineRow {
            id: line.id,
            line_no: line.line_no,
            description: line.description.clone(),
            quantity: quantity_text(line.quantity_milli),
            fulfilled: quantity_text(line.fulfilled_milli),
            gross: money(line.gross()),
        })
        .collect();
    // Submitting takes the page's own button, which reads the lines.
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
                RELEASE_APPROVED,
                RELEASE_PARTLY_APPROVED,
                CANCEL_LINE,
                CANCEL_LINE_PARTLY,
            ]
            .contains(&a.name.as_str())
        })
        .collect();
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
    let release = release_view(&state, &user, &lang, &order).await?;
    let approvals = if order.status == SalesOrderStatus::CreditHold {
        wl_kernel::approvals::cards_for(&state.db, &order.subject(), &user, &lang).await?
    } else {
        Vec::new()
    };
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        hold,
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
    Ok(ReleaseView {
        shown: true,
        blocked,
        description,
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
    let note = form.get("note").map_or("", |v| v.trim());
    let (_, done) = sales_orders::release(&state, &user, order.id, note).await?;
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
                order.state_before_hold.as_deref().unwrap_or("confirmed")
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
    let order = sales_orders::create(&state, Some(user.id), new).await?;
    Ok((
        Toast::success(text(&lang, "orders.created")),
        Redirect::route("wl_sales.sales_order", &[&order.id])?,
    )
        .into_response())
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

/// `POST /sales-orders/{id}/cancel-line`: cancels open quantity of one line.
/// Fields: `line_no` (the position of the line), `quantity` (units) and
/// `reason`.
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
    sales_orders::cancel_line(&state, &user, order.id, line.id, quantity, get("reason")).await?;
    Ok((
        Toast::success(text(&lang, "orders.line_cancelled")),
        Redirect::route("wl_sales.sales_order", &[&order.id])?,
    )
        .into_response())
}
