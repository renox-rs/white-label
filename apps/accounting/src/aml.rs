//! The AML case: the MLRO's internal record of a suspicion (04 §6 *AML
//! case*, §7.1 *AML case*, `ACC.AML.01`, `ACC.ACL.05`; BL-ACC-10).
//!
//! A case is **visible only to the MLRO and deputy** (`aml.manage`): other
//! staff and the portal never see it, in no page, list, history or export
//! (no tipping off). A member of staff may raise a concern (`aml.raise`) and
//! sees nothing after. The status moves only through the machine in
//! [`crate::aml_machine`]; the services are in [`crate::aml_cases`].
//!
//! A case is either a [`AmlKind::Concern`] raised by a person, or a
//! [`AmlKind::Sanctions`] case opened by the system on a confirmed
//! sanctions match (`ACC.RUL.12`): one open sanctions case per client
//! company, with one [`AmlMatch`] for each confirmed screening linked to it.

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::{PartyId, Subject};

use crate::cdd::ScreenedRole;

/// The subject type of an AML case.
pub const AML_CASE_SUBJECT_TYPE: &str = "accounting.aml_case";

/// Where an AML case stands (§7.1 *AML case*).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AmlStatus {
    /// A concern was raised, or a sanctions match was confirmed.
    #[default]
    Open,
    /// The MLRO is investigating.
    UnderReview,
    /// A suspicious transaction report was filed on goAML (terminal).
    Reported,
    /// Reviewed; no report needed, the reason recorded (terminal).
    ClosedNoAction,
}

impl AmlStatus {
    /// Whether the case is still being worked: `open` or `under_review`.
    pub fn is_open(self) -> bool {
        matches!(self, AmlStatus::Open | AmlStatus::UnderReview)
    }
}

/// How a case came about.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AmlKind {
    /// A concern raised by a member of staff.
    #[default]
    Concern,
    /// A confirmed sanctions match (`ACC.RUL.12`).
    Sanctions,
}

/// What the MLRO decides about the client's engagements after a report
/// (§7.1 *AML case* `report`).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AmlDecision {
    /// Nothing changes.
    #[default]
    Continue,
    /// The engagements are suspended with the neutral reason *on hold* and
    /// the client company is blocked with the restricted reason
    /// `aml_concern`.
    Suspend,
    /// The CDD file is exited ([`crate::cdd_files::exit`]).
    Exit,
}

/// An AML case about a client company.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "accounting_aml_cases",
    state_machine,
    index(client_party_id),
    index(status)
)]
pub struct AmlCase {
    /// The primary key.
    pub id: i64,
    /// The client company, a party.
    pub client_party_id: i64,
    /// A raised concern, or a sanctions match.
    pub kind: AmlKind,
    /// The facts, pre-filled for a sanctions case.
    pub facts: String,
    /// The member of staff who raised it; `None` for the system.
    pub raised_by: Option<i64>,
    /// Set when the rule had to block the client company itself (it was not
    /// blocked by the related-person block).
    pub block_note: Option<String>,
    /// Why `close` found nothing to report.
    pub closing_reason: Option<String>,
    /// The MLRO who closed it.
    pub closed_by: Option<i64>,
    /// The day it was closed.
    pub closed_on: Option<NaiveDate>,
    /// The goAML reference of the report.
    pub goaml_reference: Option<String>,
    /// The date the report was filed.
    pub goaml_date: Option<NaiveDate>,
    /// The MLRO who recorded the report.
    pub reported_by: Option<i64>,
    /// What the MLRO decided about the engagements after the report.
    pub engagement_decision: Option<AmlDecision>,
    /// The MLRO's note with the decision.
    pub decision_note: Option<String>,
    /// The MLRO who recorded it.
    pub decided_by: Option<i64>,
    /// The day it was recorded.
    pub decided_on: Option<NaiveDate>,
    /// The lifecycle state; changed only by a transition.
    pub status: AmlStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A confirmed sanctions match linked to a case, with the freeze and report
/// tasks wl-parties opened for it.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "accounting_aml_matches", index(case_id), index(screening_id))]
pub struct AmlMatch {
    /// The primary key.
    pub id: i64,
    /// The case.
    #[model(references = "accounting_aml_cases")]
    pub case_id: i64,
    /// The client company the match is about.
    pub client_party_id: i64,
    /// The confirmed screening (a wl-parties record).
    pub screening_id: i64,
    /// The party that matched: the client, its UBO, director or signatory.
    pub matched_party_id: i64,
    /// What the matched party is to the client.
    pub role: ScreenedRole,
    /// The sanctions list of the match.
    pub list_name: Option<String>,
    /// The entry on that list.
    pub list_entry: Option<String>,
    /// When the possible match was flagged.
    pub flagged_at: Option<DateTime>,
    /// The freeze task (a wl-parties record).
    pub freeze_task_id: Option<i64>,
    /// The report task (a wl-parties record).
    pub report_task_id: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl AmlCase {
    /// The case as a kernel subject: `accounting.aml_case:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(AML_CASE_SUBJECT_TYPE, self.id)
    }

    /// The client company.
    pub fn client(&self) -> PartyId {
        PartyId(self.client_party_id)
    }

    /// The case shown to the MLRO: `AML-0003`.
    pub fn number(&self) -> String {
        format!("AML-{:04}", self.id)
    }

    /// The open sanctions case of `client`, if there is one (one per client
    /// company).
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn open_sanctions_of(db: &Db, client: PartyId) -> Result<Option<AmlCase>> {
        Ok(AmlCase::query()
            .where_(AmlCase::CLIENT_PARTY_ID.eq(client.0))
            .where_(AmlCase::KIND.eq(AmlKind::Sanctions))
            .order_by(AmlCase::ID)
            .get(db)
            .await?
            .into_iter()
            .find(|case| case.status.is_open()))
    }

    /// The matches linked to the case, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn matches(&self, db: &Db) -> Result<Vec<AmlMatch>> {
        AmlMatch::query()
            .where_(AmlMatch::CASE_ID.eq(self.id))
            .order_by(AmlMatch::ID)
            .get(db)
            .await
    }

    /// Every case about `client`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_client(db: &Db, client: PartyId) -> Result<Vec<AmlCase>> {
        AmlCase::query()
            .where_(AmlCase::CLIENT_PARTY_ID.eq(client.0))
            .order_by(AmlCase::ID)
            .get(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_are_stored_in_snake_case() {
        let names: Vec<_> = AmlStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            names,
            ["open", "under_review", "reported", "closed_no_action"]
        );
        assert!(AmlStatus::Open.is_open() && AmlStatus::UnderReview.is_open());
        assert!(!AmlStatus::Reported.is_open() && !AmlStatus::ClosedNoAction.is_open());
        assert_eq!(AmlKind::Sanctions.as_str(), "sanctions");
        let decisions: Vec<_> = AmlDecision::ALL.iter().map(|d| d.as_str()).collect();
        assert_eq!(decisions, ["continue", "suspend", "exit"]);
    }

    #[test]
    fn the_number_and_subject_name_the_case() {
        let case = AmlCase {
            id: 3,
            client_party_id: 9,
            ..AmlCase::default()
        };
        assert_eq!(case.number(), "AML-0003");
        assert_eq!(case.subject().to_string(), "accounting.aml_case:3");
        assert_eq!(case.client(), PartyId(9));
    }
}
