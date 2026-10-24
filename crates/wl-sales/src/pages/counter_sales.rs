//! The counter sale pages (wl-sales §4.3, §5.4), behind `pos.sell`.
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /counter-sales` | `wl_sales.counter_sales` | `pos.sell` |
//! | `POST /counter-sales` | `wl_sales.counter_sale_open` | `pos.sell` |
//! | `GET /counter-sales/{id}` | `wl_sales.counter_sale` | `pos.sell` |
//! | `POST /counter-sales/{id}/lines` | `wl_sales.counter_sale_line` | `pos.sell` |
//! | `POST /counter-sales/{id}/complete` | `wl_sales.counter_sale_complete` | `pos.sell` |
//!
//! The loyalty member is attached from `POST /counter-sales/{id}/member`
//! ([`crate::pages::loyalty`]).
//!

use std::collections::BTreeMap;

use renox::grid::{Column, Grid, GridPage, GridRequest, Summary};
use renox::prelude::*;
use renox::validation::{Errors, ValidationError};
use serde::Serialize;
use wl_kernel::PartyId;
use wl_kernel::invoicing::VatCategory;

use crate::counter_sale::{CounterSale, CounterSaleStatus};
use crate::counter_sale_machine::{METHODS, Tender};
use crate::counter_sales::{self, NewCounterSale};
use crate::notices::aed;
use crate::pages::campaigns::parse_fils;
use crate::sale_line::{NewLine, SaleLine};
use crate::{permissions, texts};

/// The texts a template reads as `tx['counter.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The routes, behind `pos.sell`.
pub fn routes() -> Routes {
    Routes::new()
        .get("/counter-sales", index)
        .name("wl_sales.counter_sales")
        .post("/counter-sales", open)
        .name("wl_sales.counter_sale_open")
        .get("/counter-sales/{id}", show)
        .name("wl_sales.counter_sale")
        .post("/counter-sales/{id}/lines", add_line)
        .name("wl_sales.counter_sale_line")
        .post("/counter-sales/{id}/complete", complete)
        .name("wl_sales.counter_sale_complete")
        .require_permission(permissions::POS_SELL)
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
fn status_kind(status: CounterSaleStatus) -> &'static str {
    match status {
        CounterSaleStatus::Open => "info",
        CounterSaleStatus::Completed => "success",
    }
}

/// `text` ("1", "2.5") as thousandths; `None` for anything else.
pub(crate) fn parse_milli(text: &str) -> Option<i64> {
    let text = text.trim();
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    let digits = |s: &str| s.chars().all(|c| c.is_ascii_digit());
    if (whole.is_empty() && fraction.is_empty()) || !digits(whole) || !digits(fraction) {
        return None;
    }
    if fraction.len() > 3 {
        return None;
    }
    let whole: i64 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let fraction: i64 = format!("{fraction:0<3}").parse().ok()?;
    whole.checked_mul(1000)?.checked_add(fraction)
}

/// The counter sales grid: search on the receipt and the customer, filter
/// by status, sum of the totals.
pub(crate) fn grid(lang: &Lang, can_export: bool) -> Grid {
    let t = |key: &str| text(lang, key);
    let statuses: Vec<(String, String)> = CounterSaleStatus::ALL
        .iter()
        .map(|status| {
            (
                status.as_str().to_owned(),
                text(lang, &format!("counter.status.{}", status.as_str())),
            )
        })
        .collect();
    let grid = Grid::new("sales-counter-sales")
        .title(&t("counter.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("number", &t("counter.number"))
                .frozen()
                .mobile()
                .searchable()
                .copyable()
                .link("/counter-sales/{id}"),
        )
        .column(
            Column::related(
                "customer",
                &t("counter.customer_name"),
                "wl_parties_parties",
                "party_id",
                "display_name",
            )
            .mobile()
            .searchable(),
        )
        .column(
            Column::money("gross_total", &wl_kernel::money_label(&t("counter.total")))
                .mobile()
                .summary(Summary::Sum),
        )
        .column(
            Column::money("vat_total", &wl_kernel::money_label(&t("counter.vat")))
                .hidden()
                .summary(Summary::Sum),
        )
        .column(
            Column::select("status", &t("fields.status"), statuses)
                .badges(&[("open", "info"), ("completed", "success")])
                .mobile(),
        )
        .column(Column::datetime("completed_at", &t("counter.completed_at")).hidden())
        .column(Column::datetime("created_at", &t("counter.created_at")).hidden())
        .sort_by("-created_at")
        .row_url("/counter-sales/{id}")
        .cards_on_mobile()
        .empty_state(&t("counter.empty"), Some(&t("counter.empty_hint")));
    if can_export { grid.exports() } else { grid }
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-sales/counter_sales.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    sales: GridPage<CounterSale>,
}

/// `GET /counter-sales`: the sales, newest first (`?export=` with `pos.review`).
async fn index(user: AuthUser, lang: Lang, request: GridRequest) -> Result<Response> {
    let can_export = user.has_permission(permissions::POS_REVIEW);
    let grid = grid(&lang, can_export);
    if request.param("export").is_some() && !can_export {
        return Err(Error::Forbidden);
    }
    let query = CounterSale::query();
    if let Some(file) = grid.export(query.clone(), &request).await? {
        return Ok(file);
    }
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        sales: grid.page(query, &request).await?,
    })
    .into_response())
}

/// `POST /counter-sales`: opens a cart for the signed-in cashier.
async fn open(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let get = |key: &str| form.get(key).map_or("", |v| v.trim());
    let mut new = NewCounterSale::new();
    if !get("party_id").is_empty() {
        let party: i64 = get("party_id")
            .parse()
            .map_err(|_| invalid("party_id", text(&lang, "counter.errors.number"), &form))?;
        new = new.party(PartyId(party));
    }
    let sale = counter_sales::open(&state, user.id, new).await?;
    Ok((
        Toast::success(text(&lang, "counter.opened")),
        Redirect::route("wl_sales.counter_sale", &[&sale.id])?,
    )
        .into_response())
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
    /// The line with VAT, `AED 105.00`.
    pub gross: String,
    /// Whether the loyalty programme leaves it out.
    pub excluded: bool,
}

/// One tender as the page shows it.
#[derive(Serialize, Debug, Clone)]
pub struct PaymentRow {
    /// The primary key.
    pub id: i64,
    /// The method.
    pub method: String,
    /// The amount, `AED 105.00`.
    pub amount: String,
}

/// What the sale page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-sales/counter_sale.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    sale: CounterSale,
    title: String,
    status_label: String,
    status_kind: &'static str,
    net: String,
    vat: String,
    gross: String,
    lines: Vec<LineRow>,
    payments: Vec<PaymentRow>,
    member: Option<String>,
    can_edit: bool,
    can_attach: bool,
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

/// `GET /counter-sales/{id}`: the cart or the receipt.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(sale): Found<CounterSale>,
) -> Result<View> {
    let lines = SaleLine::of(&state.db, &sale.subject()).await?;
    let totals = crate::sale_line::totals(&lines);
    let rows = lines
        .iter()
        .map(|line| LineRow {
            id: line.id,
            description: line.description.clone(),
            quantity: quantity_text(line.quantity_milli),
            gross: money(line.gross()),
            excluded: line.loyalty_excluded,
        })
        .collect();
    let payments = sale
        .payments(&state.db)
        .await?
        .into_iter()
        .map(|p| PaymentRow {
            id: p.id,
            method: text(&lang, &format!("counter.method.{}", p.method)),
            amount: money(p.amount),
        })
        .collect();
    let member =
        crate::loyalty_account::label_of(&state, sale.loyalty_account_id, &lang.locale).await?;
    let redeem = crate::pages::redeem::view_of(
        &state,
        &user,
        &lang,
        &sale.subject(),
        sale.is_open(),
        sale.loyalty_account_id.is_some(),
    )
    .await?;
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        title: sale.number.clone(),
        status_label: text(&lang, &format!("counter.status.{}", sale.status.as_str())),
        status_kind: status_kind(sale.status),
        net: money(totals.net),
        vat: money(totals.vat),
        gross: money(totals.gross),
        lines: rows,
        payments,
        member,
        can_edit: sale.is_open(),
        can_attach: sale.is_open() && user.has_permission(permissions::LOYALTY_ENROL),
        redeem,
        sale,
    }))
}

/// `POST /counter-sales/{id}/lines`: adds a line to the open cart.
async fn add_line(
    State(state): State<AppState>,
    lang: Lang,
    Found(sale): Found<CounterSale>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let get = |key: &str| form.get(key).map_or("", |v| v.trim());
    if get("item_code").is_empty() {
        return Err(invalid(
            "item_code",
            text(&lang, "counter.errors.item"),
            &form,
        ));
    }
    let quantity = match get("quantity") {
        "" => 1000,
        v => parse_milli(v)
            .filter(|q| *q > 0)
            .ok_or_else(|| invalid("quantity", text(&lang, "counter.errors.number"), &form))?,
    };
    let price = parse_fils(get("unit_price"))
        .filter(|p| *p >= 0)
        .ok_or_else(|| invalid("unit_price", text(&lang, "counter.errors.amount"), &form))?;
    let discount = match get("discount") {
        "" => 0,
        v => parse_fils(v)
            .filter(|d| *d >= 0)
            .ok_or_else(|| invalid("discount", text(&lang, "counter.errors.amount"), &form))?,
    };
    let category = match get("vat") {
        "" | "standard" => VatCategory::Standard,
        "zero_rated" => VatCategory::ZeroRated,
        "exempt" => VatCategory::Exempt,
        _ => return Err(invalid("vat", text(&lang, "counter.errors.choice"), &form)),
    };
    let mut line = NewLine::new(get("item_code"), price)
        .quantity_milli(quantity)
        .discount(discount)
        .vat_category(category);
    if !get("description").is_empty() {
        line = line.description(get("description"));
    }
    if matches!(get("loyalty_excluded"), "1" | "on") {
        line = line.loyalty_excluded();
    }
    counter_sales::add_line(&state, sale.id, line).await?;
    Ok((
        Toast::success(text(&lang, "counter.line_added")),
        Redirect::route("wl_sales.counter_sale", &[&sale.id])?,
    )
        .into_response())
}

/// `POST /counter-sales/{id}/complete`: takes the tenders (`cash`, `card`,
/// `account`, `other` amounts, `card_payment` the `wl-payments` id of the
/// card payment) and completes the sale.
async fn complete(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(sale): Found<CounterSale>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let get = |key: &str| form.get(key).map_or("", |v| v.trim());
    let mut tenders = Vec::new();
    for method in METHODS {
        if get(method).is_empty() {
            continue;
        }
        let amount = parse_fils(get(method))
            .filter(|a| *a > 0)
            .ok_or_else(|| invalid(method, text(&lang, "counter.errors.amount"), &form))?;
        let mut tender = Tender::new(method, amount);
        let payment = get(&format!("{method}_payment"));
        if !payment.is_empty() {
            let id: i64 = payment.parse().map_err(|_| {
                invalid(
                    &format!("{method}_payment"),
                    text(&lang, "counter.errors.number"),
                    &form,
                )
            })?;
            tender = tender.payment(id);
        }
        tenders.push(tender);
    }
    counter_sales::complete(&state, &user, sale.id, tenders).await?;
    Ok((
        Toast::success(text(&lang, "counter.completed")),
        Redirect::route("wl_sales.counter_sale", &[&sale.id])?,
    )
        .into_response())
}
