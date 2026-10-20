//! What a suspension holds (04 §4.2, §4.5, §7.1 *Engagement* `suspend`,
//! *Task*, *Subscription*; BL-ACC-03).
//!
//! After an engagement's `suspend` or `auto_suspend` commits, [`hold`]:
//!
//! - holds its deadline tasks (wl-ticketing, through the kernel's
//!   [`DeadlineTasks`](wl_kernel::deadlines::DeadlineTasks)): they keep their
//!   state and due date, client reminders stop, and `mark_overdue` still runs
//!   because the due date is a fact; tasks generated meanwhile are created
//!   held;
//! - pauses the retainer (wl-invoicing, through the kernel's
//!   [`Subscriptions`](wl_kernel::subscriptions::Subscriptions)) with the
//!   cause *agreement* and the suspension's neutral reason (*on hold* after a
//!   block, on top of the *party block* cause wl-invoicing adds itself); a
//!   cycle invoice already issued stays;
//! - and, through the engagement's status, stops the request lists
//!   (`ACC.RUL.02` records the period as skipped) and the client reminders
//!   of open requests (`POR.RUL.01`) in [`crate::request_lists`].
//!
//! `ACC.RUL.09` then sends or prepares the statutory deadline notice
//! ([`crate::deadline_notices`]). Time may still be logged against a
//! suspended engagement ([`Engagement::accepts_time_entries`]). Nothing here
//! ends the engagement.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::deadlines::{DeadlineTaskSummary, ReleasedTasks, deadline_tasks};
use wl_kernel::subscriptions::{PauseCause, PauseSubscription, SubscriptionSummary, subscriptions};

use crate::engagement::Engagement;
use crate::reasons;

/// What [`hold`] did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Held {
    /// The engagement's tasks afterwards (the open ones held).
    pub tasks: Vec<DeadlineTaskSummary>,
    /// The retainer afterwards.
    pub retainer: Vec<SubscriptionSummary>,
}

/// Holds the tasks of the suspended `engagement` and pauses its retainer
/// from `on`.
///
/// # Errors
///
/// The task and subscription services' errors.
pub async fn hold(state: &AppState, engagement: &Engagement, on: NaiveDate) -> Result<Held> {
    let subject = engagement.subject();
    let tasks = deadline_tasks(state).hold(state, &subject, on).await?;
    let reason = engagement
        .reason_code
        .clone()
        .unwrap_or_else(|| reasons::ON_HOLD.to_owned());
    let pause = PauseSubscription::new(PauseCause::Agreement, on).reason(reason);
    let retainer = subscriptions(state).pause(state, &subject, pause).await?;
    Ok(Held { tasks, retainer })
}

/// What [`release`] did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Released {
    /// The tasks released, those that went overdue while held, and the
    /// manager's task listing them.
    pub tasks: ReleasedTasks,
    /// The retainer afterwards.
    pub retainer: Vec<SubscriptionSummary>,
}

/// Releases the tasks of the resumed `engagement` and resumes its retainer
/// on `on` (the cause *agreement* removed; the next cycle date recalculated
/// by wl-invoicing, skipped cycles not billed).
///
/// # Errors
///
/// The task and subscription services' errors.
pub async fn release(state: &AppState, engagement: &Engagement, on: NaiveDate) -> Result<Released> {
    let subject = engagement.subject();
    let tasks = deadline_tasks(state).release(state, &subject, on).await?;
    let retainer = subscriptions(state)
        .resume(state, &subject, PauseCause::Agreement, on)
        .await?;
    Ok(Released { tasks, retainer })
}
