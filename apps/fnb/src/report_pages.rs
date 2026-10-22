//! The reports of failed deliveries and refunds after completion, and the
//! rider fault entries (06 F&B §4.2, §10; BL-FNB-08).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /reports/refunds` | `fnb.report_refunds` | `fnb_reports.view`, filters `from`, `to`, `brand`, `outlet`, `channel` |
//! | `GET /reports/failed-deliveries` | `fnb.report_failed` | `fnb_reports.view`, the same filters |
//! | `GET /rider-faults` | `fnb.rider_faults` | `rider_faults.view`, filter `employee` |

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::Serialize;

use crate::catalogue::{Brand, Outlet};
use crate::pages::{Texts, tx};
use crate::permissions;
use crate::reports::{self, ReportFilter, RiderFault};
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

/// What the filter form shows back.
#[derive(Serialize, Debug, Clone, Default)]
pub(crate) struct FilterView {
    from: String,
    to: String,
    brand: String,
    outlet: String,
    channel: String,
}

fn filter_view(query: &BTreeMap<String, String>) -> FilterView {
    let get = |name: &str| query.get(name).cloned().unwrap_or_default();
    FilterView {
        from: get("from"),
        to: get("to"),
        brand: get("brand"),
        outlet: get("outlet"),
        channel: get("channel"),
    }
}

async fn brand_options(db: &Db) -> Result<Vec<[String; 2]>> {
    Ok(Brand::query()
        .order_by(Brand::NAME)
        .get(db)
        .await?
        .into_iter()
        .map(|b| [b.id.to_string(), b.name])
        .collect())
}

async fn outlet_options(db: &Db) -> Result<Vec<[String; 2]>> {
    Ok(Outlet::query()
        .order_by(Outlet::NAME)
        .get(db)
        .await?
        .into_iter()
        .map(|o| [o.id.to_string(), o.name])
        .collect())
}

/// A group of the refunds report.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct GroupRow {
    category: String,
    party: String,
    count: i64,
    gross: i64,
    net: i64,
}

/// A refund of the refunds report.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct RefundRow {
    order: String,
    order_url: String,
    channel: String,
    category: String,
    party: String,
    resolution: String,
    day: String,
    gross: i64,
    net: i64,
}

/// What the refunds report needs.
#[derive(Serialize, ViewData)]
#[view(template = "fnb/report_refunds.html")]
pub(crate) struct RefundsPage {
    tx: Texts,
    filter: FilterView,
    brands: Vec<[String; 2]>,
    outlets: Vec<[String; 2]>,
    groups: Vec<GroupRow>,
    refunds: Vec<RefundRow>,
    gross: i64,
    net: i64,
}

async fn refunds(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Query(query): Query<BTreeMap<String, String>>,
) -> Result<Response> {
    if !user.has_permission(permissions::REPORTS_VIEW) {
        return Err(Error::Forbidden);
    }
    let report = reports::refunds(&state.db, &filter_of(&query)).await?;
    let category = |c: &str| t(&lang, &format!("fnb.screen.payout.category.{c}"));
    let party = |p: &str| t(&lang, &format!("fnb.screen.payout.party.{p}"));
    let groups = report
        .groups
        .iter()
        .map(|g| GroupRow {
            category: category(&g.category),
            party: party(&g.party),
            count: g.count,
            gross: g.gross,
            net: g.net,
        })
        .collect();
    let rows = report
        .facts
        .iter()
        .map(|f| RefundRow {
            order: f.order_number.clone(),
            order_url: f.order_id.map(|id| format!("/orders/{id}")).unwrap_or_default(),
            channel: f.channel.clone(),
            category: category(&f.category),
            party: party(&f.party),
            resolution: t(&lang, &format!("fnb.screen.report.resolution.{}", f.resolution)),
            day: f.occurred_on.to_string(),
            gross: f.gross,
            net: f.net,
        })
        .collect();
    Ok(View::typed(RefundsPage {
        tx: tx(&lang),
        filter: filter_view(&query),
        brands: brand_options(&state.db).await?,
        outlets: outlet_options(&state.db).await?,
        groups,
        refunds: rows,
        gross: report.gross,
        net: report.net,
    })
    .into_response())
}

/// A failed order of the failed deliveries report.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct FailedRow {
    number: String,
    url: String,
    channel: String,
    day: String,
    party: String,
    total: i64,
}

/// A reason of the failed deliveries report.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ReasonGroup {
    reason: String,
    count: usize,
    gross: i64,
    orders: Vec<FailedRow>,
}

/// What the failed deliveries report needs.
#[derive(Serialize, ViewData)]
#[view(template = "fnb/report_failed.html")]
pub(crate) struct FailedPage {
    tx: Texts,
    filter: FilterView,
    brands: Vec<[String; 2]>,
    outlets: Vec<[String; 2]>,
    groups: Vec<ReasonGroup>,
    total: usize,
}

async fn failed(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Query(query): Query<BTreeMap<String, String>>,
) -> Result<Response> {
    if !user.has_permission(permissions::REPORTS_VIEW) {
        return Err(Error::Forbidden);
    }
    let found = reports::failed_deliveries(&state.db, &filter_of(&query)).await?;
    let groups: Vec<ReasonGroup> = found
        .iter()
        .map(|g| ReasonGroup {
            reason: t(&lang, &format!("fnb.screen.reason.{}", g.reason)),
            count: g.orders.len(),
            gross: g.gross,
            orders: g
                .orders
                .iter()
                .map(|o| FailedRow {
                    number: o.number.clone(),
                    url: format!("/orders/{}", o.id),
                    channel: o.channel.clone(),
                    day: o
                        .failed_at
                        .map(|at| wl_kernel::documents::dubai_date(at).to_string())
                        .unwrap_or_default(),
                    party: o
                        .responsible_party
                        .as_deref()
                        .map(|p| t(&lang, &format!("fnb.screen.payout.party.{p}")))
                        .unwrap_or_default(),
                    total: o.total,
                })
                .collect(),
        })
        .collect();
    let total = groups.iter().map(|g| g.count).sum();
    Ok(View::typed(FailedPage {
        tx: tx(&lang),
        filter: filter_view(&query),
        brands: brand_options(&state.db).await?,
        outlets: outlet_options(&state.db).await?,
        groups,
        total,
    })
    .into_response())
}

/// A fault of the rider faults list.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct FaultRow {
    id: i64,
    rider: String,
    employee_url: String,
    order: String,
    order_url: String,
    category: String,
    amount: i64,
    day: String,
}

/// What the rider faults list needs.
#[derive(Serialize, ViewData)]
#[view(template = "fnb/rider_faults.html")]
pub(crate) struct RiderFaultsPage {
    tx: Texts,
    faults: Vec<FaultRow>,
}

async fn rider_faults(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Query(query): Query<BTreeMap<String, String>>,
) -> Result<Response> {
    if !user.has_permission(permissions::RIDER_FAULTS_VIEW) {
        return Err(Error::Forbidden);
    }
    let employee = query.get("employee").and_then(|v| v.trim().parse::<i64>().ok());
    let mut rows = Vec::new();
    for fault in RiderFault::newest(&state.db, employee, 200).await? {
        let rider = match fault.rider_user_id {
            Some(id) => User::find(&state.db, id)
                .await?
                .map(|u| u.name)
                .unwrap_or_default(),
            None => String::new(),
        };
        let category = if fault.category.is_empty() {
            String::new()
        } else {
            let returns = format!("fnb.screen.payout.category.{}", fault.category);
            let found = t(&lang, &returns);
            if found == returns {
                t(&lang, &format!("fnb.screen.reason.{}", fault.category))
            } else {
                found
            }
        };
        rows.push(FaultRow {
            id: fault.id,
            rider,
            employee_url: fault
                .employee_id
                .map(|id| format!("/hr/employees/{id}"))
                .unwrap_or_default(),
            order: fault.order_number.clone(),
            order_url: format!("/orders/{}", fault.order_id),
            category,
            amount: fault.amount,
            day: fault.occurred_on.to_string(),
        });
    }
    Ok(View::typed(RiderFaultsPage {
        tx: tx(&lang),
        faults: rows,
    })
    .into_response())
}
