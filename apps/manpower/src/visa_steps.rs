//! A visa case step that is late (09 Manpower §4.2 row *A visa step is late*,
//! §7.2 `MPW.RUL.14`; BL-MPW-18).
//!
//! Each step of a visa case has a target number of days (wl-hr's step
//! template). The rule runs daily at 02:00 Asia/Dubai ([`TASK`]) over the open
//! steps the kernel's [`VisaSteps`](wl_kernel::visa_steps::VisaSteps) lists
//! (wl-hr provides it: a step of a case `open`, `in_progress` or `on_hold`, not
//! done, with a target, started). A step passes its target when today minus its
//! start is more than the target in calendar days (day 5 of 5 is on time, day 6
//! is late):
//!
//! - **First notice:** the PRO is told, once per step.
//! - **Second notice:** [`SECOND_NOTICE_DAYS`] (7) days after the first, HR and
//!   the MD are told, once per step, if the step is still not done.
//! - A step done, a case `cancelled`, `rejected` or `completed`, or a step
//!   without a target never gets a notice (the kernel lists none of them).
//!
//! Decisions:
//!
//! - **The claim is the row.** `manpower_visa_step_notices` has one row per
//!   step with the day of each notice; the first is claimed by inserting the row
//!   (the unique index makes a second run find it), the second by setting its
//!   day where it is empty. A notice nobody could be told of (nobody holds the
//!   role) gives the claim back so a later run tries again.
//! - **A first notice that was missed** (the run did not happen) goes at the
//!   next run; the second is counted from the day the first went out.
//! - **Roles:** the PRO is `pro`, HR `hr`, the MD `md` ([`crate::permissions`],
//!   [`crate::seed`]); the kernel picks the user who holds the role.
//! - **Targets are not deadlines.** The texts call them targets; the statutory
//!   limit of a step is an open item with the authority (AU-21).

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;
use wl_kernel::visa_steps::{OpenVisaStep, visa_steps};
use wl_kernel::{Subject, role_tasks};

use crate::deployments::worker_name;
use crate::notices;
use crate::permissions::{HR_ROLE, MD_ROLE, PRO_ROLE};
use crate::texts::english;

/// The rule id.
pub const RULE: &str = "MPW.RUL.14";

/// The name of the daily scheduled task that runs the rule.
pub const TASK: &str = "manpower:visa-step-daily";

/// M: the days between the first and the second notice.
pub const SECOND_NOTICE_DAYS: i64 = 7;

/// The notices of a step that was late.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "manpower_visa_step_notices", unique(step_id))]
pub struct VisaStepNotice {
    /// The primary key.
    pub id: i64,
    /// The wl-hr visa case step.
    pub step_id: i64,
    /// The day the PRO was told.
    pub first_notice_on: Option<NaiveDate>,
    /// The day HR and the MD were told.
    pub second_notice_on: Option<NaiveDate>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// Whether `step` is past its target on `on`.
pub fn is_late(step: &OpenVisaStep, on: NaiveDate) -> bool {
    (on - step.started_on).num_days() > step.target_days
}

/// What a run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Steps whose first notice (the PRO) went out.
    pub first: usize,
    /// Steps whose second notice (HR and the MD) went out.
    pub second: usize,
}

/// The users who hold `role`.
async fn users(state: &AppState, role: &str) -> Result<Vec<i64>> {
    Ok(role_tasks::assignee(&state.db, role)
        .await?
        .into_iter()
        .collect())
}

/// The title and body of the notice about `step`.
async fn words(
    state: &AppState,
    step: &OpenVisaStep,
    on: NaiveDate,
    key: &str,
) -> (String, String) {
    let employee = worker_name(state, "en", step.employee).await;
    let late = (on - step.started_on).num_days();
    let params: [(&str, &(dyn std::fmt::Display + Sync)); 6] = [
        ("worker", &employee),
        ("step", &step.step_type),
        ("case", &step.case_type),
        ("target", &step.target_days),
        ("started", &step.started_on),
        ("days", &late),
    ];
    (
        english(&format!("{key}_title"), &params),
        english(&format!("{key}_body"), &params),
    )
}

/// Tells `roles` about `step`; whether anybody was told.
async fn tell(
    state: &AppState,
    step: &OpenVisaStep,
    on: NaiveDate,
    key: &str,
    roles: &[&str],
) -> Result<bool> {
    let mut ids = Vec::new();
    for role in roles {
        for user in users(state, role).await? {
            if !ids.contains(&user) {
                ids.push(user);
            }
        }
    }
    let (title, body) = words(state, step, on, key).await;
    let subject: Subject = step.case.clone();
    notices::tell(state, subject, &step.step_type, &ids, &title, &body).await
}

/// The first notice for `step`, if due. Returns whether it went out.
async fn first_notice(state: &AppState, step: &OpenVisaStep, on: NaiveDate) -> Result<bool> {
    let row = VisaStepNotice::create(
        &state.db,
        VisaStepNotice {
            step_id: step.step,
            first_notice_on: Some(on),
            ..VisaStepNotice::default()
        },
    )
    .await;
    // The unique index: another run claimed it.
    let Ok(row) = row else { return Ok(false) };
    let told = tell(state, step, on, "manpower.visa_step.first", &[PRO_ROLE]).await;
    if !matches!(told, Ok(true)) {
        VisaStepNotice::query()
            .where_(VisaStepNotice::ID.eq(row.id))
            .delete(&state.db)
            .await?;
    }
    told
}

/// The second notice for `step`, if due. Returns whether it went out.
async fn second_notice(
    state: &AppState,
    step: &OpenVisaStep,
    notice: &VisaStepNotice,
    on: NaiveDate,
) -> Result<bool> {
    let Some(first) = notice.first_notice_on else {
        return Ok(false);
    };
    if notice.second_notice_on.is_some() || (on - first).num_days() < SECOND_NOTICE_DAYS {
        return Ok(false);
    }
    let claimed = VisaStepNotice::query()
        .where_(VisaStepNotice::ID.eq(notice.id))
        .where_(VisaStepNotice::SECOND_NOTICE_ON.is_null())
        .update(&state.db, &[("second_notice_on", &Some(on))])
        .await?;
    if claimed == 0 {
        return Ok(false);
    }
    let told = tell(
        state,
        step,
        on,
        "manpower.visa_step.second",
        &[HR_ROLE, MD_ROLE],
    )
    .await;
    if !matches!(told, Ok(true)) {
        let none: Option<NaiveDate> = None;
        VisaStepNotice::query()
            .where_(VisaStepNotice::ID.eq(notice.id))
            .update(&state.db, &[("second_notice_on", &none)])
            .await?;
    }
    told
}

/// `MPW.RUL.14` on `on`: the notices of every open visa step that is late.
///
/// # Errors
///
/// The first error of a notice; the other steps are still tried.
pub async fn run_on(state: &AppState, on: NaiveDate) -> Result<Report> {
    let steps = visa_steps(state).open_steps(state).await?;
    let mut report = Report::default();
    let mut failure = None;
    for step in steps.iter().filter(|step| is_late(step, on)) {
        let done = async {
            match VisaStepNotice::query()
                .where_(VisaStepNotice::STEP_ID.eq(step.step))
                .first(&state.db)
                .await?
            {
                None => Ok(first_notice(state, step, on).await?.then_some(true)),
                Some(notice) => Ok(second_notice(state, step, &notice, on)
                    .await?
                    .then_some(false)),
            }
        }
        .await;
        match done {
            Ok(Some(true)) => report.first += 1,
            Ok(Some(false)) => report.second += 1,
            Ok(None) => {}
            Err(error) => {
                eprintln!("{RULE}: step {}: {error:?}", step.step);
                failure.get_or_insert(error);
            }
        }
    }
    failure.map_or(Ok(report), Err)
}

/// The scheduled run.
///
/// # Errors
///
/// As [`run_on`].
pub async fn run(state: &AppState) -> Result {
    run_on(state, today()).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::workforce::EmployeeId;

    fn step() -> OpenVisaStep {
        OpenVisaStep {
            step: 1,
            case: Subject::new("hr.visa_case", 1),
            employee: EmployeeId(1),
            case_type: "new".to_owned(),
            step_type: "medical_fitness".to_owned(),
            started_on: NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(),
            target_days: 5,
        }
    }

    #[test]
    fn day_five_of_five_is_on_time_and_day_six_is_late() {
        let day = |d| NaiveDate::from_ymd_opt(2026, 11, d).unwrap();
        assert!(!is_late(&step(), day(6)));
        assert!(is_late(&step(), day(7)));
    }
}
