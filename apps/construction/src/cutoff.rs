//! The monthly draft IPA (`CON.RUL.03`, 02 Construction §4.4 step 1, §7.2;
//! BL-CON-04).
//!
//! On the cut-off day of the month (D = 25; the last day of a month that has
//! no such day) at 02:00 Asia/Dubai, every `in_progress` project that has no
//! IPA in `draft` gets one. The draft takes the next number of the project
//! and its **previous certified** is the cumulative certified amount of the
//! latest IPA (by number) that is `certified` or `invoiced`; an IPA still
//! `submitted` is never counted. When an earlier IPA is still `submitted`
//! the draft is marked *previous pending* and the QS and the project manager
//! get a task; otherwise the QS does.
//!
//! One draft per project per cut-off: a re-run finds the draft (or the
//! cut-off recorded on an IPA that was submitted since) and does nothing,
//! and a unique index on `(project_id, cutoff_on)` stands behind it.
//!
//! Decision: D is the default of the spec; per-client settings come with the
//! settings module, so it is a constant here ([`CUTOFF_DAY`]).

use renox::chrono::{Datelike, NaiveDate};
use renox::prelude::*;
use wl_kernel::documents::today;

use crate::certificate::previous_certified_in;
use crate::ipa::{self, Ipa, IpaStatus};
use crate::permissions;
use crate::project::{Project, ProjectStatus};
use crate::tasks::{self, Task};
use crate::texts::english;

/// The cut-off day of the month (`CON.RUL.03`, D).
pub const CUTOFF_DAY: u32 = 25;

/// The name of the scheduled task.
pub const TASK: &str = "construction:ipa-cutoff";

/// When the task runs: 02:00 Asia/Dubai (§7.2).
pub const TASK_AT: &str = "02:00";

/// The zone it runs in.
pub const TASK_ZONE: &str = "Asia/Dubai";

/// The cut-off date of `month` in `year`: day `day`, or the last day of the
/// month when it has no such day.
pub fn cutoff_date(year: i32, month: u32, day: u32) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(year, month, day).or_else(|| {
        let (next_year, next_month) = if month == 12 {
            (year + 1, 1)
        } else {
            (year, month + 1)
        };
        NaiveDate::from_ymd_opt(next_year, next_month, 1)?.pred_opt()
    })
}

/// What a run did.
#[derive(Debug, Default)]
pub struct CutoffRun {
    /// The drafts made, one per project.
    pub drafts: Vec<Ipa>,
}

/// Makes today's drafts when today is the cut-off day.
///
/// # Errors
///
/// The database's and ticketing's errors.
pub async fn run(state: &AppState) -> Result<CutoffRun> {
    run_on(state, today(), CUTOFF_DAY).await
}

/// Makes the drafts of the cut-off `day` when `day` is the cut-off day `d` of
/// its month; a run on any other day makes none.
///
/// # Errors
///
/// The database's and ticketing's errors.
pub async fn run_on(state: &AppState, day: NaiveDate, d: u32) -> Result<CutoffRun> {
    let mut done = CutoffRun::default();
    if cutoff_date(day.year(), day.month(), d) != Some(day) {
        return Ok(done);
    }
    let projects = Project::query()
        .where_(Project::STATUS.eq(ProjectStatus::InProgress))
        .order_by(Project::ID)
        .get(&state.db)
        .await?;
    for project in projects {
        if let Some(made) = draft_for(state, &project, day).await? {
            tell_the_team(state, &project, &made).await?;
            done.drafts.push(made.draft);
        }
    }
    Ok(done)
}

/// A draft that was made, and the number of the IPA it waits for.
struct Made {
    draft: Ipa,
    pending_number: Option<i64>,
}

/// The draft of `project` for the cut-off `day`, made in one transaction;
/// `None` when the project has a draft already or this cut-off was done.
async fn draft_for(state: &AppState, project: &Project, day: NaiveDate) -> Result<Option<Made>> {
    let mut tx = state.db.begin().await?;
    if !ipa::in_status_in(&mut tx, project.id, IpaStatus::Draft)
        .await?
        .is_empty()
    {
        return Ok(None);
    }
    let done = Ipa::query()
        .where_(Ipa::PROJECT_ID.eq(project.id))
        .where_(Ipa::CUTOFF_ON.eq(day))
        .exists(&mut tx)
        .await?;
    if done {
        return Ok(None);
    }
    let number = ipa::next_number_in(&mut tx, project.id).await?;
    let pending = ipa::in_status_in(&mut tx, project.id, IpaStatus::Submitted)
        .await?
        .pop();
    let previous = previous_certified_in(&mut tx, project.id, number).await?;
    let mut draft = Ipa {
        number,
        previous_pending: pending.is_some(),
        previous_certified: previous,
        cutoff_on: Some(day),
        valuation_on: Some(day),
        ..Ipa::new(project.id)
    };
    draft.insert(&mut tx).await?;
    tx.commit().await?;
    Ok(Some(Made {
        draft,
        pending_number: pending.map(|waiting| waiting.number),
    }))
}

/// Tells the QS, and the project manager when an earlier IPA is pending.
async fn tell_the_team(state: &AppState, project: &Project, made: &Made) -> Result {
    let draft = &made.draft;
    let (title, body) = match made.pending_number {
        Some(waiting) => (
            english(
                "construction.ipa.cutoff_pending_title",
                &[("number", &draft.number), ("project", &project.name)],
            ),
            english(
                "construction.ipa.cutoff_pending_body",
                &[("pending", &waiting), ("number", &draft.number)],
            ),
        ),
        None => (
            english(
                "construction.ipa.cutoff_title",
                &[("number", &draft.number), ("project", &project.name)],
            ),
            english("construction.ipa.cutoff_body", &[("number", &draft.number)]),
        ),
    };
    tasks::open(
        state,
        project,
        Task {
            subject: draft.subject(),
            role: permissions::QS_ROLE,
            assignee: None,
            source_key: format!("construction.ipa_cutoff_qs:{}", draft.id),
            title: title.clone(),
            description: body.clone(),
            due: None,
        },
    )
    .await?;
    if made.pending_number.is_some() {
        tasks::open(
            state,
            project,
            Task {
                subject: draft.subject(),
                role: permissions::PROJECT_MANAGER_ROLE,
                assignee: project.manager_user_id,
                source_key: format!("construction.ipa_cutoff_pm:{}", draft.id),
                title,
                description: body,
                due: None,
            },
        )
        .await?;
    }
    Ok(())
}

/// Registers the daily task that makes the drafts on the cut-off day.
pub(crate) fn schedule(app: &mut Registry) {
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run(&state).await.map(|_| ())
        })
        .timezone(TASK_ZONE);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cutoff_is_day_25_or_the_last_day_of_a_shorter_month() {
        let day = |y, m, d| NaiveDate::from_ymd_opt(y, m, d);
        assert_eq!(cutoff_date(2027, 2, 25), day(2027, 2, 25));
        assert_eq!(cutoff_date(2027, 2, 31), day(2027, 2, 28));
        assert_eq!(cutoff_date(2028, 2, 31), day(2028, 2, 29));
        assert_eq!(cutoff_date(2027, 4, 31), day(2027, 4, 30));
        assert_eq!(cutoff_date(2027, 12, 31), day(2027, 12, 31));
    }
}
