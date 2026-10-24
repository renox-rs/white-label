//! The estimate approval: one round of an estimate sent to the customer (08
//! §4.4 step 4, `FIN.QUO.01`, `DOC.SIG.01`, `AUT.JOB.02`).
//!
//! Sending an estimate (or added work found during the job) makes a round with
//! a link token. The customer opens the link, reads the lines and approves
//! them, or declines added work; the advisor can also record an approval given
//! by phone or in person. An approval is the customer's consent to the cost,
//! the time and the warranty (08 §2: consent in writing, a warranty of at
//! least 15 days): it records who, how (`link`, `signature`, `recorded`),
//! when, the promised date and the warranty days.
//!
//! Decision: the signature is the drawn or typed signature the form sent, kept
//! as text; wl-documents' signature types (`DOC.SIG.01`) are not wired, see
//! the deferred list.

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

/// The least warranty on a repair and its parts, in days (08 §2, Cabinet
/// Resolution 66 of 2023).
pub const MIN_WARRANTY_DAYS: i64 = 15;

/// Where an estimate round stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ApprovalStatus {
    /// Sent; the customer has not answered.
    #[default]
    Pending,
    /// The customer approved it.
    Approved,
    /// The customer declined it.
    Declined,
    /// Replaced by a later round or ended with the job card.
    Superseded,
}

/// How the customer answered.
pub const METHODS: [&str; 3] = ["link", "signature", "recorded"];

/// An estimate round.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "auto_estimate_approvals", unique(token), index(job_card_id))]
pub struct EstimateApproval {
    /// The primary key.
    pub id: i64,
    /// The job card.
    #[model(references = "auto_job_cards")]
    pub job_card_id: i64,
    /// 1 for the estimate, 2 and up for the added work after it.
    pub revision: i64,
    /// The token in the customer's link.
    pub token: String,
    /// The total of the lines of the round before VAT, in fils.
    pub net_total: i64,
    /// The day the vehicle is promised for (consent to the time).
    pub promised_for: Option<NaiveDate>,
    /// The warranty on the repair and its parts, in days (consent to the
    /// warranty).
    pub warranty_days: i64,
    /// Where the round stands.
    pub status: ApprovalStatus,
    /// The advisor who sent it.
    pub sent_by: Option<i64>,
    /// When it was sent.
    pub sent_at: Option<DateTime>,
    /// When the customer answered.
    pub decided_at: Option<DateTime>,
    /// The name the customer gave.
    pub decided_by_name: Option<String>,
    /// The advisor who recorded the answer, when the customer gave it by phone
    /// or in person.
    pub decided_by_user: Option<i64>,
    /// `link`, `signature` or `recorded`.
    pub method: Option<String>,
    /// The signature, as the form sent it.
    pub signature: Option<String>,
    /// A note with the answer: why added work was declined.
    pub note: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl EstimateApproval {
    /// The path of the page where the customer answers.
    pub fn path(&self) -> String {
        format!("/auto/approve/{}", self.token)
    }

    /// Whether the customer has not answered.
    pub fn is_pending(&self) -> bool {
        self.status == ApprovalStatus::Pending
    }

    /// The round with this token.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn by_token(db: &Db, token: &str) -> Result<Option<EstimateApproval>> {
        EstimateApproval::query()
            .where_(EstimateApproval::TOKEN.eq(token.trim()))
            .first(db)
            .await
    }

    /// The rounds of `job_card_id`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_job_card(db: &Db, job_card_id: i64) -> Result<Vec<EstimateApproval>> {
        EstimateApproval::query()
            .where_(EstimateApproval::JOB_CARD_ID.eq(job_card_id))
            .order_by(EstimateApproval::ID)
            .get(db)
            .await
    }

    /// The round of `job_card_id` the customer has not answered.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn pending_of(db: &Db, job_card_id: i64) -> Result<Option<EstimateApproval>> {
        EstimateApproval::query()
            .where_(EstimateApproval::JOB_CARD_ID.eq(job_card_id))
            .where_(EstimateApproval::STATUS.eq(ApprovalStatus::Pending))
            .order_by_desc(EstimateApproval::ID)
            .first(db)
            .await
    }
}

/// A new token for a customer's link.
pub fn new_token() -> String {
    renox::random_token()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_link_is_the_token_under_the_approve_page() {
        let round = EstimateApproval {
            token: "abc123".to_owned(),
            ..EstimateApproval::default()
        };
        assert_eq!(round.path(), "/auto/approve/abc123");
        assert!(round.is_pending());
    }

    #[test]
    fn tokens_are_not_repeated() {
        assert_ne!(new_token(), new_token());
        assert!(new_token().len() >= 16);
    }
}
