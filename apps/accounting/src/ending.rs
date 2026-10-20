//! Ending an engagement mid-period (04 §4.5 *Engagement ends mid-period*,
//! §7.1 *Engagement* `end`; BL-ACC-06).
//!
//! **Before `end`** the partner works the End checklist ([`checklist`]); the
//! transition is refused while an item is open, and the guard lists them
//! all:
//!
//! - every open return is filed or cancelled ([`crate::returns::cancel`],
//!   BL-ACC-17);
//! - unbilled time (WIP) is billed or written off ([`crate::wip`],
//!   `ACC.ACL.09`); WIP is the time module's, a seam that answers zero until
//!   `wl-attendance` exists (Class B: documented, not faked into 04);
//! - a final invoice or credit is issued: the retainer's cancel makes the
//!   pro rata document, so a retainer engagement is settled; another
//!   engagement needs time billed, or the partner's confirmation that
//!   nothing is left to bill ([`confirm_nothing_to_bill`]).
//!
//! **A future end date** is scheduled, not run ([`schedule`]): the
//! engagement stays `active`, and the daily run ([`run_scheduled`], 01:30
//! Asia/Dubai) ends it on that date as the system with the guards checked
//! again. When one fails the end does not run, the partner gets one task
//! listing the open items, and the run tries again each day until the guards
//! pass (the engagement then ends with the scheduled date as its end date,
//! the pro rata computed from it, and the task closes), or the partner
//! changes or removes the date ([`unschedule`]).
//!
//! **After `end`** ([`after_end`], once `ACC.RUL.09` has listed the deadlines
//! of the next 90 days, [`crate::deadline_notices`]): the retainer is
//! cancelled with the pro rata final invoice or credit (a refund to a
//! blocked client is not paid: wl-invoicing leaves it, `PTY.ACL.03`), the
//! upcoming tasks are waived and the started ones listed for the manager
//! (`waive a started task` stays off), and the portal turns read-only for 90
//! days ([`crate::portal`]).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::deadlines::{EndedTasks, deadline_tasks};
use wl_kernel::documents::today;
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::subscriptions::{CancelSubscription, SubscriptionSummary, subscriptions};
use wl_kernel::tickets::task_creator;
use wl_kernel::wip::unbilled_time;

use crate::cdd::CddFile;
use crate::db::{refused, set_engagement};
use crate::engagement::{Engagement, EngagementStatus, FeeBasis};
use crate::engagement_machine::{AUTO_END, EngagementInput};
use crate::permissions::{self, PARTNER_ROLE};
use crate::reasons;
use crate::returns;
use crate::tax_return::Return;
use crate::texts::english;

/// The scheduled task and command of the daily run of the scheduled ends.
pub const SCHEDULED_END_TASK: &str = "accounting:scheduled_ends";
/// When it runs, Asia/Dubai.
pub const SCHEDULED_END_AT: &str = "01:30";

/// The key of the partner's task for a scheduled end that could not run.
pub fn scheduled_key(engagement: i64) -> String {
    format!("accounting.scheduled_end:{engagement}")
}

/// What stands between an engagement and its end.
#[derive(Debug, Clone)]
pub struct Checklist {
    /// The returns still open: neither filed nor cancelled.
    pub open_returns: Vec<Return>,
    /// The unbilled time left, in fils.
    pub unbilled: i64,
    /// Whether a final invoice or credit is, or will be, issued (the
    /// retainer's pro rata, time billed, or nothing to bill confirmed).
    pub settled: bool,
}

impl Checklist {
    /// Whether nothing is left open.
    pub fn is_clear(&self) -> bool {
        self.open_returns.is_empty() && self.unbilled == 0 && self.settled
    }

    /// The open items, one line each, in the order of the checklist.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        for ret in &self.open_returns {
            problems.push(english(
                "accounting.ending.problem.open_return",
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
                    (
                        "status",
                        &english(
                            &format!("accounting.returns.status.{}", ret.status.as_str()),
                            &[],
                        )
                        .to_lowercase(),
                    ),
                ],
            ));
        }
        if self.unbilled > 0 {
            problems.push(english(
                "accounting.ending.problem.unbilled",
                &[("amount", &crate::money_text(self.unbilled))],
            ));
        }
        if !self.settled {
            problems.push(english("accounting.ending.problem.no_settlement", &[]));
        }
        problems
    }
}

/// The End checklist of `engagement`.
///
/// # Errors
///
/// The database's and the time module's errors.
pub async fn checklist(state: &AppState, engagement: &Engagement) -> Result<Checklist> {
    let open_returns = returns::open_of(&state.db, engagement.id).await?;
    let wip = unbilled_time(state)
        .summary(state, &engagement.subject())
        .await?;
    let retainer = engagement.fee_basis == FeeBasis::Retainer && engagement.retainer_fee > 0;
    let settled = retainer || wip.billed > 0 || engagement.nothing_to_bill_on.is_some();
    Ok(Checklist {
        open_returns,
        unbilled: wip.unbilled,
        settled,
    })
}

/// The input of an `end` or `auto_end` of `engagement` on `on`: the open
/// items of the checklist, and whether the client's relationship ended
/// through CDD (a restricted end: nothing goes to the client
/// automatically, `ACC.ACL.14`).
///
/// # Errors
///
/// The database's and the time module's errors.
pub async fn end_input(
    state: &AppState,
    engagement: &Engagement,
    on: NaiveDate,
) -> Result<EngagementInput> {
    let restricted = CddFile::current(&state.db, engagement.client())
        .await?
        .is_some_and(|file| file.ended_relationship());
    Ok(EngagementInput {
        restricted,
        end_problems: checklist(state, engagement).await?.problems(),
        ..EngagementInput::on(on)
    })
}

/// `Forbidden` unless `user` holds `engagements.end`.
fn require_end(user: &AuthUser) -> Result {
    if user.has_permission(permissions::ENGAGEMENTS_END) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

/// The engagement `id` if it can still end (`active` or `suspended`).
async fn endable(state: &AppState, id: i64) -> Result<Engagement> {
    let engagement = Engagement::find_or_404(&state.db, id).await?;
    if !matches!(
        engagement.status,
        EngagementStatus::Active | EngagementStatus::Suspended
    ) {
        return Err(refused(english(
            "accounting.ending.problem.not_endable",
            &[],
        )));
    }
    Ok(engagement)
}

/// `by` (the partner) confirms that nothing is left to bill on engagement
/// `id`: neither a retainer's pro rata nor billed time gives a final invoice
/// or credit, and no unbilled time is left. It satisfies the checklist's
/// last item.
///
/// # Errors
///
/// `Forbidden` without `engagements.end`; a 422 while unbilled time is left
/// or unless the engagement can end.
pub async fn confirm_nothing_to_bill(
    state: &AppState,
    by: &AuthUser,
    id: i64,
) -> Result<Engagement> {
    require_end(by)?;
    let engagement = endable(state, id).await?;
    let list = checklist(state, &engagement).await?;
    if list.unbilled > 0 {
        return Err(refused(english(
            "accounting.ending.problem.unbilled",
            &[("amount", &crate::money_text(list.unbilled))],
        )));
    }
    let mut tx = state.db.begin().await?;
    set_engagement(
        &mut tx,
        id,
        &[
            ("nothing_to_bill_on", &Some(today())),
            ("nothing_to_bill_by", &Some(by.id)),
        ],
    )
    .await?;
    tx.commit().await?;
    Engagement::find_or_404(&state.db, id).await
}

/// `by` (the partner) schedules the end of engagement `id` for `on` (after
/// today) with the neutral `reason` of [`reasons::END_REASONS`]; replaces an
/// earlier date. The engagement stays as it is until the daily run ends it.
///
/// # Errors
///
/// `Forbidden` without `engagements.end`; a 422 for a date that is not after
/// today, a reason not in the list, or an engagement that cannot end.
pub async fn schedule(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    on: NaiveDate,
    reason: &str,
) -> Result<Engagement> {
    require_end(by)?;
    let engagement = endable(state, id).await?;
    if on <= today() {
        return Err(refused(english("accounting.ending.problem.past_date", &[])));
    }
    let reason = reason.trim();
    if !reasons::is_one_of(Some(reason), &reasons::END_REASONS) {
        return Err(refused(english("accounting.problems.reason_unknown", &[])));
    }
    let mut tx = state.db.begin().await?;
    set_engagement(
        &mut tx,
        engagement.id,
        &[
            ("end_scheduled_on", &Some(on)),
            ("end_scheduled_reason", &Some(reason.to_owned())),
            ("end_scheduled_by", &Some(by.id)),
        ],
    )
    .await?;
    tx.commit().await?;
    Engagement::find_or_404(&state.db, id).await
}

/// `by` (the partner) removes the scheduled end of engagement `id`; the
/// partner's task for a refused try closes.
///
/// # Errors
///
/// `Forbidden` without `engagements.end`.
pub async fn unschedule(state: &AppState, by: &AuthUser, id: i64) -> Result<Engagement> {
    require_end(by)?;
    let engagement = Engagement::find_or_404(&state.db, id).await?;
    let mut tx = state.db.begin().await?;
    set_engagement(
        &mut tx,
        id,
        &[
            ("end_scheduled_on", &None::<NaiveDate>),
            ("end_scheduled_reason", &None::<String>),
            ("end_scheduled_by", &None::<i64>),
        ],
    )
    .await?;
    tx.commit().await?;
    let note = english("accounting.ending.task_removed", &[]);
    close_task(state, &engagement, &note).await?;
    Engagement::find_or_404(&state.db, id).await
}

/// What a run of the scheduled ends did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScheduledEnds {
    /// The engagements ended.
    pub ended: Vec<i64>,
    /// The engagements whose end the checklist refused; each has the
    /// partner's task.
    pub refused: Vec<i64>,
}

/// Runs the scheduled ends due on or before `on`: each engagement whose
/// scheduled date has come ends as the system with the scheduled date as its
/// end date, or, when the checklist has an open item, stays as it is and the
/// partner gets one task listing the items (tried again the next day).
///
/// # Errors
///
/// The database's and the services' errors.
pub async fn run_scheduled(state: &AppState, on: NaiveDate) -> Result<ScheduledEnds> {
    let mut report = ScheduledEnds::default();
    let scheduled = Engagement::query()
        .where_(Engagement::END_SCHEDULED_ON.is_not_null())
        .order_by(Engagement::ID)
        .get(&state.db)
        .await?;
    for engagement in scheduled {
        let Some(date) = engagement.end_scheduled_on else {
            continue;
        };
        if date > on
            || !matches!(
                engagement.status,
                EngagementStatus::Active | EngagementStatus::Suspended
            )
        {
            continue;
        }
        let input = end_input(state, &engagement, date).await?;
        if !input.end_problems.is_empty() {
            open_task(state, &engagement, date, &input.end_problems).await?;
            report.refused.push(engagement.id);
            continue;
        }
        let reason = engagement
            .end_scheduled_reason
            .clone()
            .unwrap_or_else(|| reasons::OTHER.to_owned());
        let run = engagement
            .transition(AUTO_END)
            .by_system()
            .reason(&reason)
            .input(&input)
            .run(state)
            .await;
        match run {
            Ok(_) => report.ended.push(engagement.id),
            Err(TransitionError::NotFrom(_) | TransitionError::Conflict) => {}
            Err(TransitionError::Guard(message)) => {
                open_task(state, &engagement, date, &[message]).await?;
                report.refused.push(engagement.id);
            }
            Err(other) => return Err(other.into_error()),
        }
    }
    Ok(report)
}

/// Gives the partner one task listing `problems` of the scheduled end of
/// `engagement` on `date`.
async fn open_task(
    state: &AppState,
    engagement: &Engagement,
    date: NaiveDate,
    problems: &[String],
) -> Result {
    let number = engagement.number();
    let title = english(
        "accounting.ending.task_title",
        &[("number", &number), ("date", &date)],
    );
    let description = english(
        "accounting.ending.task_description",
        &[
            ("number", &number),
            ("date", &date),
            ("items", &problems.join("\n")),
            (
                "link",
                &format!("/accounting/engagements/{}", engagement.id),
            ),
        ],
    );
    let task = RoleTask::new(
        engagement.subject(),
        PARTNER_ROLE,
        &title,
        &scheduled_key(engagement.id),
    )
    .description(&description);
    role_tasks::open(state, task).await?;
    Ok(())
}

/// Closes the partner's task for a scheduled end of `engagement`, if open.
async fn close_task(state: &AppState, engagement: &Engagement, note: &str) -> Result {
    let creator = task_creator(state);
    if let Some(open) = creator
        .find_open(state, &scheduled_key(engagement.id))
        .await?
    {
        creator.complete(state, open.id, note).await?;
    }
    Ok(())
}

/// What [`after_end`] did.
#[derive(Debug, Clone, Default)]
pub struct Ended {
    /// The retainer afterwards (cancelled, with its pro rata document).
    pub retainer: Vec<SubscriptionSummary>,
    /// The tasks waived, and those left for the manager.
    pub tasks: EndedTasks,
}

/// The work after engagement `id` ended: the retainer is cancelled from the
/// end date with the pro rata final invoice or credit, the upcoming tasks
/// are waived and the open ones left listed for the manager; the partner's
/// task for a scheduled end closes. Call it after `ACC.RUL.09` listed the
/// deadlines, which the waiver would take out of the notice.
///
/// # Errors
///
/// The subscription and task services' errors.
pub async fn after_end(state: &AppState, id: i64) -> Result<Ended> {
    let engagement = Engagement::find_or_404(&state.db, id).await?;
    let on = engagement.ended_on.unwrap_or_else(today);
    let note = english("accounting.ending.task_done", &[]);
    close_task(state, &engagement, &note).await?;
    let subject = engagement.subject();
    let cancel =
        CancelSubscription::new(on).reason(english("accounting.ending.cancel_reason", &[]));
    let retainer = subscriptions(state).cancel(state, &subject, cancel).await?;
    let tasks = deadline_tasks(state)
        .end_source(state, &subject, on)
        .await?;
    Ok(Ended { retainer, tasks })
}

/// The scheduled run, for the daily task and its command.
async fn daily(state: &AppState) -> Result<ScheduledEnds> {
    run_scheduled(state, today()).await
}

/// Registers the daily run of the scheduled ends and its command.
pub(crate) fn register(app: &mut Registry) {
    app.schedule()
        .daily_at(
            SCHEDULED_END_AT,
            SCHEDULED_END_TASK,
            |state: AppState| async move { daily(&state).await.map(|_| ()) },
        )
        .timezone("Asia/Dubai");
    app.command(
        SCHEDULED_END_TASK,
        "End the engagements whose scheduled end date has come, or tell the partner what stops it (BL-ACC-06)",
        |_args, state: AppState| async move {
            let report = daily(&state).await?;
            println!(
                "{} engagements ended, {} refused by the End checklist.",
                report.ended.len(),
                report.refused.len()
            );
            Ok(())
        },
    );
}
