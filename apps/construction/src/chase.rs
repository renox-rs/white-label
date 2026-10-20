//! Chasing a variation the client does not answer (`CON.RUL.09`, 02
//! Construction §4.5 row "The client does not answer", §7.2; BL-CON-13), run
//! daily at 02:00 Asia/Dubai.
//!
//! N = 14 and M = 30 calendar days (Asia/Dubai) after the latest entry into
//! `submitted` ([`Variation::submitted_on`]), while the variation is still
//! `submitted`:
//!
//! - at N the project manager is reminded to chase the client;
//! - at M the MD is notified.
//!
//! Both are sent **once per submission**: the day is kept on the variation
//! ([`Variation::chased_on`], [`Variation::escalated_on`]) and `submit` clears
//! it, so a resubmitted revision has its own clock from its own `submit`. A
//! variation that was answered (it left `submitted`: approved, rejected) or
//! withdrawn is not chased. A day the run missed is made up the next day it
//! runs. Nothing is ever sent to the client.

use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use wl_kernel::documents::today;

use crate::expiry::managers;
use crate::notices::{holders, tell};
use crate::permissions;
use crate::project::Project;
use crate::texts::english;
use crate::variation::{Variation, VariationStatus};

/// The days after the submission the project manager is reminded (`CON.RUL.09`,
/// N).
pub const REMIND_DAYS: u64 = 14;

/// The days after the submission the MD is notified (`CON.RUL.09`, M).
pub const ESCALATE_DAYS: u64 = 30;

/// What a daily run did.
#[derive(Debug, Default)]
pub struct Run {
    /// The variations whose project manager was reminded.
    pub reminded: Vec<Variation>,
    /// The variations whose MD was notified.
    pub escalated: Vec<Variation>,
}

/// The day the project manager is reminded about a variation submitted on
/// `submitted_on`.
pub fn remind_day(submitted_on: NaiveDate) -> Option<NaiveDate> {
    submitted_on.checked_add_days(Days::new(REMIND_DAYS))
}

/// The day the MD is notified about a variation submitted on `submitted_on`.
pub fn escalate_day(submitted_on: NaiveDate) -> Option<NaiveDate> {
    submitted_on.checked_add_days(Days::new(ESCALATE_DAYS))
}

/// Whether `day` has come for the day `due` gives (a project without a
/// date never comes).
fn has_come(due: Option<NaiveDate>, day: NaiveDate) -> bool {
    due.is_some_and(|due| day >= due)
}

/// Reminds the project manager about `variation` (the claim on `chased_on`
/// makes it once). Returns whether it did.
async fn remind(state: &AppState, variation: &Variation, day: NaiveDate) -> Result<bool> {
    let claim = Some(day);
    let claimed = Variation::query()
        .where_(Variation::ID.eq(variation.id))
        .where_(Variation::CHASED_ON.is_null())
        .where_(Variation::STATUS.eq(VariationStatus::Submitted))
        .update(&state.db, &[("chased_on", &claim)])
        .await?;
    if claimed == 0 {
        return Ok(false);
    }
    let project = Project::find_or_404(&state.db, variation.project_id).await?;
    let label = variation.label();
    let days = REMIND_DAYS;
    let submitted_on = variation.submitted_on.unwrap_or(day);
    let title = english(
        "construction.variation.chase_title",
        &[
            ("label", &label),
            ("project", &project.name),
            ("days", &days),
        ],
    );
    let body = english(
        "construction.variation.chase_body",
        &[("date", &submitted_on)],
    );
    let users = managers(state, &project).await?;
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

/// Notifies the MD about `variation` (the claim on `escalated_on` makes it
/// once). Returns whether it did.
async fn escalate(state: &AppState, variation: &Variation, day: NaiveDate) -> Result<bool> {
    let claim = Some(day);
    let claimed = Variation::query()
        .where_(Variation::ID.eq(variation.id))
        .where_(Variation::ESCALATED_ON.is_null())
        .where_(Variation::STATUS.eq(VariationStatus::Submitted))
        .update(&state.db, &[("escalated_on", &claim)])
        .await?;
    if claimed == 0 {
        return Ok(false);
    }
    let project = Project::find_or_404(&state.db, variation.project_id).await?;
    let label = variation.label();
    let days = ESCALATE_DAYS;
    let submitted_on = variation.submitted_on.unwrap_or(day);
    let title = english(
        "construction.variation.escalated_title",
        &[
            ("label", &label),
            ("project", &project.name),
            ("days", &days),
        ],
    );
    let body = english(
        "construction.variation.escalated_body",
        &[("date", &submitted_on)],
    );
    let users = holders(state, &project, permissions::MD_ROLE).await?;
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

/// Runs the rule for `day`: every `submitted` variation whose 14th day has
/// come and was not chased yet gets its reminder, and every one whose 30th day
/// has come and was not escalated yet notifies the MD.
///
/// # Errors
///
/// The database's and notifier's errors.
pub async fn run_on(state: &AppState, day: NaiveDate) -> Result<Run> {
    let submitted = Variation::query()
        .where_(Variation::STATUS.eq(VariationStatus::Submitted))
        .order_by(Variation::ID)
        .get(&state.db)
        .await?;
    let mut run = Run::default();
    for variation in submitted {
        let Some(submitted_on) = variation.submitted_on else {
            continue;
        };
        if variation.chased_on.is_none()
            && has_come(remind_day(submitted_on), day)
            && remind(state, &variation, day).await?
        {
            run.reminded
                .push(Variation::find_or_404(&state.db, variation.id).await?);
        }
        if variation.escalated_on.is_none()
            && has_come(escalate_day(submitted_on), day)
            && escalate(state, &variation, day).await?
        {
            run.escalated
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
    fn the_reminder_comes_after_fourteen_days_and_the_notice_after_thirty() {
        assert_eq!(remind_day(day(2027, 3, 1)), Some(day(2027, 3, 15)));
        assert_eq!(escalate_day(day(2027, 3, 1)), Some(day(2027, 3, 31)));
        assert!(!has_come(remind_day(day(2027, 3, 1)), day(2027, 3, 14)));
        assert!(has_come(remind_day(day(2027, 3, 1)), day(2027, 3, 15)));
        assert!(has_come(remind_day(day(2027, 3, 1)), day(2027, 3, 16)));
        assert!(!has_come(None, day(2027, 3, 16)));
    }
}
