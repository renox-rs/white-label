//! A negative final settlement (wl-hr §4 *Final settlement*: *Negative net*,
//! `HRM.ACL.06`, §10 `⚠ open: lawyer (L-23)`, issue #213).
//!
//! The recoveries (advances, loans, overpayments, fines, court debts, damage)
//! are offset against the dues **only within the Art. 25 caps**
//! ([`crate::caps`]); until a lawyer confirms otherwise (L-23) the system never
//! offsets more. What is above the caps is the amount due from the employee.
//!
//! - The settlement counts as **one period**: each per-type cap applies once,
//!   as a share of the dues (overpayments 20%, fines 5%, court debts 25%),
//!   damage at most 5 days' wage of the last salary structure (wage / 30 per
//!   day), advances and loans only the 50% total of the dues.
//! - The **offset** is the recoveries within their type caps, but at most 50%
//!   of the dues. The **net** is the dues minus the offset, so it is never
//!   below 0 and, with dues above 0, at least half of them. The amount **due
//!   from the employee** is the recoveries minus the offset.
//! - **Rounding:** the caps are whole fils, rounded down ([`cap_of`]).
//! - **Net 0** (only when the dues are 0): `pay` succeeds with no payment.
//! - **`pay`** creates a receivable for the amount due from the employee on
//!   the employee's party (a non-VAT receivable, not a tax invoice) and a task
//!   for the accountant ([`sync_receivable_in`], [`crate::settlement_events`]);
//!   it never creates a payment for it.
//!
//! [`recalculate_in`] keeps `recoveries_offset`, `net` and `owed_by_employee`
//! of a settlement in line with its dues and recoveries; it runs when the
//! amounts change, when a supplementary run adds a recovery, at `approve`, at
//! `pay` and at `resolve`.

use renox::db::{Transaction, now};
use renox::prelude::*;
use wl_kernel::documents::today;

use crate::caps::{CapScope, cap_of};
use crate::employee::Employee;
use crate::receivable::Receivable;
use crate::salary::SalaryStructure;
use crate::settlement::FinalSettlement;
use crate::settlement_payment::paid_total_in;
use crate::texts::english;

/// The recoveries of a settlement by the kind of cap that applies, AED fils.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Recoveries {
    /// Overpayment recoveries (20% of the dues).
    pub overpayment: i64,
    /// Advances and loans (the 50% total only).
    pub advance: i64,
    /// Fines (5% of the dues).
    pub fine: i64,
    /// Court debts (25% of the dues).
    pub court_debt: i64,
    /// Damage (5 days' wage of the last salary structure).
    pub damage: i64,
}

impl Recoveries {
    /// The recoveries of `settlement`.
    pub fn of(settlement: &FinalSettlement) -> Self {
        Recoveries {
            overpayment: settlement.recoveries,
            advance: settlement.advance_recoveries,
            fine: settlement.fine_recoveries,
            court_debt: settlement.court_debt_recoveries,
            damage: settlement.damage_recoveries,
        }
    }

    /// All of them.
    pub fn total(&self) -> i64 {
        self.overpayment + self.advance + self.fine + self.court_debt + self.damage
    }
}

/// What is offset, paid and owed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Offset {
    /// The recoveries offset within the caps.
    pub offset: i64,
    /// The dues minus the offset.
    pub net: i64,
    /// The recoveries above the caps.
    pub owed: i64,
}

/// The offset of `recoveries` against `dues` for a last fixed wage of `wage`
/// (all fils). Nothing is offset above a cap and no figure is negative.
pub fn offset_of(dues: i64, recoveries: &Recoveries, wage: i64) -> Offset {
    let dues = dues.max(0);
    let within = |amount: i64, scope: CapScope, base: i64| amount.max(0).min(cap_of(scope, base));
    let allowed = within(recoveries.overpayment, CapScope::Overpayment, dues)
        + within(recoveries.fine, CapScope::Fine, dues)
        + within(recoveries.court_debt, CapScope::CourtDebt, dues)
        + within(recoveries.damage, CapScope::Damage, wage)
        + recoveries.advance.max(0);
    let offset = allowed.min(cap_of(CapScope::Total, dues));
    let total: i64 = [
        recoveries.overpayment,
        recoveries.advance,
        recoveries.fine,
        recoveries.court_debt,
        recoveries.damage,
    ]
    .iter()
    .map(|amount| (*amount).max(0))
    .sum();
    Offset {
        offset,
        net: dues - offset,
        owed: total - offset,
    }
}

/// Works out the offset of settlement `id` from its dues, its recoveries and
/// the last salary structure, stores it and returns the settlement.
pub async fn recalculate_in(tx: &mut Transaction, id: i64) -> Result<FinalSettlement> {
    let settlement = FinalSettlement::load_in(&mut *tx, id).await?;
    let day = settlement.last_working_day.unwrap_or_else(today);
    let wage = SalaryStructure::effective_in(&mut *tx, settlement.employee_id, day)
        .await?
        .map_or(0, |structure| structure.fixed_wage());
    let result = offset_of(settlement.dues(), &Recoveries::of(&settlement), wage);
    FinalSettlement::query()
        .where_(FinalSettlement::ID.eq(id))
        .update(
            &mut *tx,
            &[
                ("recoveries_offset", &result.offset),
                ("net", &result.net),
                ("owed_by_employee", &result.owed),
                ("updated_at", &now()),
            ],
        )
        .await?;
    FinalSettlement::load_in(&mut *tx, id).await
}

/// Makes the receivables of settlement `id` cover what the employee owes: the
/// recoveries above the caps once the settlement was paid, and the part of a
/// revised net below what was paid. Only the missing part is added, so a
/// second call changes nothing. Returns the receivable made, if any.
pub async fn sync_receivable_in(tx: &mut Transaction, id: i64) -> Result<Option<Receivable>> {
    let settlement = FinalSettlement::load_in(&mut *tx, id).await?;
    let Some(employee) = Employee::find(&mut *tx, settlement.employee_id).await? else {
        return Ok(None);
    };
    let wanted = settlement.overpaid
        + if settlement.paid_on.is_some() {
            settlement.owed_by_employee
        } else {
            0
        };
    let existing: i64 = Receivable::query()
        .where_(Receivable::SETTLEMENT_ID.eq(id))
        .get(&mut *tx)
        .await?
        .iter()
        .map(|r| r.amount)
        .sum();
    if wanted <= existing {
        return Ok(None);
    }
    let mut receivable = Receivable {
        employee_id: employee.id,
        party_id: employee.party_id,
        amount: wanted - existing,
        settlement_id: Some(id),
        status: "open".to_owned(),
        note: Some(english("wl_hr.settlement.receivable_note", &[("id", &id)])),
        ..Receivable::default()
    };
    receivable.insert(&mut *tx).await?;
    Ok(Some(receivable))
}

/// What of the net is still unpaid.
pub async fn unpaid_in(tx: &mut Transaction, settlement: &FinalSettlement) -> Result<i64> {
    let paid = paid_total_in(&mut *tx, settlement.id).await?;
    Ok((settlement.net - paid).max(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    const WAGE: i64 = 300_000;

    fn only(overpayment: i64, advance: i64, fine: i64, court: i64, damage: i64) -> Recoveries {
        Recoveries {
            overpayment,
            advance,
            fine,
            court_debt: court,
            damage,
        }
    }

    #[test]
    fn an_advance_above_half_the_dues_leaves_the_rest_to_the_employee() {
        // Dues 4 000, advance 6 000: offset 2 000, net 2 000, owed 4 000.
        let result = offset_of(400_000, &only(0, 600_000, 0, 0, 0), WAGE);
        assert_eq!(
            result,
            Offset {
                offset: 200_000,
                net: 200_000,
                owed: 400_000
            }
        );
    }

    #[test]
    fn an_overpayment_is_offset_to_20_percent_of_the_dues() {
        // Dues 10 000, overpayment 3 000: offset 2 000, net 8 000, owed 1 000.
        let result = offset_of(1_000_000, &only(300_000, 0, 0, 0, 0), WAGE);
        assert_eq!(
            result,
            Offset {
                offset: 200_000,
                net: 800_000,
                owed: 100_000
            }
        );
    }

    #[test]
    fn no_dues_means_nothing_is_offset_and_all_is_owed() {
        let result = offset_of(0, &only(0, 100_000, 0, 0, 0), WAGE);
        assert_eq!(
            result,
            Offset {
                offset: 0,
                net: 0,
                owed: 100_000
            }
        );
        let none = offset_of(0, &Recoveries::default(), WAGE);
        assert_eq!((none.offset, none.net, none.owed), (0, 0, 0));
    }

    #[test]
    fn damage_is_capped_by_five_days_of_the_last_wage_not_by_the_dues() {
        // 5 days of 3 000 / 30 = 500; dues 10 000 would allow 50% = 5 000.
        let result = offset_of(1_000_000, &only(0, 0, 0, 0, 90_000), WAGE);
        assert_eq!(result.offset, 50_000);
        assert_eq!(result.owed, 40_000);
    }

    #[test]
    fn each_type_within_its_cap_is_still_held_to_half_the_dues() {
        // Dues 10 000: overpayment 2 000 + court 2 500 + fine 500 + advance 1 000
        // = 6 000 within the type caps, but the total cap is 5 000.
        let result = offset_of(1_000_000, &only(200_000, 100_000, 50_000, 250_000, 0), WAGE);
        assert_eq!(result.offset, 500_000);
        assert_eq!(result.net, 500_000);
        assert_eq!(result.owed, 100_000);
    }

    #[test]
    fn the_net_is_never_below_zero_and_never_below_half_the_dues() {
        let result = offset_of(99, &only(0, 1_000_000, 0, 0, 0), 0);
        assert_eq!(result.offset, 49);
        assert_eq!(result.net, 50);
    }
}
