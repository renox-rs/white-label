//! The checks that run when a record is saved or corrected (wl-attendance §4
//! `record` and `correct` effects): the overlap check, then the overtime cap
//! and the midday break of `HRM.RUL.13`; `HRM.RUL.14` comes with its own
//! issue. They flag, they never refuse.

use renox::db::Transaction;
use renox::prelude::*;

use crate::flags::Pending;
use crate::location::WorkLocation;
use crate::{midday, overlap, overtime};
use crate::record::AttendanceRecord;

/// Runs every check on `record` inside `tx`; what they add waits in
/// `pending` until the transaction has committed.
pub async fn run_in(
    state: &AppState,
    tx: &mut Transaction,
    record: &AttendanceRecord,
    location: &WorkLocation,
    pending: &mut Pending,
) -> Result {
    overlap::flag_in(state, tx, record, pending).await?;
    overtime::check_in(state, tx, record, pending).await?;
    midday::check_in(state, tx, record, location, pending).await
}
