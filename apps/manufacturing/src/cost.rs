//! The cost of a work order (07 Manufacturing §7.1 *Work order*, `MFG.CST.01`
//! as far as the abort needs it). Money is fils, quantities are thousandths.
//!
//! - **Issued value**: what each issue was posted with, quantity times its
//!   unit cost.
//! - **Returned value**: a returned quantity of an item and lot is valued at
//!   the average unit cost of this work order's issues of that item and lot
//!   (issued value over issued quantity): 300 kg at AED 8 and 200 kg at AED 9
//!   make AED 8.40 a kg, so 100 kg back are AED 840.
//! - **Scrap value** of an abort: issued less returned less the good output
//!   at the standard material cost per unit, never below 0.
//! - **Labour and overhead**: the hours from `start` to the stop time, time
//!   `on_hold` included, at the hourly rate of the line, 0 when the work
//!   order never started.
//! - **Actual cost**: issued less returned plus labour and overhead. The
//!   **loss** of an abort is the actual cost less the good output at the
//!   standard cost per unit; it may be negative (a gain) and is stored as
//!   calculated.

use renox::db::{DateTime, Transaction};
use renox::prelude::*;

use crate::db::set_order;
use crate::issue::MaterialIssue;
use crate::plant::Line;
use crate::work_order::WorkOrder;

/// The value of `quantity_milli` at `unit_cost` fils a base unit, rounded.
pub fn value_of(quantity_milli: i64, unit_cost: i64) -> i64 {
    let value = i128::from(quantity_milli) * i128::from(unit_cost);
    i64::try_from((2 * value + 1_000) / 2_000).unwrap_or(0)
}

/// The issues that count: posted ones, not the short part that waits.
fn posted(issues: &[MaterialIssue]) -> impl Iterator<Item = &MaterialIssue> {
    issues
        .iter()
        .filter(|issue| issue.pending_issue_id.is_none())
}

/// What was issued, in fils.
pub fn issued_value(issues: &[MaterialIssue]) -> i64 {
    posted(issues)
        .map(|issue| value_of(issue.quantity_milli, issue.unit_cost))
        .sum()
}

/// The quantity issued of `item` and `lot`, in thousandths.
pub fn issued_quantity(issues: &[MaterialIssue], item: i64, lot: Option<i64>) -> i64 {
    posted(issues)
        .filter(|issue| issue.item_id == item && issue.lot_id == lot)
        .map(|issue| issue.quantity_milli)
        .sum()
}

/// The quantity of `item` and `lot` already returned, in thousandths.
pub fn returned_quantity(issues: &[MaterialIssue], item: i64, lot: Option<i64>) -> i64 {
    posted(issues)
        .filter(|issue| issue.item_id == item && issue.lot_id == lot)
        .map(|issue| issue.returned_milli)
        .sum()
}

/// The value of `quantity_milli` of `item` and `lot` at the average unit cost
/// of this work order's issues of it.
pub fn average_value(
    issues: &[MaterialIssue],
    item: i64,
    lot: Option<i64>,
    quantity_milli: i64,
) -> i64 {
    let (quantity, value) = posted(issues)
        .filter(|issue| issue.item_id == item && issue.lot_id == lot)
        .fold((0_i128, 0_i128), |(q, v), issue| {
            (
                q + i128::from(issue.quantity_milli),
                v + i128::from(value_of(issue.quantity_milli, issue.unit_cost)),
            )
        });
    if quantity == 0 {
        return 0;
    }
    i64::try_from((2 * value * i128::from(quantity_milli) + quantity) / (2 * quantity)).unwrap_or(0)
}

/// The value of everything already returned.
pub fn returned_value(issues: &[MaterialIssue]) -> i64 {
    let mut seen: Vec<(i64, Option<i64>)> = Vec::new();
    let mut total = 0;
    for issue in posted(issues) {
        let key = (issue.item_id, issue.lot_id);
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        total += average_value(
            issues,
            issue.item_id,
            issue.lot_id,
            returned_quantity(issues, issue.item_id, issue.lot_id),
        );
    }
    total
}

/// The scrap value of an abort: `issued` less `returned` less the good output
/// at the standard material cost per unit, never below 0.
pub fn scrap_value(issued: i64, returned: i64, good_milli: i64, std_material_cost: i64) -> i64 {
    (issued - returned - value_of(good_milli, std_material_cost)).max(0)
}

/// Labour and overhead from `started` to `stopped` at `rate` fils an hour; 0
/// when the work order never started. The time is counted in whole minutes,
/// rounded to the nearest: a shift of 4 h 59 min 59 s is 5 hours, not 4 h 59
/// min (the clocks of the two moments are read a few milliseconds apart, so
/// a cut would bill a minute less for the same shift on some runs).
pub fn labour_cost(rate: i64, started: Option<DateTime>, stopped: DateTime) -> i64 {
    let Some(started) = started else {
        return 0;
    };
    let seconds = (stopped - started).num_seconds().max(0);
    let minutes = (seconds + 30) / 60;
    let value = i128::from(rate) * i128::from(minutes);
    i64::try_from((2 * value + 60) / 120).unwrap_or(0)
}

/// The loss of an abort: the actual cost less the good output at the
/// standard cost per unit.
pub fn loss(actual: i64, good_milli: i64, std_unit_cost: i64) -> i64 {
    actual - value_of(good_milli, std_unit_cost)
}

/// The actual cost of `order` as of `stopped`: materials issued less
/// returned, plus labour and overhead at the rate of its line.
///
/// # Errors
///
/// A database error.
pub async fn actual_cost(
    tx: &mut Transaction,
    order: &WorkOrder,
    stopped: DateTime,
) -> Result<i64> {
    let issues = MaterialIssue::of_order(&mut *tx, order.id).await?;
    let rate = match order.line_id {
        Some(line) => Line::find(&mut *tx, line)
            .await?
            .map_or(0, |l| l.hourly_rate),
        None => 0,
    };
    Ok(issued_value(&issues) - returned_value(&issues)
        + labour_cost(rate, order.started_at, stopped))
}

/// The effect of `close`: the actual cost is calculated and kept
/// (`MFG.CST.01`).
pub(crate) async fn close(tx: &mut Transaction, id: i64) -> Result {
    let order = WorkOrder::find_or_404(&mut *tx, id).await?;
    let stopped = order.stopped_at.unwrap_or_else(renox::db::now);
    let actual = actual_cost(tx, &order, stopped).await?;
    set_order(&mut *tx, id, &[("actual_cost", &Some(actual))]).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(item: i64, lot: i64, quantity: i64, cost: i64) -> MaterialIssue {
        MaterialIssue {
            item_id: item,
            lot_id: Some(lot),
            quantity_milli: quantity,
            unit_cost: cost,
            ..MaterialIssue::default()
        }
    }

    #[test]
    fn a_return_is_valued_at_the_average_cost_of_the_issues_of_its_lot() {
        let issues = [issue(1, 1, 300_000, 800), issue(1, 1, 200_000, 900)];
        assert_eq!(issued_value(&issues), 420_000);
        assert_eq!(average_value(&issues, 1, Some(1), 100_000), 84_000);
    }

    #[test]
    fn the_scrap_value_of_the_issue_example() {
        // 500 kg at AED 8, 200 kg back, 50 good units at AED 18 of material.
        let issues = [issue(1, 1, 500_000, 800)];
        let returned = average_value(&issues, 1, Some(1), 200_000);
        assert_eq!(returned, 160_000);
        assert_eq!(
            scrap_value(issued_value(&issues), returned, 50_000, 1_800),
            150_000
        );
        assert_eq!(
            scrap_value(issued_value(&issues), 0, 50_000, 1_800),
            310_000
        );
        assert_eq!(scrap_value(1_000, 0, 50_000, 1_800), 0);
    }

    #[test]
    fn the_loss_counts_the_hours_of_the_line() {
        let start = renox::db::now();
        let stop = start + renox::chrono::Duration::hours(5);
        assert_eq!(labour_cost(12_000, Some(start), stop), 60_000);
        assert_eq!(labour_cost(12_000, None, stop), 0);
        assert_eq!(loss(240_000 + 60_000, 50_000, 2_400), 180_000);
        assert_eq!(loss(100_000, 50_000, 2_400), -20_000);
    }

    #[test]
    fn the_hours_are_counted_in_minutes_rounded_to_the_nearest() {
        let start = renox::db::now();
        let at = |seconds: i64| start + renox::chrono::Duration::seconds(seconds);
        // A shift read a second short of 5 hours is still 5 hours.
        assert_eq!(labour_cost(12_000, Some(start), at(5 * 3_600 - 1)), 60_000);
        assert_eq!(labour_cost(12_000, Some(start), at(5 * 3_600 + 29)), 60_000);
        // From half a minute on, the next minute counts; a clock set back counts 0.
        assert_eq!(labour_cost(12_000, Some(start), at(5 * 3_600 + 30)), 60_200);
        assert_eq!(labour_cost(12_000, Some(start), at(29)), 0);
        assert_eq!(labour_cost(12_000, Some(start), at(-3_600)), 0);
    }

    #[test]
    fn a_waiting_issue_is_not_counted() {
        let mut waiting = issue(1, 1, 10_000, 800);
        waiting.pending_issue_id = Some(4);
        assert_eq!(issued_value(&[waiting]), 0);
    }
}
