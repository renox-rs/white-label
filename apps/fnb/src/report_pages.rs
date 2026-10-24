//! The reports of failed deliveries and refunds after completion, and the
//! rider fault entries (06 F&B §4.2, §10; BL-FNB-08).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /reports/refunds` | `fnb.report_refunds` | `fnb_reports.view`, filters `from`, `to`, `brand`, `outlet`, `channel` |
//! | `GET /reports/failed-deliveries` | `fnb.report_failed` | `fnb_reports.view`, the same filters |
//! | `GET /rider-faults` | `fnb.rider_faults` | `rider_faults.view`, filter `employee` |

use std::collections::BTreeMap;

use renox::chrono::{NaiveDate, TimeDelta};
use renox::db::{DateTime, Query as ModelQuery};
use renox::grid::{Column, Grid, GridRequest, Summary};
use renox::prelude::*;
use renox::serde_json::json;
use serde::Serialize;
use wl_kernel::documents::DUBAI_OFFSET_SECONDS;

use crate::order::{FnbOrder, reason};
use crate::pages::{Texts, tx};
use crate::payouts;
use crate::permissions;
use crate::reports::{RefundFact, ReportFilter, RiderFault};
use crate::texts::t;

/// The templates, compiled in.
pub(crate) const VIEWS: &[(&str, &str)] = &[
    (
        "fnb/report_refunds.html",
        include_str!("../views/fnb/report_refunds.html"),
    ),
    (
        "fnb/report_failed.html",
        include_str!("../views/fnb/report_failed.html"),
    ),
    (
        "fnb/rider_faults.html",
        include_str!("../views/fnb/rider_faults.html"),
    ),
];

/// The routes.
pub fn routes() -> Routes {
    Routes::new()
        .get("/reports/refunds", refunds)
        .name("fnb.report_refunds")
        .get("/reports/failed-deliveries", failed)
        .name("fnb.report_failed")
        .get("/rider-faults", rider_faults)
        .name("fnb.rider_faults")
        .require_auth()
}

/// The filters as the query gives them.
fn filter_of(query: &BTreeMap<String, String>) -> ReportFilter {
    let day = |name: &str| {
        query
            .get(name)
            .and_then(|v| NaiveDate::parse_from_str(v.trim(), "%Y-%m-%d").ok())
    };
    let id = |name: &str| query.get(name).and_then(|v| v.trim().parse::<i64>().ok());
    ReportFilter {
        from: day("from"),
        to: day("to"),
        brand: id("brand"),
        outlet: id("outlet"),
        channel: query
            .get("channel")
            .map(|c| c.trim().to_owned())
            .filter(|c| !c.is_empty()),
    }
}

/// The first instant of the Dubai day `day`, in UTC.
fn day_start(day: NaiveDate) -> DateTime {
    day.and_hms_opt(0, 0, 0).unwrap_or_default().and_utc()
        - TimeDelta::seconds(i64::from(DUBAI_OFFSET_SECONDS))
}

/// The party words of the refund and fault lists, as select options.
fn party_options(lang: &Lang) -> Vec<(String, String)> {
    [
        "business",
        "aggregator",
        "rider",
        "driver_or_rider",
        "supplier",
        "customer",
    ]
    .iter()
    .map(|code| {
        (
            (*code).to_owned(),
            t(lang, &format!("fnb.screen.payout.party.{code}")),
        )
    })
    .collect()
}

/// The complaint category words, as select options.
fn category_options(lang: &Lang) -> Vec<(String, String)> {
    payouts::CATEGORIES
        .iter()
        .map(|code| {
            (
                (*code).to_owned(),
                t(lang, &format!("fnb.screen.payout.category.{code}")),
            )
        })
        .collect()
}

/// The failure reasons of an order, as select options.
fn reason_options(lang: &Lang) -> Vec<(String, String)> {
    [
        reason::CUSTOMER_UNREACHABLE,
        reason::WRONG_ADDRESS,
        reason::REFUSED,
        reason::REFUSED_LATE,
        reason::REFUSED_WRONG_ITEM,
        reason::NO_CASH,
        reason::RIDER_ISSUE,
        reason::RUN_ABORTED,
    ]
    .iter()
    .map(|code| {
        (
            (*code).to_owned(),
            t(lang, &format!("fnb.screen.reason.{code}")),
        )
    })
    .collect()
}

/// The refunds report grid: the refunds after completion, grouped by
/// category and party, with sums of the net and gross. The period, brand,
/// outlet and channel of the URL narrow it as before; the same filters are
/// in the grid's own.
pub(crate) fn refunds_grid(lang: &Lang) -> Grid {
    let t = tx(lang);
    let resolutions: Vec<(String, String)> = ["refund", "credit_note", "refunded_by_channel"]
        .iter()
        .map(|code| {
            (
                (*code).to_owned(),
                crate::texts::t(lang, &format!("fnb.screen.report.resolution.{code}")),
            )
        })
        .collect();
    Grid::new("fnb-report-refunds")
        .title(&t["report.refunds_title"])
        .per_page(25)
        .remember()
        .advanced_filter()
        .exports()
        .column(
            Column::text("order_number", &t["number"])
                .frozen()
                .mobile()
                .searchable()
                .link("/orders/{order_id}"),
        )
        .column(Column::text("channel", &t["channel"]).searchable())
        .column(Column::select("category", &t["report.category"], category_options(lang)).mobile())
        .column(Column::select("party", &t["party"], party_options(lang)))
        .column(Column::select("resolution", &t["report.how"], resolutions).hidden())
        .column(Column::date("occurred_on", &t["report.day"]))
        .column(
            Column::related(
                "brand",
                &t["report.brand"],
                "fnb_brands",
                "brand_id",
                "name",
            )
            .hidden(),
        )
        .column(
            Column::related(
                "outlet",
                &t["report.outlet"],
                "fnb_outlets",
                "outlet_id",
                "name",
            )
            .hidden(),
        )
        .column(
            Column::money("net", &t["report.net"])
                .hidden()
                .summary(Summary::Sum),
        )
        .column(
            Column::money("gross", &t["report.gross"])
                .mobile()
                .summary(Summary::Sum),
        )
        .groups(&["category", "party"])
        .sort_by("-id")
        .cards_on_mobile()
        .empty_state(&t["report.refunds_empty"], None)
}

/// The refunds that count (not taken back), narrowed by the URL's filter.
fn refunds_query(filter: &ReportFilter) -> ModelQuery<RefundFact> {
    let mut query = RefundFact::query().where_(RefundFact::REVERSED_AT.is_null());
    if let Some(from) = filter.from {
        query = query.where_(RefundFact::OCCURRED_ON.gte(from));
    }
    if let Some(to) = filter.to {
        query = query.where_(RefundFact::OCCURRED_ON.lte(to));
    }
    if let Some(brand) = filter.brand {
        query = query.where_(RefundFact::BRAND_ID.eq(brand));
    }
    if let Some(outlet) = filter.outlet {
        query = query.where_(RefundFact::OUTLET_ID.eq(outlet));
    }
    if let Some(channel) = filter.channel.as_deref() {
        query = query.where_(RefundFact::CHANNEL.eq(channel.trim().to_lowercase()));
    }
    query
}

/// What the refunds report needs.
#[derive(Serialize, ViewData)]
#[view(template = "fnb/report_refunds.html")]
pub(crate) struct RefundsPage {
    tx: Texts,
    refunds: renox::grid::GridPage<RefundFact>,
}

async fn refunds(
    State(_state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Query(query): Query<BTreeMap<String, String>>,
    request: GridRequest,
) -> Result<Response> {
    if !user.has_permission(permissions::REPORTS_VIEW) {
        return Err(Error::Forbidden);
    }
    let grid = refunds_grid(&lang);
    let base = refunds_query(&filter_of(&query));
    if let Some(file) = grid.export(base.clone(), &request).await? {
        return Ok(file);
    }
    Ok(View::typed(RefundsPage {
        tx: tx(&lang),
        refunds: grid.page(base, &request).await?,
    })
    .into_response())
}

/// The failed deliveries grid: the orders whose delivery failed, grouped by
/// reason, with the sum of what they were worth.
pub(crate) fn failed_grid(lang: &Lang) -> Grid {
    let t = tx(lang);
    Grid::new("fnb-report-failed")
        .title(&t["report.failed_title"])
        .per_page(25)
        .remember()
        .advanced_filter()
        .exports()
        .column(
            Column::text("number", &t["number"])
                .frozen()
                .mobile()
                .searchable()
                .link("/orders/{id}"),
        )
        .column(Column::select("failure_reason", &t["reason"], reason_options(lang)).mobile())
        .column(Column::text("channel", &t["channel"]).searchable())
        .column(Column::datetime("failed_at", &t["report.day"]))
        .column(Column::select(
            "responsible_party",
            &t["party"],
            party_options(lang),
        ))
        .column(
            Column::related(
                "brand",
                &t["report.brand"],
                "fnb_brands",
                "brand_id",
                "name",
            )
            .hidden(),
        )
        .column(
            Column::related(
                "outlet",
                &t["report.outlet"],
                "fnb_outlets",
                "outlet_id",
                "name",
            )
            .hidden(),
        )
        .column(
            Column::money("total", &t["total"])
                .mobile()
                .summary(Summary::Sum),
        )
        .groups(&["failure_reason", "responsible_party", "channel"])
        .group_by("failure_reason")
        .sort_by("-id")
        .row_url("/orders/{id}")
        .cards_on_mobile()
        .empty_state(&t["report.failed_empty"], None)
}

/// The orders with a failed delivery, narrowed by the URL's filter.
fn failed_query(filter: &ReportFilter) -> ModelQuery<FnbOrder> {
    let mut query = FnbOrder::query().where_(FnbOrder::FAILURE_REASON.is_not_null());
    if let Some(from) = filter.from {
        query = query.where_(FnbOrder::FAILED_AT.gte(day_start(from)));
    }
    if let Some(to) = filter.to {
        let next = to.succ_opt().unwrap_or(to);
        query = query.where_(FnbOrder::FAILED_AT.lt(day_start(next)));
    }
    if let Some(brand) = filter.brand {
        query = query.where_(FnbOrder::BRAND_ID.eq(brand));
    }
    if let Some(outlet) = filter.outlet {
        query = query.where_(FnbOrder::OUTLET_ID.eq(outlet));
    }
    if let Some(channel) = filter.channel.as_deref() {
        query = query.where_(FnbOrder::CHANNEL.eq(channel.trim().to_lowercase()));
    }
    query
}

/// What the failed deliveries report needs.
#[derive(Serialize, ViewData)]
#[view(template = "fnb/report_failed.html")]
pub(crate) struct FailedPage {
    tx: Texts,
    orders: renox::grid::GridPage<FnbOrder>,
}

async fn failed(
    State(_state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Query(query): Query<BTreeMap<String, String>>,
    request: GridRequest,
) -> Result<Response> {
    if !user.has_permission(permissions::REPORTS_VIEW) {
        return Err(Error::Forbidden);
    }
    let grid = failed_grid(&lang);
    let base = failed_query(&filter_of(&query));
    if let Some(file) = grid.export(base.clone(), &request).await? {
        return Ok(file);
    }
    Ok(View::typed(FailedPage {
        tx: tx(&lang),
        orders: grid.page(base, &request).await?,
    })
    .into_response())
}

/// The rider faults grid: search on the rider and order, filter by category
/// and day, sum of the amounts. The rider links to the employee when there is
/// one (`custom`).
pub(crate) fn faults_grid(lang: &Lang) -> Grid {
    let t = tx(lang);
    let mut categories = category_options(lang);
    categories.extend(reason_options(lang));
    Grid::new("fnb-rider-faults")
        .title(&t["report.faults_title"])
        .per_page(25)
        .remember()
        .advanced_filter()
        .exports()
        .column(
            Column::custom("rider", &t["report.rider"])
                .frozen()
                .mobile(),
        )
        .column(
            Column::related(
                "rider_name",
                &t["report.rider"],
                "users",
                "rider_user_id",
                "name",
            )
            .searchable()
            .hidden(),
        )
        .column(
            Column::text("order_number", &t["number"])
                .mobile()
                .searchable()
                .link("/orders/{order_id}"),
        )
        .column(Column::select("category", &t["report.category"], categories).mobile())
        .column(
            Column::money("amount", &t["amount"])
                .mobile()
                .summary(Summary::Sum),
        )
        .column(Column::date("occurred_on", &t["report.day"]))
        .sort_by("-id")
        .cards_on_mobile()
        .empty_state(&t["report.faults_empty"], None)
}

/// What the rider faults list needs.
#[derive(Serialize, ViewData)]
#[view(template = "fnb/rider_faults.html")]
pub(crate) struct RiderFaultsPage {
    tx: Texts,
    faults: renox::grid::GridPage<RiderFault>,
}

async fn rider_faults(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Query(query): Query<BTreeMap<String, String>>,
    request: GridRequest,
) -> Result<Response> {
    if !user.has_permission(permissions::RIDER_FAULTS_VIEW) {
        return Err(Error::Forbidden);
    }
    let grid = faults_grid(&lang);
    let mut base = RiderFault::query();
    if let Some(employee) = query
        .get("employee")
        .and_then(|v| v.trim().parse::<i64>().ok())
    {
        base = base.where_(RiderFault::EMPLOYEE_ID.eq(employee));
    }
    if let Some(file) = grid.export(base.clone(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(base, &request).await?;
    let mut names = BTreeMap::new();
    for fault in page.items() {
        if let Some(id) = fault.rider_user_id {
            if let Some(found) = User::find(&state.db, id).await? {
                names.insert(id, found.name);
            }
        }
    }
    let faults = page.extend(|fault| {
        json!({
            "rider": fault
                .rider_user_id
                .and_then(|id| names.get(&id))
                .cloned()
                .unwrap_or_default(),
            "employee_url": fault.employee_id.map(|id| format!("/hr/employees/{id}")),
        })
    });
    Ok(View::typed(RiderFaultsPage {
        tx: tx(&lang),
        faults,
    })
    .into_response())
}
