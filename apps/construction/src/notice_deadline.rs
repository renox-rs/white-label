//! The notice deadline of a delay event (`CON.RUL.10`, 02 Construction §4.5
//! step 3, §7.2; BL-CON-14), run daily at 02:00 Asia/Dubai.
//!
//! A project may set a notice period in days ([`Project::notice_days`], none
//! by default). The **notice deadline** of a variation is its event date plus
//! that period. While the variation is still a `draft` the project manager and
//! the QS are warned 7 and 2 days before the deadline, once each (the days are
//! kept on the variation). The rule does nothing for a project without a
//! notice period, nor for a variation without an event date (required only for
//! the kind `time`).
//!
//! A notice sent after the deadline is never refused: `submit` records the
//! notice date and marks the variation `notice_late` (see
//! [`crate::variation::submit_effects`]).
//!
//! Decisions where the spec is silent: a day the run missed is made up the
//! next day it runs (the 7-day warning is skipped when the 2-day one is due,
//! and nothing is sent after the deadline), and the QS are the holders of the
//! QS role in the project.

use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use wl_kernel::documents::today;

use crate::expiry::managers;
use crate::notices::{holders, tell};
use crate::permissions;
use crate::project::Project;
use crate::texts::english;
use crate::variation::{Variation, VariationStatus};

/// The days before the deadline of the first warning.
pub const FIRST_WARNING_DAYS: u64 = 7;

/// The days before the deadline of the second warning.
pub const SECOND_WARNING_DAYS: u64 = 2;

/// What a daily run did.
#[derive(Debug, Default)]
pub struct Run {
    /// The variations warned 7 days before the deadline.
    pub first: Vec<Variation>,
    /// The variations warned 2 days before the deadline.
    pub second: Vec<Variation>,
}

/// The notice deadline of `variation`: its event date plus the notice period
/// of `project`; none without a period or an event date.
pub fn deadline(project: &Project, variation: &Variation) -> Option<NaiveDate> {
    let days = u64::try_from(project.notice_days?).ok()?;
    variation.event_date?.checked_add_days(Days::new(days))
}

/// The day `days` before `deadline`.
fn before(deadline: NaiveDate, days: u64) -> Option<NaiveDate> {
    deadline.checked_sub_days(Days::new(days))
}

/// Warns the project manager and the QS about `variation`, `days` before its
/// deadline. The claim on the day column makes it once; returns whether it
/// warned.
async fn warn(
    state: &AppState,
    project: &Project,
    variation: &Variation,
    due: NaiveDate,
    days: u64,
    day: NaiveDate,
) -> Result<bool> {
    let claim = Some(day);
    let column = if days == FIRST_WARNING_DAYS {
        ("notice_warned_7_on", Variation::NOTICE_WARNED_7_ON)
    } else {
        ("notice_warned_2_on", Variation::NOTICE_WARNED_2_ON)
    };
    let claimed = Variation::query()
        .where_(Variation::ID.eq(variation.id))
        .where_(column.1.is_null())
        .where_(Variation::STATUS.eq(VariationStatus::Draft))
        .update(&state.db, &[(column.0, &claim)])
        .await?;
    if claimed == 0 {
        return Ok(false);
    }
    let label = variation.label();
    let event_date = variation.event_date.unwrap_or(due);
    let title = english(
        "construction.variation.notice_title",
        &[
            ("label", &label),
            ("project", &project.name),
            ("days", &days),
        ],
    );
    let body = english(
        "construction.variation.notice_body",
        &[("event_date", &event_date), ("deadline", &due)],
    );
    let mut users = managers(state, project).await?;
    for user in holders(state, project, permissions::QS_ROLE).await? {
        if !users.contains(&user) {
            users.push(user);
        }
    }
    tell(
        state,
        variation.subject(),
        &project.code,
        &users,
        &title,
        &body,
    )
    .await?;
    Ok(true)
}

/// Runs the rule for `day`: every `draft` variation of a project with a notice
/// period and with an event date gets its warning 7 days and 2 days before the
/// deadline, once each.
///
/// # Errors
///
/// The database's and notifier's errors.
pub async fn run_on(state: &AppState, day: NaiveDate) -> Result<Run> {
    let drafts = Variation::query()
        .where_(Variation::STATUS.eq(VariationStatus::Draft))
        .where_(Variation::EVENT_DATE.is_not_null())
        .order_by(Variation::ID)
        .get(&state.db)
        .await?;
    let mut run = Run::default();
    for variation in drafts {
        let project = Project::find_or_404(&state.db, variation.project_id).await?;
        let Some(due) = deadline(&project, &variation) else {
            continue;
        };
        if day > due {
            continue;
        }
        let second_due = before(due, SECOND_WARNING_DAYS).is_some_and(|at| day >= at);
        let first_due = before(due, FIRST_WARNING_DAYS).is_some_and(|at| day >= at);
        if second_due {
            if variation.notice_warned_2_on.is_none()
                && warn(state, &project, &variation, due, SECOND_WARNING_DAYS, day).await?
            {
                run.second
                    .push(Variation::find_or_404(&state.db, variation.id).await?);
            }
        } else if first_due
            && variation.notice_warned_7_on.is_none()
            && warn(state, &project, &variation, due, FIRST_WARNING_DAYS, day).await?
        {
            run.first
                .push(Variation::find_or_404(&state.db, variation.id).await?);
        }
    }
    Ok(run)
}

/// The daily run at 02:00 Asia/Dubai.
///
/// # Errors
///
/// As [`run_on`].
pub async fn daily(state: &AppState) -> Result {
    run_on(state, today()).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn the_deadline_is_the_event_date_plus_the_notice_period() {
        let mut project = Project::default();
        let variation = Variation {
            event_date: Some(day(2027, 4, 1)),
            ..Variation::default()
        };
        assert_eq!(deadline(&project, &variation), None);
        project.notice_days = Some(28);
        assert_eq!(deadline(&project, &variation), Some(day(2027, 4, 29)));
        assert_eq!(
            before(day(2027, 4, 29), FIRST_WARNING_DAYS),
            Some(day(2027, 4, 22))
        );
        assert_eq!(
            before(day(2027, 4, 29), SECOND_WARNING_DAYS),
            Some(day(2027, 4, 27))
        );
        assert_eq!(deadline(&project, &Variation::default()), None);
    }
}
