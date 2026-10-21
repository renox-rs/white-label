//! `HRM.ACL.06` (wl-hr §6 *Access rules*, §7 *Maximum total deductions*,
//! issue #205): a salary recovery stays within the Art. 25 caps, per type and
//! in total per payroll line. The caps are computed on the line's gross fixed
//! wage for the period ([`PayrollLine::fixed_wage`]).
//!
//! | Type | Cap |
//! |---|---|
//! | overpayment recovery | 20% of the wage |
//! | fine | 5% |
//! | court debt | 25% |
//! | damage, cash shortage | 5 days' wage a month (the wage / 30 per day) |
//! | advance recovery, loan instalment, accommodation, transport, settlement offset | none of their own |
//! | all deductions together | 50% |
//!
//! Decisions (spec §5.3 step 3, *Edge cases decided*):
//!
//! - **Refused, never trimmed.** A line above a cap is an exception of the
//!   run ([`problems_in`]); the amounts are not changed. `approve` of the run
//!   is refused while one exists, for every approver, and there is no override
//!   (the check is an effect of the transition, so no permission skips it).
//! - **Rounding.** Caps are whole fils (AED to 2 decimals), rounded down.
//! - **Order.** When the total is exceeded no deduction is singled out: the
//!   preparer reschedules an instalment.
//! - A type cap and the total are reported separately, so a line can show
//!   both.

use renox::db::Transaction;
use renox::prelude::*;
use serde::Serialize;

use crate::adjustment::{Adjustment, AdjustmentKind};
use crate::payroll::PayrollLine;

/// The total of all deductions: 50% of the wage (Art. 25).
pub const TOTAL_PERCENT: i64 = 50;
/// The cap of overpayment recoveries: 20%.
pub const OVERPAYMENT_PERCENT: i64 = 20;
/// The cap of fines: 5%.
pub const FINE_PERCENT: i64 = 5;
/// The cap of court debts: 25%.
pub const COURT_DEBT_PERCENT: i64 = 25;
/// The cap of damage recoveries: this many days' wage a month.
pub const DAMAGE_DAYS: i64 = 5;
/// The days a month a day's wage is the monthly wage divided by (§7).
pub const DAYS_PER_MONTH: i64 = 30;

/// What a cap applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapScope {
    /// Overpayment recoveries.
    Overpayment,
    /// Fines.
    Fine,
    /// Court debts.
    CourtDebt,
    /// Damage and cash shortages.
    Damage,
    /// All deductions of the line together.
    Total,
}

impl CapScope {
    /// The key in texts and JSON.
    pub fn key(self) -> &'static str {
        match self {
            CapScope::Overpayment => "overpayment",
            CapScope::Fine => "fine",
            CapScope::CourtDebt => "court_debt",
            CapScope::Damage => "damage",
            CapScope::Total => "total",
        }
    }
}

impl AdjustmentKind {
    /// The type cap that applies to this kind; `None` when only the total
    /// does.
    pub fn cap_scope(self) -> Option<CapScope> {
        match self {
            AdjustmentKind::OverpaymentRecovery => Some(CapScope::Overpayment),
            AdjustmentKind::Fine => Some(CapScope::Fine),
            AdjustmentKind::CourtDebt => Some(CapScope::CourtDebt),
            AdjustmentKind::DamageRecovery | AdjustmentKind::CashShortageRecovery => {
                Some(CapScope::Damage)
            }
            AdjustmentKind::AdvanceRecovery
            | AdjustmentKind::LoanInstalment
            | AdjustmentKind::AccommodationDeduction
            | AdjustmentKind::TransportDeduction
            | AdjustmentKind::SettlementOffset => None,
        }
    }
}

/// The cap of `scope` for a gross fixed `wage` in fils, rounded down to a
/// fils.
pub fn cap_of(scope: CapScope, wage: i64) -> i64 {
    let wage = wage.max(0);
    match scope {
        CapScope::Overpayment => wage * OVERPAYMENT_PERCENT / 100,
        CapScope::Fine => wage * FINE_PERCENT / 100,
        CapScope::CourtDebt => wage * COURT_DEBT_PERCENT / 100,
        CapScope::Damage => wage * DAMAGE_DAYS / DAYS_PER_MONTH,
        CapScope::Total => wage * TOTAL_PERCENT / 100,
    }
}

/// One cap a line is above.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CapProblem {
    /// The payroll line.
    pub line_id: i64,
    /// The employee.
    pub employee_id: i64,
    /// Which cap.
    pub scope: CapScope,
    /// The deductions counted against it, fils.
    pub amount: i64,
    /// The cap, fils.
    pub cap: i64,
}

/// The caps `deductions` (kind and fils) are above on a line with `wage`:
/// each type cap in turn, then the total. Nothing is trimmed.
pub fn check(
    line_id: i64,
    employee_id: i64,
    wage: i64,
    deductions: &[(AdjustmentKind, i64)],
) -> Vec<CapProblem> {
    let mut problems = Vec::new();
    for scope in [
        CapScope::Overpayment,
        CapScope::Fine,
        CapScope::CourtDebt,
        CapScope::Damage,
    ] {
        let amount: i64 = deductions
            .iter()
            .filter(|(kind, _)| kind.cap_scope() == Some(scope))
            .map(|(_, amount)| amount)
            .sum();
        let cap = cap_of(scope, wage);
        if amount > cap {
            problems.push(CapProblem {
                line_id,
                employee_id,
                scope,
                amount,
                cap,
            });
        }
    }
    let total: i64 = deductions.iter().map(|(_, amount)| amount).sum();
    let cap = cap_of(CapScope::Total, wage);
    if total > cap {
        problems.push(CapProblem {
            line_id,
            employee_id,
            scope: CapScope::Total,
            amount: total,
            cap,
        });
    }
    problems
}

/// The caps exceeded in run `run_id`, line by line, from the adjustments the
/// run carries. Empty when every line is within its caps.
pub async fn problems_in(tx: &mut Transaction, run_id: i64) -> Result<Vec<CapProblem>> {
    let lines = PayrollLine::query()
        .where_(PayrollLine::RUN_ID.eq(run_id))
        .order_by(PayrollLine::ID)
        .get(&mut *tx)
        .await?;
    let adjustments = Adjustment::of_run_in(&mut *tx, run_id).await?;
    let mut problems = Vec::new();
    for line in lines {
        let deductions: Vec<(AdjustmentKind, i64)> = adjustments
            .iter()
            .filter(|a| a.employee_id == line.employee_id)
            .map(|a| (a.kind, a.amount))
            .collect();
        problems.extend(check(
            line.id,
            line.employee_id,
            line.fixed_wage,
            &deductions,
        ));
    }
    Ok(problems)
}

/// [`problems_in`] on a read-only transaction of `db`.
pub async fn problems(db: &Db, run_id: i64) -> Result<Vec<CapProblem>> {
    let mut tx = db.begin().await?;
    problems_in(&mut tx, run_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const WAGE: i64 = 300_000;

    #[test]
    fn the_caps_of_a_wage_of_3000_dirhams() {
        assert_eq!(cap_of(CapScope::Overpayment, WAGE), 60_000);
        assert_eq!(cap_of(CapScope::Fine, WAGE), 15_000);
        assert_eq!(cap_of(CapScope::CourtDebt, WAGE), 75_000);
        assert_eq!(cap_of(CapScope::Damage, WAGE), 50_000);
        assert_eq!(cap_of(CapScope::Total, WAGE), 150_000);
    }

    #[test]
    fn a_cap_is_rounded_down_to_a_fils() {
        // 5 days of 1 000.01 / 30 = 166.67 -> 166 fils, not 167.
        assert_eq!(cap_of(CapScope::Damage, 100_001), 16_666);
        assert_eq!(cap_of(CapScope::Fine, 999), 49);
    }

    #[test]
    fn a_deduction_above_its_type_cap_is_reported_and_one_at_the_cap_is_not() {
        let over = check(1, 2, WAGE, &[(AdjustmentKind::OverpaymentRecovery, 70_000)]);
        assert_eq!(over.len(), 1);
        assert_eq!(over[0].scope, CapScope::Overpayment);
        assert_eq!((over[0].amount, over[0].cap), (70_000, 60_000));
        assert!(check(1, 2, WAGE, &[(AdjustmentKind::OverpaymentRecovery, 60_000)]).is_empty());
    }

    #[test]
    fn each_type_within_its_cap_can_still_exceed_the_total() {
        let kinds = [
            (AdjustmentKind::OverpaymentRecovery, 60_000),
            (AdjustmentKind::CourtDebt, 75_000),
            (AdjustmentKind::Fine, 15_000),
            (AdjustmentKind::AdvanceRecovery, 10_000),
        ];
        let problems = check(1, 2, WAGE, &kinds);
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].scope, CapScope::Total);
        assert_eq!((problems[0].amount, problems[0].cap), (160_000, 150_000));
    }

    #[test]
    fn an_advance_has_only_the_total() {
        assert!(check(1, 2, WAGE, &[(AdjustmentKind::AdvanceRecovery, 150_000)]).is_empty());
        assert_eq!(
            check(1, 2, WAGE, &[(AdjustmentKind::AdvanceRecovery, 150_001)]).len(),
            1
        );
    }

    #[test]
    fn damage_and_cash_shortage_share_the_damage_cap() {
        let both = [
            (AdjustmentKind::DamageRecovery, 30_000),
            (AdjustmentKind::CashShortageRecovery, 30_000),
        ];
        let problems = check(1, 2, WAGE, &both);
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].scope, CapScope::Damage);
    }
}
