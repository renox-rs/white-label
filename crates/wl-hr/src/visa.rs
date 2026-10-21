//! The visa case as far as the refusal and the exits need it (wl-hr §4 *Visa
//! case*, §8 `VisaStepTemplate`, issues #193, #195, #197): the *cancellation*
//! case that opens when an employee is reported as absconded, from the
//! *absconding* variant of the cancellation step template (it starts from the
//! MoHRE absence report and needs no worker signature), `cancel`, `reject` and
//! *cancellation not needed*. The steps, fees and the other transitions
//! (`start`, `hold`, `complete`) come with the visa issues. A step carries its
//! target number of days and an optional flag; [`OpenSteps`] lists the open
//! steps with their start day for the vertical's late-step rule (09:
//! `MPW.RUL.14`, BL-MPW-18). The step template that creates the steps with
//! their targets comes with the visa lifecycle issue.
//!
//! - **`reject`** (`visas.manage`; `in_progress` or `on_hold` to `rejected`):
//!   the authority's reference and the reason are recorded, fees already posted
//!   stay posted and the employee's state does not change (HR decides). After
//!   the commit [`on_rejected`] works out the follow-up: for a *new* case a task
//!   for HR to cancel onboarding or open another case; for a *renewal* a task
//!   due N days before the current residence visa expires (default 30; today
//!   when that day has passed) to open a case by another route or to give
//!   notice; the MD is told; `VisaCaseRejected` carries the deadline and the
//!   visa expiry so the vertical tells the active deployments. wl-hr does not
//!   call wl-documents and builds no `affected` list: `DocumentExpiring`
//!   (`affected`) comes from wl-documents' own `DOC.RUL.02` once the visa enters
//!   its look-ahead window. Other types (`cancellation`, `transfer`) get the
//!   event and nothing else (decision: spec silent).
//! - A *renewal* case carries the expiry of the visa it renews
//!   (`current_expiry`), copied from the visa document when the case opens
//!   (`HRM.RUL.04`).
//! - **`cancel_by_system`** is `cancel` run by `withdraw_notice` and
//!   `reinstate` ([`cancel_open_cancellations`]).
//! - **Cancellation not needed** ([`record_cancellation_not_needed`]): on a
//!   *new* case whose permit was issued, a record by a user with `visas.manage`
//!   with a reason. `cancel_onboarding` accepts it in place of a completed
//!   cancellation case.

use renox::chrono::{Duration, FixedOffset, NaiveDate, TimeZone, Utc};
use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionError, Transitioned};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::{DUBAI_OFFSET_SECONDS, dubai_date, today};
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::visa_steps::{OpenVisaStep, VisaSteps, VisaStepsFuture};
use wl_kernel::workforce::{EmployeeId, VISA_CASE_SUBJECT_TYPE};
use wl_kernel::{Subject, VisaCaseRejected};

use crate::config::config_of;
use crate::employee::Employee;
use crate::notices;
use crate::permissions;
use crate::settlement_events::users_of;
use crate::texts::english;

/// A cancellation case.
pub const TYPE_CANCELLATION: &str = "cancellation";
/// A *new* visa case (a candidate or new hire).
pub const TYPE_NEW: &str = "new";
/// A *renewal* case of a residence visa.
pub const TYPE_RENEWAL: &str = "renewal";

/// The work permit step: when done, the permit was issued.
pub const STEP_WORK_PERMIT: &str = "work_permit";
/// The entry permit step: when done, the permit was issued.
pub const STEP_ENTRY_PERMIT: &str = "entry_permit";

/// The standard variant of the cancellation step template.
pub const VARIANT_STANDARD: &str = "standard";
/// The *absconding* variant: it starts from the MoHRE absence report.
pub const VARIANT_ABSCONDING: &str = "absconding";
/// The *death* variant: it starts from the death certificate.
pub const VARIANT_DEATH: &str = "death";

/// Abandoned (the exit was reversed).
pub const CANCEL: &str = "cancel";
/// `cancel` run by the system for `withdraw_notice` and `reinstate`.
pub const CANCEL_BY_SYSTEM: &str = "cancel_by_system";
/// Refused by the authority.
pub const REJECT: &str = "reject";

/// What `reject` is given besides the reason: the authority's reference.
#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq, Eq)]
#[serde(default)]
pub struct RejectInput {
    /// The authority's reference of the refusal.
    pub reference: String,
}

/// The states of a case that is not finished.
pub const LIVE: [VisaCaseStatus; 3] = [
    VisaCaseStatus::Open,
    VisaCaseStatus::InProgress,
    VisaCaseStatus::OnHold,
];

/// Where a visa case stands (§4).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum VisaCaseStatus {
    /// Created; no step done.
    #[default]
    Open,
    /// At least one step done.
    InProgress,
    /// Waiting.
    OnHold,
    /// All mandatory steps done.
    Completed,
    /// Refused by the authority.
    Rejected,
    /// Abandoned.
    Cancelled,
}

/// One visa process.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_hr_visa_cases",
    state_machine,
    index(employee_id, case_type, status)
)]
pub struct VisaCase {
    /// The primary key.
    pub id: i64,
    /// The employee.
    #[model(references = "wl_hr_employees")]
    pub employee_id: i64,
    /// `new`, `renewal` or `cancellation`.
    pub case_type: String,
    /// The step template variant of a cancellation.
    pub variant: String,
    /// Where it stands; changed only by the machine.
    pub status: VisaCaseStatus,
    /// The MoHRE absence report an absconding case starts from.
    pub report_number: Option<String>,
    /// The date of that report.
    pub report_date: Option<NaiveDate>,
    /// For a *renewal*: the expiry date of the residence visa it renews.
    pub current_expiry: Option<NaiveDate>,
    /// The authority's reference of the refusal (`reject`).
    pub rejection_reference: Option<String>,
    /// Why the authority refused (`reject`).
    pub rejection_reason: Option<String>,
    /// The day the refusal was recorded.
    pub rejected_on: Option<NaiveDate>,
    /// For a refused *renewal*: the day the follow-up task is due.
    pub deadline: Option<NaiveDate>,
    /// The user who recorded *cancellation not needed*.
    pub cancellation_not_needed_by: Option<i64>,
    /// Why no cancellation case is needed.
    pub cancellation_not_needed_reason: Option<String>,
    /// The day *cancellation not needed* was recorded.
    pub cancellation_not_needed_on: Option<NaiveDate>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl StateMachine for VisaCase {
    type State = VisaCaseStatus;
    const COLUMN: renox::db::Col<VisaCase, VisaCaseStatus> = VisaCase::STATUS;

    fn machine() -> Machine<Self> {
        use VisaCaseStatus::{Cancelled, Completed, InProgress, OnHold, Open, Rejected};
        Machine::new()
            .initial([Open])
            .state(Open)
            .tone("neutral")
            .state(InProgress)
            .tone("info")
            .state(OnHold)
            .tone("warning")
            .state(Completed)
            .tone("success")
            .terminal()
            .state(Rejected)
            .tone("error")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .transition(
                Transition::new(CANCEL)
                    .label("Cancel")
                    .permission(permissions::VISAS_MANAGE)
                    .from([Open, InProgress, OnHold])
                    .to(Cancelled)
                    .reason_required(),
            )
            .transition(
                Transition::new(CANCEL_BY_SYSTEM)
                    .label("Cancel")
                    .system()
                    .from([Open, InProgress, OnHold])
                    .to(Cancelled)
                    .reason_required(),
            )
            .transition(
                Transition::new(REJECT)
                    .label("Reject")
                    .tone("danger")
                    .permission(permissions::VISAS_MANAGE)
                    .from([InProgress, OnHold])
                    .to(Rejected)
                    .reason_required()
                    .effect(|case: &VisaCase, ctx, tx| {
                        let id = case.id;
                        let reference = ctx
                            .input_as::<RejectInput>()
                            .unwrap_or_default()
                            .reference
                            .trim()
                            .to_owned();
                        let reason = ctx.reason().unwrap_or_default().trim().to_owned();
                        Box::pin(async move {
                            // Not a guard: a guard reads no input when the page
                            // asks which actions are offered.
                            if reference.is_empty() {
                                return Err(Error::Status(
                                    StatusCode::UNPROCESSABLE_ENTITY,
                                    english("wl_hr.visa.reference_required", &[]),
                                ));
                            }
                            VisaCase::query()
                                .where_(VisaCase::ID.eq(id))
                                .update(
                                    &mut *tx,
                                    &[
                                        ("rejection_reference", &Some(reference)),
                                        ("rejection_reason", &Some(reason)),
                                        ("rejected_on", &Some(today())),
                                        ("updated_at", &now()),
                                    ],
                                )
                                .await?;
                            Ok(())
                        })
                    }),
            )
    }
}

impl VisaCase {
    /// The cancellation cases of `employee`, oldest first.
    pub async fn cancellations_of(db: &Db, employee: i64) -> Result<Vec<VisaCase>> {
        VisaCase::query()
            .where_(VisaCase::EMPLOYEE_ID.eq(employee))
            .where_(VisaCase::CASE_TYPE.eq(TYPE_CANCELLATION))
            .order_by(VisaCase::ID)
            .get(db)
            .await
    }
}

/// Opens the *cancellation* case of `employee_id` from the *absconding*
/// variant of the step template, started by the absence report `report`
/// (number and date), inside `tx`. A cancellation case that is already open
/// (the one `give_notice` opened) is not duplicated: it becomes the
/// absconding variant and takes the report.
pub async fn open_absconding_in(
    tx: &mut Transaction,
    employee_id: i64,
    report_number: &str,
    report_date: NaiveDate,
) -> Result<VisaCase> {
    open_variant_in(
        tx,
        employee_id,
        VARIANT_ABSCONDING,
        Some(report_number),
        Some(report_date),
    )
    .await
}

/// Opens the *cancellation* case of `employee_id` from the *death* variant of
/// the step template (it starts from the death certificate), inside `tx`.
/// An open cancellation case (the one `give_notice` or `report_absconding`
/// opened) becomes the death variant; no second one is opened.
pub async fn open_death_in(tx: &mut Transaction, employee_id: i64) -> Result<VisaCase> {
    open_variant_in(tx, employee_id, VARIANT_DEATH, None, None).await
}

async fn open_variant_in(
    tx: &mut Transaction,
    employee_id: i64,
    variant: &str,
    report_number: Option<&str>,
    report_date: Option<NaiveDate>,
) -> Result<VisaCase> {
    let open = VisaCase::query()
        .where_(VisaCase::EMPLOYEE_ID.eq(employee_id))
        .where_(VisaCase::CASE_TYPE.eq(TYPE_CANCELLATION))
        .where_(VisaCase::STATUS.is_in([
            VisaCaseStatus::Open,
            VisaCaseStatus::InProgress,
            VisaCaseStatus::OnHold,
        ]))
        .order_by_desc(VisaCase::ID)
        .first(&mut *tx)
        .await?;
    if let Some(mut case) = open {
        let number = report_number
            .map(str::to_owned)
            .or(case.report_number.clone());
        let date = report_date.or(case.report_date);
        VisaCase::query()
            .where_(VisaCase::ID.eq(case.id))
            .update(
                &mut *tx,
                &[
                    ("variant", &variant.to_owned()),
                    ("report_number", &number),
                    ("report_date", &date),
                    ("updated_at", &now()),
                ],
            )
            .await?;
        case.variant = variant.to_owned();
        case.report_number = number;
        case.report_date = date;
        return Ok(case);
    }
    let mut case = VisaCase {
        employee_id,
        case_type: TYPE_CANCELLATION.to_owned(),
        variant: variant.to_owned(),
        status: VisaCaseStatus::Open,
        report_number: report_number.map(str::to_owned),
        report_date,
        ..VisaCase::default()
    };
    case.insert(&mut *tx).await?;
    Ok(case)
}

/// One step of a visa case. Only what the exits read is modelled so far: a
/// work permit or entry permit step that is done means the permit was issued.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_hr_visa_case_steps", index(case_id))]
pub struct VisaCaseStep {
    /// The primary key.
    pub id: i64,
    /// The case.
    #[model(references = "wl_hr_visa_cases")]
    pub case_id: i64,
    /// `work_permit`, `entry_permit`, `medical_fitness`, ...
    pub step_type: String,
    /// The day it was done; empty while open.
    pub done_on: Option<NaiveDate>,
    /// The reference number.
    pub reference: Option<String>,
    /// The target number of days from the previous mandatory step (or the
    /// case's creation); empty: no target, never late (`MPW.RUL.14`). The
    /// targets are the configured ones, not statutory limits (`AU-21`).
    pub target_days: Option<i64>,
    /// Whether the step is optional: an optional step does not start the next
    /// one when it is done.
    pub optional: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl VisaCase {
    /// The case as a subject (`hr.visa_case:7`).
    pub fn subject(&self) -> Subject {
        Subject::new(VISA_CASE_SUBJECT_TYPE, self.id)
    }

    /// The cases of `employee` of `case_type`, oldest first, inside `tx`.
    pub async fn of_type_in(
        tx: &mut Transaction,
        employee: i64,
        case_type: &str,
    ) -> Result<Vec<VisaCase>> {
        VisaCase::query()
            .where_(VisaCase::EMPLOYEE_ID.eq(employee))
            .where_(VisaCase::CASE_TYPE.eq(case_type))
            .order_by(VisaCase::ID)
            .get(&mut *tx)
            .await
    }

    /// Whether the case is still being worked.
    pub fn is_live(&self) -> bool {
        LIVE.contains(&self.status)
    }
}

/// Whether a work permit or entry permit step of a *new* case of `employee` is
/// done: the permit was issued.
pub async fn permit_issued_in(tx: &mut Transaction, employee: i64) -> Result<bool> {
    let ids: Vec<i64> = VisaCase::of_type_in(&mut *tx, employee, TYPE_NEW)
        .await?
        .into_iter()
        .map(|case| case.id)
        .collect();
    if ids.is_empty() {
        return Ok(false);
    }
    let steps = VisaCaseStep::query()
        .where_(VisaCaseStep::CASE_ID.is_in(ids))
        .where_(VisaCaseStep::DONE_ON.is_not_null())
        .get(&mut *tx)
        .await?;
    Ok(steps
        .iter()
        .any(|step| step.step_type == STEP_WORK_PERMIT || step.step_type == STEP_ENTRY_PERMIT))
}

/// The PRO (`visas.manage`) records the authority's refusal of case `id`:
/// `reference` and `reason` are required.
///
/// # Errors
///
/// 403 without `visas.manage`; 422 when a reference or reason is missing or
/// the case is not `in_progress` or `on_hold`.
pub async fn reject(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    reference: &str,
    reason: &str,
) -> Result<VisaCase> {
    let case = VisaCase::find_or_404(&state.db, id).await?;
    let _outcome = case
        .transition(REJECT)
        .by(by)
        .reason(reason.trim())
        .input(&RejectInput {
            reference: reference.to_owned(),
        })
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    VisaCase::find_or_404(&state.db, id).await
}

/// 18:00 on `day` in Asia/Dubai: when a task due on that day is due.
fn due_at(day: NaiveDate) -> Option<DateTime> {
    let dubai = FixedOffset::east_opt(DUBAI_OFFSET_SECONDS)?;
    let local = day.and_hms_opt(18, 0, 0)?;
    dubai
        .from_local_datetime(&local)
        .single()
        .map(|at| at.with_timezone(&Utc))
}

/// What follows a `reject`, after the commit: the deadline is worked out and
/// kept on the case, `VisaCaseRejected` is emitted, HR gets its task and the
/// MD is told. A task is opened once per case (its source key).
pub(crate) async fn on_rejected(state: &AppState, id: i64) -> Result {
    let Some(case) = VisaCase::find(&state.db, id).await? else {
        return Ok(());
    };
    let Some(employee) = Employee::find(&state.db, case.employee_id).await? else {
        return Ok(());
    };
    let config = config_of(state);
    let day = today();
    let renewal = case.case_type == TYPE_RENEWAL;
    let deadline = renewal.then(|| {
        case.current_expiry
            .map_or(day, |expiry| {
                expiry - Duration::days(config.visa_rejection_task_days)
            })
            .max(day)
    });
    if deadline.is_some() {
        VisaCase::query()
            .where_(VisaCase::ID.eq(id))
            .update(
                &state.db,
                &[("deadline", &deadline), ("updated_at", &now())],
            )
            .await?;
    }
    let visa_expiry = if renewal { case.current_expiry } else { None };
    let name = notices::name_of(state, &employee).await;
    let key = format!("hr.visa_rejected:{id}");
    let task = if case.case_type == TYPE_NEW {
        let title = english("wl_hr.visa.task_new_title", &[("employee", &name)]);
        let description = english("wl_hr.visa.task_new_description", &[]);
        Some(
            RoleTask::new(employee.subject(), &config.hr_role, &title, &key)
                .description(&description),
        )
    } else if let Some(due) = deadline {
        let title = english("wl_hr.visa.task_renewal_title", &[("employee", &name)]);
        let description = english(
            "wl_hr.visa.task_renewal_description",
            &[("deadline", &due.to_string())],
        );
        let task = RoleTask::new(employee.subject(), &config.hr_role, &title, &key)
            .description(&description);
        Some(match due_at(due) {
            Some(at) => task.due_at(at),
            None => task,
        })
    } else {
        None
    };
    if let Some(task) = task {
        role_tasks::open(state, task).await?;
    }
    let md = users_of(state, &[&config.md_role]).await?;
    notices::visa_rejected(state, &case, &employee, &name, deadline, &md).await?;
    state
        .emit(VisaCaseRejected {
            case: case.subject(),
            case_type: case.case_type.clone(),
            employee: EmployeeId(case.employee_id),
            deadline,
            visa_expiry,
        })
        .await
}

/// The open steps of the live cases (`MPW.RUL.14` reads them through the
/// kernel's [`VisaSteps`]): a step that is not done and has a target, started on
/// the day the previous mandatory step was done, or on the day the case was
/// created (Asia/Dubai) for the first one. A step whose previous mandatory step
/// is not done has not started and is not listed.
#[derive(Debug, Clone, Copy, Default)]
pub struct OpenSteps;

impl OpenSteps {
    async fn collect(db: &Db, only: Option<i64>) -> Result<Vec<OpenVisaStep>> {
        let cases = VisaCase::query()
            .where_(VisaCase::STATUS.is_in(LIVE))
            .order_by(VisaCase::ID)
            .get(db)
            .await?;
        let mut open = Vec::new();
        for case in cases {
            let steps = VisaCaseStep::query()
                .where_(VisaCaseStep::CASE_ID.eq(case.id))
                .order_by(VisaCaseStep::ID)
                .get(db)
                .await?;
            let mut start = case.created_at.map(dubai_date);
            for step in steps {
                let Some(started_on) = start else { break };
                if step.done_on.is_none()
                    && let Some(target_days) = step.target_days
                    && only.is_none_or(|id| id == step.id)
                {
                    open.push(OpenVisaStep {
                        step: step.id,
                        case: case.subject(),
                        employee: EmployeeId(case.employee_id),
                        case_type: case.case_type.clone(),
                        step_type: step.step_type.clone(),
                        started_on,
                        target_days,
                    });
                }
                if !step.optional {
                    start = step.done_on;
                }
            }
        }
        Ok(open)
    }
}

impl VisaSteps for OpenSteps {
    fn open_steps<'a>(&'a self, state: &'a AppState) -> VisaStepsFuture<'a, Vec<OpenVisaStep>> {
        Box::pin(async move { Self::collect(&state.db, None).await })
    }

    fn is_open<'a>(&'a self, state: &'a AppState, step: i64) -> VisaStepsFuture<'a, bool> {
        Box::pin(async move { Ok(!Self::collect(&state.db, Some(step)).await?.is_empty()) })
    }
}

/// The listener of the case's transitions.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: Transitioned<VisaCase>, state: AppState| async move {
            if event.transition == REJECT {
                return on_rejected(&state, event.key).await;
            }
            Ok(())
        },
    );
}

/// Cancels the live *cancellation* cases of `employee_id` as the system, with
/// `reason`: what `withdraw_notice` and `reinstate` call. Returns how many.
pub async fn cancel_open_cancellations(
    state: &AppState,
    employee_id: i64,
    reason: &str,
) -> Result<usize> {
    let mut cancelled = 0;
    for case in VisaCase::cancellations_of(&state.db, employee_id).await? {
        if !case.is_live() {
            continue;
        }
        let _outcome = case
            .transition(CANCEL_BY_SYSTEM)
            .by_system()
            .reason(reason)
            .run(state)
            .await
            .map_err(TransitionError::into_error)?;
        cancelled += 1;
    }
    Ok(cancelled)
}

/// The PRO (`visas.manage`) records that no cancellation case is needed for the
/// *new* case `id`, whose permit was issued: who, why and when.
///
/// # Errors
///
/// 403 without `visas.manage`; 422 without a reason, for a case that is not a
/// *new* case, or when no permit was issued.
pub async fn record_cancellation_not_needed(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    reason: &str,
) -> Result<VisaCase> {
    if !by.has_permission(permissions::VISAS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let refuse = |key: &str| Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key, &[]));
    let case = VisaCase::find_or_404(&state.db, id).await?;
    if reason.trim().is_empty() {
        return Err(refuse("wl_hr.visa.not_needed_reason_required"));
    }
    if case.case_type != TYPE_NEW {
        return Err(refuse("wl_hr.visa.not_needed_new_only"));
    }
    let issued = {
        let mut tx = state.db.begin().await?;
        let steps = VisaCaseStep::query()
            .where_(VisaCaseStep::CASE_ID.eq(id))
            .where_(VisaCaseStep::DONE_ON.is_not_null())
            .get(&mut tx)
            .await?;
        steps
            .iter()
            .any(|s| s.step_type == STEP_WORK_PERMIT || s.step_type == STEP_ENTRY_PERMIT)
    };
    if !issued {
        return Err(refuse("wl_hr.visa.not_needed_no_permit"));
    }
    VisaCase::query()
        .where_(VisaCase::ID.eq(id))
        .update(
            &state.db,
            &[
                ("cancellation_not_needed_by", &Some(by.id)),
                (
                    "cancellation_not_needed_reason",
                    &Some(reason.trim().to_owned()),
                ),
                ("cancellation_not_needed_on", &Some(today())),
                ("updated_at", &now()),
            ],
        )
        .await?;
    VisaCase::find_or_404(&state.db, id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_case_starts_open_and_cancel_leaves_the_live_states() {
        let machine = VisaCase::machine();
        assert_eq!(machine.initial_states(), ["open"]);
        let cancel = machine.find(CANCEL).unwrap();
        assert_eq!(cancel.from_states(), ["open", "in_progress", "on_hold"]);
        assert_eq!(cancel.to_state(), Some("cancelled"));
    }

    #[test]
    fn reject_leaves_in_progress_and_on_hold_only() {
        let machine = VisaCase::machine();
        let reject = machine.find(REJECT).unwrap();
        assert_eq!(reject.from_states(), ["in_progress", "on_hold"]);
        assert_eq!(reject.to_state(), Some("rejected"));
        assert!(!reject.is_system());
        let twin = machine.find(CANCEL_BY_SYSTEM).unwrap();
        assert!(twin.is_system());
        assert_eq!(twin.to_state(), Some("cancelled"));
    }
}
