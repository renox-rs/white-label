//! Releasing a credit hold (09 Manpower §7.1 *Client contract* `release`,
//! §7.2 `MPW.RUL.10`, `MPW.ACL.14`; BL-MPW-03).
//!
//! A hold ends when the client pays or the bounced cheque is resolved: the
//! system then releases the contract to the state it had before the hold. An
//! account manager may also release earlier, but releasing while a hold
//! condition remains needs the MD's approval, and the requester cannot
//! approve (`SOD`).
//!
//! - The state before the hold has two ways back, and a Renox transition has
//!   one target, so each release is a pair: `release` (to `active`) and
//!   `release_terminating` (to `terminating`), each guarded by the state
//!   before the hold; a contract held while `terminating` is never released to
//!   `active`. The user's `release` and `release_terminating` have the
//!   kernel's approval policy of the action `manpower.release`
//!   ([`release_policy`]): one step by the MD (or the company admin), never
//!   the requester, and only for a run whose `amount` is above 0, the money
//!   still owed. The kernel runs `release_approved` and
//!   `release_terminating_approved` at the final approval. `auto_release` and
//!   `auto_release_terminating` are the system's, for a hold with no condition
//!   left.
//! - **The hold condition** is the one `MPW.RUL.10` holds by ([`remaining`]):
//!   an invoice issued under the contract that is open, not `disputed`, with a
//!   balance and a due date more than N days back, an invoice that caused an
//!   `overdue` reason and still has a balance (a dispute raised after the hold
//!   does not release it), and a bounced cheque not yet resolved. Invoices and
//!   cheques an MD-approved release covered do not count and do not hold
//!   again (`covered` on the reason); a new invoice or a new bounce does.
//! - **The system release** ([`settle`]) runs after a payment of the client,
//!   on `ChequeBounceResolved` and in the 02:00 run: it closes the reasons
//!   whose cause is gone, and releases the contract when no reason that comes
//!   from the facts is left. A reason `other` (the account manager's own, such
//!   as a liquidation) is never closed by the system, only by a release. With
//!   two open reasons, clearing one releases nothing. A release by the system
//!   withdraws the pending request of a manual release with the reason
//!   *released by payment*.
//! - A manual `release` when no condition remains runs at once; otherwise it
//!   waits for the MD ([`release`]). The account manager is told of every
//!   release and of a rejection; the client is told nothing.

use renox::chrono::NaiveDate;
use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionCtx, TransitionError, Transitioned};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{
    ApprovalDecided, ApprovalEnd, ApprovalRequest, PolicyDefault, announce, withdraw_because,
};
use wl_kernel::cheques::{ChequeBounceResolved, ChequeId, cheques};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus, invoicer};

use crate::contract::{
    AUTO_RELEASE, AUTO_RELEASE_TERMINATING, ClientContract, ContractStatus, RELEASE,
    RELEASE_TERMINATING, SUBJECT_TYPE,
};
use crate::db::{answer, refused, set_contract};
use crate::hold::{ContractHold, HoldReason, overdue_invoices};
use crate::notices;
use crate::permissions;
use crate::texts::english;

/// The approval action of a manual release: both `release` and
/// `release_terminating` ask the MD under it.
pub const ACTION: &str = "manpower.release";

/// The default approval policy of a manual release (`MPW.ACL.14`): one step by
/// the MD, only when money is still owed (`amount` above 0), never the
/// requester.
pub fn release_policy() -> PolicyDefault {
    PolicyDefault::new(ACTION)
        .amount_above(0)
        .step([permissions::MD_ROLE, wl_kernel::COMPANY_ADMIN_ROLE])
}

/// What a release run carries to its effect.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseInput {
    /// What is still owed that holds the contract, in fils; above 0 means the
    /// MD approves (the approval policy reads it).
    #[serde(default)]
    pub amount: i64,
    /// The user who asks; none for the system.
    #[serde(default)]
    pub by: Option<i64>,
}

/// The input of a release run.
pub(crate) fn release_input(ctx: &TransitionCtx) -> ReleaseInput {
    ctx.input_as::<ReleaseInput>().unwrap_or_default()
}

/// Why `contract` cannot go back to `target`, if it cannot: the state before
/// the hold must be `target`.
pub fn release_problem(contract: &ClientContract, target: ContractStatus) -> Option<String> {
    if contract.status_before_hold == Some(target) {
        None
    } else {
        Some(english("manpower.release.state_before", &[]))
    }
}

/// The effect of a release inside its transaction: the hold is cleared (the
/// state before, the day and who held it) and the open reasons are closed as
/// released. When money was still owed (`amount` above 0) the MD approved it,
/// so the reasons are `covered`: the rule does not hold again for them.
pub(crate) async fn release_effects(
    tx: &mut Transaction,
    contract: &ClientContract,
    input: &ReleaseInput,
) -> Result {
    let none_text: Option<String> = None;
    let none_at: Option<DateTime> = None;
    let none_user: Option<i64> = None;
    set_contract(
        &mut *tx,
        contract.id,
        &[
            ("status_before_hold", &none_text),
            ("held_at", &none_at),
            ("held_by", &none_user),
        ],
    )
    .await?;
    close_open(&mut *tx, contract.id, "released", input.amount > 0).await
}

/// Closes the open reasons of contract `contract_id` with `resolution`.
pub(crate) async fn close_open(
    tx: &mut Transaction,
    contract_id: i64,
    resolution: &str,
    covered: bool,
) -> Result {
    let stamp = now();
    ContractHold::query()
        .where_(ContractHold::CLIENT_CONTRACT_ID.eq(contract_id))
        .where_(ContractHold::RESOLVED_AT.is_null())
        .update(
            &mut *tx,
            &[
                ("resolved_at", &Some(stamp)),
                ("resolution", &Some(resolution.to_owned())),
                ("covered", &covered),
                ("updated_at", &stamp),
            ],
        )
        .await?;
    Ok(())
}

/// The contract `id`, or 404.
async fn load(state: &AppState, id: i64) -> Result<ClientContract> {
    ClientContract::find_or_404(&state.db, id).await
}

/// One thing that still holds the contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remaining {
    /// The invoice number or the cheque number.
    pub label: String,
    /// What is still owed, in fils.
    pub amount: i64,
}

/// Closes the open reasons of `contract` whose cause is gone: an invoice paid,
/// written off or credited, a cheque resolved (`ChequeBounceResolved`, or the
/// register no longer owes it). A reason `other` stays; so does the reason of
/// an invoice that is only disputed now.
async fn close_gone(state: &AppState, contract_id: i64) -> Result<usize> {
    let mut closed = 0;
    for row in ContractHold::open_of(&state.db, contract_id).await? {
        let resolution = match (row.reason, row.invoice_id, row.cheque_id) {
            (HoldReason::Overdue, Some(invoice), _) => {
                match invoicer(state).find(&state.db, InvoiceId(invoice)).await? {
                    Some(summary) if summary.status == InvoiceStatus::WrittenOff => {
                        Some("written_off")
                    }
                    Some(summary) if summary.status == InvoiceStatus::Credited => Some("credited"),
                    Some(summary) if summary.balance == 0 || !summary.status.is_open() => {
                        Some("payment")
                    }
                    _ => None,
                }
            }
            // A cheque the register does not know stays until the event.
            (HoldReason::BouncedCheque, _, Some(cheque)) => cheques(state)
                .find(&state.db, ChequeId(cheque))
                .await?
                .filter(|summary| !summary.status.is_owed())
                .map(|_| "cheque_resolved"),
            _ => None,
        };
        if let Some(resolution) = resolution {
            let stamp = now();
            ContractHold::query()
                .where_(ContractHold::ID.eq(row.id))
                .update(
                    &state.db,
                    &[
                        ("resolved_at", &Some(stamp)),
                        ("resolution", &Some(resolution.to_owned())),
                        ("updated_at", &stamp),
                    ],
                )
                .await?;
            closed += 1;
        }
    }
    Ok(closed)
}

/// What still holds `contract` on `on`, as `MPW.RUL.10` reads the facts: the
/// open reasons of an invoice or a cheque that is not settled, and the
/// invoices that are overdue now without a reason or a cover. Call
/// [`close_gone`] first.
///
/// # Errors
///
/// The invoicing and payments contracts' errors.
pub async fn remaining(
    state: &AppState,
    contract: &ClientContract,
    on: NaiveDate,
) -> Result<Vec<Remaining>> {
    let rows = ContractHold::of_contract(&state.db, contract.id).await?;
    let mut left = Vec::new();
    for row in rows.iter().filter(|row| row.is_open()) {
        match row.reason {
            HoldReason::Overdue | HoldReason::BouncedCheque => left.push(Remaining {
                label: row.label.clone().unwrap_or_default(),
                amount: row.amount,
            }),
            HoldReason::Other => {}
        }
    }
    for invoice in overdue_invoices(state, contract, on).await? {
        let known = rows
            .iter()
            .any(|row| row.invoice_id == Some(invoice.id.0) && (row.is_open() || row.covered));
        if !known {
            left.push(Remaining {
                label: invoice.number.clone().unwrap_or_default(),
                amount: invoice.balance,
            });
        }
    }
    Ok(left)
}

/// Whether the account manager's own reason (`other`) still holds `contract`.
async fn other_reason_open(state: &AppState, contract_id: i64) -> Result<bool> {
    Ok(ContractHold::open_of(&state.db, contract_id)
        .await?
        .iter()
        .any(|row| row.reason == HoldReason::Other))
}

/// What a manual `release` did.
#[derive(Debug, Clone)]
pub struct Released {
    /// The contract as it is now: released, or still `on_hold` while the MD
    /// decides.
    pub contract: ClientContract,
    /// The approval request when the MD must decide.
    pub approval_request: Option<i64>,
}

/// The release transition that goes back to the state before the hold.
fn transition_for(contract: &ClientContract, system: bool) -> Result<&'static str> {
    match (contract.status_before_hold, system) {
        (Some(ContractStatus::Active), false) => Ok(RELEASE),
        (Some(ContractStatus::Terminating), false) => Ok(RELEASE_TERMINATING),
        (Some(ContractStatus::Active), true) => Ok(AUTO_RELEASE),
        (Some(ContractStatus::Terminating), true) => Ok(AUTO_RELEASE_TERMINATING),
        _ => Err(refused(english("manpower.release.state_before", &[]))),
    }
}

/// `by` asks to release the credit hold of contract `id`. When no hold
/// condition remains the contract goes back to its state before the hold at
/// once; otherwise an approval request goes to the MD (never `by`) and the
/// contract stays `on_hold` until it is approved.
///
/// # Errors
///
/// `Forbidden` without `contracts.manage` for the client; a 422 unless the
/// contract is `on_hold`.
pub async fn release(state: &AppState, by: &AuthUser, id: i64) -> Result<Released> {
    let contract = load(state, id).await?;
    if !by.has_permission_in(permissions::CONTRACTS_MANAGE, &contract.scope()) {
        return Err(Error::Forbidden);
    }
    if contract.status != ContractStatus::OnHold {
        return Err(refused(english("manpower.release.not_on_hold", &[])));
    }
    let on = today();
    close_gone(state, id).await?;
    let owed: i64 = remaining(state, &contract, on)
        .await?
        .iter()
        .map(|left| left.amount)
        .sum();
    let input = ReleaseInput {
        amount: owed,
        by: Some(by.id),
    };
    let name = transition_for(&contract, false)?;
    let outcome = contract
        .transition(name)
        .by(by)
        .input(&input)
        .run(state)
        .await
        .map_err(answer)?;
    announce(state, &outcome).await?;
    let approval_request = match &outcome {
        renox::state_machine::Outcome::Pending(pending) => pending
            .data
            .get(wl_kernel::approvals::submit::REQUEST_KEY)
            .and_then(renox::serde_json::Value::as_i64),
        renox::state_machine::Outcome::Done(_) => None,
    };
    outcome.emit(state).await?;
    Ok(Released {
        contract: load(state, id).await?,
        approval_request,
    })
}

/// The ids of the pending manual release requests of `contract`.
pub(crate) async fn pending_release_ids(
    state: &AppState,
    contract: &ClientContract,
) -> Result<Vec<i64>> {
    Ok(
        ApprovalRequest::pending_about(&state.db, &contract.subject())
            .await?
            .iter()
            .filter(|request| request.action == ACTION)
            .map(|request| request.id)
            .collect(),
    )
}

/// The system looks at contract `id` on `on`: the reasons whose cause is gone
/// are closed, and when nothing that comes from the facts holds it any more
/// (and the account manager's own reason is not open) the contract goes back
/// to its state before the hold. A pending manual request is withdrawn with
/// the reason *released by payment*. Returns whether it released.
///
/// # Errors
///
/// The database's and the contracts' errors.
pub async fn settle(state: &AppState, id: i64, on: NaiveDate) -> Result<bool> {
    let contract = load(state, id).await?;
    if contract.status != ContractStatus::OnHold {
        return Ok(false);
    }
    close_gone(state, id).await?;
    if !remaining(state, &contract, on).await?.is_empty() || other_reason_open(state, id).await? {
        return Ok(false);
    }
    let name = transition_for(&contract, true)?;
    let pending = pending_release_ids(state, &contract).await?;
    let mut tx = state.db.begin().await?;
    let outcome = match contract
        .transition(name)
        .by_system()
        .input(&ReleaseInput::default())
        .run_in(&mut tx)
        .await
    {
        Ok(outcome) => outcome,
        // Another run released, ended or terminated it first.
        Err(TransitionError::NotFrom(_) | TransitionError::Conflict) => return Ok(false),
        Err(other) => return Err(answer(other)),
    };
    tx.commit().await?;
    outcome.emit(state).await?;
    let reason = english("manpower.release.by_payment", &[]);
    for request in pending {
        withdraw_because(state, request, &reason).await?;
    }
    Ok(true)
}

/// The cheque `event.cheque` was resolved: its reasons close at once and
/// every contract of the client is looked at again.
///
/// # Errors
///
/// As [`settle`].
pub async fn on_cheque_resolved(state: &AppState, event: &ChequeBounceResolved) -> Result {
    let stamp = now();
    ContractHold::query()
        .where_(ContractHold::CHEQUE_ID.eq(event.cheque.0))
        .where_(ContractHold::RESOLVED_AT.is_null())
        .update(
            &state.db,
            &[
                ("resolved_at", &Some(stamp)),
                ("resolution", &Some("cheque_resolved".to_owned())),
                ("updated_at", &stamp),
            ],
        )
        .await?;
    for contract in ClientContract::query()
        .where_(ClientContract::CLIENT_PARTY_ID.eq(event.party.0))
        .where_(ClientContract::STATUS.eq(ContractStatus::OnHold))
        .order_by(ClientContract::ID)
        .get(&state.db)
        .await?
    {
        settle(state, contract.id, today()).await?;
    }
    Ok(())
}

/// Tells the account manager of `contract` with the texts of `key`
/// (`<key>_title`, `<key>_body`); the client is told nothing.
async fn tell_manager(
    state: &AppState,
    contract: &ClientContract,
    key: &str,
    note: &str,
) -> Result {
    let managers =
        notices::account_managers(state, contract.client(), contract.account_manager_user_id)
            .await?;
    let params: [(&str, &(dyn std::fmt::Display + Sync)); 2] =
        [("contract", &contract.reference), ("note", &note)];
    notices::tell(
        state,
        contract.subject(),
        &contract.reference,
        &managers,
        &english(&format!("{key}_title"), &params),
        &english(&format!("{key}_body"), &params),
    )
    .await?;
    Ok(())
}

/// The release of a hold: the account manager is told, whoever released.
async fn on_released(state: &AppState, id: i64) -> Result {
    let contract = load(state, id).await?;
    tell_manager(
        state,
        &contract,
        "manpower.release.released",
        &contract.status.as_str().replace('_', " "),
    )
    .await
}

/// A manual release the MD did not approve: the account manager is told, with
/// the MD's comment; the contract stays `on_hold`. (The requester also gets
/// the kernel's own notice.)
async fn on_decided(state: &AppState, event: &ApprovalDecided) -> Result {
    if event.subject.subject_type != SUBJECT_TYPE
        || !matches!(
            event.decision,
            ApprovalEnd::Rejected | ApprovalEnd::Returned | ApprovalEnd::Expired
        )
    {
        return Ok(());
    }
    let Some(request) = ApprovalRequest::find(&state.db, event.request).await? else {
        return Ok(());
    };
    if request.action != ACTION {
        return Ok(());
    }
    let comment = request
        .decisions(&state.db)
        .await?
        .into_iter()
        .rev()
        .find_map(|decision| decision.comment)
        .unwrap_or_default();
    let contract = load(state, event.subject.subject_id).await?;
    tell_manager(state, &contract, "manpower.release.rejected", &comment).await
}

/// The release family of transitions, by name.
fn is_release(name: &str) -> bool {
    [
        RELEASE,
        RELEASE_TERMINATING,
        AUTO_RELEASE,
        AUTO_RELEASE_TERMINATING,
    ]
    .iter()
    .any(|release| name == *release || name == wl_kernel::machines::approved(release))
}

/// The listeners: a cleared cheque looks at the client's contracts again, a
/// release tells the account manager, a refusal of the MD too.
pub(crate) fn listen(app: &mut Registry) {
    app.listen(|event: ChequeBounceResolved, state: AppState| async move {
        on_cheque_resolved(&state, &event).await
    });
    app.listen(
        |event: Transitioned<ClientContract>, state: AppState| async move {
            if is_release(&event.transition) && event.from == ContractStatus::OnHold.as_str() {
                on_released(&state, event.key).await?;
            }
            Ok(())
        },
    );
    app.listen(|event: ApprovalDecided, state: AppState| async move {
        on_decided(&state, &event).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_contract_goes_back_only_to_the_state_before_the_hold() {
        let held = |before| ClientContract {
            status: ContractStatus::OnHold,
            status_before_hold: before,
            ..ClientContract::default()
        };
        let active = held(Some(ContractStatus::Active));
        assert_eq!(release_problem(&active, ContractStatus::Active), None);
        assert!(release_problem(&active, ContractStatus::Terminating).is_some());
        let terminating = held(Some(ContractStatus::Terminating));
        assert_eq!(
            release_problem(&terminating, ContractStatus::Terminating),
            None
        );
        assert!(release_problem(&terminating, ContractStatus::Active).is_some());
        assert!(release_problem(&held(None), ContractStatus::Active).is_some());
    }

    #[test]
    fn the_names_of_the_release_family_are_found() {
        assert!(is_release("release"));
        assert!(is_release("release_approved"));
        assert!(is_release("release_terminating_approved"));
        assert!(is_release("auto_release"));
        assert!(!is_release("hold"));
        assert!(!is_release("terminate"));
    }

    #[test]
    fn the_policy_asks_the_md_only_for_money_still_owed() {
        let policy = release_policy();
        assert_eq!(policy.action(), ACTION);
    }
}
