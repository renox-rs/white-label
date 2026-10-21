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
//!
//! `accept`, `reject` and `cancel` run through the kernel's generic
//! transition route, with the reason the form gives. The system transitions
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
use crate::sales_order_machine::{SUBMIT, SUBMIT_FOR_ACCEPTANCE, SUBMIT_ONLINE};
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
}

/// `GET /sales-orders`: the latest orders, newest first.
async fn index(State(db): State<Db>, user: AuthUser, lang: Lang) -> Result<View> {
    let orders = SalesOrder::query()
        .order_by_desc(SalesOrder::ID)
        .get(&db)
        .await?
        .into_iter()
        .take(50)
        .map(|order| OrderRow {
            id: order.id,
            number: order.number.clone(),
            customer: order.party_id,
            channel: order.channel.clone(),
            total: money(order.gross_total),
            status_label: text(&lang, &format!("orders.status.{}", order.status.as_str())),
            status_kind: status_kind(order.status),
        })
        .collect();
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        orders,
        can_create: user.has_permission(permissions::SALES_ORDERS_CREATE),
    }))
}

/// One line as the page shows it.
#[derive(Serialize, Debug, Clone)]
pub struct LineRow {
    /// The primary key.
    pub id: i64,
    /// The description.
    pub description: String,
    /// The quantity in units.
    pub quantity: String,
    /// The quantity handed over, in units.
    pub fulfilled: String,
    /// The line with VAT, `AED 105.00`.
    pub gross: String,
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
    can_edit: bool,
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
        .filter(|a| ![SUBMIT, SUBMIT_ONLINE, SUBMIT_FOR_ACCEPTANCE].contains(&a.name.as_str()))
        .collect();
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        title: order.number.clone(),
        status_label: text(&lang, &format!("orders.status.{}", order.status.as_str())),
        status_kind: status_kind(order.status),
        net: money(t.net),
        vat: money(t.vat),
        gross: money(t.gross),
        lines: rows,
        actions,
        can_edit: order.is_draft() && user.has_permission(permissions::SALES_ORDERS_CREATE),
        order,
    }))
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
        return Err(invalid("item_code", text(&lang, "orders.errors.item"), &form));
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
