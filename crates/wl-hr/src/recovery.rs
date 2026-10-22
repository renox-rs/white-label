//! Spreading an overpayment over the next regular runs within the Art. 25
//! caps (wl-hr §4 *Run types*, §5.3 step 2, §5.6 step 1, `HRM.ACL.06`).
//!
//! An *overpayment recovery* is at most 20% of the wage a month, and all
//! deductions of the month together at most 50%. [`plan`] gives the
//! instalments, month by month, from a first period, leaving out the room
//! other deductions of that month already use; [`create_plan_in`] stores them
//! as [`Adjustment`]s, which the `calculate` of the run of each period takes
//! into the line (and which `approve` checks again).
//!
//! Decisions:
//!
//! - **Greedy.** Each month takes as much as its room allows, so AED 750 on a
//!   wage of AED 3,000 is AED 600 now and AED 150 the month after.
//! - **No room, no plan.** A wage of zero (no salary known) has no room:
//!   [`plan`] returns the whole amount as left over and the caller decides
//!   (a receivable, a refusal). At most [`MAX_MONTHS`] months are planned.

use renox::db::Transaction;
use renox::prelude::*;

use crate::adjustment::{Adjustment, AdjustmentKind};
use crate::caps::{CapScope, cap_of};
use crate::hours::next_period;

/// The most months a recovery is spread over.
pub const MAX_MONTHS: usize = 36;

/// What the deductions already booked in a period use of its caps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Used {
    /// Overpayment recoveries, fils.
    pub overpayment: i64,
    /// All deductions, fils.
    pub total: i64,
}

/// The instalments of `amount` fils for a wage of `wage`, from period
/// `first`: `(period, fils)` in order, and what is left over when the months
/// run out. `used` gives what a period's other deductions use.
pub fn plan(
    wage: i64,
    amount: i64,
    first: &str,
    used: impl Fn(&str) -> Used,
) -> (Vec<(String, i64)>, i64) {
    let mut left = amount.max(0);
    let mut period = first.to_owned();
    let mut instalments = Vec::new();
    for _ in 0..MAX_MONTHS {
        if left == 0 {
            break;
        }
        let booked = used(&period);
        let room = (cap_of(CapScope::Overpayment, wage) - booked.overpayment)
            .min(cap_of(CapScope::Total, wage) - booked.total);
        if room > 0 {
            let take = left.min(room);
            instalments.push((period.clone(), take));
            left -= take;
        }
        let Some(next) = next_period(&period) else {
            break;
        };
        period = next;
    }
    (instalments, left)
}

/// What the adjustments of `employee_id` already use in `period`, inside
/// `tx`.
pub async fn used_in(tx: &mut Transaction, employee_id: i64, period: &str) -> Result<Used> {
    let booked = Adjustment::query()
        .where_(Adjustment::EMPLOYEE_ID.eq(employee_id))
        .where_(Adjustment::PERIOD.eq(period))
        .get(&mut *tx)
        .await?;
    Ok(Used {
        overpayment: booked
            .iter()
            .filter(|a| a.kind == AdjustmentKind::OverpaymentRecovery)
            .map(|a| a.amount)
            .sum(),
        total: booked.iter().map(|a| a.amount).sum(),
    })
}

/// Stores the plan for `amount` fils of overpayment recovery of
/// `employee_id` from period `first`, inside `tx`, as one adjustment per
/// month. `difference_id` links them to the hours difference they came from.
/// Returns the instalments and the amount left over.
pub async fn create_plan_in(
    tx: &mut Transaction,
    employee_id: i64,
    wage: i64,
    amount: i64,
    first: &str,
    difference_id: Option<i64>,
    note: &str,
) -> Result<(Vec<Adjustment>, i64)> {
    // The room of each month is read first: the closure cannot wait.
    let mut booked = std::collections::BTreeMap::new();
    let mut period = first.to_owned();
    for _ in 0..MAX_MONTHS {
        booked.insert(
            period.clone(),
            used_in(&mut *tx, employee_id, &period).await?,
        );
        let Some(next) = next_period(&period) else {
            break;
        };
        period = next;
    }
    let (instalments, left) = plan(wage, amount, first, |p| {
        booked.get(p).copied().unwrap_or_default()
    });
    let mut created = Vec::new();
    for (period, fils) in instalments {
        let mut adjustment = Adjustment {
            employee_id,
            kind: AdjustmentKind::OverpaymentRecovery,
            amount: fils,
            period,
            difference_id,
            note: Some(note.to_owned()),
            ..Adjustment::default()
        };
        adjustment.insert(&mut *tx).await?;
        created.push(adjustment);
    }
    Ok((created, left))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recovery_of_750_on_a_wage_of_3000_is_600_now_and_150_next_month() {
        let (plan, left) = plan(300_000, 75_000, "2026-11", |_| Used::default());
        assert_eq!(
            plan,
            vec![
                ("2026-11".to_owned(), 60_000),
                ("2026-12".to_owned(), 15_000)
            ]
        );
        assert_eq!(left, 0);
    }

    #[test]
    fn a_recovery_of_900_is_600_and_300_over_two_runs() {
        let (plan, _) = plan(300_000, 90_000, "2026-12", |_| Used::default());
        assert_eq!(
            plan,
            vec![
                ("2026-12".to_owned(), 60_000),
                ("2027-01".to_owned(), 30_000)
            ]
        );
    }

    #[test]
    fn a_month_with_other_deductions_gives_only_the_room_left_of_the_total() {
        // 50% of 3 000 is 1 500; 1 200 are used, so 300 of the 600 are free.
        let used = |period: &str| {
            if period == "2026-11" {
                Used {
                    overpayment: 0,
                    total: 120_000,
                }
            } else {
                Used::default()
            }
        };
        let (plan, _) = plan(300_000, 90_000, "2026-11", used);
        assert_eq!(
            plan,
            vec![
                ("2026-11".to_owned(), 30_000),
                ("2026-12".to_owned(), 60_000)
            ]
        );
    }

    #[test]
    fn a_month_whose_overpayment_cap_is_used_is_skipped() {
        let used = |period: &str| {
            if period == "2026-11" {
                Used {
                    overpayment: 60_000,
                    total: 60_000,
                }
            } else {
                Used::default()
            }
        };
        let (plan, _) = plan(300_000, 10_000, "2026-11", used);
        assert_eq!(plan, vec![("2026-12".to_owned(), 10_000)]);
    }

    #[test]
    fn without_a_wage_nothing_is_planned() {
        let (plan, left) = plan(0, 10_000, "2026-11", |_| Used::default());
        assert!(plan.is_empty());
        assert_eq!(left, 10_000);
    }
}
