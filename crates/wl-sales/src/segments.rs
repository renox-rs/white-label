//! Working out who is in a segment (wl-sales §5.10, §6 `Segments`;
//! BL-SAL-25), and the module's [`Segments`] for `wl-notify`.
//!
//! A segment is evaluated whenever it is used: the preview, the estimate of a
//! broadcast, the send, an export. The candidates are the parties the
//! module has data about (a completed counter sale, a fulfilled order, a
//! loyalty account, an active package); each is tested against the rules,
//! combined with *all* or *any*.
//!
//! Decisions:
//! - A **sale** is a completed counter sale or a sales order that was
//!   fulfilled in part or whole; its value is the net total before VAT. A
//!   return is not netted off: the sale still happened.
//! - Blocked, archived and merged parties are never members (the party
//!   directory says), and neither is a member whose loyalty account is
//!   paused (the party is blocked, BL-SAL-26). A paused or closed account
//!   counts as not enrolled.
//! - Birthday month, party group and branch rules are not built: they need
//!   data the party directory does not give. A vertical adds its own kinds
//!   with [`SegmentRule`].

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::chrono::{NaiveDate, TimeDelta};
use renox::db::DateTime;
use renox::prelude::*;
use renox::serde_json::{Value, json};
use wl_kernel::PartyId;
use wl_kernel::documents::{dubai_date, today};
use wl_kernel::parties::{PartyStatus, directory};
use wl_kernel::segments::{SegmentFuture, SegmentId, Segments};

use crate::counter_sale::{CounterSale, CounterSaleStatus};
use crate::loyalty_account::{AccountStatus, LoyaltyAccount};
use crate::package::{Package, PackageStatus};
use crate::sale_line::{KIND_ITEM, SaleLine};
use crate::sales_order::{SalesOrder, SalesOrderStatus};
use crate::segment::{MatchMode, Rule, Segment};

/// The future a [`SegmentRule`] answers with.
pub type RuleFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// A rule kind a vertical adds (wl-sales §8 `SegmentRule`): 05 no booking in
/// N days, 08 vehicle make and service due, 10 course completed. A segment
/// names it with [`Rule::Vertical`].
pub trait SegmentRule: Send + Sync + 'static {
    /// The kind the segment's rule names.
    fn name(&self) -> &'static str;

    /// Whether `party` meets the rule with `params` at `at`.
    fn matches<'a>(
        &'a self,
        state: &'a AppState,
        party: PartyId,
        params: &'a Value,
        at: DateTime,
    ) -> RuleFuture<'a, bool>;
}

/// The vertical's rule kinds, provided by the module.
#[derive(Clone, Default)]
pub struct SegmentRules(pub Vec<Arc<dyn SegmentRule>>);

/// What a party bought, from the module's own sales.
#[derive(Default)]
struct Activity {
    /// The day and the net value of each sale.
    sales: Vec<(NaiveDate, i64)>,
    /// The day and the code of each item sold.
    items: Vec<(NaiveDate, String)>,
}

/// What the module knows about a party's loyalty account and packages.
#[derive(Default)]
struct Standing {
    /// The points of the open account that is not paused.
    balance: Option<i64>,
    /// Whether an open account is paused (the party is blocked).
    paused: bool,
    /// The credits left in active packages.
    credits: i64,
}

/// Everything the rules read, loaded once per evaluation.
struct Facts {
    activity: BTreeMap<i64, Activity>,
    standing: BTreeMap<i64, Standing>,
}

/// Whether any rule of the segment reads the items sold.
fn needs_items(rules: &[Rule]) -> bool {
    rules.iter().any(|r| matches!(r, Rule::BoughtItem { .. }))
}

/// The day of a sale that completed at `at`, else today.
fn day_of(at: Option<DateTime>) -> NaiveDate {
    at.map_or_else(today, dubai_date)
}

/// Loads the sales, accounts and packages the rules read.
async fn load(db: &Db, items: bool) -> Result<Facts> {
    let mut activity: BTreeMap<i64, Activity> = BTreeMap::new();
    let counter = CounterSale::query()
        .where_(CounterSale::STATUS.eq(CounterSaleStatus::Completed))
        .get(db)
        .await?;
    for sale in counter {
        let Some(party) = sale.party_id else {
            continue;
        };
        let day = day_of(sale.completed_at);
        let entry = activity.entry(party).or_default();
        entry.sales.push((day, sale.net_total));
        if items {
            for line in SaleLine::of(db, &sale.subject()).await? {
                if line.kind == KIND_ITEM {
                    entry.items.push((day, line.item_code.clone()));
                }
            }
        }
    }
    let orders = SalesOrder::query().order_by(SalesOrder::ID).get(db).await?;
    for order in orders {
        if !matches!(
            order.status,
            SalesOrderStatus::PartiallyFulfilled
                | SalesOrderStatus::Fulfilled
                | SalesOrderStatus::Closed
        ) {
            continue;
        }
        let day = day_of(order.fulfilled_at.or(order.confirmed_at));
        let entry = activity.entry(order.party_id).or_default();
        entry.sales.push((day, order.net_total));
        if items {
            for line in SaleLine::of(db, &order.subject()).await? {
                if line.kind == KIND_ITEM {
                    entry.items.push((day, line.item_code.clone()));
                }
            }
        }
    }
    let mut standing: BTreeMap<i64, Standing> = BTreeMap::new();
    let accounts = LoyaltyAccount::query()
        .where_(LoyaltyAccount::STATUS.eq(AccountStatus::Active))
        .get(db)
        .await?;
    for account in accounts {
        let entry = standing.entry(account.party_id).or_default();
        if account.paused {
            entry.paused = true;
        } else {
            entry.balance = Some(entry.balance.unwrap_or(0) + account.points_balance);
        }
    }
    let packages = Package::query()
        .where_(Package::STATUS.eq(PackageStatus::Active))
        .get(db)
        .await?;
    for package in packages {
        standing.entry(package.holder_party_id).or_default().credits += package.remaining();
    }
    Ok(Facts { activity, standing })
}

/// What each party spent (net, before VAT) on sales dated from `from` to
/// `to`, both included; parties with no sale in the period are not listed.
pub(crate) async fn spend_between(
    db: &Db,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<BTreeMap<i64, i64>> {
    let facts = load(db, false).await?;
    let mut spend = BTreeMap::new();
    for (party, activity) in &facts.activity {
        let total: Vec<i64> = activity
            .sales
            .iter()
            .filter(|(day, _)| *day >= from && *day <= to)
            .map(|(_, net)| *net)
            .collect();
        if !total.is_empty() {
            spend.insert(*party, total.iter().sum());
        }
    }
    Ok(spend)
}

/// The parties the facts say anything about.
fn candidates(facts: &Facts) -> BTreeSet<i64> {
    facts
        .activity
        .keys()
        .chain(facts.standing.keys())
        .copied()
        .collect()
}

/// Whether the built-in `rule` holds for a party with this activity and
/// standing on `on`; `None` for a vertical rule.
fn builtin(rule: &Rule, on: NaiveDate, activity: &Activity, standing: &Standing) -> Option<bool> {
    let since = |days: i64| on - TimeDelta::days(days);
    Some(match rule {
        Rule::NoSaleForDays { days } => activity
            .sales
            .iter()
            .map(|(day, _)| *day)
            .max()
            .is_some_and(|last| last <= since(*days)),
        Rule::SpendOver { amount, days } => {
            let from = since(*days);
            activity
                .sales
                .iter()
                .filter(|(day, _)| *day > from && *day <= on)
                .map(|(_, net)| *net)
                .sum::<i64>()
                > *amount
        }
        Rule::SalesAtLeast { count, days } => {
            let from = since(*days);
            activity
                .sales
                .iter()
                .filter(|(day, _)| *day > from && *day <= on)
                .count() as i64
                >= *count
        }
        Rule::BoughtItem { item, days } => activity.items.iter().any(|(day, code)| {
            code.eq_ignore_ascii_case(item.trim())
                && days.is_none_or(|d| *day > since(d))
                && *day <= on
        }),
        Rule::LoyaltyBalanceAtLeast { points } => {
            standing.balance.is_some_and(|balance| balance >= *points)
        }
        Rule::PackageCreditsAtLeast { credits } => standing.credits >= *credits,
        Rule::EnrolledInLoyalty { enrolled } => standing.balance.is_some() == *enrolled,
        Rule::Vertical { .. } => return None,
    })
}

/// Whether `party` meets the rules of `segment`.
#[allow(clippy::too_many_arguments)]
async fn meets(
    state: &AppState,
    segment: &Segment,
    rules: &[Rule],
    verticals: &SegmentRules,
    party: i64,
    on: NaiveDate,
    at: DateTime,
    facts: &Facts,
) -> Result<bool> {
    let nothing = Activity::default();
    let none = Standing::default();
    let activity = facts.activity.get(&party).unwrap_or(&nothing);
    let standing = facts.standing.get(&party).unwrap_or(&none);
    for rule in rules {
        let holds = match builtin(rule, on, activity, standing) {
            Some(holds) => holds,
            None => match rule {
                Rule::Vertical { name, params } => {
                    match verticals.0.iter().find(|v| v.name() == name.as_str()) {
                        Some(vertical) => {
                            vertical.matches(state, PartyId(party), params, at).await?
                        }
                        // A kind nobody registered matches nobody.
                        None => false,
                    }
                }
                _ => false,
            },
        };
        match segment.match_mode {
            MatchMode::All if !holds => return Ok(false),
            MatchMode::Any if holds => return Ok(true),
            _ => {}
        }
    }
    Ok(segment.match_mode == MatchMode::All)
}

/// The parties in `segment` as of `at`, each once, in id order. Blocked,
/// archived and merged parties, and members whose loyalty account is
/// paused, are left out.
///
/// # Errors
///
/// The database's errors and a vertical rule's.
pub async fn members_at(state: &AppState, segment: &Segment, at: DateTime) -> Result<Vec<PartyId>> {
    let rules = segment.rules();
    if rules.is_empty() {
        return Ok(Vec::new());
    }
    let verticals = state
        .provided::<SegmentRules>()
        .map(|rules| (*rules).clone())
        .unwrap_or_default();
    let facts = load(&state.db, needs_items(&rules)).await?;
    let on = dubai_date(at);
    let parties = directory(state);
    let mut members = Vec::new();
    for party in candidates(&facts) {
        if facts.standing.get(&party).is_some_and(|s| s.paused) {
            continue;
        }
        let status = parties.status(&state.db, PartyId(party)).await?;
        if matches!(
            status,
            Some(PartyStatus::Blocked | PartyStatus::Archived | PartyStatus::Merged)
        ) {
            continue;
        }
        if meets(state, segment, &rules, &verticals, party, on, at, &facts).await? {
            members.push(PartyId(party));
        }
    }
    Ok(members)
}

/// Counts the members of `segment` now and stores the count on it: the
/// estimate the editor shows.
///
/// # Errors
///
/// The database's errors.
pub async fn preview(state: &AppState, segment: &Segment) -> Result<i64> {
    let at = renox::db::now();
    let count = members_at(state, segment, at).await?.len() as i64;
    Segment::query()
        .where_(Segment::ID.eq(segment.id))
        .update(
            &state.db,
            &[
                ("last_member_count", &Some(count)),
                ("last_counted_at", &Some(at)),
            ],
        )
        .await?;
    Ok(count)
}

/// The members of `segment` as a CSV (party id and name) for the user `by`
/// to download. The export is audited (`NTF.ACL.01`).
///
/// # Errors
///
/// The database's errors.
pub async fn export_csv(state: &AppState, segment: &Segment, by: i64) -> Result<String> {
    let members = members_at(state, segment, renox::db::now()).await?;
    let parties = directory(state);
    let mut csv = String::from("party_id,name\n");
    for party in &members {
        let name = parties
            .display_name(&state.db, *party, "en")
            .await?
            .unwrap_or_default();
        csv.push_str(&format!("{},\"{}\"\n", party.0, name.replace('"', "\"\"")));
    }
    let entry = renox::audit::Entry::new("sales.segment_exported")
        .user(by)
        .subject(Segment::TABLE, segment.id)
        .data(json!({ "members": members.len() }));
    renox::audit::record(&state.db, entry).await?;
    Ok(csv)
}

/// The segments of this module for `wl-notify` (`Segments`, wl-sales §6).
#[derive(Debug, Clone, Copy, Default)]
pub struct SalesSegments;

impl Segments for SalesSegments {
    /// Decision: without the app state the parties cannot be checked, so
    /// this answers nobody; `wl-notify` asks
    /// [`members_in`](Segments::members_in).
    fn members<'a>(
        &'a self,
        _db: &'a Db,
        _segment: SegmentId,
        _at: DateTime,
    ) -> SegmentFuture<'a, Vec<PartyId>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn members_in<'a>(
        &'a self,
        state: &'a AppState,
        segment: SegmentId,
        at: DateTime,
    ) -> SegmentFuture<'a, Vec<PartyId>> {
        Box::pin(async move {
            match Segment::find(&state.db, segment.0).await? {
                Some(row) => members_at(state, &row, at).await,
                None => Ok(Vec::new()),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, d).unwrap()
    }

    fn activity(sales: &[(NaiveDate, i64)]) -> Activity {
        Activity {
            sales: sales.to_vec(),
            items: Vec::new(),
        }
    }

    #[test]
    fn no_sale_for_days_needs_a_sale_that_old() {
        let on = day(10, 20);
        let rule = Rule::NoSaleForDays { days: 60 };
        let standing = Standing::default();
        assert_eq!(builtin(&rule, on, &activity(&[(day(8, 1), 100)]), &standing), Some(true));
        assert_eq!(builtin(&rule, on, &activity(&[(day(9, 15), 100)]), &standing), Some(false));
        assert_eq!(builtin(&rule, on, &activity(&[]), &standing), Some(false));
    }

    #[test]
    fn spend_is_over_the_amount_within_the_period() {
        let on = day(10, 20);
        let rule = Rule::SpendOver {
            amount: 50_000,
            days: 365,
        };
        let standing = Standing::default();
        let big = activity(&[(day(3, 1), 30_000), (day(4, 1), 20_001)]);
        assert_eq!(builtin(&rule, on, &big, &standing), Some(true));
        let exact = activity(&[(day(3, 1), 30_000), (day(4, 1), 20_000)]);
        assert_eq!(builtin(&rule, on, &exact, &standing), Some(false));
        let old = activity(&[(NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(), 90_000)]);
        assert_eq!(builtin(&rule, on, &old, &standing), Some(false));
    }

    #[test]
    fn a_paused_account_is_not_enrolled() {
        let on = day(10, 20);
        let rule = Rule::EnrolledInLoyalty { enrolled: true };
        let enrolled = Standing {
            balance: Some(10),
            ..Standing::default()
        };
        assert_eq!(builtin(&rule, on, &Activity::default(), &enrolled), Some(true));
        assert_eq!(
            builtin(&rule, on, &Activity::default(), &Standing::default()),
            Some(false)
        );
    }

    #[test]
    fn a_vertical_rule_is_left_to_the_vertical() {
        let rule = Rule::Vertical {
            name: "no_booking".into(),
            params: Value::Null,
        };
        assert_eq!(
            builtin(&rule, day(1, 1), &Activity::default(), &Standing::default()),
            None
        );
    }

    #[test]
    fn items_match_without_case_and_inside_the_period() {
        let on = day(10, 20);
        let mut a = Activity::default();
        a.items.push((day(10, 1), "Shampoo".into()));
        let standing = Standing::default();
        let any = Rule::BoughtItem {
            item: "shampoo".into(),
            days: None,
        };
        let recent = Rule::BoughtItem {
            item: "SHAMPOO".into(),
            days: Some(7),
        };
        assert_eq!(builtin(&any, on, &a, &standing), Some(true));
        assert_eq!(builtin(&recent, on, &a, &standing), Some(false));
    }
}
