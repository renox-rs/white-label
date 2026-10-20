//! `ACC.RUL.10`: the compliance calendar is regenerated after an approved
//! tax profile change (04 §7.2; BL-ACC-15). The rule run starts here once the
//! approval of a [`TaxProfileChange`](crate::tax_profile_change::TaxProfileChange)
//! commits (BL-ACC-14); it works on every `active` or `suspended` engagement
//! of the client.
//!
//! From the effective date on, the tasks of the old profile are compared with
//! what the new profile plans ([`calendar::plan_ahead`]); a task for a period
//! that ended before the effective date is not compared and stays as it is,
//! whatever its state. For the others:
//!
//! | Existing task | Result |
//! |---|---|
//! | same type, period and due date | kept unchanged |
//! | same type and period, another due date, `upcoming` | keeps its id and gets the new due date (audited) |
//! | same type and period, another due date, started, in review or overdue | listed for the manager |
//! | no such period under the new profile, `upcoming` | `waive` with the reason *tax profile changed*, no next occurrence |
//! | no such period, started, in review or overdue | listed for the manager, never waived |
//! | completed or closed as missed | left as it is |
//!
//! Tasks the new profile plans and no task covers are created (held when the
//! engagement is suspended); a planned task that is already there is never
//! created twice, so a second run changes nothing. Returns in progress for a
//! period the new profile no longer plans are listed too. The manager gets one
//! task listing the items to decide.
//!
//! Decision: the transitional first period (from the effective date to the
//! new profile's first period end on or after it) and the regular period after
//! it are created as `once` tasks followed by the recurring one, so that the
//! next occurrence a completion generates never duplicates a task made here.
//! A deregistration waives every upcoming task of the tax from the effective
//! date; the final return the template would create is not built (the
//! calendar has no deregistration template).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::automation::{RuleRunSpec, RunOutcome, run_rule};
use wl_kernel::deadlines::{
    DeadlineTaskId, DeadlineTaskStatus, DeadlineTaskSummary, TaskRecurrence, WaiveTask,
    deadline_tasks,
};
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::tickets::{Assignee, OpenTicket, TASK_CATEGORY, TicketRef, task_creator};

use crate::calendar::{self, CT_RETURN, Follow, Reach, VAT_RETURN};
use crate::engagement::Engagement;
use crate::permissions::MANAGER_ROLE;
use crate::tax_profile::TaxProfile;
use crate::tax_profile_change::TaxProfileChange;
use crate::tax_return::Return;
use crate::texts::english;

/// The rule's id.
pub const RULE: &str = "ACC.RUL.10";

/// A task that kept its id and got a new due date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moved {
    /// The task.
    pub task: DeadlineTaskId,
    /// The due date it had.
    pub from: NaiveDate,
    /// The due date it has.
    pub to: NaiveDate,
}

/// What one regeneration did for one engagement.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Regenerated {
    /// The tasks that match the new profile and were left as they are.
    pub kept: Vec<DeadlineTaskId>,
    /// The `upcoming` tasks that got a new due date.
    pub rescheduled: Vec<Moved>,
    /// The `upcoming` tasks that no longer apply, waived.
    pub waived: Vec<DeadlineTaskId>,
    /// The tasks created.
    pub created: Vec<DeadlineTaskId>,
    /// The started, in review or overdue tasks that no longer match, left
    /// for the manager.
    pub listed: Vec<DeadlineTaskId>,
    /// The returns in progress for a period the new profile no longer plans.
    pub returns: Vec<i64>,
    /// The manager's task listing them, when there were any.
    pub manager_task: Option<TicketRef>,
}

/// The reason a task is waived or moved with.
fn reason() -> String {
    english("accounting.regeneration.reason", &[])
}

/// Whether `task` is a task of the compliance calendar.
fn is_calendar(task: &DeadlineTaskSummary) -> bool {
    task.task_type == VAT_RETURN || task.task_type == CT_RETURN
}

/// The last period end of the open calendar tasks of `task_type`.
fn last_period(compared: &[&DeadlineTaskSummary], task_type: &str) -> Option<NaiveDate> {
    compared
        .iter()
        .filter(|t| t.task_type == task_type && !t.status.is_terminal())
        .filter_map(|t| t.period_end)
        .max()
}

/// The line of the manager's task for a task left as it is.
fn task_line(task: &DeadlineTaskSummary, new_due: Option<NaiveDate>) -> String {
    match new_due {
        Some(due) => english(
            "accounting.regeneration.line_moved",
            &[
                ("title", &task.title),
                ("status", &task.status.as_str()),
                ("due", &task.due_date),
                ("new", &due),
            ],
        ),
        None => english(
            "accounting.regeneration.line_task",
            &[
                ("title", &task.title),
                ("status", &task.status.as_str()),
                ("due", &task.due_date),
            ],
        ),
    }
}

/// Regenerates the calendar of `engagement` from the day `from` for the tax
/// profile version `profile`; `occurrence` (the version's id) names the
/// manager's task. Nothing for an engagement that is not `active` or
/// `suspended`.
///
/// # Errors
///
/// The database's and the task services' errors.
pub async fn regenerate(
    state: &AppState,
    engagement: &Engagement,
    profile: &TaxProfile,
    from: NaiveDate,
    occurrence: i64,
) -> Result<Regenerated> {
    let mut done = Regenerated::default();
    if !engagement.accepts_time_entries() {
        return Ok(done);
    }
    let api = deadline_tasks(state);
    let existing = api.of_subject(&state.db, &engagement.subject()).await?;
    let compared: Vec<&DeadlineTaskSummary> = existing
        .iter()
        .filter(|t| {
            is_calendar(t)
                && t.status != DeadlineTaskStatus::NotRequired
                && t.period_end.is_some_and(|end| end >= from)
        })
        .collect();
    let reach = Reach {
        vat: last_period(&compared, VAT_RETURN),
        ct: last_period(&compared, CT_RETURN),
    };
    let ahead = calendar::plan_ahead(profile, from, &reach);
    let mut matched = vec![false; ahead.len()];
    let mut lines: Vec<String> = Vec::new();
    let why = reason();

    for task in &compared {
        let slot = ahead.iter().enumerate().position(|(i, a)| {
            !matched[i]
                && a.task.task_type == task.task_type
                && Some(a.task.period_end) == task.period_end
        });
        match slot {
            Some(i) => {
                matched[i] = true;
                let due = ahead[i].task.due_date;
                if task.status.is_terminal() || task.due_date == due {
                    done.kept.push(task.id);
                } else if task.status == DeadlineTaskStatus::Upcoming
                    && api.reschedule(state, task.id, due, &why).await?
                {
                    done.rescheduled.push(Moved {
                        task: task.id,
                        from: task.due_date,
                        to: due,
                    });
                } else {
                    done.listed.push(task.id);
                    lines.push(task_line(task, Some(due)));
                }
            }
            None => {
                if task.status.is_terminal() {
                    done.kept.push(task.id);
                } else if task.status == DeadlineTaskStatus::Upcoming
                    && api
                        .waive(state, task.id, WaiveTask::new(&why).without_next())
                        .await?
                {
                    done.waived.push(task.id);
                } else {
                    done.listed.push(task.id);
                    lines.push(task_line(task, None));
                }
            }
        }
    }

    // What the new profile plans and no task covers is created; a transition
    // period brings the regular one after it, when the first is created.
    let mut create = vec![false; ahead.len()];
    let mut exists = matched.clone();
    for (i, a) in ahead.iter().enumerate() {
        let first = ahead
            .iter()
            .position(|b| b.task.task_type == a.task.task_type)
            .unwrap_or(i);
        // An open task of an earlier period generates this one itself when
        // it is completed (its next occurrence): creating it now would make
        // it twice.
        let months = calendar::step_months(profile, a.task.task_type);
        let follows_open_task = a.follow == Follow::First
            && months > 0
            && existing.iter().any(|t| {
                t.task_type == a.task.task_type
                    && !t.status.is_terminal()
                    && t.status != DeadlineTaskStatus::NotRequired
                    && t.period_end.is_some_and(|end| {
                        end < from && calendar::month_end_after(end, months) == a.task.period_end
                    })
            });
        create[i] = !matched[i]
            && !follows_open_task
            && match a.follow {
                Follow::Transition => create[first],
                Follow::First | Follow::Reach => true,
            };
        exists[i] = matched[i] || create[i] || follows_open_task;
    }
    for (i, a) in ahead.iter().enumerate() {
        if !create[i] {
            continue;
        }
        let later = ahead
            .iter()
            .enumerate()
            .skip(i + 1)
            .any(|(k, b)| b.task.task_type == a.task.task_type && exists[k]);
        let recurrence = if later {
            TaskRecurrence::Once
        } else {
            a.task.recurrence
        };
        let new = calendar::new_task(engagement, &a.task, recurrence).description(&english(
            "accounting.regeneration.description",
            &[("from", &from)],
        ));
        if let Some(id) = api.create(state, new).await? {
            done.created.push(id);
        }
    }

    // The returns in progress for a period the new profile no longer plans.
    for ret in Return::of_engagement(&state.db, engagement.id).await? {
        if !ret.status.is_open() || ret.period_end < from {
            continue;
        }
        let still_planned = ahead.iter().any(|a| {
            a.task.task_type == ret.return_type.task_type()
                && a.task.period_end == ret.period_end
                && a.task.due_date == ret.due_date
        });
        if !still_planned {
            done.returns.push(ret.id);
            lines.push(english(
                "accounting.regeneration.line_return",
                &[
                    ("number", &ret.number()),
                    (
                        "type",
                        &english(
                            &format!("accounting.returns.type.{}", ret.return_type.as_str()),
                            &[],
                        ),
                    ),
                    ("period", &ret.period_end),
                    ("status", &ret.status.as_str()),
                ],
            ));
        }
    }

    if !lines.is_empty() {
        done.manager_task = manager_list(state, engagement, occurrence, from, &lines).await?;
    }
    Ok(done)
}

/// The key the manager's task of a regeneration is found by.
pub fn list_key(engagement: i64, occurrence: i64) -> String {
    format!("accounting.calendar_regeneration:{engagement}:{occurrence}")
}

/// Opens (or finds) the manager's task listing `lines`: for the
/// engagement's manager, else for the holder of the manager role.
async fn manager_list(
    state: &AppState,
    engagement: &Engagement,
    occurrence: i64,
    from: NaiveDate,
    lines: &[String],
) -> Result<Option<TicketRef>> {
    let title = english(
        "accounting.regeneration.list_title",
        &[("number", &engagement.number())],
    );
    let description = english(
        "accounting.regeneration.list_description",
        &[
            ("number", &engagement.number()),
            ("from", &from),
            ("lines", &lines.join("\n")),
            (
                "link",
                &format!("/accounting/engagements/{}", engagement.id),
            ),
        ],
    );
    let key = list_key(engagement.id, occurrence);
    let creator = task_creator(state);
    if let Some(found) = creator.find_open(state, &key).await? {
        return Ok(Some(found));
    }
    match engagement.manager_user_id {
        Some(manager) => {
            let ticket = OpenTicket::new(engagement.subject(), TASK_CATEGORY, &title)
                .description(&description)
                .assign_to(Assignee::user(manager))
                .source_key(&key);
            creator.open(state, ticket).await
        }
        None => {
            let task = RoleTask::new(engagement.subject(), MANAGER_ROLE, &title, &key)
                .description(&description);
            role_tasks::open(state, task).await
        }
    }
}

/// Regenerates the calendars of every `active` or `suspended` engagement of
/// the client of the approved change `change`, from its effective date.
///
/// # Errors
///
/// The database's and the task services' errors.
pub async fn apply(state: &AppState, change: i64) -> Result<Vec<Regenerated>> {
    let change = TaxProfileChange::find_or_404(&state.db, change).await?;
    let Some(profile_id) = change.profile_id else {
        return Ok(Vec::new());
    };
    let profile = TaxProfile::find_or_404(&state.db, profile_id).await?;
    let mut all = Vec::new();
    for engagement in Engagement::of_client(&state.db, change.client()).await? {
        if engagement.accepts_time_entries() {
            all.push(
                regenerate(
                    state,
                    &engagement,
                    &profile,
                    change.effective_date,
                    profile.id,
                )
                .await?,
            );
        }
    }
    Ok(all)
}

/// Runs `ACC.RUL.10` for the approved change `change`, once per change: a
/// second call for the same change does nothing.
///
/// # Errors
///
/// The database's errors (a failing action is retried by the rule run).
pub async fn run_for_change(state: &AppState, change: i64) -> Result<RunOutcome> {
    let row = TaxProfileChange::find_or_404(&state.db, change).await?;
    let subject = row.subject();
    let key = format!("{RULE}:{subject}");
    let spec = RuleRunSpec::new(RULE, subject, key);
    run_rule(state, spec, move |state: AppState| async move {
        apply(&state, change).await.map(|_| ())
    })
    .await
}
