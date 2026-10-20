//! The CDD file: the due diligence record of a client (04 §6 *CDD file*, §7.1
//! *CDD file*), with the parties it screens.
//!
//! The screened parties ([`CddParty`]) are the client company and every UBO,
//! director and signatory on the file. CDD `approve` and engagement
//! `activate` read the latest screening of each of them (BL-ACC-01,
//! BL-ACC-08). The machine is in [`crate::cdd_machine`].
//!
//! The exit or rejection reason ([`CddFile::decision_reason`]) is visible to
//! the MLRO and deputy only (`parties.block_reason.view`); it is kept on the
//! file, never in the transition history.

use renox::chrono::{Months, NaiveDate};
use renox::db::{DateTime, Executor};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::{PartyId, Subject};

/// The subject type of a CDD file.
pub const CDD_FILE_SUBJECT_TYPE: &str = "accounting.cdd_file";

/// Where a CDD file stands (§7.1 *CDD file*).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CddStatus {
    /// Information and documents being collected.
    #[default]
    InProgress,
    /// Risk assessed; waiting for approval.
    PendingApproval,
    /// The client may be served.
    Approved,
    /// The periodic review date is reached.
    ReviewDue,
    /// Not accepted at onboarding or at a review (terminal).
    Rejected,
    /// The relationship was ended by the MLRO (terminal).
    Exited,
}

/// The risk rating of a client (`ACC.KYC.02`).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RiskRating {
    /// Not rated yet: the file cannot be submitted.
    #[default]
    Unrated,
    /// Low risk.
    Low,
    /// Medium risk.
    Medium,
    /// High risk (`ACC.ACL.06`).
    High,
}

impl RiskRating {
    /// The months to the next periodic review once approved.
    ///
    /// Decision: the spec schedules reviews "by risk rating" without values;
    /// the usual UAE practice is used: high every year, medium every two
    /// years, low every three. An unrated file is never approved.
    pub fn review_months(self) -> u32 {
        match self {
            RiskRating::High => 12,
            RiskRating::Medium => 24,
            RiskRating::Low | RiskRating::Unrated => 36,
        }
    }

    /// The next review date for a file approved on `on`.
    pub fn next_review(self, on: NaiveDate) -> NaiveDate {
        on.checked_add_months(Months::new(self.review_months()))
            .unwrap_or(on)
    }
}

/// The part a screened party plays for the client.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ScreenedRole {
    /// The client company itself.
    #[default]
    Client,
    /// An ultimate beneficial owner.
    Ubo,
    /// A director.
    Director,
    /// An authorised signatory.
    Signatory,
}

/// The due diligence record of a client company.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "accounting_cdd_files",
    state_machine,
    index(client_party_id),
    index(status)
)]
pub struct CddFile {
    /// The primary key.
    pub id: i64,
    /// The client company, a party.
    pub client_party_id: i64,
    /// The risk rating.
    pub risk_rating: RiskRating,
    /// The risk factors, as the preparer rated them.
    pub risk_factors: String,
    /// The enhanced due diligence notes.
    pub edd_notes: String,
    /// Whether the identity documents of the parties are attached.
    pub identity_documents_attached: bool,
    /// The next periodic review, set on `approve` from the rating.
    pub next_review_date: Option<NaiveDate>,
    /// The MLRO who approved it.
    pub approved_by: Option<i64>,
    /// When it was approved.
    pub approved_at: Option<DateTime>,
    /// Why it was rejected or exited: the MLRO and deputy only.
    pub decision_reason: Option<String>,
    /// The lifecycle state; changed only by a transition.
    pub status: CddStatus,
    /// The user who opened it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A party the CDD file screens.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "accounting_cdd_parties", index(cdd_file_id))]
pub struct CddParty {
    /// The primary key.
    pub id: i64,
    /// The CDD file.
    #[model(references = "accounting_cdd_files")]
    pub cdd_file_id: i64,
    /// The party screened.
    pub party_id: i64,
    /// What it is to the client.
    pub role: ScreenedRole,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl CddParty {
    /// The party.
    pub fn party(&self) -> PartyId {
        PartyId(self.party_id)
    }
}

impl CddFile {
    /// The file as a kernel subject: `accounting.cdd_file:4`.
    pub fn subject(&self) -> Subject {
        Subject::new(CDD_FILE_SUBJECT_TYPE, self.id)
    }

    /// The client company.
    pub fn client(&self) -> PartyId {
        PartyId(self.client_party_id)
    }

    /// The file shown to people: `CDD-0004`.
    pub fn number(&self) -> String {
        format!("CDD-{:04}", self.id)
    }

    /// The current file of `client`: the latest one.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn current<'c>(db: impl Executor<'c>, client: PartyId) -> Result<Option<CddFile>> {
        Ok(CddFile::query()
            .where_(CddFile::CLIENT_PARTY_ID.eq(client.0))
            .order_by_desc(CddFile::ID)
            .limit(1)
            .get(db)
            .await?
            .into_iter()
            .next())
    }

    /// The parties the file screens, in the order they were added. The
    /// client company is always among them.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn parties<'c>(&self, db: impl Executor<'c>) -> Result<Vec<CddParty>> {
        CddParty::query()
            .where_(CddParty::CDD_FILE_ID.eq(self.id))
            .order_by(CddParty::ID)
            .get(db)
            .await
    }

    /// Whether the client's relationship ended through CDD: the file is
    /// `exited` or `rejected`.
    pub fn ended_relationship(&self) -> bool {
        matches!(self.status, CddStatus::Exited | CddStatus::Rejected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_and_roles_are_stored_in_snake_case() {
        let names: Vec<_> = CddStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            names,
            [
                "in_progress",
                "pending_approval",
                "approved",
                "review_due",
                "rejected",
                "exited"
            ]
        );
        let roles: Vec<_> = ScreenedRole::ALL.iter().map(|r| r.as_str()).collect();
        assert_eq!(roles, ["client", "ubo", "director", "signatory"]);
    }

    #[test]
    fn the_review_follows_the_rating() {
        let on = NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();
        assert_eq!(RiskRating::High.next_review(on).to_string(), "2027-11-03");
        assert_eq!(RiskRating::Medium.next_review(on).to_string(), "2028-11-03");
        assert_eq!(RiskRating::Low.next_review(on).to_string(), "2029-11-03");
    }
}
