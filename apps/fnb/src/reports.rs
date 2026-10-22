//! Rider faults, and the reports of failed deliveries and refunds after
//! completion (06 F&B §4.2 row *Complaint after delivery*, §10; BL-FNB-08).
//!
//! - **Refund facts.** When a *no goods* return of an order is resolved
//!   (`ReturnResolved`: a complaint after delivery, the refund of a reversed
//!   delivery, an aggregator's refund taken from a payout) the app writes one
//!   [`RefundFact`] with the category, the responsible party, the amount with
//!   and without VAT, the brand, outlet and channel of the order and the
//!   business day. A return taken back (`ReturnReversed`) leaves the report.
//!   The party values are the wl-sales list (*business*, *driver_or_rider*,
//!   *aggregator*, *supplier*, *customer*).
//! - **Rider faults.** A return whose party is the rider, and a failed
//!   delivery closed with the rider responsible (`close_failed`), write a
//!   [`RiderFault`] for the rider of the delivery's last run: order, category,
//!   amount, date. It is seen by the holders of `rider_faults.view` (the
//!   kitchen manager and HR), not by other riders, and it never touches pay.
//! - **Reports.** [`refunds`] groups the facts by category and party;
//!   [`failed_deliveries`] groups the failed orders by reason. Filters: period
//!   (the business day), brand, outlet, channel.
//!
//! Decisions: the entry lives in this app (`/rider-faults`), because the HR
//! employee record has no timeline other modules can write to yet; it keeps
//! the employee id so the record can show it later. The period is the day the
//! return was resolved and the day the delivery failed in Asia/Dubai (the
//! business date of BL-FNB-15 is not built).

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::dubai_date;
use wl_kernel::sales::{ReturnResolved, ReturnReversed, SALES_ORDER_SUBJECT_TYPE};
use wl_kernel::workforce::workforce;

use crate::money::net_of;
use crate::order::{FnbOrder, party};

/// The responsible parties that stand for the rider: the order's word and the
/// sales settings'.
pub const RIDER_PARTIES: [&str; 2] = [party::RIDER, "driver_or_rider"];

/// Where a rider fault came from.
pub mod source {
    /// A sales return naming the rider resolved.
    pub const RETURN: &str = "return";
    /// A failed delivery closed with the rider responsible.
    pub const CLOSE: &str = "close";
}

/// A refund after completion, as the report reads it.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "fnb_refund_facts",
    unique(sales_return_id),
    index(occurred_on)
)]
pub struct RefundFact {
    /// The primary key.
    pub id: i64,
    /// The sales return.
    pub sales_return_id: i64,
    /// Its sales order.
    pub sales_order_id: i64,
    /// The F&B order.
    pub order_id: Option<i64>,
    /// The order's number.
    pub order_number: String,
    /// The brand of the order.
    pub brand_id: Option<i64>,
    /// The outlet of the order.
    pub outlet_id: Option<i64>,
    /// The channel of the order.
    pub channel: String,
    /// The complaint category.
    pub category: String,
    /// The responsible party.
    pub party: String,
    /// `refund`, `credit_note` or `refunded_by_channel`.
    pub resolution: String,
    /// With VAT, in fils.
    pub gross: i64,
    /// Without VAT, in fils.
    pub net: i64,
    /// The business day the return was resolved.
    pub occurred_on: NaiveDate,
    /// When the return was taken back.
    pub reversed_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// An entry on a rider's record: a fault of the rider, with no pay effect.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "fnb_rider_faults", unique(fault_key), index(employee_id))]
pub struct RiderFault {
    /// The primary key.
    pub id: i64,
    /// `return:<id>` or `close:<order>`: one entry each.
    pub fault_key: String,
    /// The order.
    pub order_id: i64,
    /// The order's number.
    pub order_number: String,
    /// The sales return, when the fault came from one.
    pub sales_return_id: Option<i64>,
    /// The rider of the delivery's last run (a user).
    pub rider_user_id: Option<i64>,
    /// The rider's employee record.
    pub employee_id: Option<i64>,
    /// The complaint category or the failure reason.
    pub category: String,
    /// What the fault cost, in fils.
    pub amount: i64,
    /// The day of the fault.
    pub occurred_on: NaiveDate,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl RiderFault {
    /// The faults, newest first; of one employee when `employee` is given.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn newest(db: &Db, employee: Option<i64>, limit: u64) -> Result<Vec<RiderFault>> {
        let mut query = RiderFault::query()
            .order_by_desc(RiderFault::ID)
            .limit(limit);
        if let Some(employee) = employee {
            query = query.where_(RiderFault::EMPLOYEE_ID.eq(employee));
        }
        query.get(db).await
    }
}

/// Writes the fault of the rider of `order`'s last run, once per `key`.
async fn record_fault(
    state: &AppState,
    order: &FnbOrder,
    key: &str,
    sales_return_id: Option<i64>,
    category: &str,
    amount: i64,
) -> Result {
    if RiderFault::query()
        .where_(RiderFault::FAULT_KEY.eq(key))
        .first(&state.db)
        .await?
        .is_some()
    {
        return Ok(());
    }
    let employee = match order.rider_user_id {
        Some(user) => workforce(state)
            .employee_of_user(&state.db, user)
            .await?
            .map(|employee| employee.0),
        None => None,
    };
    let mut fault = RiderFault {
        fault_key: key.to_owned(),
        order_id: order.id,
        order_number: order.number.clone(),
        sales_return_id,
        rider_user_id: order.rider_user_id,
        employee_id: employee,
        category: category.to_owned(),
        amount,
        occurred_on: dubai_date(renox::db::now()),
        ..RiderFault::default()
    };
    fault.insert(&state.db).await?;
    Ok(())
}

/// `ReturnResolved`: a *no goods* return of an F&B order becomes a refund
/// fact, and a return naming the rider a rider fault.
///
/// # Errors
///
/// Database errors.
pub(crate) async fn on_return_resolved(state: &AppState, event: ReturnResolved) -> Result {
    if event.kind != "no_goods" || event.sale.subject_type != SALES_ORDER_SUBJECT_TYPE {
        return Ok(());
    }
    let Some(order) = FnbOrder::of_sales_order(&state.db, event.sale.subject_id).await? else {
        return Ok(());
    };
    let category = event
        .complaint_category
        .clone()
        .unwrap_or_else(|| "other".to_owned());
    let responsible = event.responsible_party.clone().unwrap_or_default();
    let exists = RefundFact::query()
        .where_(RefundFact::SALES_RETURN_ID.eq(event.sales_return.0))
        .first(&state.db)
        .await?
        .is_some();
    if !exists {
        let mut fact = RefundFact {
            sales_return_id: event.sales_return.0,
            sales_order_id: order.sales_order_id,
            order_id: Some(order.id),
            order_number: order.number.clone(),
            brand_id: order.brand_id,
            outlet_id: order.outlet_id,
            channel: order.channel.clone(),
            category: category.clone(),
            party: responsible.clone(),
            resolution: event.resolution.clone(),
            gross: event.value,
            net: net_of(event.value),
            occurred_on: dubai_date(renox::db::now()),
            ..RefundFact::default()
        };
        fact.insert(&state.db).await?;
    }
    if RIDER_PARTIES.contains(&responsible.as_str()) {
        record_fault(
            state,
            &order,
            &format!("{}:{}", source::RETURN, event.sales_return.0),
            Some(event.sales_return.0),
            &category,
            event.value,
        )
        .await?;
    }
    Ok(())
}

/// `ReturnReversed`: the refund no longer counts in the report.
///
/// # Errors
///
/// Database errors.
pub(crate) async fn on_return_reversed(state: &AppState, event: ReturnReversed) -> Result {
    let at = Some(renox::db::now());
    RefundFact::query()
        .where_(RefundFact::SALES_RETURN_ID.eq(event.sales_return.0))
        .update(&state.db, &[("reversed_at", &at)])
        .await?;
    Ok(())
}

/// A failed delivery was closed (`close_failed`): when the rider is the party
/// responsible, the fault goes on the rider's record, once per order.
///
/// # Errors
///
/// Database errors.
pub(crate) async fn on_closed(state: &AppState, order: &FnbOrder) -> Result {
    if order.responsible_party.as_deref() != Some(party::RIDER) {
        return Ok(());
    }
    let category = order
        .failure_reason
        .clone()
        .unwrap_or_else(|| "other".to_owned());
    record_fault(
        state,
        order,
        &format!("{}:{}", source::CLOSE, order.id),
        None,
        &category,
        order.refund_amount,
    )
    .await
}

/// The period and the filters of a report.
#[derive(Debug, Clone, Default)]
pub struct ReportFilter {
    /// The first business day, or none.
    pub from: Option<NaiveDate>,
    /// The last business day, or none.
    pub to: Option<NaiveDate>,
    /// A brand, or none.
    pub brand: Option<i64>,
    /// An outlet, or none.
    pub outlet: Option<i64>,
    /// A channel, or none.
    pub channel: Option<String>,
}

impl ReportFilter {
    /// Whether `day` is inside the period.
    fn has_day(&self, day: NaiveDate) -> bool {
        self.from.is_none_or(|from| day >= from) && self.to.is_none_or(|to| day <= to)
    }

    /// Whether an order of the given brand, outlet and channel passes.
    fn has(&self, brand: Option<i64>, outlet: Option<i64>, channel: &str) -> bool {
        self.brand.is_none_or(|b| brand == Some(b))
            && self.outlet.is_none_or(|o| outlet == Some(o))
            && self
                .channel
                .as_deref()
                .is_none_or(|c| c.trim().is_empty() || c.trim().eq_ignore_ascii_case(channel))
    }
}

/// One group of the refunds report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefundGroup {
    /// The complaint category.
    pub category: String,
    /// The responsible party.
    pub party: String,
    /// How many refunds.
    pub count: i64,
    /// With VAT, in fils.
    pub gross: i64,
    /// Without VAT, in fils.
    pub net: i64,
}

/// The refunds after completion in the period: the groups by category and
/// party, and the refunds behind them.
#[derive(Debug, Clone, Default)]
pub struct RefundReport {
    /// The groups, by category then party.
    pub groups: Vec<RefundGroup>,
    /// The refunds, newest first.
    pub facts: Vec<RefundFact>,
    /// The total with VAT, in fils.
    pub gross: i64,
    /// The total without VAT, in fils.
    pub net: i64,
}

/// The refunds report: refunds after completion by category and responsible
/// party, including the aggregators' refunds deducted from payouts (the
/// resolution `refunded_by_channel`), net and gross.
///
/// # Errors
///
/// Database errors.
pub async fn refunds(db: &Db, filter: &ReportFilter) -> Result<RefundReport> {
    let all = RefundFact::query()
        .order_by_desc(RefundFact::ID)
        .get(db)
        .await?;
    let facts: Vec<RefundFact> = all
        .into_iter()
        .filter(|f| f.reversed_at.is_none())
        .filter(|f| filter.has_day(f.occurred_on))
        .filter(|f| filter.has(f.brand_id, f.outlet_id, &f.channel))
        .collect();
    let mut groups: Vec<RefundGroup> = Vec::new();
    for fact in &facts {
        match groups
            .iter_mut()
            .find(|g| g.category == fact.category && g.party == fact.party)
        {
            Some(group) => {
                group.count += 1;
                group.gross += fact.gross;
                group.net += fact.net;
            }
            None => groups.push(RefundGroup {
                category: fact.category.clone(),
                party: fact.party.clone(),
                count: 1,
                gross: fact.gross,
                net: fact.net,
            }),
        }
    }
    groups.sort_by(|a, b| (&a.category, &a.party).cmp(&(&b.category, &b.party)));
    Ok(RefundReport {
        gross: facts.iter().map(|f| f.gross).sum(),
        net: facts.iter().map(|f| f.net).sum(),
        groups,
        facts,
    })
}

/// One group of the failed deliveries report.
#[derive(Debug, Clone)]
pub struct FailureGroup {
    /// The order's failure reason (one of [`crate::order::reason`]).
    pub reason: String,
    /// The orders that failed for it.
    pub orders: Vec<FnbOrder>,
    /// What they were worth with VAT, in fils.
    pub gross: i64,
}

/// The failed own-rider deliveries in the period, grouped by reason.
///
/// # Errors
///
/// Database errors.
pub async fn failed_deliveries(db: &Db, filter: &ReportFilter) -> Result<Vec<FailureGroup>> {
    let failed = FnbOrder::query()
        .where_(FnbOrder::FAILURE_REASON.is_not_null())
        .order_by_desc(FnbOrder::ID)
        .get(db)
        .await?;
    let mut groups: Vec<FailureGroup> = Vec::new();
    for order in failed {
        let Some(reason) = order.failure_reason.clone() else {
            continue;
        };
        let day = order.failed_at.map(dubai_date);
        if day.is_some_and(|d| !filter.has_day(d))
            || (day.is_none() && (filter.from.is_some() || filter.to.is_some()))
        {
            continue;
        }
        if !filter.has(order.brand_id, order.outlet_id, &order.channel) {
            continue;
        }
        match groups.iter_mut().find(|g| g.reason == reason) {
            Some(group) => {
                group.gross += order.total;
                group.orders.push(order);
            }
            None => groups.push(FailureGroup {
                reason,
                gross: order.total,
                orders: vec![order],
            }),
        }
    }
    groups.sort_by(|a, b| a.reason.cmp(&b.reason));
    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_filter_keeps_the_period_and_the_channel() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
        let filter = ReportFilter {
            from: NaiveDate::from_ymd_opt(2026, 10, 1),
            to: NaiveDate::from_ymd_opt(2026, 10, 31),
            channel: Some("Aggregator_A".to_owned()),
            ..ReportFilter::default()
        };
        assert!(filter.has_day(day));
        assert!(!filter.has_day(NaiveDate::from_ymd_opt(2026, 11, 1).unwrap()));
        assert!(filter.has(None, None, "aggregator_a"));
        assert!(!filter.has(None, None, "website"));
        assert!(ReportFilter::default().has(Some(1), Some(2), "anything"));
    }

    #[test]
    fn the_rider_is_both_words() {
        assert!(RIDER_PARTIES.contains(&"rider"));
        assert!(RIDER_PARTIES.contains(&"driver_or_rider"));
    }
}
