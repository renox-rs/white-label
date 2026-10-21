//! The checks that run when a record is saved or corrected (wl-attendance §4
//! `record` and `correct` effects): the overlap check, the overtime cap and
//! the midday break of `HRM.RUL.13`, and `HRM.RUL.14` for the permit, the
//! visa and the qualifications. They flag, they never refuse.
//!
//! What reads other modules through the kernel ([`permit::failures`]) is
//! asked first, by [`prepare`], before the transaction of the record opens;
//! [`run_in`] then flags inside it.

use renox::chrono::NaiveDate;
use renox::db::Transaction;
use renox::prelude::*;
use wl_kernel::workforce::EmployeeId;

use crate::flags::{self, FlagType, Pending};
use crate::location::WorkLocation;
use crate::record::AttendanceRecord;
use crate::{midday, overlap, overtime, permit};

/// What was asked of the other modules before the transaction opened.
#[derive(Debug, Clone, Default)]
pub struct Prepared {
    /// The note of the `permit_invalid` flag; none when everything is valid.
    pub permit_note: Option<String>,
}

/// Asks the other modules what the checks of `employee` on `date` at
/// `location` need.
pub async fn prepare(
    state: &AppState,
    employee: EmployeeId,
    location: &WorkLocation,
    date: NaiveDate,
) -> Result<Prepared> {
    let failed = permit::failures(state, employee, location, date).await?;
    Ok(Prepared {
        permit_note: (!failed.is_empty()).then(|| permit::note_of(&failed, date)),
    })
}

/// Runs every check on `record` inside `tx`; what they add waits in
/// `pending` until the transaction has committed.
pub async fn run_in(
    state: &AppState,
    tx: &mut Transaction,
    record: &AttendanceRecord,
    location: &WorkLocation,
    prepared: &Prepared,
    pending: &mut Pending,
) -> Result {
    overlap::flag_in(state, tx, record, pending).await?;
    overtime::check_in(state, tx, record, pending).await?;
    midday::check_in(state, tx, record, location, pending).await?;
    if let Some(note) = &prepared.permit_note
        && record.counts()
    {
        flags::add_in(tx, record.id, FlagType::PermitInvalid, note, pending).await?;
    }
    Ok(())
}
