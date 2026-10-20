//! The practical completion of a project and the end of its defects
//! liability period (`complete` and `close`, 02 Construction §4.6 steps 3 to
//! 6, §7.1 *Project*). They are built as far as the retention and the
//! guarantees need them (BL-CON-07, BL-CON-09).
//!
//! ```text
//! in_progress --complete--> practical_completion --close--> closed
//! ```
//!
//! - `complete` (`projects.manage`) records the handover certificate and the
//!   day it is dated. The defects liability end date is that day plus the
//!   project's defects liability days, and the **first** retention part is
//!   made due ([`crate::retention::make_part_due_in`]). The accountant is
//!   told after the commit ([`crate::release::announce_due`]).
//! - `close` (`projects.manage`) needs the defects liability end date to have
//!   passed. It makes the **final** retention part due and gives the
//!   accountant one release task per `active` performance or retention bond
//!   ([`crate::expiry::release_tasks`]).
//!
//! Decisions where the spec is silent:
//!
//! - "No open snags marked blocking" (`complete`) and "no open defect
//!   tickets" (`close`) are guards on the tickets module's data, which this
//!   vertical reads only through the kernel's ticket contract; they come with
//!   the snag and defect issues.
//! - "Passed" means the day after the end date: a project closes the day
//!   after its defects liability period ends.
//! - The end date is the completion day plus the defects liability days
//!   (365 by default).

use renox::chrono::{Days, NaiveDate};
use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;

use crate::project::{CLOSE, COMPLETE, Project};
use crate::retention::{RetentionPart, make_part_due_in};
use crate::texts::english;
use crate::{expiry, release};

/// What a `complete` run carries to its guard and effect.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompleteInput {
    /// The handover certificate, a document reference.
    #[serde(default)]
    pub certificate: String,
    /// The day the handover certificate is dated.
    #[serde(default)]
    pub completed_on: Option<NaiveDate>,
}

/// The input of a `complete` run.
pub(crate) fn complete_input(ctx: &TransitionCtx) -> CompleteInput {
    ctx.input_as::<CompleteInput>().unwrap_or_default()
}

/// Why the project cannot be completed with `input`, if it cannot: the
/// handover certificate and its date are recorded.
pub fn complete_problem(input: &CompleteInput) -> Option<String> {
    if input.certificate.trim().is_empty() {
        return Some(english("construction.complete.certificate_needed", &[]));
    }
    if input.completed_on.is_none() {
        return Some(english("construction.complete.date_needed", &[]));
    }
    None
}

/// The last day of the defects liability period of a project completed on
/// `completed_on` with `days` days of it.
pub fn defects_end(completed_on: NaiveDate, days: i64) -> Option<NaiveDate> {
    completed_on.checked_add_days(Days::new(u64::try_from(days).unwrap_or(0)))
}

/// The effect of `complete` inside its transaction: the certificate, the
/// defects liability end date and the first retention part due.
pub(crate) async fn complete_effects(
    tx: &mut Transaction,
    project: &Project,
    input: &CompleteInput,
) -> Result {
    let Some(completed_on) = input.completed_on else {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            english("construction.complete.date_needed", &[]),
        ));
    };
    let certificate = Some(input.certificate.trim().to_owned());
    let completed = Some(completed_on);
    let ends = defects_end(completed_on, project.defects_liability_days);
    Project::query()
        .where_(Project::ID.eq(project.id))
        .update(
            &mut *tx,
            &[
                ("handover_certificate", &certificate),
                ("completed_on", &completed),
                ("defects_liability_ends_on", &ends),
            ],
        )
        .await?;
    make_part_due_in(&mut *tx, project.id, RetentionPart::First).await?;
    Ok(())
}

/// Why the project cannot be closed on `today`, if it cannot: the defects
/// liability period has ended.
pub fn close_problem(project: &Project, today: NaiveDate) -> Option<String> {
    match project.defects_liability_ends_on {
        Some(end) if today > end => None,
        Some(end) => Some(english(
            "construction.close.defects_running",
            &[("date", &end)],
        )),
        None => Some(english("construction.close.no_end_date", &[])),
    }
}

/// The guard of `close` for `project`, read on today's date.
pub(crate) fn close_guard(project: &Project) -> Option<String> {
    close_problem(project, today())
}

/// The effect of `close` inside its transaction: the day it closed and the
/// final retention part due.
pub(crate) async fn close_effects(tx: &mut Transaction, id: i64) -> Result {
    let at = Some(renox::db::now());
    Project::query()
        .where_(Project::ID.eq(id))
        .update(&mut *tx, &[("closed_at", &at)])
        .await?;
    make_part_due_in(&mut *tx, id, RetentionPart::Final).await?;
    Ok(())
}

/// What a user enters to complete a project.
#[derive(Debug, Clone)]
pub struct CompleteRequest {
    /// The handover certificate, a document reference.
    pub certificate: String,
    /// The day it is dated.
    pub completed_on: NaiveDate,
}

/// `user` completes project `id`: the handover certificate is recorded, the
/// defects liability period starts and the first retention part is due.
///
/// # Errors
///
/// `Forbidden` without `projects.manage` in the project, `NotFrom` unless the
/// project is `in_progress`, `Guard` (422) naming the missing input. Nothing
/// is written then.
pub async fn complete(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    request: CompleteRequest,
) -> std::result::Result<Project, TransitionError> {
    let project = Project::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    let input = CompleteInput {
        certificate: request.certificate.trim().to_owned(),
        completed_on: Some(request.completed_on),
    };
    let _outcome = project
        .transition(COMPLETE)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    release::announce_due(state, id).await?;
    Ok(Project::find_or_404(&state.db, id).await?)
}

/// `user` closes project `id` once its defects liability period has ended:
/// the final retention part is due and the accountant gets a release task for
/// each `active` performance or retention bond.
///
/// # Errors
///
/// `Forbidden` without `projects.manage` in the project, `NotFrom` unless the
/// project is `practical_completion`, `Guard` (422) while the defects
/// liability period runs. Nothing is written then.
pub async fn close(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<Project, TransitionError> {
    let project = Project::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    let _outcome = project.transition(CLOSE).by(user).run(state).await?;
    release::announce_due(state, id).await?;
    expiry::release_tasks(state, id).await?;
    Ok(Project::find_or_404(&state.db, id).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn the_certificate_and_its_date_are_needed() {
        let mut input = CompleteInput {
            certificate: "HANDOVER-1.pdf".into(),
            completed_on: Some(day(2027, 6, 15)),
        };
        assert_eq!(complete_problem(&input), None);
        input.certificate = " ".into();
        assert!(complete_problem(&input).is_some_and(|p| p.contains("handover certificate")));
        input.certificate = "HANDOVER-1.pdf".into();
        input.completed_on = None;
        assert!(complete_problem(&input).is_some_and(|p| p.contains("dated")));
    }

    #[test]
    fn the_defects_period_runs_the_days_of_the_project_from_completion() {
        assert_eq!(defects_end(day(2027, 6, 15), 365), Some(day(2028, 6, 14)));
        assert_eq!(defects_end(day(2027, 6, 15), 0), Some(day(2027, 6, 15)));
        assert_eq!(defects_end(day(2027, 6, 15), -3), Some(day(2027, 6, 15)));
    }

    #[test]
    fn a_project_closes_the_day_after_its_defects_period_ends() {
        let mut project = Project::new("TWR-01", "Tower", wl_kernel::PartyId(1));
        assert!(
            close_problem(&project, day(2028, 6, 20))
                .is_some_and(|p| p.contains("no defects liability end date"))
        );
        project.defects_liability_ends_on = Some(day(2028, 6, 15));
        assert!(
            close_problem(&project, day(2028, 6, 15)).is_some_and(|p| p.contains("2028-06-15"))
        );
        assert_eq!(close_problem(&project, day(2028, 6, 16)), None);
    }
}
