//! The engagement: the service contract with a client company (04 §6
//! *Engagement*, §7.1 *Engagement*).
//!
//! The client is a wl-parties party (a company), reached only through the
//! kernel's [`PartyId`]. The status moves only through the machine in
//! [`crate::engagement_machine`]; the services that run it are in
//! [`crate::engagements`].
//!
//! The suspension, end or decline reason is a neutral code of
//! [`crate::reasons`]: a restricted reason (sanctions, AML concern) is never
//! stored here (`ACC.ACL.14`). [`Engagement::client_messages_suppressed`]
//! says whether automatic messages to the client are off for the current
//! reason; it does not say why.

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::{PartyId, Subject};

/// The subject type of an engagement.
pub const ENGAGEMENT_SUBJECT_TYPE: &str = "accounting.engagement";

/// Where an engagement stands (§7.1 *Engagement*).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum EngagementStatus {
    /// Proposal accepted; CDD and the engagement letter in progress.
    #[default]
    Onboarding,
    /// Services running; calendar and billing on.
    Active,
    /// Paused for non-payment, at the client's request, or because the
    /// client is blocked; tasks held.
    Suspended,
    /// Services stopped (terminal).
    Ended,
    /// Not accepted: CDD failed, letter not signed, or the client withdrew
    /// (terminal).
    Declined,
}

/// How the engagement is billed.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FeeBasis {
    /// A recurring retainer (a wl-invoicing subscription).
    #[default]
    Retainer,
    /// A fixed fee.
    Fixed,
    /// Time and materials.
    Time,
}

/// An engagement with a client company.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "accounting_engagements",
    state_machine,
    index(client_party_id),
    index(status)
)]
pub struct Engagement {
    /// The primary key.
    pub id: i64,
    /// The client company, a party.
    pub client_party_id: i64,
    /// The services, as people read them ("Bookkeeping and VAT").
    pub title: String,
    /// How it is billed.
    pub fee_basis: FeeBasis,
    /// The monthly retainer, without VAT, in fils; 0 for none.
    pub retainer_fee: i64,
    /// The first day of service; the activation date when empty.
    pub start_date: Option<NaiveDate>,
    /// The manager running it.
    pub manager_user_id: Option<i64>,
    /// Whether the client signed the engagement letter.
    pub letter_signed: bool,
    /// The day the letter was signed.
    pub letter_signed_on: Option<NaiveDate>,
    /// The neutral code of the suspension, end or decline reason
    /// ([`crate::reasons`]); never a restricted reason.
    pub reason_code: Option<String>,
    /// Whether automatic messages to the client are off for the current
    /// reason (a restricted suspension or end, `ACC.ACL.14`): the MLRO
    /// decides what the client is told.
    pub client_messages_suppressed: bool,
    /// The manager's note of why it last resumed (what changed, or the
    /// client's instruction).
    pub resume_note: Option<String>,
    /// The day it was activated.
    pub activated_on: Option<NaiveDate>,
    /// The day it ended.
    pub ended_on: Option<NaiveDate>,
    /// The lifecycle state; changed only by a transition.
    pub status: EngagementStatus,
    /// The user who created it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Engagement {
    /// The engagement as a kernel subject: `accounting.engagement:7`.
    pub fn subject(&self) -> Subject {
        Subject::new(ENGAGEMENT_SUBJECT_TYPE, self.id)
    }

    /// The client company.
    pub fn client(&self) -> PartyId {
        PartyId(self.client_party_id)
    }

    /// The engagement shown to people: `ENG-0007`.
    pub fn number(&self) -> String {
        format!("ENG-{:04}", self.id)
    }

    /// Whether an automatic message may go to the client now: never while
    /// the current reason is restricted (`ACC.ACL.14`). The later notices
    /// (`ACC.RUL.09`, decline and end notices) ask it before sending.
    pub fn may_message_client(&self) -> bool {
        !self.client_messages_suppressed
    }

    /// Whether time may be logged against it: while `active`, and while
    /// `suspended` (work already done stays billable, 04 §4.5). Decision:
    /// time entries are wl-attendance's (`HRM.TMS.01`, not built yet); this is
    /// the check its entry form asks of the engagement.
    pub fn accepts_time_entries(&self) -> bool {
        matches!(
            self.status,
            EngagementStatus::Active | EngagementStatus::Suspended
        )
    }

    /// The engagements of `client`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_client(db: &Db, client: PartyId) -> Result<Vec<Engagement>> {
        Engagement::query()
            .where_(Engagement::CLIENT_PARTY_ID.eq(client.0))
            .order_by(Engagement::ID)
            .get(db)
            .await
    }
}

/// What a new engagement is made of.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct NewEngagement {
    /// The client company.
    pub client: PartyId,
    /// The services.
    pub title: String,
    /// How it is billed.
    pub fee_basis: FeeBasis,
    /// The monthly retainer in fils.
    pub retainer_fee: i64,
    /// The first day of service, if agreed.
    pub start_date: Option<NaiveDate>,
    /// The manager running it.
    pub manager_user_id: Option<i64>,
}

impl NewEngagement {
    /// A retainer engagement of `client` for `title` at `retainer_fee` fils a
    /// month.
    pub fn retainer(client: PartyId, title: &str, retainer_fee: i64) -> Self {
        NewEngagement {
            client,
            title: title.to_owned(),
            fee_basis: FeeBasis::Retainer,
            retainer_fee,
            start_date: None,
            manager_user_id: None,
        }
    }

    /// Billed as `basis`.
    pub fn fee_basis(mut self, basis: FeeBasis) -> Self {
        self.fee_basis = basis;
        self
    }

    /// Starting on `date`.
    pub fn start_date(mut self, date: NaiveDate) -> Self {
        self.start_date = Some(date);
        self
    }

    /// Run by the manager `user`.
    pub fn manager(mut self, user: i64) -> Self {
        self.manager_user_id = Some(user);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_are_stored_in_snake_case() {
        let names: Vec<_> = EngagementStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            names,
            ["onboarding", "active", "suspended", "ended", "declined"]
        );
        assert_eq!(FeeBasis::default().as_str(), "retainer");
    }

    #[test]
    fn the_number_and_subject_name_the_engagement() {
        let engagement = Engagement {
            id: 7,
            client_party_id: 3,
            ..Engagement::default()
        };
        assert_eq!(engagement.number(), "ENG-0007");
        assert_eq!(engagement.subject().to_string(), "accounting.engagement:7");
        assert_eq!(engagement.client(), PartyId(3));
        assert!(engagement.may_message_client());
    }
}
