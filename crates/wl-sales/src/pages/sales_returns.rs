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

use renox::grid::{Column, Grid, GridPage, GridRequest, Summary};
use renox::prelude::*;
use renox::serde_json::json;
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

/// The returns grid: search on the number and the customer, filters by
/// status, kind and resolution, sum of the values. The sale (a receipt or
/// order number) is drawn by the template (`custom`).
pub(crate) fn grid(lang: &Lang, can_export: bool) -> Grid {
    let t = |key: &str| text(lang, key);
    let words = |prefix: &str, names: Vec<&str>| -> Vec<(String, String)> {
        names
            .into_iter()
            .map(|n| (n.to_owned(), text(lang, &format!("{prefix}.{n}"))))
            .collect()
    };
    let grid = Grid::new("sales-returns")
        .title(&t("returns.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("number", &t("returns.number"))
                .frozen()
                .mobile()
                .searchable()
                .copyable()
                .link("/sales-returns/{id}"),
        )
        .column(Column::custom("sale", &t("returns.sale")).mobile())
        .column(
            Column::related(
                "customer",
                &t("orders.customer_name"),
                "wl_parties_parties",
                "party_id",
                "display_name",
            )
            .hidden()
            .searchable(),
        )
        .column(
            Column::select(
                "kind",
                &t("returns.kind"),
                words(
                    "returns.kind",
                    ReturnKind::ALL.iter().map(|k| k.as_str()).collect(),
                ),
            )
            .hidden(),
        )
        .column(
            Column::select(
                "resolution",
                &t("returns.resolution"),
                words(
                    "returns.resolution",
                    ReturnResolution::ALL.iter().map(|r| r.as_str()).collect(),
                ),
            )
            .hidden(),
        )
        .column(
            Column::money("gross_value", &wl_kernel::money_label(&t("returns.value")))
                .mobile()
                .summary(Summary::Sum),
        )
        .column(
            Column::select(
                "status",
                &t("fields.status"),
                words(
                    "returns.status",
                    ReturnStatus::ALL.iter().map(|s| s.as_str()).collect(),
                ),
            )
            .badges(&[
                ("requested", "info"),
                ("received", "info"),
                ("resolved", "success"),
                ("reversed", "warning"),
                ("rejected", "warning"),
            ])
            .mobile(),
        )
        .column(Column::datetime("created_at", &t("returns.created_at")).hidden())
        .sort_by("-created_at")
        .row_url("/sales-returns/{id}")
        .cards_on_mobile()
        .empty_state(&t("returns.empty"), Some(&t("returns.empty_hint")));
    if can_export { grid.exports() } else { grid }
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-sales/sales_returns.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    returns: GridPage<SalesReturn>,
}

/// The sale a return is about, by its receipt or order number (the subject
/// `type:id` where its module has no label).
async fn sale_label(db: &Db, r: &SalesReturn) -> Result<String> {
    let sale = r.sale();
    Ok(wl_kernel::documents::subject_label(db, &sale)
        .await?
        .unwrap_or_else(|| sale.to_string()))
}

/// `GET /sales-returns`: the returns, newest first (`?export=` with
/// `returns.handle`).
async fn index(
    State(db): State<Db>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    let can_export = user.has_permission(permissions::RETURNS_HANDLE);
    let grid = grid(&lang, can_export);
    if request.param("export").is_some() && !can_export {
        return Err(Error::Forbidden);
    }
    let query = SalesReturn::query();
    if let Some(file) = grid.export(query.clone(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let mut labels = Vec::new();
    for r in page.items() {
        labels.push(sale_label(&db, r).await?);
    }
    let mut labels = labels.into_iter();
    let returns = page.extend(|_| json!({ "sale": labels.next().unwrap_or_default() }));
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        returns,
    })
    .into_response())
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
