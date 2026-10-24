//! The client approver, the escalation contact and the deemed-approval days of
//! a deployment's timesheets (09 Manpower §4.3 row *The client does not
//! approve the timesheet*, §6 *Client contract*, §7 *Timesheet*, §7.2
//! `HRM.RUL.15`; BL-MPW-19).
//!
//! wl-attendance escalates (E = 6 days) and may deem a timesheet approved (N
//! days), but it does not know the contract. It asks the kernel's
//! [`ApproverResolver`], which the vertical provides here for a timesheet whose
//! context is a deployment (`manpower.deployment:7`):
//!
//! - **`account_manager`:** the user named on the deployment's contract, else
//!   the holder of the account manager role in the client company
//!   ([`crate::notices::account_managers`]).
//! - **`escalation_contacts`:** the contract's escalation contact, a party
//!   ([`NotifyRecipient::Party`], reached through its contact points); none when
//!   the field is empty, so wl-attendance tells the manager only.
//! - **`client_approvers`:** the portal users who act for the client company
//!   with the portal role `owner` or `approver` on the day
//!   ([`crate::permissions::CLIENT_APPROVER_PORTAL_ROLES`]); a `viewer` is not
//!   one. Without wl-portal there are none.
//! - **`deemed_approval_days`:** the contract's days now; **`_at`** the days in
//!   force when the timesheet was submitted ([`ContractTerms`]): a contract
//!   changed from 10 to 5 days leaves a timesheet already submitted at 10. A
//!   contract without the clause (empty) never deems; a value below 1 is no
//!   clause either.
//!
//! The resolver returns the contract's values and nothing else: the account
//! manager's manager is found by wl-attendance. A timesheet of any other context
//! gets no answer.

use renox::db::{DateTime, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::attendance::LabourFuture;
use wl_kernel::documents::today;
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::parties::directory;
use wl_kernel::portal::portal_users;
use wl_kernel::timesheets::{ApproverResolver, ContextInfo};
use wl_kernel::{PartyId, Subject};

use crate::contract::{ClientContract, ContractStatus};
use crate::db::{refused, set_contract};
use crate::deployment::{self, Deployment};
use crate::notices;
use crate::permissions::{self, CLIENT_APPROVER_PORTAL_ROLES};
use crate::screens::role_text;
use crate::site::Site;
use crate::texts::refusal;

/// What the contract said about timesheets from a moment on: one row each time
/// `set_timesheet_terms` changed it. The first change keeps the values that were
/// there before as a row effective from the beginning, so the history
/// is whole; a contract nobody changed has no rows and its own columns apply.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "manpower_contract_terms", index(client_contract_id))]
pub struct ContractTerms {
    /// The primary key.
    pub id: i64,
    /// The contract.
    pub client_contract_id: i64,
    /// The deemed-approval days from then on; empty: no clause.
    pub deemed_approval_days: Option<i64>,
    /// The escalation contact (a party) from then on; empty: none.
    pub escalation_contact_party_id: Option<i64>,
    /// From when the values apply.
    pub effective_from: DateTime,
    /// The user who changed them.
    pub changed_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ContractTerms {
    /// The terms in force for `contract` at `at`: the latest row effective by
    /// then; none when nobody changed them since the contract was made.
    ///
    /// # Errors
    ///
    /// A database error.
    pub async fn in_force(db: &Db, contract: i64, at: DateTime) -> Result<Option<ContractTerms>> {
        ContractTerms::query()
            .where_(ContractTerms::CLIENT_CONTRACT_ID.eq(contract))
            .where_(ContractTerms::EFFECTIVE_FROM.lte(at))
            .order_by_desc(ContractTerms::EFFECTIVE_FROM)
            .order_by_desc(ContractTerms::ID)
            .first(db)
            .await
    }
}

/// The contract of the deployment `context` names.
async fn contract_of(
    state: &AppState,
    context: Option<&Subject>,
) -> Result<Option<ClientContract>> {
    let Some(context) = context.filter(|c| c.subject_type == deployment::SUBJECT_TYPE) else {
        return Ok(None);
    };
    let Some(found) = Deployment::find(&state.db, context.subject_id).await? else {
        return Ok(None);
    };
    ClientContract::find(&state.db, found.client_contract_id).await
}

/// The days of a clause: whole days of 1 or more.
fn clause(days: Option<i64>) -> Option<i64> {
    days.filter(|days| *days >= 1)
}

/// The vertical's [`ApproverResolver`].
pub(crate) struct Approvers;

impl ApproverResolver for Approvers {
    fn account_manager<'a>(
        &'a self,
        state: &'a AppState,
        context: Option<&'a Subject>,
    ) -> LabourFuture<'a, Option<i64>> {
        Box::pin(async move {
            let Some(contract) = contract_of(state, context).await? else {
                return Ok(None);
            };
            Ok(notices::account_managers(
                state,
                contract.client(),
                contract.account_manager_user_id,
            )
            .await?
            .first()
            .copied())
        })
    }

    fn escalation_contacts<'a>(
        &'a self,
        state: &'a AppState,
        context: Option<&'a Subject>,
    ) -> LabourFuture<'a, Vec<NotifyRecipient>> {
        Box::pin(async move {
            Ok(contract_of(state, context)
                .await?
                .and_then(|contract| contract.escalation_contact_party_id)
                .map(|party| NotifyRecipient::Party { id: PartyId(party) })
                .into_iter()
                .collect())
        })
    }

    fn client_approvers<'a>(
        &'a self,
        state: &'a AppState,
        context: Option<&'a Subject>,
    ) -> LabourFuture<'a, Vec<NotifyRecipient>> {
        Box::pin(async move {
            let Some(contract) = contract_of(state, context).await? else {
                return Ok(Vec::new());
            };
            Ok(portal_users(state)
                .users_for(&state.db, contract.client(), today())
                .await?
                .into_iter()
                .filter(|user| user.has_any_role(CLIENT_APPROVER_PORTAL_ROLES))
                .map(|user| NotifyRecipient::User { id: user.user_id })
                .collect())
        })
    }

    fn describe_context<'a>(
        &'a self,
        state: &'a AppState,
        context: &'a Subject,
    ) -> LabourFuture<'a, Option<ContextInfo>> {
        Box::pin(async move {
            if context.subject_type != deployment::SUBJECT_TYPE {
                return Ok(None);
            }
            let Some(deployment) = Deployment::find(&state.db, context.subject_id).await? else {
                return Ok(None);
            };
            let contract = ClientContract::find(&state.db, deployment.client_contract_id).await?;
            let site = Site::find(&state.db, deployment.site_id).await?;
            let client = directory(state)
                .display_name(&state.db, deployment.client(), "en")
                .await?
                .unwrap_or_default();
            let place = site.map_or_else(String::new, |site| format!(" at {}", site.name));
            let reference = contract.map_or_else(String::new, |c| format!(", {}", c.reference));
            Ok(Some(ContextInfo {
                label: format!("{}{place}{reference}", role_text(&deployment.role)),
                client_party: Some(deployment.client_party_id),
                client,
            }))
        })
    }

    fn deemed_approval_days<'a>(
        &'a self,
        state: &'a AppState,
        context: Option<&'a Subject>,
    ) -> LabourFuture<'a, Option<i64>> {
        Box::pin(async move {
            Ok(contract_of(state, context)
                .await?
                .and_then(|contract| clause(contract.deemed_approval_days)))
        })
    }

    fn deemed_approval_days_at<'a>(
        &'a self,
        state: &'a AppState,
        context: Option<&'a Subject>,
        submitted_at: DateTime,
    ) -> LabourFuture<'a, Option<i64>> {
        Box::pin(async move {
            let Some(contract) = contract_of(state, context).await? else {
                return Ok(None);
            };
            let days = match ContractTerms::in_force(&state.db, contract.id, submitted_at).await? {
                Some(terms) => terms.deemed_approval_days,
                None => contract.deemed_approval_days,
            };
            Ok(clause(days))
        })
    }
}

/// `by` sets the timesheet terms of contract `id`: the deemed-approval days
/// (whole days of 1 or more; `None` removes the clause) and the escalation
/// contact (`None` removes it). The terms apply from now to the timesheets
/// submitted from now on; one already submitted keeps the days it was submitted
/// with ([`ContractTerms`]).
///
/// # Errors
///
/// `Forbidden` without `contracts.manage` for the client; a 422 for days below
/// 1, a contact that is not a known party, or a contract that is `cancelled` or
/// `ended`.
pub async fn set_terms(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    deemed_approval_days: Option<i64>,
    escalation_contact: Option<PartyId>,
) -> Result<ClientContract> {
    let contract = ClientContract::find_or_404(&state.db, id).await?;
    if !by.has_permission_in(permissions::CONTRACTS_MANAGE, &contract.scope()) {
        return Err(Error::Forbidden);
    }
    if matches!(
        contract.status,
        ContractStatus::Cancelled | ContractStatus::Ended
    ) {
        return Err(refused(refusal("manpower.problems.terms_closed", &[])));
    }
    if deemed_approval_days.is_some_and(|days| days < 1) {
        return Err(refused(refusal("manpower.problems.deemed_days", &[])));
    }
    if let Some(contact) = escalation_contact
        && directory(state).find(&state.db, contact).await?.is_none()
    {
        return Err(refused(refusal("manpower.problems.client_unknown", &[])));
    }
    let contact_id = escalation_contact.map(|party| party.0);
    let mut tx = state.db.begin().await?;
    let known = ContractTerms::query()
        .where_(ContractTerms::CLIENT_CONTRACT_ID.eq(id))
        .count(&mut tx)
        .await?;
    if known == 0 {
        // The values before the first change, valid from the beginning.
        ContractTerms::create(
            &mut tx,
            ContractTerms {
                client_contract_id: id,
                deemed_approval_days: contract.deemed_approval_days,
                escalation_contact_party_id: contract.escalation_contact_party_id,
                effective_from: DateTime::UNIX_EPOCH,
                changed_by: contract.created_by,
                ..ContractTerms::default()
            },
        )
        .await?;
    }
    ContractTerms::create(
        &mut tx,
        ContractTerms {
            client_contract_id: id,
            deemed_approval_days,
            escalation_contact_party_id: contact_id,
            effective_from: now(),
            changed_by: Some(by.id),
            ..ContractTerms::default()
        },
    )
    .await?;
    set_contract(
        &mut tx,
        id,
        &[
            ("deemed_approval_days", &deemed_approval_days),
            ("escalation_contact_party_id", &contact_id),
        ],
    )
    .await?;
    tx.commit().await?;
    ClientContract::find_or_404(&state.db, id).await
}
