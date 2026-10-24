//! Test drives on a job card (08 §6 *Job card*, `AUT.ACL.22`, BL-AUT-06): the
//! driver, the times and the mileage out and in. A drive that has not come
//! back (no mileage in) blocks `pass_qc`; the mileage in is at least the
//! mileage out.
//!
//! Whoever works the job card records a drive: an advisor, or the technician
//! it is assigned to. The driver is the user who records it; the drivers are
//! *involved* in a later incident ([`crate::incidents`]).

use renox::db::{DateTime, now};
use renox::prelude::*;

use crate::db::refused;
use crate::job_card::JobCard;
use crate::job_cards::may_work;
use crate::test_drive::TestDrive;
use crate::texts::english;

/// A test drive to record. The times default to now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewDrive {
    /// When the vehicle left.
    pub out_at: Option<DateTime>,
    /// The odometer reading out.
    pub odometer_out: i64,
    /// When it came back, for a drive recorded after the fact.
    pub in_at: Option<DateTime>,
    /// The odometer reading back, for a drive recorded after the fact.
    pub odometer_in: Option<i64>,
}

impl NewDrive {
    /// A drive that leaves now at `odometer_out` km.
    pub fn out(odometer_out: i64) -> Self {
        NewDrive {
            odometer_out,
            ..NewDrive::default()
        }
    }

    /// Left at `at`.
    #[must_use]
    pub fn left_at(mut self, at: DateTime) -> Self {
        self.out_at = Some(at);
        self
    }

    /// Came back at `at` with `odometer_in` km.
    #[must_use]
    pub fn back(mut self, at: DateTime, odometer_in: i64) -> Self {
        self.in_at = Some(at);
        self.odometer_in = Some(odometer_in);
        self
    }
}

/// How many test drives of job card `job_card_id` have not come back.
///
/// # Errors
///
/// The database's errors.
pub async fn open_count(db: &Db, job_card_id: i64) -> Result<i64> {
    let drives = TestDrive::of_job_card(db, job_card_id).await?;
    Ok(i64::try_from(drives.iter().filter(|d| d.is_open()).count()).unwrap_or(i64::MAX))
}

/// Records a test drive of job card `job_card_id`.
///
/// # Errors
///
/// A 403 for someone who may not work the job card; a 422 in words for a
/// closed job card, a mileage below the check-in's, a drive already out, a
/// return before the departure or a mileage back below the mileage out.
pub async fn record(
    state: &AppState,
    by: &AuthUser,
    job_card_id: i64,
    new: NewDrive,
) -> Result<TestDrive> {
    let (card, out_at) = check_new(state, by, job_card_id, &new).await?;
    let mut drive = TestDrive {
        job_card_id: card.id,
        driver_user_id: by.id,
        out_at,
        odometer_out: new.odometer_out,
        in_at: new.in_at,
        odometer_in: new.odometer_in,
        ..TestDrive::default()
    };
    drive.save(&state.db).await?;
    Ok(drive)
}

/// What [`record`] and the late entry of a drive ([`crate::late_times`])
/// check before a drive is written: the job card, the user's right to work it,
/// the mileage and the times. Returns the job card and the time out.
///
/// # Errors
///
/// The errors [`record`] lists.
pub(crate) async fn check_new(
    state: &AppState,
    by: &AuthUser,
    job_card_id: i64,
    new: &NewDrive,
) -> Result<(JobCard, DateTime)> {
    let card = JobCard::find_or_404(&state.db, job_card_id).await?;
    if !may_work(by, &card) {
        return Err(Error::Forbidden);
    }
    if !card.status.is_open() {
        return Err(refused(english("auto.drives.problem.closed", &[])));
    }
    if new.odometer_out < card.odometer_in {
        return Err(refused(english(
            "auto.drives.problem.odometer_out",
            &[("km", &card.odometer_in)],
        )));
    }
    if open_count(&state.db, card.id).await? > 0 {
        return Err(refused(english("auto.drives.problem.out", &[])));
    }
    let out_at = new.out_at.unwrap_or_else(now);
    match (new.in_at, new.odometer_in) {
        (None, None) => {}
        (Some(back_at), Some(back_km)) => check_return(out_at, new.odometer_out, back_at, back_km)?,
        _ => return Err(refused(english("auto.drives.problem.half_return", &[]))),
    }
    Ok((card, out_at))
}

/// The return must be after the departure and the mileage not lower.
fn check_return(out_at: DateTime, out_km: i64, back_at: DateTime, back_km: i64) -> Result {
    if back_at < out_at {
        return Err(refused(english("auto.drives.problem.time", &[])));
    }
    if back_km < out_km {
        return Err(refused(english(
            "auto.drives.problem.odometer_in",
            &[("km", &out_km)],
        )));
    }
    Ok(())
}

/// The vehicle of test drive `id` is back: its mileage in (and the time, now
/// when `at` is `None`).
///
/// # Errors
///
/// A 403 for someone who may not work the job card; a 422 in words for a
/// drive that is back already, a mileage below the mileage out or a time
/// before the departure.
pub async fn finish(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    odometer_in: i64,
    at: Option<DateTime>,
) -> Result<TestDrive> {
    let drive = TestDrive::find_or_404(&state.db, id).await?;
    let card = JobCard::find_or_404(&state.db, drive.job_card_id).await?;
    if !may_work(by, &card) {
        return Err(Error::Forbidden);
    }
    if !drive.is_open() {
        return Err(refused(english("auto.drives.problem.back", &[])));
    }
    let back_at = at.unwrap_or_else(now);
    check_return(drive.out_at, drive.odometer_out, back_at, odometer_in)?;
    TestDrive::query()
        .where_(TestDrive::ID.eq(drive.id))
        .where_(TestDrive::ODOMETER_IN.is_null())
        .update(
            &state.db,
            &[
                ("in_at", &Some(back_at)),
                ("odometer_in", &Some(odometer_in)),
                ("updated_at", &now()),
            ],
        )
        .await?;
    TestDrive::find_or_404(&state.db, id).await
}
