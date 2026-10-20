//! The expiry of a guarantee (`CON.RUL.11` and `expire`, 02 Construction
//! §4.6 rows "A guarantee is about to expire before it may be released",
//! §7.1 *Guarantee*, §7.2; BL-CON-09), run daily at 02:00 Asia/Dubai.
//!
//! - **`CON.RUL.11`**: N = 60 days before the expiry of an `active`
//!   guarantee, if the project has not reached the guarantee's release point
//!   (completion for the advance guarantee, the end of the defects liability
//!   period for a performance or retention bond), the accountant gets a task
//!   to ask the bank for an extension, because the beneficiary may call a
//!   guarantee that is not extended, and the project manager is notified
//!   ([`remind_on`]). The task is made once per expiry date: `extend` resets
//!   the reminders ([`crate::guarantee::Guarantee::reminded_for`]), so the
//!   new expiry gets its own task. A day the run missed is made up the next
//!   day it runs. A `called` guarantee gets no task.
//! - **`expire`**: the day after the expiry date the system expires the
//!   `active` guarantee ([`expire_on`]); the accountant and the project
//!   manager are told, because a guarantee never expires silently
//!   ([`listen`], from the transition's event). A `called` guarantee does not
//!   expire.
//! - **Project `close`**: the accountant gets one release task per `active`
//!   performance or retention bond ([`release_tasks`]), because the bond is
//!   released at the end of the defects liability period (§4.6 step 6).

use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError, Transitioned};
use wl_kernel::documents::today;

use crate::guarantee::{EXPIRE, Guarantee, GuaranteeKind, GuaranteeStatus};
use crate::notices::{aed, holders, tell, tell_role};
use crate::permissions;
use crate::project::{Project, ProjectStatus};
use crate::tasks::{self, Task};
use crate::texts::english;

/// The days before the expiry the accountant gets the task (`CON.RUL.11`, N).
pub const NOTICE_DAYS: u64 = 60;

/// What a daily run did.
#[derive(Debug, Default)]
pub struct Run {
    /// The guarantees that were expired.
    pub expired: Vec<Guarantee>,
    /// The guarantees an extension task was made for.
    pub reminded: Vec<Guarantee>,
}

/// The day the extension task is made for a guarantee that expires on
/// `expires_on`.
pub fn reminder_day(expires_on: NaiveDate) -> Option<NaiveDate> {
    expires_on.checked_sub_days(Days::new(NOTICE_DAYS))
}

/// Whether `project` has reached the release point of a guarantee of `kind`
/// on `day`: completion for the advance guarantee, the end of the defects
/// liability period for a performance or retention bond.
pub fn release_point_reached(project: &Project, kind: GuaranteeKind, day: NaiveDate) -> bool {
    match kind {
        GuaranteeKind::Advance => {
            project.completed_on.is_some()
                || matches!(
                    project.status,
                    ProjectStatus::PracticalCompletion | ProjectStatus::Closed
                )
        }
        GuaranteeKind::Performance | GuaranteeKind::RetentionBond => {
            project.status == ProjectStatus::Closed
                || project
                    .defects_liability_ends_on
                    .is_some_and(|end| day > end)
        }
    }
}

/// The kind as the messages show it: `retention bond`.
fn kind_text(guarantee: &Guarantee) -> String {
    match guarantee.kind {
        GuaranteeKind::Advance => "advance guarantee",
        GuaranteeKind::Performance => "performance bond",
        GuaranteeKind::RetentionBond => "retention bond",
    }
    .to_owned()
}

/// The users who manage `project`: its project manager, else the holders of
/// the role.
async fn managers(state: &AppState, project: &Project) -> Result<Vec<i64>> {
    match project.manager_user_id {
        Some(user) => Ok(vec![user]),
        None => holders(state, project, permissions::PROJECT_MANAGER_ROLE).await,
    }
}

/// Makes the extension tasks of `day`: for every `active` guarantee whose
/// reminder day has come, that expires on or after `day`, whose project has
/// not reached its release point and which has no task for this expiry yet,
/// a task for the accountant and a notice for the project manager.
///
/// # Errors
///
/// The database's, ticketing's and notifier's errors.
pub async fn remind_on(state: &AppState, day: NaiveDate) -> Result<Vec<Guarantee>> {
    let active = Guarantee::query()
        .where_(Guarantee::STATUS.eq(GuaranteeStatus::Active))
        .order_by(Guarantee::ID)
        .get(&state.db)
        .await?;
    let mut reminded = Vec::new();
    for guarantee in active {
        let Some(expires_on) = guarantee.expires_on else {
            continue;
        };
        let due = reminder_day(expires_on).is_some_and(|reminder| day >= reminder);
        if !due || day > expires_on || guarantee.reminded_for.is_some() {
            continue;
        }
        let project = Project::find_or_404(&state.db, guarantee.project_id).await?;
        if release_point_reached(&project, guarantee.kind, day) {
            continue;
        }
        let kind = kind_text(&guarantee);
        tasks::open(
            state,
            &project,
            Task {
                subject: guarantee.subject(),
                role: permissions::ACCOUNTANT_ROLE,
                assignee: None,
                source_key: format!(
                    "construction.guarantee_extension:{}:{expires_on}",
                    guarantee.id
                ),
                title: english(
                    "construction.guarantee.extension_title",
                    &[
                        ("kind", &kind),
                        ("project", &project.name),
                        ("date", &expires_on),
                    ],
                ),
                description: english(
                    "construction.guarantee.extension_body",
                    &[
                        ("bank", &guarantee.bank),
                        ("amount", &aed(guarantee.amount)),
                        ("date", &expires_on),
                    ],
                ),
                due: Some(expires_on),
            },
        )
        .await?;
        // Once per expiry date: `extend` clears the claim.
        let claim = Some(expires_on);
        let claimed = Guarantee::query()
            .where_(Guarantee::ID.eq(guarantee.id))
            .where_(Guarantee::REMINDED_FOR.is_null())
            .update(&state.db, &[("reminded_for", &claim)])
            .await?;
        if claimed == 0 {
            continue;
        }
        let users = managers(state, &project).await?;
        tell(
            state,
            guarantee.subject(),
            &project.code,
            &users,
            &english(
                "construction.guarantee.expiring_title",
                &[
                    ("kind", &kind),
                    ("project", &project.name),
                    ("date", &expires_on),
                ],
            ),
            &english(
                "construction.guarantee.expiring_body",
                &[("date", &expires_on)],
            ),
        )
        .await?;
        reminded.push(Guarantee::find_or_404(&state.db, guarantee.id).await?);
    }
    Ok(reminded)
}

/// Expires the `active` guarantees whose expiry date is before `day`, as the
/// system. The accountant and the project manager are told from the event.
///
/// # Errors
///
/// The database's errors.
pub async fn expire_on(state: &AppState, day: NaiveDate) -> Result<Vec<Guarantee>> {
    let active = Guarantee::query()
        .where_(Guarantee::STATUS.eq(GuaranteeStatus::Active))
        .order_by(Guarantee::ID)
        .get(&state.db)
        .await?;
    let mut expired = Vec::new();
    for guarantee in active {
        if !guarantee
            .expires_on
            .is_some_and(|expires_on| day > expires_on)
        {
            continue;
        }
        let _outcome = guarantee
            .transition(EXPIRE)
            .by_system()
            .run(state)
            .await
            .map_err(TransitionError::into_error)?;
        expired.push(Guarantee::find_or_404(&state.db, guarantee.id).await?);
    }
    Ok(expired)
}

/// Runs the rule and the expiry of `day`: the guarantees that expire first,
/// then the extension tasks of those that do not.
///
/// # Errors
///
/// As [`remind_on`] and [`expire_on`].
pub async fn run_on(state: &AppState, day: NaiveDate) -> Result<Run> {
    let expired = expire_on(state, day).await?;
    let reminded = remind_on(state, day).await?;
    Ok(Run { expired, reminded })
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

/// Gives the accountant one task per `active` performance or retention bond
/// of project `project_id` to release it (project `close`). Returns how many
/// tasks it made.
///
/// # Errors
///
/// The database's and ticketing's errors.
pub async fn release_tasks(state: &AppState, project_id: i64) -> Result<usize> {
    let project = Project::find_or_404(&state.db, project_id).await?;
    let mut made = 0;
    for guarantee in crate::guarantee::active_of(&state.db, project_id).await? {
        if guarantee.kind == GuaranteeKind::Advance {
            continue;
        }
        let kind = kind_text(&guarantee);
        let opened = tasks::open(
            state,
            &project,
            Task {
                subject: guarantee.subject(),
                role: permissions::ACCOUNTANT_ROLE,
                assignee: None,
                source_key: format!("construction.guarantee_release:{}", guarantee.id),
                title: english(
                    "construction.guarantee.release_title",
                    &[("kind", &kind), ("project", &project.name)],
                ),
                description: english(
                    "construction.guarantee.release_body",
                    &[
                        ("bank", &guarantee.bank),
                        ("amount", &aed(guarantee.amount)),
                    ],
                ),
                due: None,
            },
        )
        .await?;
        if opened.is_some() {
            made += 1;
        }
    }
    Ok(made)
}

/// Tells the accountant and the project manager that guarantee `id` expired.
async fn tell_expired(state: &AppState, id: i64) -> Result {
    let guarantee = Guarantee::find_or_404(&state.db, id).await?;
    let project = Project::find_or_404(&state.db, guarantee.project_id).await?;
    let kind = kind_text(&guarantee);
    let title = english(
        "construction.guarantee.expired_title",
        &[("kind", &kind), ("project", &project.name)],
    );
    let expires_on = guarantee.expires_on.unwrap_or_default();
    let body = english(
        "construction.guarantee.expired_body",
        &[
            ("bank", &guarantee.bank),
            ("amount", &aed(guarantee.amount)),
            ("date", &expires_on),
        ],
    );
    tell_role(
        state,
        &project,
        guarantee.subject(),
        permissions::ACCOUNTANT_ROLE,
        &title,
        &body,
    )
    .await?;
    let users = managers(state, &project).await?;
    tell(
        state,
        guarantee.subject(),
        &project.code,
        &users,
        &title,
        &body,
    )
    .await?;
    Ok(())
}

/// Tells the accountant and the project manager when a guarantee expires, so
/// that it never expires silently.
pub(crate) fn listen(app: &mut Registry) {
    app.listen(
        |event: Transitioned<Guarantee>, state: AppState| async move {
            if event.transition == EXPIRE && event.to == GuaranteeStatus::Expired.as_str() {
                tell_expired(&state, event.key).await?;
            }
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn the_task_comes_sixty_days_before_the_expiry() {
        assert_eq!(reminder_day(day(2027, 9, 30)), Some(day(2027, 8, 1)));
        assert_eq!(reminder_day(day(2028, 7, 31)), Some(day(2028, 6, 1)));
    }

    #[test]
    fn the_release_point_is_completion_for_an_advance_and_the_end_of_the_defects_period_for_a_bond()
    {
        let mut project = Project::new("TWR-01", "Tower", wl_kernel::PartyId(1));
        let advance = GuaranteeKind::Advance;
        let bond = GuaranteeKind::Performance;
        assert!(!release_point_reached(&project, advance, day(2028, 1, 1)));
        assert!(!release_point_reached(&project, bond, day(2028, 1, 1)));

        project.status = ProjectStatus::PracticalCompletion;
        project.completed_on = Some(day(2027, 6, 15));
        project.defects_liability_ends_on = Some(day(2028, 6, 15));
        assert!(release_point_reached(&project, advance, day(2027, 6, 16)));
        assert!(!release_point_reached(&project, bond, day(2028, 6, 15)));
        assert!(release_point_reached(&project, bond, day(2028, 6, 16)));
        assert!(!release_point_reached(
            &project,
            GuaranteeKind::RetentionBond,
            day(2028, 6, 15)
        ));

        project.status = ProjectStatus::Closed;
        assert!(release_point_reached(&project, bond, day(2028, 1, 1)));
    }
}
