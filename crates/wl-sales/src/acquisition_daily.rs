//! The nightly acquisition run (wl-sales §5.8 step 4, issue #139): the task
//! `sales:acquisitions` at 00:15 Asia/Dubai, also callable as [`run_daily`].
//!
//! For each live acquisition with a first sale that no longer stands (voided,
//! cancelled or credited in full) it links the party's next sale as the first
//! sale (the acquired date stays), or with none clears the first sale, and
//! `CustomerAcquisitionChanged` is emitted
//! ([`crate::acquisition::replace_first_sale`]). Each acquisition is handled
//! on its own, so a failure on one is printed and counted and the run goes
//! on. A second run finds nothing to do.

use renox::prelude::*;

use crate::acquisition::{PartyAcquisition, Replaced, replace_first_sale};

/// The daily scheduled task, and the command, that run [`run_daily`].
pub const TASK: &str = "sales:acquisitions";

/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "00:15";

/// The time zone of [`TASK`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// What a run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// First sales replaced by the party's next sale.
    pub replaced: usize,
    /// First sales cleared because the party has no other sale.
    pub cleared: usize,
    /// Acquisitions that failed.
    pub failed: usize,
}

/// Runs the nightly pass.
pub async fn run_daily(state: &AppState) -> Result<Report> {
    let mut report = Report::default();
    let linked = PartyAcquisition::query()
        .where_(PartyAcquisition::FIRST_SALE_ID.is_not_null())
        .where_(PartyAcquisition::SUPERSEDED_BY.is_null())
        .order_by(PartyAcquisition::ID)
        .get(&state.db)
        .await?;
    for acquisition in linked {
        match replace_first_sale(state, &acquisition).await {
            Ok(Some(Replaced::By(_))) => report.replaced += 1,
            Ok(Some(Replaced::Cleared)) => report.cleared += 1,
            Ok(None) => {}
            Err(error) => {
                eprintln!("{TASK}: acquisition {} failed: {error}", acquisition.id);
                report.failed += 1;
            }
        }
    }
    Ok(report)
}

/// Registers the daily [`TASK`] and the command of the same name.
pub(crate) fn schedule(app: &mut Registry) {
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run_daily(&state).await.map(|_| ())
        })
        .timezone(TASK_ZONE);
    app.command(
        TASK,
        "Replace or clear the first sale of acquisitions whose first sale was voided, \
         cancelled or credited in full",
        |_args, state: AppState| async move {
            let report = run_daily(&state).await?;
            println!(
                "{} first sales replaced, {} cleared, {} failed.",
                report.replaced, report.cleared, report.failed
            );
            Ok(())
        },
    );
}
