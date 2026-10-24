//! The substitute vehicle of a warranty repair (08 §2 consumer protection,
//! §4.4 row *Parts do not arrive*, §7.2 `AUT.RUL.16`; BL-AUT-04).
//!
//! A job card linked to a ticket of category *warranty* that is not `ready` 7
//! days after check-in, whatever its state, gets a **substitute vehicle task**
//! (once per job card, [`crate::parts_delays`]). The task is closed only by
//! recording the substitute (plate, from, and to when the car comes back) or by
//! the customer's refusal with a reason: offering a car verbally closes
//! nothing. A job card cannot be delivered while its substitute is open: asked
//! for and not recorded or refused, or recorded and not returned.

use renox::chrono::NaiveDate;
use renox::db::{DateTime, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;
use wl_kernel::tickets::task_creator;

use crate::db::refused;
use crate::job_card::JobCard;
use crate::permissions;
use crate::texts::english;

/// The category of a customer's warranty claim (wl-ticketing).
pub const WARRANTY_CATEGORY: &str = "warranty";

/// The source key of the task to provide a substitute vehicle.
pub fn task_key(job_card_id: i64) -> String {
    format!("auto.substitute:{job_card_id}")
}

/// The substitute vehicle of a job card.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "auto_substitutes", unique(job_card_id))]
pub struct Substitute {
    /// The primary key.
    pub id: i64,
    /// The job card.
    #[model(references = "auto_job_cards")]
    pub job_card_id: i64,
    /// When the substitute was asked for (the task opened).
    pub requested_at: DateTime,
    /// The plate of the car lent.
    pub plate: Option<String>,
    /// The day the car was lent.
    pub from_date: Option<NaiveDate>,
    /// The day it came back.
    pub to_date: Option<NaiveDate>,
    /// Why the customer refused a substitute.
    pub refused_reason: Option<String>,
    /// The user who recorded the car or the refusal.
    pub recorded_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Substitute {
    /// Whether a car was recorded as lent.
    pub fn lent(&self) -> bool {
        self.plate.is_some()
    }

    /// Whether the customer refused one.
    pub fn refused(&self) -> bool {
        self.refused_reason.is_some()
    }

    /// Whether the task is still open: neither a car nor a refusal.
    pub fn requested_only(&self) -> bool {
        !self.lent() && !self.refused()
    }

    /// Whether it stops the delivery: asked for and not answered, or lent and
    /// not returned.
    pub fn is_open(&self) -> bool {
        self.requested_only() || (self.lent() && self.to_date.is_none())
    }

    /// The substitute of job card `job_card_id`, if one was asked for.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_job_card(db: &Db, job_card_id: i64) -> Result<Option<Substitute>> {
        Substitute::query()
            .where_(Substitute::JOB_CARD_ID.eq(job_card_id))
            .first(db)
            .await
    }
}

/// Whether job card `job_card_id` has a substitute that stops its delivery.
///
/// # Errors
///
/// The database's errors.
pub async fn is_open(db: &Db, job_card_id: i64) -> Result<bool> {
    Ok(Substitute::of_job_card(db, job_card_id)
        .await?
        .is_some_and(|substitute| substitute.is_open()))
}

/// The substitute row of a job card that has none: the task is opened by the
/// rule. Once per job card.
///
/// # Errors
///
/// The database's errors.
pub async fn request(state: &AppState, card: &JobCard) -> Result<Substitute> {
    if let Some(found) = Substitute::of_job_card(&state.db, card.id).await? {
        return Ok(found);
    }
    let mut row = Substitute {
        job_card_id: card.id,
        requested_at: now(),
        ..Substitute::default()
    };
    row.save(&state.db).await?;
    Ok(row)
}

/// Closes the task to provide a substitute, with `note`.
async fn close_task(state: &AppState, job_card_id: i64, note: &str) -> Result {
    let creator = task_creator(state);
    if let Some(ticket) = creator.find_open(state, &task_key(job_card_id)).await? {
        creator.complete(state, ticket.id, note).await?;
    }
    Ok(())
}

/// The substitute row the advisor works on: the one the rule asked for, or a
/// new one when the advisor lends a car before the rule asks.
async fn row_for(state: &AppState, card: &JobCard) -> Result<Substitute> {
    request(state, card).await
}

/// Records the car lent to the customer: the plate and the day it was lent,
/// and the day it came back when it has.
///
/// # Errors
///
/// A 403 without `job_cards.advise`; a 422 for a blank plate, a return before
/// the day lent, a substitute the customer refused, or a closed job card
/// without one.
pub async fn record(
    state: &AppState,
    by: &AuthUser,
    job_card_id: i64,
    plate: &str,
    from: NaiveDate,
    to: Option<NaiveDate>,
) -> Result<Substitute> {
    if !by.has_permission(permissions::JOB_CARDS_ADVISE) {
        return Err(Error::Forbidden);
    }
    let card = JobCard::find_or_404(&state.db, job_card_id).await?;
    let plate = plate.trim().to_owned();
    if plate.is_empty() {
        return Err(refused(english("auto.substitute.problem.plate", &[])));
    }
    if to.is_some_and(|to| to < from) {
        return Err(refused(english("auto.substitute.problem.dates", &[])));
    }
    let row = row_for(state, &card).await?;
    if row.refused() {
        return Err(refused(english("auto.substitute.problem.refused", &[])));
    }
    Substitute::query()
        .where_(Substitute::ID.eq(row.id))
        .update(
            &state.db,
            &[
                ("plate", &Some(plate.clone())),
                ("from_date", &Some(from)),
                ("to_date", &to),
                ("recorded_by", &Some(by.id)),
                ("updated_at", &now()),
            ],
        )
        .await?;
    close_task(state, card.id, &plate).await?;
    Substitute::find_or_404(&state.db, row.id).await
}

/// Records the day the substitute car came back.
///
/// # Errors
///
/// A 403 without `job_cards.advise`; a 422 when no car was lent or the day is
/// before the day it was lent.
pub async fn return_vehicle(
    state: &AppState,
    by: &AuthUser,
    job_card_id: i64,
    to: NaiveDate,
) -> Result<Substitute> {
    if !by.has_permission(permissions::JOB_CARDS_ADVISE) {
        return Err(Error::Forbidden);
    }
    let row = Substitute::of_job_card(&state.db, job_card_id)
        .await?
        .filter(Substitute::lent)
        .ok_or_else(|| refused(english("auto.substitute.problem.none_lent", &[])))?;
    if row.from_date.is_some_and(|from| to < from) {
        return Err(refused(english("auto.substitute.problem.dates", &[])));
    }
    Substitute::query()
        .where_(Substitute::ID.eq(row.id))
        .update(
            &state.db,
            &[
                ("to_date", &Some(to)),
                ("recorded_by", &Some(by.id)),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Substitute::find_or_404(&state.db, row.id).await
}

/// Records that the customer refused a substitute vehicle, with the reason.
///
/// # Errors
///
/// A 403 without `job_cards.advise`; a 422 for a blank reason or a car already
/// recorded.
pub async fn refuse(
    state: &AppState,
    by: &AuthUser,
    job_card_id: i64,
    reason: &str,
) -> Result<Substitute> {
    if !by.has_permission(permissions::JOB_CARDS_ADVISE) {
        return Err(Error::Forbidden);
    }
    let card = JobCard::find_or_404(&state.db, job_card_id).await?;
    let reason = reason.trim().to_owned();
    if reason.is_empty() {
        return Err(refused(english("auto.substitute.problem.reason", &[])));
    }
    let row = row_for(state, &card).await?;
    if row.lent() {
        return Err(refused(english("auto.substitute.problem.lent", &[])));
    }
    Substitute::query()
        .where_(Substitute::ID.eq(row.id))
        .update(
            &state.db,
            &[
                ("refused_reason", &Some(reason.clone())),
                ("recorded_by", &Some(by.id)),
                ("updated_at", &now()),
            ],
        )
        .await?;
    close_task(state, card.id, &reason).await?;
    Substitute::find_or_404(&state.db, row.id).await
}

/// Today in Asia/Dubai, for a form that defaults to it.
pub fn default_day() -> NaiveDate {
    today()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_substitute_stops_the_delivery_until_it_is_answered_and_returned() {
        let mut row = Substitute::default();
        assert!(row.is_open(), "asked for and not answered");
        row.plate = Some("Dubai A 12345".to_owned());
        assert!(row.is_open(), "lent and not returned");
        row.to_date = NaiveDate::from_ymd_opt(2026, 11, 12);
        assert!(!row.is_open());
        let refusal = Substitute {
            refused_reason: Some("has a car".to_owned()),
            ..Substitute::default()
        };
        assert!(!refusal.is_open());
        assert_eq!(task_key(4), "auto.substitute:4");
    }
}
