//! The checks that run when a record is saved or corrected (wl-attendance §4
//! `record` and `correct` effects): the overlap check, then the overtime cap
//! of `HRM.RUL.13`; the midday break and `HRM.RUL.14` come with their own
//! issues. They flag, they never refuse.

use renox::db::Transaction;
use renox::prelude::*;

use crate::flags::Pending;
use crate::location::WorkLocation;
use crate::{overlap, overtime};
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
    let _ = location;
    overlap::flag_in(state, tx, record, pending).await?;
    overtime::check_in(state, tx, record, pending).await
}
