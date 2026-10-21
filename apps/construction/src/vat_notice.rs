//! The notice that VAT on the retention becomes due (`CON.RUL.07`, 02
//! Construction §7.2, §9 "VAT"; BL-CON-07).
//!
//! VAT on a supply becomes due at the latest one year after the work. Here the
//! year is counted from the completion certificate, or from the termination
//! effective date for a terminated project without one. `N` = 30 days before
//! the end of that year, if a retention part is still `held` or `due`, the
//! accountant is told that VAT on the retention becomes due.
//!
//! - The notice is made once per project ([`RetentionNotice`], unique per
//!   project): a run on the same day again, or a later day, does nothing new.
//!   A day the run missed is made up the next day it runs.
//! - An `invoiced` or later part stops the rule: the part is no longer `held`
//!   or `due`.
//! - The accountant (`retentions.manage`) may dismiss the notice with a
//!   reason ([`dismiss`]); it changes nothing else. Whether VAT is due at the
//!   limit on retention the client keeps is `⚠ open: accountant (A-13)`, so
//!   the rule warns while any part is `held` or `due`.

use renox::chrono::{Days, Months, NaiveDate};
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::TransitionError;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;

use crate::notices::tell_role;
use crate::permissions;
use crate::project::Project;
use crate::retention::{Retention, RetentionStatus};
use crate::texts::english;

/// The months after the completion certificate VAT is due at the latest.
pub const LIMIT_MONTHS: u32 = 12;

/// The days before the limit the accountant is told (`CON.RUL.07`, N).
pub const NOTICE_DAYS: u64 = 30;

/// The notice made for a project, which the accountant may dismiss.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "construction_retention_notices", unique(project_id))]
pub struct RetentionNotice {
    /// The primary key.
    pub id: i64,
    /// The project.
    pub project_id: i64,
    /// The day the year ends.
    pub limit_on: NaiveDate,
    /// The day the accountant was told.
    pub notified_on: NaiveDate,
    /// The user who dismissed it.
    pub dismissed_by: Option<i64>,
    /// Why it was dismissed.
    pub dismissed_reason: Option<String>,
    /// When it was dismissed.
    pub dismissed_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl RetentionNotice {
    /// Whether the accountant dismissed it.
    pub fn is_dismissed(&self) -> bool {
        self.dismissed_at.is_some()
    }
}

/// The day the year ends for `project`: twelve months after its completion
/// certificate, else after its termination effective date. `None` for a
/// project that has neither.
pub fn limit_of(project: &Project) -> Option<NaiveDate> {
    project
        .completed_on
        .or(project.termination_effective_on)?
        .checked_add_months(Months::new(LIMIT_MONTHS))
}

/// The day the accountant is told for a year that ends on `limit`.
pub fn notice_day(limit: NaiveDate) -> Option<NaiveDate> {
    limit.checked_sub_days(Days::new(NOTICE_DAYS))
}

/// Tells the accountant about every project whose notice day has come on
/// `day` and which still has a retention part `held` or `due`. Returns the
/// notices made.
///
/// # Errors
///
/// The database's errors and the notifier's.
pub async fn run_on(state: &AppState, day: NaiveDate) -> Result<Vec<RetentionNotice>> {
    let open = Retention::query()
        .where_(Retention::STATUS.is_in([RetentionStatus::Held, RetentionStatus::Due]))
        .order_by(Retention::PROJECT_ID)
        .get(&state.db)
        .await?;
    let mut ids: Vec<i64> = open.iter().map(|part| part.project_id).collect();
    ids.dedup();
    let mut made = Vec::new();
    for id in ids {
        let project = Project::find_or_404(&state.db, id).await?;
        let Some(limit) = limit_of(&project) else {
            continue;
        };
        if !notice_day(limit).is_some_and(|due| day >= due) {
            continue;
        }
        let known = RetentionNotice::query()
            .where_(RetentionNotice::PROJECT_ID.eq(id))
            .exists(&state.db)
            .await?;
        if known {
            continue;
        }
        let mut notice = RetentionNotice {
            project_id: id,
            limit_on: limit,
            notified_on: day,
            ..RetentionNotice::default()
        };
        notice.save(&state.db).await?;
        let title = english(
            "construction.vat_notice.title",
            &[("project", &project.name)],
        );
        let body = english(
            "construction.vat_notice.body",
            &[("project", &project.name), ("date", &limit)],
        );
        tell_role(
            state,
            &project,
            project.subject(),
            permissions::ACCOUNTANT_ROLE,
            &title,
            &body,
        )
        .await?;
        made.push(notice);
    }
    Ok(made)
}

/// Runs the rule for today.
///
/// # Errors
///
/// As [`run_on`].
pub async fn run(state: &AppState) -> Result<Vec<RetentionNotice>> {
    run_on(state, today()).await
}

/// The notice of project `project_id`, if one was made.
///
/// # Errors
///
/// A database error.
pub async fn notice_of(db: &Db, project_id: i64) -> Result<Option<RetentionNotice>> {
    RetentionNotice::query()
        .where_(RetentionNotice::PROJECT_ID.eq(project_id))
        .first(db)
        .await
}

/// `user` dismisses the notice of project `project_id` with a reason. It
/// changes nothing else: the parts keep their states.
///
/// # Errors
///
/// `Forbidden` without `retentions.manage` in the project, `Guard` (422)
/// without a reason or when the project has no notice.
pub async fn dismiss(
    state: &AppState,
    user: &AuthUser,
    project_id: i64,
    reason: &str,
) -> std::result::Result<RetentionNotice, TransitionError> {
    let project = Project::find(&state.db, project_id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    if !user.has_permission_in(permissions::RETENTIONS_MANAGE, &project.scope()) {
        return Err(TransitionError::Forbidden);
    }
    if reason.trim().is_empty() {
        return Err(TransitionError::Guard(english(
            "construction.vat_notice.reason_needed",
            &[],
        )));
    }
    let Some(mut notice) = notice_of(&state.db, project_id).await? else {
        return Err(TransitionError::Guard(english(
            "construction.vat_notice.none",
            &[],
        )));
    };
    if notice.is_dismissed() {
        return Ok(notice);
    }
    notice.dismissed_by = Some(user.id);
    notice.dismissed_reason = Some(reason.trim().to_owned());
    notice.dismissed_at = Some(renox::db::now());
    notice.save(&state.db).await?;
    let entry = renox::audit::Entry::new("construction.retention_notice_dismissed")
        .user(user.id)
        .subject(Project::TABLE, project_id)
        .data(json!({ "reason": reason.trim() }));
    renox::audit::record(&state.db, entry).await?;
    Ok(notice)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn the_year_is_counted_from_the_completion_else_from_the_termination() {
        let mut project = Project::new("TWR-01", "Tower", wl_kernel::PartyId(1));
        assert_eq!(limit_of(&project), None);
        project.termination_effective_on = Some(day(2027, 5, 31));
        assert_eq!(limit_of(&project), Some(day(2028, 5, 31)));
        project.completed_on = Some(day(2027, 6, 15));
        assert_eq!(limit_of(&project), Some(day(2028, 6, 15)));
    }

    #[test]
    fn the_accountant_is_told_thirty_days_before_the_year_ends() {
        assert_eq!(notice_day(day(2028, 6, 15)), Some(day(2028, 5, 16)));
        assert_eq!(notice_day(day(2028, 5, 31)), Some(day(2028, 5, 1)));
    }
}
