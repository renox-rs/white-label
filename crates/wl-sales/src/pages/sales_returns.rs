//! The sales return pages (wl-sales §4.5, §5.6), each behind its permission.
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /sales-returns` | `wl_sales.sales_returns` | `returns.request` |
//! | `GET /sales-returns/{id}` | `wl_sales.sales_return` | `returns.request` |
//! | `POST /sales-returns` | `wl_sales.sales_return_request` | `returns.request` |
//! | `POST /sales-returns/{id}/receive` | `wl_sales.sales_return_receive` | `returns.handle` |
//! | `POST /sales-returns/{id}/resolve` | `wl_sales.sales_return_resolve` | `returns.handle` |
//! | `POST /sales-returns/{id}/transitions/{name}` | `wl_sales.sales_return_transitions.` | `returns.handle` (`reject`, `reverse`) |
//!
//! A refund after completion (BL-SAL-14) is raised from the completed order
//! (`POST /sales-orders/{id}/refund`, `wl_sales.sales_order_refund`, needs
//! `returns.request`); above the limit of the settings the manager decides it
//! under "Needs your action" (`returns.approve`), never the recorder.

use std::collections::BTreeMap;

use renox::prelude::*;
use renox::state_machine::{Available, StateMachine};
use renox::validation::{Errors, ValidationError};
use serde::Serialize;
use wl_kernel::Subject;
use wl_kernel::sales::{COUNTER_SALE_SUBJECT_TYPE, SALES_ORDER_SUBJECT_TYPE};

use crate::config::config_of;
use crate::notices::aed;
use crate::pages::counter_sales::parse_milli;
use crate::sales_return::{ReturnKind, ReturnResolution, ReturnStatus, SalesReturn};
use crate::sales_return_machine::{RECEIVE, RESOLVE, REVERSE, ResolveInput};
use crate::sales_returns::{self, NewReturn, ResolveOutcome};
use crate::{permissions, texts};
use wl_kernel::approvals::ApprovalRequest;

/// The texts a template reads as `tx['returns.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The routes of the pages, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .get("/sales-returns", index)
                .name("wl_sales.sales_returns")
                .get("/sales-returns/{id}", show)
                .name("wl_sales.sales_return")
                .post("/sales-returns", request)
                .name("wl_sales.sales_return_request")
                .require_permission(permissions::RETURNS_REQUEST),
        )
        .merge(
            Routes::new()
                .post("/sales-returns/{id}/receive", receive)
                .name("wl_sales.sales_return_receive")
                .post("/sales-returns/{id}/resolve", resolve)
                .name("wl_sales.sales_return_resolve")
                .group(
                    "/sales-returns",
                    "wl_sales.sales_return_transitions.",
                    wl_kernel::machines::routes::<SalesReturn>(),
                )
                .require_permission(permissions::RETURNS_HANDLE),
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
fn status_kind(status: ReturnStatus) -> &'static str {
    match status {
        ReturnStatus::Requested | ReturnStatus::Received => "info",
        ReturnStatus::Resolved => "success",
        ReturnStatus::Reversed | ReturnStatus::Rejected => "warning",
    }
}

/// One return as the list shows it.
#[derive(Serialize, Debug, Clone)]
pub struct ReturnRow {
    /// The primary key.
    pub id: i64,
    /// The return number.
    pub number: String,
    /// The sale returned, e.g. `sales.counter_sale:4`.
    pub sale: String,
    /// The value with VAT, `AED 52.50`.
    pub value: String,
    /// The status in words.
    pub status_label: String,
    /// The badge colour.
    pub status_kind: &'static str,
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-sales/sales_returns.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    returns: Vec<ReturnRow>,
}

/// The sale a return is about, by its receipt or order number (the subject
/// `type:id` where its module has no label).
async fn sale_label(db: &Db, r: &SalesReturn) -> Result<String> {
    let sale = r.sale();
    Ok(wl_kernel::documents::subject_label(db, &sale)
        .await?
        .unwrap_or_else(|| sale.to_string()))
}

/// `GET /sales-returns`: the latest returns, newest first.
async fn index(State(db): State<Db>, lang: Lang) -> Result<View> {
    let latest = SalesReturn::query()
        .order_by_desc(SalesReturn::ID)
        .get(&db)
        .await?
        .into_iter()
        .take(50);
    let mut returns = Vec::new();
    for r in latest {
        returns.push(ReturnRow {
            id: r.id,
            number: r.number.clone(),
            sale: sale_label(&db, &r).await?,
            value: money(r.gross_value),
            status_label: text(&lang, &format!("returns.status.{}", r.status.as_str())),
            status_kind: status_kind(r.status),
        });
    }
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        returns,
    }))
}

/// What the return page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-sales/sales_return.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    sales_return: SalesReturn,
    title: String,
    status_label: String,
    status_kind: &'static str,
    kind_label: String,
    resolution_label: Option<String>,
    sale: String,
    net: String,
    gross: String,
    actions: Vec<Available>,
    can_receive: bool,
    can_resolve: bool,
    category_label: String,
    party_label: String,
    waiting_alert: String,
    reverse_hint: String,
    resolution_options: Vec<[String; 2]>,
    category_options: Vec<[String; 2]>,
    party_options: Vec<[String; 2]>,
}

/// The options of a select: the stored name and its words.
pub(crate) fn options(lang: &Lang, prefix: &str, names: &[String]) -> Vec<[String; 2]> {
    names
        .iter()
        .map(|n| [n.clone(), text(lang, &format!("{prefix}.{n}"))])
        .collect()
}

/// `GET /sales-returns/{id}`: the return and what the viewer may do.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(r): Found<SalesReturn>,
) -> Result<View> {
    let handles = user.has_permission(permissions::RETURNS_HANDLE);
    let config = config_of(&state);
    let waiting = ApprovalRequest::pending_about(&state.db, &r.subject()).await?;
    let by_channel = r.resolution == Some(ReturnResolution::RefundedByChannel);
    // Receiving and resolving have their own sheets (they take input); only a
    // refund made by the channel is reversed, so `reverse` is offered for that
    // one and the page says why it is not offered for the others.
    let actions = r
        .available(&user, &state.db)
        .await?
        .into_iter()
        .filter(|a| a.name != RECEIVE && a.name != RESOLVE && (a.name != REVERSE || by_channel))
        .collect();
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        title: r.number.clone(),
        status_label: text(&lang, &format!("returns.status.{}", r.status.as_str())),
        status_kind: status_kind(r.status),
        kind_label: text(&lang, &format!("returns.kind.{}", r.kind.as_str())),
        resolution_label: r
            .resolution
            .map(|x| text(&lang, &format!("returns.resolution.{}", x.as_str()))),
        sale: sale_label(&state.db, &r).await?,
        net: money(r.net_value),
        gross: money(r.gross_value),
        actions,
        can_receive: handles && r.status == ReturnStatus::Requested && r.kind == ReturnKind::Goods,
        can_resolve: handles
            && waiting.is_empty()
            && (r.status == ReturnStatus::Received
                || (r.status == ReturnStatus::Requested && r.kind == ReturnKind::NoGoods)),
        category_label: r
            .complaint_category
            .as_deref()
            .map(|c| text(&lang, &format!("returns.category.{c}")))
            .unwrap_or_default(),
        party_label: r
            .responsible_party
            .as_deref()
            .map(|c| text(&lang, &format!("returns.party.{c}")))
            .unwrap_or_default(),
        waiting_alert: if waiting.is_empty() {
            String::new()
        } else {
            text(&lang, "returns.waiting_alert")
        },
        reverse_hint: if r.status == ReturnStatus::Resolved && !by_channel && handles {
            text(&lang, "returns.reverse_hint")
        } else {
            String::new()
        },
        resolution_options: [
            "credit_note",
            "refund",
            "replacement",
            "refunded_by_channel",
        ]
        .iter()
        .map(|n| {
            [
                (*n).to_owned(),
                text(&lang, &format!("returns.resolution.{n}")),
            ]
        })
        .collect(),
        category_options: options(&lang, "returns.category", &config.complaint_categories),
        party_options: options(&lang, "returns.party", &config.responsible_parties),
        sales_return: r,
    }))
}

/// `POST /sales-returns`: requests a return. Fields: `sale_type`
/// (`counter_sale` or `sales_order`), `sale_id`, `kind` (`goods` or
/// `no_goods`), `reason` and `lines` as `line id:quantity` pairs separated
/// by commas (`12:1, 13:2.5`), with the lot the customer shows after a second
/// colon (`12:2:7`).
async fn request(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let get = |key: &str| form.get(key).map_or("", |v| v.trim());
    let sale_type = match get("sale_type") {
        "counter_sale" => COUNTER_SALE_SUBJECT_TYPE,
        "sales_order" => SALES_ORDER_SUBJECT_TYPE,
        _ => {
            return Err(invalid(
                "sale_type",
                text(&lang, "returns.errors.choice"),
                &form,
            ));
        }
    };
    let sale_id: i64 = get("sale_id")
        .parse()
        .map_err(|_| invalid("sale_id", text(&lang, "returns.errors.number"), &form))?;
    let kind = match get("kind") {
        "" | "goods" => ReturnKind::Goods,
        "no_goods" => ReturnKind::NoGoods,
        _ => return Err(invalid("kind", text(&lang, "returns.errors.choice"), &form)),
    };
    let mut new = NewReturn::new(Subject::new(sale_type, sale_id))
        .kind(kind)
        .reason(get("reason"))
        .requested_by(user.id);
    for pair in get("lines").split(',').filter(|p| !p.trim().is_empty()) {
        // `line:quantity` or `line:quantity:lot` (the lot the customer shows).
        let parsed = pair.split_once(':').and_then(|(line, rest)| {
            let (quantity, lot) = match rest.split_once(':') {
                Some((quantity, lot)) => (quantity, Some(lot.trim().parse::<i64>().ok()?)),
                None => (rest, None),
            };
            Some((
                line.trim().parse::<i64>().ok()?,
                parse_milli(quantity)?,
                lot,
            ))
        });
        let Some((line, quantity, lot)) = parsed else {
            return Err(invalid("lines", text(&lang, "returns.errors.lines"), &form));
        };
        new = new.line(line, quantity);
        if let Some(lot) = lot {
            new = new.lot(line, lot);
        }
    }
    let created = sales_returns::request(&state, new).await?;
    Ok((
        Toast::success(text(&lang, "returns.requested")),
        Redirect::route("wl_sales.sales_return", &[&created.id])?,
    )
        .into_response())
}

/// `POST /sales-returns/{id}/receive`: the goods are back; `approved` (a
/// checkbox) accepts them outside the return window.
async fn receive(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(r): Found<SalesReturn>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let approved = matches!(form.get("approved").map(String::as_str), Some("1" | "on"));
    sales_returns::receive(&state, &user, r.id, approved).await?;
    Ok((
        Toast::success(text(&lang, "returns.received")),
        Redirect::route("wl_sales.sales_return", &[&r.id])?,
    )
        .into_response())
}

/// `POST /sales-returns/{id}/resolve`: fields `resolution`, `credit_note`
/// (the id of the credit note issued), `inspected` (checkbox),
/// `complaint_category` and `responsible_party`.
async fn resolve(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(r): Found<SalesReturn>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let get = |key: &str| form.get(key).map_or("", |v| v.trim());
    let resolution = match get("resolution") {
        "credit_note" => Some(ReturnResolution::CreditNote),
        "refund" => Some(ReturnResolution::Refund),
        "replacement" => Some(ReturnResolution::Replacement),
        "refunded_by_channel" => Some(ReturnResolution::RefundedByChannel),
        "" => None,
        _ => {
            return Err(invalid(
                "resolution",
                text(&lang, "returns.errors.choice"),
                &form,
            ));
        }
    };
    let credit_note = match get("credit_note") {
        "" => None,
        v => Some(
            v.parse::<i64>()
                .map_err(|_| invalid("credit_note", text(&lang, "returns.errors.number"), &form))?,
        ),
    };
    let optional = |key: &str| Some(get(key).to_owned()).filter(|v| !v.is_empty());
    let input = ResolveInput {
        resolution,
        credit_note,
        inspected: matches!(get("inspected"), "1" | "on"),
        complaint_category: optional("complaint_category"),
        responsible_party: optional("responsible_party"),
        ..ResolveInput::default()
    };
    let toast = match sales_returns::resolve(&state, &user, r.id, input).await? {
        ResolveOutcome::Done(_) => Toast::success(text(&lang, "returns.resolved")),
        ResolveOutcome::Waiting(_) => Toast::info(text(&lang, "returns.refund_waiting")),
    };
    Ok((toast, Redirect::route("wl_sales.sales_return", &[&r.id])?).into_response())
}
