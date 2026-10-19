//! The daily campaign run (wl-sales §5, #138 A8): the task `sales:campaigns`
//! at 00:05 Asia/Dubai, also callable as [`run_daily`].
//!
//! It ends `active` campaigns whose end date is before today, then activates
//! `draft` campaigns whose start date is on or before today and whose guards
//! pass. A draft past its end date is left alone. Each campaign runs in its
//! own transaction (the machine's `run`), so a failure on one is printed and
//! counted and the run goes on. A second run on the same date finds nothing,
//! because of the machines' from-states, so the events are emitted once.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;

use crate::campaign::{Campaign, CampaignStatus};
use crate::machine::{ACTIVATE_ON_START, END_ON_END_DATE, today_meta};

/// The daily scheduled task, and the command, that run [`run_daily`].
pub const TASK: &str = "sales:campaigns";

/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "00:05";

/// The time zone of [`TASK`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// What a run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Campaigns ended.
    pub ended: usize,
    /// Campaigns activated.
    pub activated: usize,
    /// Campaigns that failed (for example a draft whose fields are missing).
    pub failed: usize,
}

/// Runs the daily campaign pass for `today`.
pub async fn run_daily(state: &AppState, today: NaiveDate) -> Result<Report> {
    let mut report = Report::default();

    let to_end = Campaign::query()
        .where_(Campaign::STATUS.eq(CampaignStatus::Active))
        .where_(Campaign::END_DATE.lt(today))
        .order_by(Campaign::ID)
        .get(&state.db)
        .await?;
    for campaign in to_end {
        match step(state, campaign, END_ON_END_DATE, today).await {
            Ok(()) => report.ended += 1,
            Err(error) => {
                eprintln!("{TASK}: ending campaign failed: {error}");
                report.failed += 1;
            }
        }
    }

    let to_start = Campaign::query()
        .where_(Campaign::STATUS.eq(CampaignStatus::Draft))
        .where_(Campaign::START_DATE.lte(today))
        .order_by(Campaign::ID)
        .get(&state.db)
        .await?;
    for campaign in to_start {
        // Past its end date: left alone (A8).
        if campaign.end_date.is_some_and(|end| end < today) {
            continue;
        }
        match step(state, campaign, ACTIVATE_ON_START, today).await {
            Ok(()) => report.activated += 1,
            Err(error) => {
                eprintln!("{TASK}: activating campaign failed: {error}");
                report.failed += 1;
            }
        }
    }
    Ok(report)
}

/// Runs the system transition `name` on `campaign` in its own transaction;
/// the engine emits `Transitioned` after the commit.
async fn step(state: &AppState, campaign: Campaign, name: &str, today: NaiveDate) -> Result<()> {
    campaign
        .transition(name)
        .by_system()
        .meta(today_meta(today))
        .run(state)
        .await
        .map(|_| ())
        .map_err(|e| e.into_error())
}

/// The `--date YYYY-MM-DD` of the command, else `default`.
fn date_arg(value: Option<&str>, default: NaiveDate) -> Result<NaiveDate> {
    match value {
        None => Ok(default),
        Some(text) => text.parse().map_err(|_| {
            Error::Status(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("--date must be YYYY-MM-DD, got `{text}`"),
            )
        }),
    }
}

/// Registers the daily [`TASK`] and the command of the same name.
pub(crate) fn schedule(app: &mut Registry) {
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run_daily(&state, wl_kernel::documents::today())
                .await
                .map(|_| ())
        })
        .timezone(TASK_ZONE);
    app.command(
        TASK,
        "End campaigns past their end date and activate those whose start date is reached \
         (--date YYYY-MM-DD runs another day)",
        |args, state: AppState| async move {
            let on = date_arg(args.value("--date"), wl_kernel::documents::today())?;
            let report = run_daily(&state, on).await?;
            println!(
                "{} campaigns activated, {} ended, {} failed.",
                report.activated, report.ended, report.failed
            );
            Ok(())
        },
    );
}
