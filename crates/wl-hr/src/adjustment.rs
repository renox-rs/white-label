//! Adjustments: one-off deductions from salary (wl-hr §3 *Adjustment*, issue
//! #205). Only the deductions the Art. 25 caps apply to are built here
//! (`HRM.ACL.06`, [`crate::caps`]); additions (bonus, advance) and the
//! approval and consent of an adjustment come with the payroll calculation.
//!
//! An adjustment is for a month (`period`). `calculate` takes the open
//! adjustments of the run's period into the lines of their employees
//! ([`link_to_run_in`]), and from then on the caps are checked on the line.

use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};

/// What a deduction is for. The caps of the spec are per type (§7).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AdjustmentKind {
    /// Recovery of an advance paid earlier: no cap of its own, only the 50%
    /// total.
    #[default]
    AdvanceRecovery,
    /// Recovery of an overpayment (a supplementary run): 20% of the wage.
    OverpaymentRecovery,
    /// A fine: 5% of the wage.
    Fine,
    /// A court debt: 25% of the wage.
    CourtDebt,
    /// Damage the worker caused: 5 days' wage a month.
    DamageRecovery,
    /// A cash shortage. Decision: it is damage, so it shares the damage cap
    /// with [`AdjustmentKind::DamageRecovery`].
    CashShortageRecovery,
    /// A loan instalment: only the 50% total.
    LoanInstalment,
    /// An accommodation deduction (needs consent): only the 50% total.
    AccommodationDeduction,
    /// A transport deduction (needs consent): only the 50% total.
    TransportDeduction,
    /// A settlement offset: only the 50% total (beyond it is BL-HRM-23).
    SettlementOffset,
}

/// One deduction from the salary of an employee in a month.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_hr_adjustments", index(employee_id, period), index(run_id))]
pub struct Adjustment {
    /// The primary key.
    pub id: i64,
    /// The employee.
    #[model(references = "wl_hr_employees")]
    pub employee_id: i64,
    /// What it is for.
    pub kind: AdjustmentKind,
    /// The amount, AED fils, positive.
    pub amount: i64,
    /// The month it is deducted in, e.g. `2026-11`.
    pub period: String,
    /// The run that took it into a line.
    pub run_id: Option<i64>,
    /// The hours difference a recovery came from (a reconciliation or an
    /// adjustment timesheet, BL-HRM-16).
    pub difference_id: Option<i64>,
    /// A note.
    pub note: Option<String>,
    /// Who entered it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Adjustment {
    /// The adjustments the run `run_id` carries, oldest first.
    pub async fn of_run_in(tx: &mut Transaction, run_id: i64) -> Result<Vec<Adjustment>> {
        Adjustment::query()
            .where_(Adjustment::RUN_ID.eq(run_id))
            .order_by(Adjustment::ID)
            .get(&mut *tx)
            .await
    }
}

/// Takes the adjustments of `period` that no run has taken, or that run
/// `run_id` took before, into run `run_id`, for the employees that have a
/// line in it. Inside `tx`.
pub async fn link_to_run_in(tx: &mut Transaction, run_id: i64, period: &str) -> Result {
    let employees: Vec<i64> = crate::payroll::PayrollLine::query()
        .where_(crate::payroll::PayrollLine::RUN_ID.eq(run_id))
        .get(&mut *tx)
        .await?
        .into_iter()
        .map(|line| line.employee_id)
        .collect();
    if employees.is_empty() {
        return Ok(());
    }
    // Free the ones this run took before: a recalculation takes them again.
    Adjustment::query()
        .where_(Adjustment::RUN_ID.eq(run_id))
        .update(
            &mut *tx,
            &[("run_id", &None::<i64>), ("updated_at", &now())],
        )
        .await?;
    Adjustment::query()
        .where_(Adjustment::PERIOD.eq(period))
        .where_(Adjustment::RUN_ID.is_null())
        .where_(Adjustment::EMPLOYEE_ID.is_in(employees))
        .update(
            &mut *tx,
            &[("run_id", &Some(run_id)), ("updated_at", &now())],
        )
        .await?;
    Ok(())
}
