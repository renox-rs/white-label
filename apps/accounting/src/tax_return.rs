//! The return: a VAT or corporate tax return of a client for a period (04 §6
//! *Return*, §7.1 *Return*; BL-ACC-16, BL-ACC-17).
//!
//! It links to the client's engagement and to the deadline task of the
//! compliance calendar that tracks the filing ([`Return::task_id`], reached
//! through the kernel's [`DeadlineTasks`](wl_kernel::deadlines::DeadlineTasks)).
//! The status moves only through the machine in [`crate::return_machine`];
//! the services that run it are in [`crate::returns`].
//!
//! The client approver's acceptance is a [`ReturnSignature`] (`DOC.SIG.01`).
//! Decision: wl-documents has no signature request yet (its `DOC.SIG.01` is
//! the acknowledgement of SOPs), so this is the minimal record the spec
//! names (name, time, IP address, document hash); a withdrawal marks it
//! withdrawn and never deletes it.

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::deadlines::DeadlineTaskId;
use wl_kernel::{PartyId, Subject};

use crate::calendar::{CT_RETURN, VAT_RETURN};

/// The subject type of a return.
pub const RETURN_SUBJECT_TYPE: &str = "accounting.return";

/// Which tax the return is for.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ReturnType {
    /// A VAT return.
    #[default]
    Vat,
    /// A corporate tax return.
    Ct,
}

impl ReturnType {
    /// The task type of the calendar task that tracks it
    /// ([`crate::calendar`]).
    pub fn task_type(self) -> &'static str {
        match self {
            ReturnType::Vat => VAT_RETURN,
            ReturnType::Ct => CT_RETURN,
        }
    }
}

/// Where a return stands (§7.1 *Return*).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ReturnStatus {
    /// Workpapers in progress.
    #[default]
    Preparing,
    /// With the reviewer.
    InReview,
    /// Sent to the client approver.
    ClientApproval,
    /// Approved by the client; ready to file.
    Approved,
    /// Filed on EmaraTax; acknowledgement attached; locked (`ACC.ACL.04`).
    Filed,
    /// Replaced by a corrected version (terminal).
    Superseded,
    /// Not filed by the firm (terminal).
    Cancelled,
}

impl ReturnStatus {
    /// Whether the firm still has to file or cancel it: `preparing`,
    /// `in_review`, `client_approval` or `approved`.
    pub fn is_open(self) -> bool {
        matches!(
            self,
            ReturnStatus::Preparing
                | ReturnStatus::InReview
                | ReturnStatus::ClientApproval
                | ReturnStatus::Approved
        )
    }
}

/// How a filed return's error is corrected (`ACC.ACL.04`).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CorrectionRoute {
    /// A voluntary disclosure to the FTA.
    #[default]
    VoluntaryDisclosure,
    /// The adjustment goes in the next return.
    NextReturn,
}

/// A VAT or corporate tax return of a client.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "accounting_returns",
    state_machine,
    index(engagement_id),
    index(client_party_id),
    index(status)
)]
pub struct Return {
    /// The primary key.
    pub id: i64,
    /// The client company, a party.
    pub client_party_id: i64,
    /// The engagement it is prepared under.
    #[model(references = "accounting_engagements")]
    pub engagement_id: i64,
    /// The deadline task of the compliance calendar that tracks it.
    pub task_id: Option<i64>,
    /// VAT or corporate tax.
    pub return_type: ReturnType,
    /// The first day of the period it covers.
    pub period_start: Option<NaiveDate>,
    /// The last day of the period it covers.
    pub period_end: NaiveDate,
    /// The day it is due to the FTA (Asia/Dubai).
    pub due_date: NaiveDate,
    /// The version: 1, then one more for each correction.
    pub version: i64,
    /// The version this one replaces.
    pub supersedes_id: Option<i64>,
    /// The staff member who prepared it.
    pub preparer_user_id: Option<i64>,
    /// The staff member who reviewed it (never the preparer, `ACC.ACL.03`).
    pub reviewer_user_id: Option<i64>,
    /// The figures, as the preparer entered them.
    pub figures: Option<String>,
    /// Whether the workpapers are attached.
    pub workpapers_attached: bool,
    /// The comment of the last `send_back`.
    pub send_back_comment: Option<String>,
    /// The reason it was cancelled ([`crate::returns::CANCEL_REASONS`]).
    pub cancel_reason: Option<String>,
    /// Whether the cancel reason is restricted: the client is told nothing
    /// automatically (`ACC.ACL.14`).
    pub client_messages_suppressed: bool,
    /// The day it was filed.
    pub filed_on: Option<NaiveDate>,
    /// The staff member who filed it.
    pub filed_by_user_id: Option<i64>,
    /// Whether the client filed it through its own EmaraTax user.
    pub filed_by_client: bool,
    /// The filer's tax agent registration number.
    pub tax_agent_registration: Option<String>,
    /// The FTA acknowledgement.
    pub acknowledgement: Option<String>,
    /// How its error is corrected.
    pub correction_route: Option<CorrectionRoute>,
    /// The voluntary disclosure reference.
    pub correction_reference: Option<String>,
    /// Why it was corrected.
    pub correction_reason: Option<String>,
    /// The lifecycle state; changed only by a transition.
    pub status: ReturnStatus,
    /// The user who opened it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Return {
    /// The return as a kernel subject: `accounting.return:7`.
    pub fn subject(&self) -> Subject {
        Subject::new(RETURN_SUBJECT_TYPE, self.id)
    }

    /// The client company.
    pub fn client(&self) -> PartyId {
        PartyId(self.client_party_id)
    }

    /// The return shown to people: `RET-0007`.
    pub fn number(&self) -> String {
        format!("RET-{:04}", self.id)
    }

    /// The deadline task that tracks it.
    pub fn task(&self) -> Option<DeadlineTaskId> {
        self.task_id.map(DeadlineTaskId)
    }

    /// The returns of engagement `engagement`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_engagement(db: &Db, engagement: i64) -> Result<Vec<Return>> {
        Return::query()
            .where_(Return::ENGAGEMENT_ID.eq(engagement))
            .order_by(Return::ID)
            .get(db)
            .await
    }

    /// The returns of `client`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_client(db: &Db, client: PartyId) -> Result<Vec<Return>> {
        Return::query()
            .where_(Return::CLIENT_PARTY_ID.eq(client.0))
            .order_by(Return::ID)
            .get(db)
            .await
    }
}

/// The client approver's acceptance of a return for filing (`DOC.SIG.01`):
/// kept when the approval is withdrawn, only marked.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "accounting_return_signatures", index(return_id))]
pub struct ReturnSignature {
    /// The primary key.
    pub id: i64,
    /// The return.
    #[model(references = "accounting_returns")]
    pub return_id: i64,
    /// The portal user who approved.
    pub signer_user_id: i64,
    /// The name of the signer as it was when signing.
    pub signer_name: String,
    /// When it was signed.
    pub signed_at: DateTime,
    /// The IP address of the signer's browser.
    pub ip_address: Option<String>,
    /// The hash of the document that was approved (the return's summary).
    pub document_hash: String,
    /// When the approval was withdrawn.
    pub withdrawn_at: Option<DateTime>,
    /// Who withdrew it: the client approver, or the reviewer on the client's
    /// written instruction.
    pub withdrawn_by: Option<i64>,
    /// Why.
    pub withdrawn_reason: Option<String>,
    /// The client's written instruction, when a reviewer withdrew.
    pub withdrawn_instruction: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ReturnSignature {
    /// The signatures of return `id`, oldest first, withdrawn ones included.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_return(db: &Db, id: i64) -> Result<Vec<ReturnSignature>> {
        ReturnSignature::query()
            .where_(ReturnSignature::RETURN_ID.eq(id))
            .order_by(ReturnSignature::ID)
            .get(db)
            .await
    }

    /// Whether the approval was withdrawn.
    pub fn is_withdrawn(&self) -> bool {
        self.withdrawn_at.is_some()
    }
}

/// A short stable hash of `text` as 16 hex digits (FNV-1a, 64 bit): the
/// document hash of a signature until wl-documents hashes the summary PDF.
pub fn document_hash(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_are_stored_in_snake_case() {
        let names: Vec<_> = ReturnStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            names,
            [
                "preparing",
                "in_review",
                "client_approval",
                "approved",
                "filed",
                "superseded",
                "cancelled"
            ]
        );
        assert_eq!(ReturnType::Ct.as_str(), "ct");
    }

    #[test]
    fn only_the_four_working_states_are_open() {
        let open: Vec<_> = ReturnStatus::ALL
            .iter()
            .filter(|s| s.is_open())
            .map(|s| s.as_str())
            .collect();
        assert_eq!(
            open,
            ["preparing", "in_review", "client_approval", "approved"]
        );
    }

    #[test]
    fn the_number_subject_and_task_type_name_the_return() {
        let ret = Return {
            id: 7,
            client_party_id: 3,
            task_id: Some(9),
            ..Return::default()
        };
        assert_eq!(ret.number(), "RET-0007");
        assert_eq!(ret.subject().to_string(), "accounting.return:7");
        assert_eq!(ret.task(), Some(DeadlineTaskId(9)));
        assert_eq!(ReturnType::Vat.task_type(), "vat_return");
        assert_eq!(ReturnType::Ct.task_type(), "ct_return");
    }

    #[test]
    fn the_hash_is_stable_and_tells_texts_apart() {
        assert_eq!(document_hash("a"), document_hash("a"));
        assert_ne!(document_hash("a"), document_hash("b"));
        assert_eq!(document_hash("").len(), 16);
    }
}
