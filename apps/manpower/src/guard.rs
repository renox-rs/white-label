//! The document and licence guard of a deployment (09 Manpower §3.3
//! `MPW.ACL.07`, §4.5, §9 *Documents that block work*, BL-MPW-08).
//!
//! `activate`, `resume` and `extend` of a deployment may run only when the
//! worker's documents pass the kernel's standard guard (`document_valid`,
//! wl-documents §5.6) for every kind whose `blocks` list the transition
//! ([`ACTIVATE_GUARDED`](crate::documents::ACTIVATE_GUARDED),
//! [`RESUME_GUARDED`](crate::documents::RESUME_GUARDED),
//! [`EXTEND_GUARDED`](crate::documents::EXTEND_GUARDED)). Nothing here knows a
//! kind: the residence visa, the work permit, the Emirates ID, the health
//! insurance and the licences of the site (SIRA for the security roles, the
//! health card, the site pass) are the client's document kinds with their
//! `blocks` entries (§9; the seed is BL-MPW-20), whose condition on the role
//! ([`ROLE_FIELD`](crate::documents::ROLE_FIELD)) and override mode are read
//! from the kind, never from code. The refusal is the guard's: one line per
//! failing kind with its expiry date, whether an override is possible and who
//! approves, followed by the day the kinds were judged on. A kind whose
//! override mode is `with_approval` is covered by an approved, unused override
//! of the same deployment and transition (`DOC.ACL.02`, the requester runs the
//! transition within 72 hours; the use is claimed around the transition by
//! [`wl_kernel::documents::guarded_transition_as`]).
//!
//! - **The day checked** ([`check_date`]): the day of the transition plus N
//!   days ([`DEFAULT_DOCUMENT_LOOK_AHEAD_DAYS`], 30), or the end date of the
//!   deployment if that is earlier (for `extend`, the new end date), so a
//!   deployment shorter than N days needs its documents valid to its end only.
//! - **Decision: the site's licences.** The spec words the site's required
//!   licences as `qualification_valid` (`HRM.QUA.01`), but §9 lists the SIRA
//!   licence, the health card and the site pass as document kinds with
//!   `blocks`, and the per-site requirement list (`MPW.CMP.01`) has no model
//!   yet. So the licences are guarded as document kinds, through the same
//!   guard, and `Must not keep a separate expiry check` holds; the site is
//!   the scope the licence is judged for and an override is approved in
//!   ([`BlockSubjects::scope`](wl_kernel::documents::BlockSubjects::scope)).
//! - **Planning only warns** ([`warn_planned`]): a new `planned` deployment is
//!   saved and the guard's [`PlannedWarning`] for its first day is shown, and
//!   the nightly `DOC.RUL.02` run lists it again inside the look-ahead window.
//!   Its `activate` is refused later.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};

use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use wl_kernel::documents::{PlannedItem, PlannedWarning, guard, guarded_transition_as};

use crate::config::DEFAULT_DOCUMENT_LOOK_AHEAD_DAYS;
use crate::deployment::Deployment;
use crate::texts::english;

/// The day the documents of a deployment are judged on for a transition on
/// `on`: `on` plus the look-ahead, or `end` if that is earlier.
pub fn check_date(on: NaiveDate, end: Option<NaiveDate>) -> NaiveDate {
    let ahead = on
        .checked_add_days(Days::new(
            u64::try_from(DEFAULT_DOCUMENT_LOOK_AHEAD_DAYS).unwrap_or(0),
        ))
        .unwrap_or(on);
    end.map_or(ahead, |end| end.min(ahead))
}

/// Runs `run`, the transition `guarded` of deployment `deployment_id`, once
/// the documents pass on `check_on` for the user `by`, who may hold approved
/// overrides; the guard claims them around `run`. A refusal names the day the
/// documents were judged on.
///
/// # Errors
///
/// A 422 listing the failing kinds, or the errors of `run`.
pub(crate) async fn guarded<T, F, Fut>(
    state: &AppState,
    by: &AuthUser,
    guarded: &str,
    deployment_id: i64,
    check_on: NaiveDate,
    run: F,
) -> Result<T>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let started = AtomicBool::new(false);
    let done = guarded_transition_as(state, guarded, deployment_id, check_on, Some(by.id), || {
        started.store(true, Ordering::SeqCst);
        run()
    })
    .await;
    match done {
        Err(Error::Status(code, message)) if !started.load(Ordering::SeqCst) => {
            let day = english(
                "manpower.deployment.checked_on",
                &[
                    ("date", &check_on),
                    ("days", &DEFAULT_DOCUMENT_LOOK_AHEAD_DAYS),
                ],
            );
            Err(Error::Status(code, format!("{message}\n{day}")))
        }
        other => other,
    }
}

/// The guard's warning for the documents of the new `planned` `deployment`
/// that will not be valid on its first day; none when they are, or without
/// wl-documents. It never refuses.
///
/// # Errors
///
/// The guard's failures.
pub async fn warn_planned(
    state: &AppState,
    deployment: &Deployment,
) -> Result<Option<PlannedWarning>> {
    let item = PlannedItem::new(deployment.id, deployment.start_date);
    let answer = guard(state)
        .check_planned(state, crate::documents::ACTIVATE_GUARDED, &[item])
        .await?;
    // A deployment is not a bookable slot; the error cannot come.
    Ok(answer.unwrap_or(None))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, month, day).unwrap()
    }

    #[test]
    fn the_check_date_is_thirty_days_on_or_the_end_date_if_earlier() {
        assert_eq!(check_date(date(11, 5), None), date(12, 5));
        assert_eq!(check_date(date(11, 5), Some(date(11, 15))), date(11, 15));
        assert_eq!(check_date(date(11, 5), Some(date(12, 31))), date(12, 5));
    }
}
