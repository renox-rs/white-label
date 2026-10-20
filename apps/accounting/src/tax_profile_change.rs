//! A change to a client's tax profile that waits for a manager's approval
//! (04 §3.3 `ACC.ACL.13`; BL-ACC-14).
//!
//! The tax profile drives the compliance calendar, so a change to one of its
//! calendar-driving fields ([`crate::tax_profile::DRIVING_FIELDS`]) takes
//! effect only after a manager other than the requester approves it, with
//! an effective date. Until then the profile in force is the old one and the
//! calendar is not touched. One change per client is pending at a time.
//!
//! ```text
//! [*] --> requested
//! requested --request--> pending_approval                  (tax_profiles.manage)
//! pending_approval --request_approved--> approved          (the approval kernel, BL-KRN-01)
//! pending_approval --back_to_requested--> declined         (rejected, returned, withdrawn, expired, failed)
//! ```
//!
//! The approval stores a **new** [`TaxProfile`] version with the effective
//! date, approved by the approver; no version is ever overwritten. Once the
//! approval commits, `ACC.RUL.10` regenerates the calendar
//! ([`crate::regeneration`]). A change to a contact or a note needs no
//! approval ([`update_contact`]).

use renox::chrono::NaiveDate;
use renox::db::{DateTime, ToDbValue, Transaction, now};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::{Machine, StateMachine, Transition, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{self, PolicyDefault, Require, Required};
use wl_kernel::documents::today;
use wl_kernel::machines::SubjectMachine;
use wl_kernel::{PartyId, Subject};

use crate::db::refused;
use crate::permissions;
use crate::tax_profile::{TaxProfile, VatPeriod, changed_fields};
use crate::texts::english;

/// The subject type of a tax profile change.
pub const TAX_PROFILE_CHANGE_SUBJECT_TYPE: &str = "accounting.tax_profile_change";
/// The `request` transition's name.
pub const REQUEST: &str = "request";
/// The transition the kernel runs on the manager's yes.
pub const REQUEST_APPROVED: &str = "request_approved";
/// The transition the kernel runs when the request ends without a yes.
pub const BACK_TO_REQUESTED: &str = "back_to_requested";
/// The approval action of a tax profile change (`ACC.ACL.13`).
pub const ACTION: &str = "accounting.tax_profile_change";
/// Hours a change may wait for its approval before it can expire.
pub const EXPIRY_HOURS: i64 = 336;

/// Where a change stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ChangeStatus {
    /// Entered; the approval request is not created yet.
    #[default]
    Requested,
    /// Waiting for a manager other than the requester.
    PendingApproval,
    /// Approved: the new version is in the profile (terminal).
    Approved,
    /// Rejected, returned, withdrawn or expired: the profile stays (terminal).
    Declined,
}

/// One requested change of a client's tax profile: the values of the version
/// it would create, and the day they apply from.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "accounting_tax_profile_changes",
    state_machine,
    index(client_party_id),
    index(status)
)]
pub struct TaxProfileChange {
    /// The primary key.
    pub id: i64,
    /// The client company, a party.
    pub client_party_id: i64,
    /// The version it changes: the newest when it was asked.
    pub base_profile_id: Option<i64>,
    /// The VAT tax registration number.
    pub trn: Option<String>,
    /// Whether the client is registered for VAT.
    pub vat_registered: bool,
    /// How often its VAT returns are due.
    pub vat_period: VatPeriod,
    /// The last day of its first VAT period: the stagger.
    pub vat_first_period_end: Option<NaiveDate>,
    /// The last day of its financial year.
    pub financial_year_end: Option<NaiveDate>,
    /// The corporate tax registration number.
    pub ct_registration_no: Option<String>,
    /// Whether it elected Small Business Relief.
    pub small_business_relief: bool,
    /// Whether it is a Qualifying Free Zone Person.
    pub qfzp: bool,
    /// The day the new values apply from; may be in the past.
    pub effective_date: NaiveDate,
    /// The calendar-driving fields that differ, comma separated.
    pub changed_fields: String,
    /// Why, as the requester wrote it.
    pub note: Option<String>,
    /// Who asked.
    pub requested_by: Option<i64>,
    /// The approval request.
    pub approval_request_id: Option<i64>,
    /// The version the approval created.
    pub profile_id: Option<i64>,
    /// The approver (on a yes) or whoever ended the request.
    pub decided_by: Option<i64>,
    /// When it was decided.
    pub decided_at: Option<DateTime>,
    /// The comment the request ended with.
    pub decision_note: Option<String>,
    /// The lifecycle state; changed only by a transition.
    pub status: ChangeStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl TaxProfileChange {
    /// The change as a kernel subject: `accounting.tax_profile_change:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(TAX_PROFILE_CHANGE_SUBJECT_TYPE, self.id)
    }

    /// The client company.
    pub fn client(&self) -> PartyId {
        PartyId(self.client_party_id)
    }

    /// The names of the fields it changes.
    pub fn fields(&self) -> Vec<&str> {
        self.changed_fields
            .split(',')
            .filter(|name| !name.is_empty())
            .collect()
    }

    /// Whether its effective date lies before `on`: the approver sees it
    /// highlighted, and the calendar is regenerated from that past date.
    pub fn is_in_the_past(&self, on: NaiveDate) -> bool {
        self.effective_date < on
    }

    /// The version it proposes, without an id: the values of the change.
    pub fn proposed(&self) -> TaxProfile {
        TaxProfile {
            client_party_id: self.client_party_id,
            trn: self.trn.clone(),
            vat_registered: self.vat_registered,
            vat_period: self.vat_period,
            vat_first_period_end: self.vat_first_period_end,
            financial_year_end: self.financial_year_end,
            ct_registration_no: self.ct_registration_no.clone(),
            small_business_relief: self.small_business_relief,
            qfzp: self.qfzp,
            effective_date: self.effective_date,
            ..TaxProfile::default()
        }
    }

    /// The change of `client` waiting for its approval, if there is one.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn pending(db: &Db, client: PartyId) -> Result<Option<TaxProfileChange>> {
        TaxProfileChange::query()
            .where_(TaxProfileChange::CLIENT_PARTY_ID.eq(client.0))
            .where_(TaxProfileChange::STATUS.eq(ChangeStatus::PendingApproval))
            .first(db)
            .await
    }

    /// The changes of `client`, newest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_client(db: &Db, client: PartyId) -> Result<Vec<TaxProfileChange>> {
        TaxProfileChange::query()
            .where_(TaxProfileChange::CLIENT_PARTY_ID.eq(client.0))
            .order_by_desc(TaxProfileChange::ID)
            .get(db)
            .await
    }
}

/// Sets columns of change `id` other than the status and stamps
/// `updated_at`.
async fn set_change(
    tx: &mut Transaction,
    id: i64,
    columns: &[(&str, &(dyn ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn ToDbValue + Sync))> = columns.to_vec();
    let stamp = now();
    all.push(("updated_at", &stamp));
    TaxProfileChange::query()
        .where_(TaxProfileChange::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}

/// `request`: requested → pending_approval; the effect creates the approval
/// request to the holders of `tax_profiles.approve` (never the requester).
fn request_transition() -> Transition<TaxProfileChange> {
    Transition::new(REQUEST)
        .label("Request approval")
        .tone("primary")
        .permission(permissions::TAX_PROFILES_MANAGE)
        .from([ChangeStatus::Requested])
        .to(ChangeStatus::PendingApproval)
        .effect(|change: &TaxProfileChange, ctx, tx| {
            let id = change.id;
            let subject = change.subject();
            let requester = ctx.actor().map(|user| user.id);
            let payload = json!({
                "change": id,
                "client": change.client_party_id,
                "effective_date": change.effective_date.to_string(),
                "fields": change.fields(),
                "past": change.is_in_the_past(today()),
            });
            Box::pin(async move {
                let Some(requester) = requester else {
                    return Err(refused(english(
                        "accounting.tax_profile.problem.no_requester",
                        &[],
                    )));
                };
                let require =
                    Require::new(ACTION, subject, REQUEST, requester, "requested").payload(payload);
                let request = match approvals::require_in(&mut *tx, &require).await? {
                    Required::Pending(request) => request,
                    // Without the policy the change would take effect
                    // unapproved; the transaction rolls back instead.
                    Required::Allowed => {
                        return Err(refused(english(
                            "accounting.tax_profile.problem.no_approval_policy",
                            &[],
                        )));
                    }
                };
                set_change(tx, id, &[("approval_request_id", &Some(request.id))]).await
            })
        })
}

/// `request_approved`: the last yes; stores the new version of the profile,
/// approved by the approver, in the same transaction.
fn approve_transition() -> Transition<TaxProfileChange> {
    Transition::new(REQUEST_APPROVED)
        .label("Approve")
        .tone("success")
        .from([ChangeStatus::PendingApproval])
        .to(ChangeStatus::Approved)
        .system()
        .effect(|change: &TaxProfileChange, ctx, tx| {
            let change = change.clone();
            let approver = ctx.meta().get("actor_id").and_then(Value::as_i64);
            Box::pin(async move {
                // The contact and notes carry over from the version it
                // follows (they are not part of the change).
                let base = TaxProfile::latest(&mut *tx, change.client()).await?;
                let mut profile = TaxProfile {
                    contact_email: base.as_ref().and_then(|b| b.contact_email.clone()),
                    notes: base.as_ref().and_then(|b| b.notes.clone()),
                    approved_by: approver,
                    approved_at: Some(now()),
                    created_by: change.requested_by,
                    ..change.proposed()
                };
                profile.insert(&mut *tx).await?;
                set_change(
                    tx,
                    change.id,
                    &[
                        ("profile_id", &Some(profile.id)),
                        ("decided_by", &approver),
                        ("decided_at", &Some(now())),
                    ],
                )
                .await
            })
        })
}

/// `back_to_requested`: the request ended without a yes; the current version
/// stays.
fn decline_transition() -> Transition<TaxProfileChange> {
    Transition::new(BACK_TO_REQUESTED)
        .label("Decline")
        .tone("neutral")
        .from([ChangeStatus::PendingApproval])
        .to(ChangeStatus::Declined)
        .system()
        .effect(|change: &TaxProfileChange, ctx, tx| {
            let id = change.id;
            let by = ctx.meta().get("actor_id").and_then(Value::as_i64);
            let note = ctx
                .reason()
                .map(|r| r.trim().to_owned())
                .filter(|r| !r.is_empty());
            Box::pin(async move {
                set_change(
                    tx,
                    id,
                    &[
                        ("decided_by", &by),
                        ("decided_at", &Some(now())),
                        ("decision_note", &note),
                    ],
                )
                .await
            })
        })
}

impl StateMachine for TaxProfileChange {
    type State = ChangeStatus;
    const COLUMN: renox::db::Col<TaxProfileChange, ChangeStatus> = TaxProfileChange::STATUS;

    fn machine() -> Machine<Self> {
        use ChangeStatus::{Approved, Declined, PendingApproval, Requested};
        Machine::new()
            .initial([Requested])
            .state(Requested)
            .tone("neutral")
            .state(PendingApproval)
            .tone("warning")
            .state(Approved)
            .tone("success")
            .terminal()
            .state(Declined)
            .tone("neutral")
            .terminal()
            .transition(request_transition())
            .transition(approve_transition())
            .transition(decline_transition())
    }
}

/// The default approval policy of a change: one step by a holder of
/// `tax_profiles.approve` other than the requester (SoD).
pub fn approval_policy() -> PolicyDefault {
    PolicyDefault::new(ACTION)
        .step_permission(permissions::TAX_PROFILES_APPROVE)
        .expiry_hours(Some(EXPIRY_HOURS))
}

/// Registers the machine, the policy and the link of the approval request.
pub(crate) fn register() {
    SubjectMachine::of::<TaxProfileChange>(TAX_PROFILE_CHANGE_SUBJECT_TYPE)
        .waiting(ChangeStatus::PendingApproval)
        .register();
    renox::state_machine::register::<TaxProfileChange>();
    approval_policy().register();
    approvals::register_subject_link(TAX_PROFILE_CHANGE_SUBJECT_TYPE, |id| {
        format!("/accounting/tax-profile-changes/{id}")
    });
}

/// `by` asks to change the tax profile of `client` to the values of
/// `proposed`, from `effective` on (a past date is allowed; the approver
/// sees it highlighted). The change waits for a manager other than `by`;
/// until then the profile in force and the calendar stay as they are.
///
/// # Errors
///
/// `Forbidden` without `tax_profiles.manage`; a 422 when the client has no
/// profile yet, when no calendar-driving field differs from the newest
/// version, when the result would be incomplete, or while another change of
/// the client is pending.
pub async fn request(
    state: &AppState,
    by: &AuthUser,
    client: PartyId,
    proposed: TaxProfile,
    effective: NaiveDate,
    note: Option<&str>,
) -> Result<TaxProfileChange> {
    if !by.has_permission(permissions::TAX_PROFILES_MANAGE) {
        return Err(Error::Forbidden);
    }
    let Some(base) = TaxProfile::latest(&state.db, client).await? else {
        return Err(refused(english(
            "accounting.tax_profile.problem.no_profile",
            &[],
        )));
    };
    let fields = changed_fields(&base, &proposed);
    if fields.is_empty() {
        return Err(refused(english(
            "accounting.tax_profile.problem.no_change",
            &[],
        )));
    }
    let missing = proposed.missing();
    if !missing.is_empty() {
        return Err(refused(english(
            "accounting.tax_profile.problem.incomplete",
            &[("missing", &missing.join(", "))],
        )));
    }
    if let Some(pending) = TaxProfileChange::pending(&state.db, client).await? {
        return Err(refused(english(
            "accounting.tax_profile.problem.pending",
            &[("on", &pending.effective_date)],
        )));
    }
    let note = note.map(str::trim).filter(|n| !n.is_empty());
    let mut tx = state.db.begin().await?;
    let mut change = TaxProfileChange {
        client_party_id: client.0,
        base_profile_id: Some(base.id),
        trn: proposed.trn,
        vat_registered: proposed.vat_registered,
        vat_period: proposed.vat_period,
        vat_first_period_end: proposed.vat_first_period_end,
        financial_year_end: proposed.financial_year_end,
        ct_registration_no: proposed.ct_registration_no,
        small_business_relief: proposed.small_business_relief,
        qfzp: proposed.qfzp,
        effective_date: effective,
        changed_fields: fields.join(","),
        note: note.map(str::to_owned),
        requested_by: Some(by.id),
        ..TaxProfileChange::default()
    };
    change.insert(&mut tx).await?;
    let outcome = change
        .transition(REQUEST)
        .by(by)
        .run_in(&mut tx)
        .await
        .map_err(TransitionError::into_error)?;
    tx.commit().await?;
    approvals::announce(state, &outcome).await?;
    outcome.emit(state).await?;
    TaxProfileChange::find_or_404(&state.db, change.id).await
}

/// `by` changes the contact e-mail and the notes of `client`'s newest tax
/// profile version. They drive nothing, so no approval is needed
/// (`ACC.ACL.13`) and the version is updated in place.
///
/// # Errors
///
/// `Forbidden` without `tax_profiles.manage`; a 422 when the client has no
/// profile.
pub async fn update_contact(
    state: &AppState,
    by: &AuthUser,
    client: PartyId,
    contact_email: Option<&str>,
    notes: Option<&str>,
) -> Result<TaxProfile> {
    if !by.has_permission(permissions::TAX_PROFILES_MANAGE) {
        return Err(Error::Forbidden);
    }
    let Some(latest) = TaxProfile::latest(&state.db, client).await? else {
        return Err(refused(english(
            "accounting.tax_profile.problem.no_profile",
            &[],
        )));
    };
    let clean = |text: Option<&str>| {
        text.map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
    };
    let email = clean(contact_email);
    let notes = clean(notes);
    TaxProfile::query()
        .where_(TaxProfile::ID.eq(latest.id))
        .update(
            &state.db,
            &[
                ("contact_email", &email),
                ("notes", &notes),
                ("updated_at", &now()),
            ],
        )
        .await?;
    TaxProfile::find_or_404(&state.db, latest.id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_has_its_states_and_system_transitions() {
        let machine = TaxProfileChange::machine();
        let states: Vec<_> = machine
            .states()
            .iter()
            .map(|s| (s.name.clone(), s.terminal))
            .collect();
        assert_eq!(
            states,
            [
                ("requested".to_owned(), false),
                ("pending_approval".to_owned(), false),
                ("approved".to_owned(), true),
                ("declined".to_owned(), true),
            ]
        );
        let transitions: Vec<_> = machine
            .transitions()
            .iter()
            .map(|t| (t.name(), t.to_state(), t.is_system()))
            .collect();
        assert_eq!(
            transitions,
            [
                (REQUEST, Some("pending_approval"), false),
                (REQUEST_APPROVED, Some("approved"), true),
                (BACK_TO_REQUESTED, Some("declined"), true),
            ]
        );
        assert_eq!(
            wl_kernel::machines::approved(REQUEST),
            REQUEST_APPROVED,
            "the kernel runs it on the manager's yes"
        );
    }

    #[test]
    fn the_policy_asks_a_holder_of_the_approve_permission() {
        let expected = PolicyDefault::new("accounting.tax_profile_change")
            .step_permission("tax_profiles.approve")
            .expiry_hours(Some(336));
        assert_eq!(approval_policy(), expected);
    }

    #[test]
    fn a_change_names_its_fields_and_its_proposed_version() {
        let change = TaxProfileChange {
            id: 3,
            client_party_id: 7,
            vat_period: VatPeriod::Quarterly,
            changed_fields: "vat_first_period_end,qfzp".into(),
            effective_date: NaiveDate::from_ymd_opt(2027, 2, 1).unwrap(),
            ..TaxProfileChange::default()
        };
        assert_eq!(change.fields(), ["vat_first_period_end", "qfzp"]);
        assert_eq!(
            change.subject().to_string(),
            "accounting.tax_profile_change:3"
        );
        assert!(change.is_in_the_past(NaiveDate::from_ymd_opt(2027, 2, 2).unwrap()));
        assert!(!change.is_in_the_past(change.effective_date));
        let proposed = change.proposed();
        assert_eq!(proposed.client_party_id, 7);
        assert_eq!(proposed.vat_period, VatPeriod::Quarterly);
        assert_eq!(proposed.effective_date, change.effective_date);
    }
}
