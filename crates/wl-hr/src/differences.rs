//! Hours that enter a regular run after the fact (wl-hr §5.3 step 2, wl-attendance
//! "Correcting a locked timesheet", issue #206).
//!
//! Two things reach payroll after the hours of a period were paid:
//!
//! - an approved **adjustment timesheet** that affects payroll (`payroll` or
//!   `both`): `TimesheetApproved` of kind `adjustment`;
//! - the **reconciliation** of a timesheet paid on provisional hours once it
//!   is approved: `TimesheetReconciliationNeeded`. The adjustment timesheet
//!   behind it emits no `TimesheetApproved` (wl-attendance §4), so it is read
//!   once, here; an adjustment of origin `provisional_reconciliation` that
//!   does arrive as `TimesheetApproved` is ignored for the same reason.
//!
//! Each hour of a category is kept as an [`HourDifference`]. `calculate` of the
//! next `regular` run of the employee's legal entity prices it with the
//! salary structure effective on the day of the hours, links it to the run
//! ([`HourDifference::run_id`]) and pays it (+) or recovers it (-). A second
//! `calculate` of the same run takes it again without doubling it; another run
//! never does. A negative difference is a deduction of kind *overpayment
//! recovery* on the line, so the Art. 25 caps check it (`HRM.ACL.06`).
//!
//! Decisions:
//!
//! - **The day of the hours.** `TimesheetApproved` gives the last day of the
//!   adjustment timesheet. `TimesheetReconciliationNeeded` carries no day: the
//!   last day of the period of the run that paid the provisional hours is
//!   used, the day the hours were paid for.
//! - **Above the cap.** A negative difference above the cap is neither
//!   trimmed nor planned: the line is flagged and `approve` refused. The
//!   preparer turns it into a recovery with an instalment plan with
//!   [`convert_to_recovery`], then recalculates.

use renox::chrono::NaiveDate;
use renox::db::{DateTime, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::attendance::{CategoryMinutes, TimesheetApproved};
use wl_kernel::documents::today;
use wl_kernel::timesheets::TimesheetReconciliationNeeded;

use crate::adjustment::Adjustment;
use crate::employee::{Employee, EmploymentType};
use crate::hours::period_bounds;
use crate::payroll::{PayrollLine, PayrollRun, PayrollRunStatus};
use crate::recovery;
use crate::salary::SalaryStructure;
use crate::texts::english;

/// The hours of an approved adjustment timesheet that affects payroll.
pub const SOURCE_ADJUSTMENT: &str = "adjustment";
/// The difference of a timesheet paid on provisional hours.
pub const SOURCE_RECONCILIATION: &str = "reconciliation";

/// Waiting for, or in, a run.
pub const OPEN: &str = "open";
/// Turned into a recovery with an instalment plan.
pub const CONVERTED: &str = "converted";

/// Hours of one category an adjustment adds (+) or takes back (-).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_hr_hour_differences",
    unique(source, timesheet_type, timesheet_id, category),
    index(employee_id, status),
    index(run_id)
)]
pub struct HourDifference {
    /// The primary key.
    pub id: i64,
    /// The employee whose hours these are.
    #[model(references = "wl_hr_employees")]
    pub employee_id: i64,
    /// [`SOURCE_ADJUSTMENT`] or [`SOURCE_RECONCILIATION`].
    pub source: String,
    /// The adjustment timesheet's subject type.
    pub timesheet_type: String,
    /// The adjustment timesheet's id.
    pub timesheet_id: i64,
    /// `normal`, `overtime`, `night`, `rest_day` or `public_holiday`.
    pub category: String,
    /// The minutes; negative for hours taken back.
    pub minutes: i64,
    /// The day whose salary structure prices the hours.
    pub work_date: NaiveDate,
    /// The amount as last calculated, AED fils; negative for a recovery.
    pub amount: i64,
    /// The run that pays or recovers it.
    pub run_id: Option<i64>,
    /// [`OPEN`] or [`CONVERTED`].
    pub status: String,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl HourDifference {
    /// The adjustment timesheet that holds the hours.
    pub fn timesheet(&self) -> Subject {
        Subject::new(&self.timesheet_type, self.timesheet_id)
    }

    /// The differences run `run_id` pays or recovers, oldest first.
    pub async fn of_run(db: &Db, run_id: i64) -> Result<Vec<HourDifference>> {
        HourDifference::query()
            .where_(HourDifference::RUN_ID.eq(run_id))
            .order_by(HourDifference::ID)
            .get(db)
            .await
    }
}

/// Keeps the hours of `totals` as differences of `employee` for the timesheet
/// `timesheet`, once: a category already kept changes nothing.
async fn keep(
    state: &AppState,
    employee: i64,
    source: &str,
    timesheet: &Subject,
    totals: &[CategoryMinutes],
    work_date: NaiveDate,
) -> Result {
    let Some(employee) = Employee::find(&state.db, employee).await? else {
        return Ok(());
    };
    // A freelancer is paid through a supplier bill, not payroll.
    if employee.employment_type == EmploymentType::Freelance {
        return Ok(());
    }
    for total in totals.iter().filter(|t| t.minutes != 0) {
        let known = HourDifference::query()
            .where_(HourDifference::SOURCE.eq(source))
            .where_(HourDifference::TIMESHEET_TYPE.eq(timesheet.subject_type.as_str()))
            .where_(HourDifference::TIMESHEET_ID.eq(timesheet.subject_id))
            .where_(HourDifference::CATEGORY.eq(total.category.as_str()))
            .exists(&state.db)
            .await?;
        if known {
            continue;
        }
        HourDifference::create(
            &state.db,
            HourDifference {
                employee_id: employee.id,
                source: source.to_owned(),
                timesheet_type: timesheet.subject_type.clone(),
                timesheet_id: timesheet.subject_id,
                category: total.category.clone(),
                minutes: total.minutes,
                work_date,
                status: OPEN.to_owned(),
                ..HourDifference::default()
            },
        )
        .await?;
    }
    Ok(())
}

/// `TimesheetApproved` of an adjustment that affects payroll.
pub(crate) async fn on_approved(state: &AppState, event: &TimesheetApproved) -> Result {
    let affects_payroll = matches!(event.affects.as_deref(), Some("payroll" | "both"));
    if event.kind != "adjustment"
        || !affects_payroll
        || event.origin.as_deref() == Some("provisional_reconciliation")
    {
        return Ok(());
    }
    keep(
        state,
        event.owner.0,
        SOURCE_ADJUSTMENT,
        &event.timesheet,
        &event.totals,
        event.period_to,
    )
    .await
}

/// `TimesheetReconciliationNeeded`.
pub(crate) async fn on_reconciliation(
    state: &AppState,
    event: &TimesheetReconciliationNeeded,
) -> Result {
    let paid = match PayrollRun::find(&state.db, event.payroll_run.subject_id).await? {
        Some(run) => period_bounds(&run.period).map(|(_, to)| to),
        None => None,
    };
    keep(
        state,
        event.owner.0,
        SOURCE_RECONCILIATION,
        &event.adjustment,
        &event.delta,
        paid.unwrap_or_else(today),
    )
    .await
}

/// Turns the negative difference `id`, which a run still being prepared
/// recovers in full, into an *overpayment recovery* spread over the next
/// months within the caps (20% of the wage a month, 50% in total): the first
/// instalment is deducted in the run's period. The run has to be recalculated
/// afterwards.
pub async fn convert_to_recovery(state: &AppState, id: i64, note: &str) -> Result<Vec<Adjustment>> {
    let difference = HourDifference::find_or_404(&state.db, id).await?;
    let refuse = |key: &str| Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key, &[]));
    let run = match difference.run_id {
        Some(run_id) => PayrollRun::find(&state.db, run_id).await?,
        None => None,
    };
    let Some(run) = run.filter(|r| {
        matches!(
            r.status,
            PayrollRunStatus::Draft | PayrollRunStatus::Calculated
        )
    }) else {
        return Err(refuse("wl_hr.payroll.difference_not_in_open_run"));
    };
    if difference.status != OPEN || difference.amount >= 0 {
        return Err(refuse("wl_hr.payroll.difference_not_negative"));
    }
    let mut tx = state.db.begin().await?;
    let wage = PayrollLine::query()
        .where_(PayrollLine::RUN_ID.eq(run.id))
        .where_(PayrollLine::EMPLOYEE_ID.eq(difference.employee_id))
        .first(&mut tx)
        .await?
        .map_or(0, |line| line.fixed_wage);
    // The recovery in full made by `calculate` gives way to the plan.
    Adjustment::query()
        .where_(Adjustment::DIFFERENCE_ID.eq(difference.id))
        .delete(&mut tx)
        .await?;
    let note = match note.trim() {
        "" => english("wl_hr.payroll.difference_note", &[]),
        given => given.to_owned(),
    };
    let (created, left) = recovery::create_plan_in(
        &mut tx,
        difference.employee_id,
        wage,
        -difference.amount,
        &run.period,
        Some(difference.id),
        &note,
    )
    .await?;
    if left > 0 {
        // Nothing is stored: the caller sees the refusal.
        return Err(refuse("wl_hr.payroll.no_room_for_recovery"));
    }
    HourDifference::query()
        .where_(HourDifference::ID.eq(difference.id))
        .update(
            &mut tx,
            &[("status", &CONVERTED.to_owned()), ("updated_at", &now())],
        )
        .await?;
    tx.commit().await?;
    Ok(created)
}

/// The salary structure that prices `difference`: the one effective on its
/// day, else the one effective now.
pub(crate) async fn structure_for(
    tx: &mut renox::db::Transaction,
    difference: &HourDifference,
    current: &SalaryStructure,
) -> Result<SalaryStructure> {
    Ok(
        SalaryStructure::effective_in(&mut *tx, difference.employee_id, difference.work_date)
            .await?
            .unwrap_or_else(|| current.clone()),
    )
}
