//! The review state of an SOP (#52 A12): how its review date stands. It is
//! never "expired"; nothing is blocked because a review date passed.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::Serialize;

use super::REVIEW_REMINDER_DAYS;
use super::model::Sop;
use crate::{Document, DocumentStatus};

/// Where an SOP stands against its review date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    /// Not accepted, so there is no review date.
    NoReviewDate,
    /// More than the reminder window left.
    Ok,
    /// The review date is within the window (or today).
    Due,
    /// The review date has passed.
    Overdue,
}

impl ReviewState {
    /// The state as a key (`no_review_date`, `ok`, `review_due`,
    /// `review_overdue`).
    pub fn key(self) -> &'static str {
        match self {
            ReviewState::NoReviewDate => "no_review_date",
            ReviewState::Ok => "ok",
            ReviewState::Due => "review_due",
            ReviewState::Overdue => "review_overdue",
        }
    }

    /// The badge tone: `warning` when due, `error` when overdue.
    pub fn tone(self) -> &'static str {
        match self {
            ReviewState::Due => "warning",
            ReviewState::Overdue => "error",
            ReviewState::NoReviewDate | ReviewState::Ok => "neutral",
        }
    }
}

/// The state of `review_date` on `today`: `NoReviewDate` without a date,
/// `Ok` while more than [`REVIEW_REMINDER_DAYS`] days remain, `Due` from
/// then through the review date itself, `Overdue` from the day after.
pub fn review_state(review_date: Option<NaiveDate>, today: NaiveDate) -> ReviewState {
    let Some(date) = review_date else {
        return ReviewState::NoReviewDate;
    };
    let left = (date - today).num_days();
    if left < 0 {
        ReviewState::Overdue
    } else if left <= REVIEW_REMINDER_DAYS {
        ReviewState::Due
    } else {
        ReviewState::Ok
    }
}

impl super::Sops {
    /// The review state of `sop` on `today`: of its current version's review
    /// date while that is accepted, else `NoReviewDate` (draft or withdrawn).
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn review_state(db: &Db, sop: &Sop, today: NaiveDate) -> Result<ReviewState> {
        let current = match sop.current_document_id {
            Some(id) => Document::find(db, id).await?,
            None => None,
        };
        Ok(match current {
            Some(d) if d.status == DocumentStatus::Accepted => review_state(d.expiry_date, today),
            _ => ReviewState::NoReviewDate,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    #[test]
    fn no_date_means_no_review_date() {
        assert_eq!(
            review_state(None, date("2026-10-19")),
            ReviewState::NoReviewDate
        );
    }

    #[test]
    fn the_window_runs_from_30_days_through_the_review_date() {
        let review = date("2026-12-01");
        assert_eq!(
            review_state(Some(review), date("2026-10-31")),
            ReviewState::Ok
        );
        assert_eq!(
            review_state(Some(review), date("2026-11-01")),
            ReviewState::Due
        );
        assert_eq!(
            review_state(Some(review), date("2026-12-01")),
            ReviewState::Due
        );
        assert_eq!(
            review_state(Some(review), date("2026-12-02")),
            ReviewState::Overdue
        );
    }

    #[test]
    fn keys_and_tones() {
        assert_eq!(ReviewState::Overdue.key(), "review_overdue");
        assert_eq!(ReviewState::Due.tone(), "warning");
        assert_eq!(ReviewState::Overdue.tone(), "error");
        assert_eq!(ReviewState::Ok.tone(), "neutral");
    }
}
