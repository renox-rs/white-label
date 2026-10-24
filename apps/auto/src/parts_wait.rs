//! One wait of a job card for parts (08 §7.1 *Job card* `wait_parts`,
//! `AUT.RUL.16`): when it began, the legal supply date, and which steps of the
//! daily rule fired for it. A job card that waits again gets a new row; the
//! earlier waits stay as history.

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

/// A wait for parts.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "auto_parts_waits", index(job_card_id))]
pub struct PartsWait {
    /// The primary key.
    pub id: i64,
    /// The job card.
    #[model(references = "auto_job_cards")]
    pub job_card_id: i64,
    /// When the wait began.
    pub started_at: DateTime,
    /// The legal supply date: the start plus the shortest period among the
    /// sources of the parts requested.
    pub legal_date: NaiveDate,
    /// The source that gave the shortest period.
    pub source: String,
    /// When the service advisor was told the date is near.
    pub advisor_notified_at: Option<DateTime>,
    /// When the customer was told (or the advisor given a call to make).
    pub customer_notified_at: Option<DateTime>,
    /// When the branch manager was told the legal date came.
    pub legal_notified_at: Option<DateTime>,
    /// When the wait ended (the parts arrived).
    pub ended_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl PartsWait {
    /// The waits of job card `job_card_id`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_job_card(db: &Db, job_card_id: i64) -> Result<Vec<PartsWait>> {
        PartsWait::query()
            .where_(PartsWait::JOB_CARD_ID.eq(job_card_id))
            .order_by(PartsWait::ID)
            .get(db)
            .await
    }

    /// The wait of job card `job_card_id` that is not over.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn open_of(db: &Db, job_card_id: i64) -> Result<Option<PartsWait>> {
        Ok(PartsWait::of_job_card(db, job_card_id)
            .await?
            .into_iter()
            .rfind(|wait| wait.ended_at.is_none()))
    }
}
