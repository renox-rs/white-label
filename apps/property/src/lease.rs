//! The lease (01 Property §6, §7.1, `PRP.LSE.01`): one record per version.
//!
//! A renewal or an amendment is a new version of the same lease: it has its
//! own row, its own dates, rent and payment plan, and the version it replaces
//! ends as `renewed` or `amended` when it is activated. All versions share
//! [`Lease::root_id`] (the id of version 1), which is what the deposit and
//! the cheques of the lease are held for ([`Lease::agreement`]).
//!
//! The status moves only through the Lease machine
//! ([`crate::lease_machine`]); nothing here writes it.

use renox::auth::permissions::Scope;
use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::{PartyId, Subject, VatCategory};

use crate::building::Building;
use crate::charge::LeaseCharge;

/// The subject type of a lease version, for approvals, documents, tasks and
/// the history.
pub const LEASE_SUBJECT_TYPE: &str = "property.lease";

/// Where a lease version stands (§7.1 *Lease*).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LeaseStatus {
    /// Being prepared; editable (`PRP.ACL.04`).
    #[default]
    Draft,
    /// Contract generated, waiting for signatures and the deposit.
    PendingSignature,
    /// In force.
    Active,
    /// Inside the renewal window.
    RenewalDue,
    /// The end date passed without renewal or notice; the tenant holds over
    /// and billing continues.
    Expired,
    /// The tenant or the landlord gave notice to end it.
    NoticeGiven,
    /// Replaced by a renewal version (terminal).
    Renewed,
    /// Replaced mid-term by an amendment version (terminal).
    Amended,
    /// Ended and moved out (terminal).
    Terminated,
    /// Never started (terminal).
    Cancelled,
}

impl LeaseStatus {
    /// The states of a lease that is in force one way or another: the unit
    /// has a tenant.
    pub const RUNNING: [LeaseStatus; 4] = [
        LeaseStatus::Active,
        LeaseStatus::RenewalDue,
        LeaseStatus::Expired,
        LeaseStatus::NoticeGiven,
    ];

    /// The states a new version may replace by renewal (§7.1 `renew`).
    pub const RENEWABLE: [LeaseStatus; 2] = [LeaseStatus::RenewalDue, LeaseStatus::Expired];

    /// The states a new version may replace by amendment (§7.1 `amend`).
    pub const AMENDABLE: [LeaseStatus; 3] = [
        LeaseStatus::Active,
        LeaseStatus::RenewalDue,
        LeaseStatus::Expired,
    ];

    /// The states of a version still being prepared or signed.
    pub const IN_PROGRESS: [LeaseStatus; 2] =
        [LeaseStatus::Draft, LeaseStatus::PendingSignature];
}

/// What a version is.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum VersionKind {
    /// The first version of a lease.
    #[default]
    New,
    /// A renewal at the end of the term.
    Renewal,
    /// A change in the middle of the term, by agreement.
    Amendment,
}

/// The property manager's decision on renewal (`PRP.RUL.01`, `PRP.RUL.09`).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RenewalDecision {
    /// Not decided yet.
    #[default]
    Undecided,
    /// Renew on the same terms.
    SameTerms,
    /// Renew on changed terms (a notice of changed terms is due).
    ChangedTerms,
    /// Do not renew.
    NotRenewing,
}

/// One version of a lease.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "property_leases",
    state_machine,
    index(root_id, version),
    index(unit_id, status),
    index(tenant_party_id)
)]
pub struct Lease {
    /// The primary key.
    pub id: i64,
    /// The id of version 1; the lease all versions belong to.
    pub root_id: i64,
    /// 1 for the first version, then 2, 3, ...
    pub version: i32,
    /// New, renewal or amendment.
    pub version_kind: VersionKind,
    /// The version this one replaces.
    pub previous_id: Option<i64>,
    /// The version that replaced this one when it was renewed (`renew`);
    /// the other way of the link `previous_id` makes.
    pub next_id: Option<i64>,
    /// The day an amendment applies from.
    pub effective_date: Option<NaiveDate>,
    /// The building; the scope roles act in.
    #[model(references = "property_buildings")]
    pub building_id: i64,
    /// The unit.
    #[model(references = "property_units")]
    pub unit_id: i64,
    /// The billed tenant: a wl-parties party. One in the MVP.
    pub tenant_party_id: i64,
    /// The first day of the term (of the version).
    pub start_date: NaiveDate,
    /// The last day of the term.
    pub end_date: NaiveDate,
    /// The annual rent, in fils.
    pub annual_rent: i64,
    /// The security deposit, in fils.
    pub deposit_amount: i64,
    /// A property manager waived the deposit.
    pub deposit_waived: bool,
    /// Exempt (residential) or standard (commercial), copied from the unit.
    pub vat_category: VatCategory,
    /// The contract's own notice period, when it differs from the 90 days of
    /// the law.
    pub contract_notice_days: Option<i32>,
    /// The clauses, as text.
    pub terms: Option<String>,
    /// Where the version stands; changed only by the machine.
    pub status: LeaseStatus,
    /// The user who drafted it.
    pub created_by: Option<i64>,
    /// The day it entered `pending_signature` (Asia/Dubai).
    pub submitted_on: Option<NaiveDate>,
    /// The date the property manager extended the signature follow-up to
    /// (`PRP.RUL.12`).
    pub follow_up_until: Option<NaiveDate>,
    /// The day the reminder at N days was sent.
    pub chase_reminded_on: Option<NaiveDate>,
    /// The due date the extend-or-cancel task was last opened for.
    pub chase_tasked_on: Option<NaiveDate>,
    /// The day it was activated.
    pub activated_on: Option<NaiveDate>,
    /// Why it was cancelled.
    pub cancel_reason: Option<String>,
    /// The day it was cancelled.
    pub cancelled_on: Option<NaiveDate>,
    /// The part of the deposit forfeited on cancellation, in fils.
    pub cancel_forfeit: i64,
    /// The clause of the signed terms that allows the forfeit.
    pub cancel_forfeit_clause: Option<String>,
    /// When the money effects of the cancellation ran.
    pub money_settled_at: Option<DateTime>,
    /// The Ejari or Tawtheeq registration number.
    pub ejari_number: Option<String>,
    /// The day it was registered.
    pub ejari_registered_on: Option<NaiveDate>,
    /// The day the registration expires.
    pub ejari_expires_on: Option<NaiveDate>,
    /// The day the registration was cancelled after move-out.
    pub ejari_cancelled_on: Option<NaiveDate>,
    /// The renewal decision.
    pub renewal_decision: RenewalDecision,
    /// The day the notice of changed terms was sent.
    pub notice_of_changes_sent_on: Option<NaiveDate>,
    /// The day it was delivered (proof recorded).
    pub notice_of_changes_delivered_on: Option<NaiveDate>,
    /// The proof of delivery, as text or a reference.
    pub notice_of_changes_proof: Option<String>,
    /// Only the current terms can be renewed without the tenant's signed
    /// acceptance (`PRP.RUL.09`).
    pub current_terms_only: bool,
    /// The day the lease entered the renewal window (`PRP.RUL.01`).
    pub renewal_flagged_on: Option<NaiveDate>,
    /// The day the first deadline warning (14 days before) was sent
    /// (`PRP.RUL.09`).
    pub deadline_warned_first_on: Option<NaiveDate>,
    /// The day the second deadline warning (3 days before) was sent.
    pub deadline_warned_second_on: Option<NaiveDate>,
    /// The submitter answered no to "changes terms against the tenant?" on a
    /// renewal or amendment draft (`PRP.ACL.17`); not answered counts as yes.
    pub terms_answered_no: bool,
    /// Why the answer is no; kept in the history at `submit`.
    pub terms_answer_reason: Option<String>,
    /// The end of the statutory renewal period the tenant holds over for.
    pub holding_over_until: Option<NaiveDate>,
    /// The day it became `expired` (`PRP.RUL.03`).
    pub expired_on: Option<NaiveDate>,
    /// The first day of the statutory renewal period it holds over for now.
    pub holding_period_start: Option<NaiveDate>,
    /// The first day of the period the holding-over task was last opened for
    /// (`PRP.RUL.14`).
    pub holding_tasked_for: Option<NaiveDate>,
    /// The day notice was given.
    pub notice_date: Option<NaiveDate>,
    /// Who gave it: `tenant` or `landlord`.
    pub notice_party: Option<String>,
    /// Why: end of term, early termination, landlord's ground, non-payment.
    pub notice_reason: Option<String>,
    /// The planned move-out date.
    pub planned_move_out: Option<NaiveDate>,
    /// The day the tenant moved out.
    pub actual_move_out: Option<NaiveDate>,
    /// Rent stays due to this date after move-out, when the contract says so.
    pub rent_due_until: Option<NaiveDate>,
    /// The day the lease was handed over with a management agreement.
    pub handed_over_on: Option<NaiveDate>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Lease {
    /// This version as a kernel subject: what approvals, documents and tasks
    /// attach to.
    pub fn subject(&self) -> Subject {
        Subject::new(LEASE_SUBJECT_TYPE, self.id)
    }

    /// The lease all versions belong to, as a subject: what the deposit and
    /// the cheques are held for. For version 1 it is [`Lease::subject`].
    pub fn agreement(&self) -> Subject {
        Subject::new(LEASE_SUBJECT_TYPE, self.root_id)
    }

    /// The lease number shown to people, the same for every version:
    /// `L-000012`.
    pub fn number(&self) -> String {
        format!("L-{:06}", self.root_id)
    }

    /// The number with the version: `L-000012 v2`.
    pub fn number_with_version(&self) -> String {
        format!("{} v{}", self.number(), self.version)
    }

    /// The tenant.
    pub fn tenant(&self) -> PartyId {
        PartyId(self.tenant_party_id)
    }

    /// The scope roles act in for this lease: its building.
    pub fn scope(&self) -> Scope {
        Building::scope_of(self.building_id)
    }

    /// Whether the version is a renewal or an amendment.
    pub fn is_follow_up(&self) -> bool {
        self.version_kind != VersionKind::New
    }

    /// The number of days of notice the contract needs before the end date
    /// for changed terms: its own period, else the 90 days of the law
    /// (Law No. 26 of 2007, Art. 14; §2).
    pub fn notice_days(&self) -> i64 {
        self.contract_notice_days.map_or(90, i64::from)
    }

    /// Every version of the lease `root_id`, oldest first.
    pub async fn versions_of(db: &Db, root_id: i64) -> Result<Vec<Lease>> {
        Lease::query()
            .where_(Lease::ROOT_ID.eq(root_id))
            .order_by(Lease::VERSION)
            .get(db)
            .await
    }

    /// The version this one replaces.
    pub async fn previous(&self, db: &Db) -> Result<Option<Lease>> {
        match self.previous_id {
            Some(id) => Lease::find(db, id).await,
            None => Ok(None),
        }
    }

    /// The charges of the payment plan, in order.
    pub async fn charges(&self, db: &Db) -> Result<Vec<LeaseCharge>> {
        LeaseCharge::of_lease(db, self.id).await
    }

    /// The renewal or amendment versions of the lease still being prepared or
    /// signed.
    pub async fn follow_ups_in_progress(db: &Db, root_id: i64) -> Result<Vec<Lease>> {
        Lease::query()
            .where_(Lease::ROOT_ID.eq(root_id))
            .where_(Lease::VERSION_KIND.is_in([VersionKind::Renewal, VersionKind::Amendment]))
            .where_(Lease::STATUS.is_in(LeaseStatus::IN_PROGRESS))
            .order_by(Lease::VERSION)
            .get(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lease_number_is_shared_by_its_versions() {
        let v2 = Lease {
            id: 40,
            root_id: 12,
            version: 2,
            ..Default::default()
        };
        assert_eq!(v2.number(), "L-000012");
        assert_eq!(v2.number_with_version(), "L-000012 v2");
        assert_eq!(v2.subject().to_string(), "property.lease:40");
        assert_eq!(v2.agreement().to_string(), "property.lease:12");
    }

    #[test]
    fn the_notice_period_is_the_contracts_or_ninety_days() {
        let mut lease = Lease::default();
        assert_eq!(lease.notice_days(), 90);
        lease.contract_notice_days = Some(120);
        assert_eq!(lease.notice_days(), 120);
    }

    #[test]
    fn statuses_are_stored_in_snake_case() {
        assert_eq!(LeaseStatus::PendingSignature.as_str(), "pending_signature");
        assert_eq!(LeaseStatus::RenewalDue.as_str(), "renewal_due");
        assert_eq!(RenewalDecision::SameTerms.as_str(), "same_terms");
        assert_eq!(VersionKind::Amendment.as_str(), "amendment");
    }
}
