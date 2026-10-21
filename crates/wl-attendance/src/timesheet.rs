//! The timesheet and its machine (wl-attendance §3, §4 *Timesheet*): the
//! hours of one owner for one period, waiting for approval.
//!
//! ```text
//! [*] --create--> open
//! [*] --open_adjustment--> open          (kind adjustment, BL-ATT-07)
//! open --submit--> submitted
//! submitted --approve | approve_on_behalf--> approved
//! submitted --reject--> open
//! approved --reopen--> open
//! approved --lock--> locked
//! ```
//!
//! `approve`, `approve_on_behalf` and `reject` (submitted to open) are in
//! [`crate::approval`] (BL-ATT-08); `deem_approve` comes with the deemed
//! approval issue.
//!
//! The status moves only through the machine. `locked` is terminal: a locked
//! timesheet is never edited or reopened (`HRM.ACL.08`), and is corrected by
//! an adjustment ([`crate::adjustment`]). `adjusted_by` of the original is
//! the list of the adjustments that point to it ([`Timesheet::adjusted_by`]):
//! the link is the adjustment's `adjusts_id`, so opening one writes nothing
//! on the original.
//!
//! Decisions:
//!
//! - The owner is an employee. A crew as owner comes with the crews.
//! - Whether the hours come from attendance is a property of the timesheet
//!   ([`HoursSource`]), set when the context creates it (02 and 09 use
//!   attendance; 04 and 10 do not); the checks of `submit` read it.
//! - `reopen` is refused for a consumed timesheet; reversing the labour cost
//!   and unlocking the attendance come with `approve` (BL-ATT-08).

use renox::chrono::NaiveDate;
use renox::db::{DateTime, now};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::workforce::EmployeeId;

use crate::service::refuse;
use crate::texts::english;

/// The subject type of a timesheet, for [`Subject`] references.
pub const TIMESHEET_SUBJECT_TYPE: &str = "attendance.timesheet";

/// The system transition that locks an approved timesheet on its first use.
pub const LOCK: &str = "lock";
/// A user reopens an approved timesheet that nothing has used.
pub const REOPEN: &str = "reopen";

/// Where a timesheet stands (§4).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TimesheetStatus {
    /// Hours being collected or entered for the period.
    #[default]
    Open,
    /// Waiting for the approver.
    Submitted,
    /// Approved; attendance locked; can still be reopened.
    Approved,
    /// Consumed by a payroll run, an invoice or an export; never changes.
    Locked,
}

/// A regular timesheet or the correction of a locked one.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TimesheetKind {
    /// Hours of an owner for a period.
    #[default]
    Regular,
    /// Signed hours that correct a `locked` regular timesheet.
    Adjustment,
}

/// Where the hours of a timesheet come from (§4 vertical mapping).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HoursSource {
    /// Attendance, allocated to cost objects (02, 09).
    #[default]
    Attendance,
    /// Entered by hand (04).
    Manual,
    /// Generated from sessions taught or time logged (10).
    Generated,
}

/// Who approves a timesheet.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ApproverKind {
    /// Staff.
    #[default]
    Internal,
    /// The client's approver, through the portal or a signed emailed link.
    External,
}

/// How a timesheet was approved.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ApprovalBasis {
    /// By the approver.
    #[default]
    Approved,
    /// By staff for the external approver, with signed evidence.
    OnBehalf,
    /// Deemed approved after the days of the contract's clause.
    Deemed,
}

/// Why an adjustment is opened.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AdjustmentOrigin {
    /// The client disputed billed hours.
    #[default]
    ClientDispute,
    /// The attendance was wrong.
    AttendanceError,
    /// Payroll was wrong.
    PayrollError,
    /// Provisional hours were reconciled when the timesheet was approved.
    ProvisionalReconciliation,
}

impl AdjustmentOrigin {
    /// The origin named `name` (`client_dispute`, `attendance_error`,
    /// `payroll_error`, `provisional_reconciliation`).
    pub fn parse(name: &str) -> Option<AdjustmentOrigin> {
        Some(match name.trim() {
            "client_dispute" => AdjustmentOrigin::ClientDispute,
            "attendance_error" => AdjustmentOrigin::AttendanceError,
            "payroll_error" => AdjustmentOrigin::PayrollError,
            "provisional_reconciliation" => AdjustmentOrigin::ProvisionalReconciliation,
            _ => return None,
        })
    }
}

/// Where an adjustment flows.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Affects {
    /// Billing only.
    #[default]
    Billing,
    /// Payroll only.
    Payroll,
    /// Both.
    Both,
}

impl Affects {
    /// The value named `name` (`billing`, `payroll`, `both`).
    pub fn parse(name: &str) -> Option<Affects> {
        Some(match name.trim() {
            "billing" => Affects::Billing,
            "payroll" => Affects::Payroll,
            "both" => Affects::Both,
            _ => return None,
        })
    }
}

/// What used a timesheet (the consumers of §3).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Consumer {
    /// A usage invoice or WIP billing (`billed_by`).
    Billing,
    /// A payroll run (`paid_by`).
    Payroll,
    /// Provisional payroll use (`paid_provisionally_by`).
    PayrollProvisional,
    /// An export (`exported_at`).
    Export,
}

/// The input of `lock`: the first consumer and its reference.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct LockInput {
    /// What used the timesheet.
    pub consumer: Consumer,
    /// The record that used it, e.g. `hr.payroll_run:5`.
    pub reference: String,
}

/// The hours of one owner for one period.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_attendance_timesheets",
    state_machine,
    index(owner_employee_id, period_from),
    index(adjusts_id)
)]
pub struct Timesheet {
    /// The primary key.
    pub id: i64,
    /// The employee whose hours these are.
    pub owner_employee_id: i64,
    /// The first day.
    pub period_from: NaiveDate,
    /// The last day.
    pub period_to: NaiveDate,
    /// The type of the context's `Subject` (deployment, project...).
    pub context_type: Option<String>,
    /// The id of the context's `Subject`.
    pub context_id: Option<i64>,
    /// Regular or adjustment.
    pub kind: TimesheetKind,
    /// Where the hours come from.
    pub hours_source: HoursSource,
    /// Who approves.
    pub approver_kind: ApproverKind,
    /// The approver's role.
    pub approver_role: Option<String>,
    /// The external approver's party.
    pub approver_party: Option<i64>,
    /// How it was approved.
    pub approval_basis: Option<ApprovalBasis>,
    /// What billed it.
    pub billed_by: Option<String>,
    /// What paid it.
    pub paid_by: Option<String>,
    /// What paid it provisionally.
    pub paid_provisionally_by: Option<String>,
    /// When it was exported.
    pub exported_at: Option<DateTime>,
    /// An adjustment: the locked timesheet it corrects.
    pub adjusts_id: Option<i64>,
    /// An adjustment: why.
    pub reason: Option<String>,
    /// An adjustment: the origin.
    pub origin: Option<AdjustmentOrigin>,
    /// An adjustment: the type of the linked ticket or dispute.
    pub linked_type: Option<String>,
    /// An adjustment: the id of the linked ticket or dispute.
    pub linked_id: Option<i64>,
    /// An adjustment: where it flows.
    pub affects: Option<Affects>,
    /// An adjustment of a client dispute: the credit note the dispute was
    /// resolved with (`invoicing.credit_note`), once it is.
    pub credit_note_id: Option<i64>,
    /// An adjustment of a client dispute: when the dispute ended `upheld`
    /// before the adjustment was approved; its `submit` is refused from then.
    pub dispute_upheld_at: Option<DateTime>,
    /// The user who created it.
    pub created_by: Option<i64>,
    /// The user who submitted it; none for the system.
    pub submitted_by: Option<i64>,
    /// When it was submitted.
    pub submitted_at: Option<DateTime>,
    /// When the approver was escalated to (`HRM.RUL.15` at E days) for the
    /// latest `submit`; cleared by the next `submit`.
    pub escalated_at: Option<DateTime>,
    /// The user who approved it; none for the system (a deemed approval).
    pub approved_by: Option<i64>,
    /// When it was approved.
    pub approved_at: Option<DateTime>,
    /// Where it stands; changed only by the machine.
    pub status: TimesheetStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Timesheet {
    /// The timesheet as a [`Subject`]: `attendance.timesheet:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(TIMESHEET_SUBJECT_TYPE, self.id)
    }

    /// The owner as an id.
    pub fn owner(&self) -> EmployeeId {
        EmployeeId(self.owner_employee_id)
    }

    /// The context (deployment, project, engagement...), if any.
    pub fn context(&self) -> Option<Subject> {
        match (&self.context_type, self.context_id) {
            (Some(kind), Some(id)) => Some(Subject::new(kind, id)),
            _ => None,
        }
    }

    /// The linked ticket or invoice dispute of an adjustment, if any.
    pub fn linked(&self) -> Option<Subject> {
        match (&self.linked_type, self.linked_id) {
            (Some(kind), Some(id)) => Some(Subject::new(kind, id)),
            _ => None,
        }
    }

    /// Whether a payroll run, an invoice or an export used it, also
    /// provisionally.
    pub fn consumed(&self) -> bool {
        self.billed_by.is_some()
            || self.paid_by.is_some()
            || self.paid_provisionally_by.is_some()
            || self.exported_at.is_some()
    }

    /// Whether it is an adjustment.
    pub fn is_adjustment(&self) -> bool {
        self.kind == TimesheetKind::Adjustment
    }

    /// The adjustments of this timesheet (`adjusted_by`), oldest first.
    pub async fn adjusted_by(&self, db: &Db) -> Result<Vec<Timesheet>> {
        Timesheet::query()
            .where_(Timesheet::ADJUSTS_ID.eq(self.id))
            .order_by(Timesheet::ID)
            .get(db)
            .await
    }

    /// A new `open` regular timesheet of `owner` for `from` to `to`, hours
    /// from attendance, approved internally.
    pub fn regular(owner: EmployeeId, from: NaiveDate, to: NaiveDate) -> Timesheet {
        Timesheet {
            owner_employee_id: owner.0,
            period_from: from,
            period_to: to,
            ..Timesheet::default()
        }
    }

    /// For `context`.
    pub fn in_context(mut self, context: &Subject) -> Timesheet {
        self.context_type = Some(context.subject_type.clone());
        self.context_id = Some(context.subject_id);
        self
    }

    /// With hours from `source`.
    pub fn from_source(mut self, source: HoursSource) -> Timesheet {
        self.hours_source = source;
        self
    }

    /// Created by user `user`.
    pub fn created_by(mut self, user: i64) -> Timesheet {
        self.created_by = Some(user);
        self
    }
}

impl StateMachine for Timesheet {
    type State = TimesheetStatus;
    const COLUMN: renox::db::Col<Timesheet, TimesheetStatus> = Timesheet::STATUS;

    fn machine() -> Machine<Self> {
        use TimesheetStatus::{Approved, Locked, Open, Submitted};
        let machine = Machine::new()
            .initial([Open])
            .state(Open)
            .tone("neutral")
            .state(Submitted)
            .tone("info")
            .state(Approved)
            .tone("success")
            .state(Locked)
            .tone("success")
            .terminal()
            .transition(
                Transition::new(LOCK)
                    .label("Lock")
                    .system()
                    .from([Approved])
                    .to(Locked)
                    .effect(|sheet: &Timesheet, ctx, tx| {
                        let id = sheet.id;
                        let input = ctx.input_as::<LockInput>();
                        Box::pin(async move {
                            let input = input.map_err(|_| {
                                refuse(english("wl_attendance.timesheet.no_consumer", &[]))
                            })?;
                            let column = match input.consumer {
                                Consumer::Billing => "billed_by",
                                Consumer::Payroll => "paid_by",
                                Consumer::PayrollProvisional => "paid_provisionally_by",
                                Consumer::Export => "exported_at",
                            };
                            // The consumer is a column, not the status.
                            if input.consumer == Consumer::Export {
                                Timesheet::query()
                                    .where_(Timesheet::ID.eq(id))
                                    .update(
                                        &mut *tx,
                                        &[(column, &Some(now())), ("updated_at", &now())],
                                    )
                                    .await?;
                            } else {
                                Timesheet::query()
                                    .where_(Timesheet::ID.eq(id))
                                    .update(
                                        &mut *tx,
                                        &[(column, &Some(input.reference)), ("updated_at", &now())],
                                    )
                                    .await?;
                            }
                            Ok(())
                        })
                    }),
            )
            .transition(
                Transition::new(REOPEN)
                    .label("Reopen")
                    .permission(crate::permissions::TIMESHEETS_REOPEN)
                    .from([Approved])
                    .to(Open)
                    .reason_required()
                    .guard("not_consumed", |sheet: &Timesheet, _ctx| {
                        let consumed = sheet.consumed();
                        let provisional = sheet.paid_provisionally_by.is_some();
                        Box::pin(async move {
                            if provisional {
                                Err(english("wl_attendance.timesheet.paid_provisionally", &[]))
                            } else if consumed {
                                Err(english("wl_attendance.timesheet.consumed", &[]))
                            } else {
                                Ok(())
                            }
                        })
                    }),
            );
        crate::submit::transitions()
            .into_iter()
            .chain(crate::approval::transitions())
            .fold(machine, Machine::transition)
    }
}

/// Locks the `approved` timesheet `timesheet_id`: `consumer` used it first
/// (`reference` names the payroll run, invoice or export).
///
/// # Errors
///
/// `NotFound`; `422` for a timesheet that is not `approved`.
pub async fn lock(
    state: &AppState,
    timesheet_id: i64,
    consumer: Consumer,
    reference: &str,
) -> Result<Timesheet> {
    let sheet = Timesheet::find_or_404(&state.db, timesheet_id).await?;
    let outcome = sheet
        .transition(LOCK)
        .by_system()
        .input(&LockInput {
            consumer,
            reference: reference.trim().to_owned(),
        })
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    outcome.emit(state).await?;
    Timesheet::find_or_404(&state.db, timesheet_id).await
}

/// `actor` reopens the `approved` timesheet `timesheet_id` with a `reason`
/// (`timesheets.reopen`).
///
/// # Errors
///
/// `NotFound`; `Forbidden` without the permission; `422` without a reason,
/// for a timesheet a consumer used, or one that is not `approved` (a `locked`
/// timesheet is never reopened, `HRM.ACL.08`).
pub async fn reopen(
    state: &AppState,
    actor: &AuthUser,
    timesheet_id: i64,
    reason: &str,
) -> Result<Timesheet> {
    let sheet = Timesheet::find_or_404(&state.db, timesheet_id).await?;
    let mut run = sheet.transition(REOPEN).by(actor);
    if !reason.trim().is_empty() {
        run = run.reason(reason.trim().to_owned());
    }
    let outcome = run.run(state).await.map_err(|e| e.into_error())?;
    outcome.emit(state).await?;
    Timesheet::find_or_404(&state.db, timesheet_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_timesheet_starts_open_and_locked_is_terminal() {
        let machine = Timesheet::machine();
        assert_eq!(machine.initial_states(), ["open"]);
        assert_eq!(machine.find(LOCK).unwrap().from_states(), ["approved"]);
        assert_eq!(machine.find(LOCK).unwrap().to_state(), Some("locked"));
        assert!(machine.find(LOCK).unwrap().is_system());
        assert_eq!(machine.find(REOPEN).unwrap().from_states(), ["approved"]);
        assert_eq!(
            machine.find("submit").unwrap().to_state(),
            Some("submitted")
        );
        assert!(machine.find("submit_by_system").unwrap().is_system());
    }

    #[test]
    fn an_origin_and_an_affect_are_read_by_name() {
        assert_eq!(
            AdjustmentOrigin::parse("attendance_error"),
            Some(AdjustmentOrigin::AttendanceError)
        );
        assert_eq!(AdjustmentOrigin::parse("oops"), None);
        assert_eq!(Affects::parse(" both "), Some(Affects::Both));
        assert_eq!(Affects::parse(""), None);
    }

    #[test]
    fn any_consumer_makes_a_timesheet_consumed() {
        let mut sheet = Timesheet::default();
        assert!(!sheet.consumed());
        sheet.paid_provisionally_by = Some("hr.payroll_run:1".into());
        assert!(sheet.consumed());
    }
}
