//! Cancelling a lease after money or cheques were received (`PRP.ACL.18`,
//! 01 Property §4.1 *When it goes wrong*, §7.1 `cancel`, BL-PRP-01, #251).
//!
//! A prospect may back out after paying rent or a deposit or handing over
//! post-dated cheques, before the lease is active. The leasing agent or the
//! accountant asks for the cancellation with a reason ([`request`]) and, where
//! the signed terms allow it, a forfeit with the clause that allows it. When
//! nothing was received the lease is `cancelled` at once. When money or
//! cheques were received the kernel's interceptor defers the run as an
//! approval request for a property manager (never the requester,
//! `PRP.ACL.18`), the lease stays where it is with a pending-approval badge
//! and `activate` is refused; rejected, returned or expired, nothing happens
//! and the requester is told by the kernel. At the final approval the kernel
//! runs `cancel_approved`.
//!
//! The money effects ([`settle`]) run after the commit of `cancel` or
//! `cancel_approved`, never before, on the money received **by then**:
//!
//! - the cheques of the version that are not deposited go back against one
//!   handover receipt (`Cheques::return_undeposited`); one already deposited
//!   waits for `clear` or `bounce` ([`CancelWait`]) and the accountant gets a
//!   task listing it; on `clear` its amount joins the refunds
//!   ([`on_payment_received`]), on `bounce` nothing is refunded
//!   ([`on_cheque_bounced`]);
//! - invoices drafted for the charges are discarded; issued ones are
//!   credited, a paid one first unallocated (its payment moves to the
//!   tenant's credit balance) and then credited in full;
//! - what was paid is asked back from the credit balance through the Refund
//!   flow (`FIN.ACL.04`: the user who approves a refund is not the one who
//!   requested it);
//! - a deposit held for a **new** lease starts its settlement and its
//!   deductions are published to the tenant, the forfeited part as a line
//!   with its clause; the refund of the rest follows the 14-day dispute window
//!   (`PRP.RUL.11`). A renewal or an amendment never touches the deposit,
//!   which the lease carries over.
//!
//! A step that fails does not stop the others: the accountant gets one task
//! that lists what to finish by hand, and the run is stamped so it does not
//! repeat. Unit `release` of a reserved unit is BL-PRP-28 (reservations are
//! Later).

use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{StateMachine, Transitioned};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::ApprovalRequest;
use wl_kernel::cheques::{ChequeBounced, ChequeId, ChequeStatus, ReturnCheques, cheques};
use wl_kernel::deposits::{CancelSettlement, deposits};
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus, ReopenReason, invoicer};
use wl_kernel::payments::{
    PaymentMethod, PaymentReceived, RefundRequest, RefundSource, payment_recorder,
};
use wl_kernel::role_tasks::RoleTask;
use wl_kernel::{LegalEntity, PartyId};

use crate::building::BUILDING_SUBJECT_TYPE;
use crate::charge::{ChargeStatus, LeaseCharge};
use crate::cheques::{counted_amount, of_version};
use crate::events::LeaseCancelled;
use crate::lease::{Lease, LeaseStatus};
use crate::lease_machine::{CANCEL, CANCEL_APPROVED, CancelInput};
use crate::permissions::ACCOUNTANT_ROLE;
use crate::texts::english;

/// What a lease has received so far, in fils.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MoneyReceived {
    /// The deposit held for a new lease.
    pub deposit: i64,
    /// What was paid against the invoices of the lease.
    pub paid: i64,
    /// The cheques received and not returned: held, deposited or cleared.
    pub cheques: i64,
}

impl MoneyReceived {
    /// The total, which the approval policy reads: above 0 means money or
    /// cheques were received. A cheque that cleared against an invoice
    /// counts in both, since only whether it is above 0 matters.
    pub fn total(&self) -> i64 {
        self.deposit + self.paid + self.cheques
    }
}

/// The money and cheques `lease` has received by now.
///
/// # Errors
///
/// The database's and the contracts' errors.
pub async fn money_received(state: &AppState, lease: &Lease) -> Result<MoneyReceived> {
    let deposit = if lease.is_follow_up() {
        0
    } else {
        deposits(state)
            .held_for(&state.db, &lease.agreement())
            .await?
            .map_or(0, |held| held.held)
    };
    let mut paid = 0;
    for charge in lease.charges(&state.db).await? {
        let Some(id) = charge.invoice_id else {
            continue;
        };
        if let Some(invoice) = invoicer(state).find(&state.db, InvoiceId(id)).await?
            && matches!(
                invoice.status,
                InvoiceStatus::Issued
                    | InvoiceStatus::PartiallyPaid
                    | InvoiceStatus::Overdue
                    | InvoiceStatus::Paid
            )
        {
            paid += (invoice.gross_total - invoice.balance).max(0);
        }
    }
    let held_cheques = counted_amount(&of_version(state, lease).await?);
    Ok(MoneyReceived {
        deposit,
        paid,
        cheques: held_cheques,
    })
}

/// What the user enters to cancel a lease.
#[derive(Debug, Clone, Default)]
pub struct CancelRequest {
    /// Why the prospect backs out; required.
    pub reason: String,
    /// The part of the deposit forfeited, in fils (0 for none).
    pub forfeit: i64,
    /// The clause of the signed terms that allows the forfeit.
    pub clause: Option<String>,
}

/// `user` asks to cancel lease `id`: at once when nothing was received, else
/// as an approval request for a property manager. The outcome is `Pending`
/// for a request, already announced to the approvers.
///
/// # Errors
///
/// `Forbidden` without `leases.cancel` in the lease's building, `NotFrom`
/// unless `draft` or `pending_signature`, `ReasonRequired` without a reason,
/// `Guard` (422) for a forfeit without its clause or above the deposit, or
/// when a request for it is already pending. Nothing is written then.
pub async fn request(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    cancel: CancelRequest,
) -> std::result::Result<renox::state_machine::Outcome<Lease>, renox::state_machine::TransitionError>
{
    use renox::state_machine::TransitionError;
    let lease = Lease::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    let money = money_received(state, &lease).await?;
    let input = CancelInput {
        reason: cancel.reason.trim().to_owned(),
        forfeit: cancel.forfeit,
        forfeit_clause: cancel.clause,
        money_received: money.total(),
    };
    let outcome = lease
        .transition(CANCEL)
        .by(user)
        .reason(input.reason.clone())
        .input(&input)
        .run(state)
        .await?;
    wl_kernel::approvals::announce(state, &outcome)
        .await
        .map_err(TransitionError::Failed)?;
    Ok(outcome)
}

/// What [`settle`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settlement {
    /// The cheques handed back.
    pub returned: Vec<ChequeId>,
    /// The handover receipt of the cheques handed back.
    pub receipt: Option<String>,
    /// The cheques waiting for `clear` or `bounce`.
    pub waiting: Vec<ChequeId>,
    /// The invoices credited in full.
    pub credited: Vec<i64>,
    /// The draft invoices discarded.
    pub discarded: Vec<i64>,
    /// The refund asked from the credit balance, in fils.
    pub refund: i64,
    /// The deposit whose settlement started.
    pub deposit: Option<i64>,
    /// What could not be done and was left to the accountant.
    pub problems: Vec<String>,
    /// Whether this run did the work (`false` for a repeat).
    pub ran: bool,
}

/// A cheque deposited when its lease was cancelled, and what became of it.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WaitOutcome {
    /// Waiting for `clear` or `bounce`.
    #[default]
    Waiting,
    /// It cleared and its amount was asked back.
    Refunded,
    /// It bounced: nothing to refund.
    Bounced,
}

/// A cheque already deposited when its lease was cancelled.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "property_cancel_waits", unique(lease_id, cheque_id))]
pub struct CancelWait {
    /// The primary key.
    pub id: i64,
    /// The cancelled lease version.
    #[model(references = "property_leases")]
    pub lease_id: i64,
    /// The cheque.
    pub cheque_id: i64,
    /// Whose cheque it is: the tenant.
    pub party_id: i64,
    /// What became of it.
    pub outcome: WaitOutcome,
    /// When the row was created.
    pub created_at: Option<renox::db::DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<renox::db::DateTime>,
}

impl CancelWait {
    /// The cheques of lease `lease_id` that are still waited for.
    pub async fn waiting_of_lease(db: &Db, lease_id: i64) -> Result<Vec<CancelWait>> {
        CancelWait::query()
            .where_(CancelWait::LEASE_ID.eq(lease_id))
            .where_(CancelWait::OUTCOME.eq(WaitOutcome::Waiting))
            .order_by(CancelWait::ID)
            .get(db)
            .await
    }
}

/// Runs the money effects of the cancelled lease `id`, on behalf of `by`.
/// Idempotent: a lease whose effects ran is left alone.
///
/// # Errors
///
/// The database's errors; a failing contract call is a problem in the result
/// and a task for the accountant, not an error.
pub async fn settle(state: &AppState, id: i64, by: Option<i64>) -> Result<Settlement> {
    let lease = Lease::find(&state.db, id).await?.ok_or(Error::NotFound)?;
    if lease.status != LeaseStatus::Cancelled || lease.money_settled_at.is_some() {
        return Ok(Settlement::default());
    }
    let mut done = Settlement {
        ran: true,
        ..Settlement::default()
    };
    let reason = lease
        .cancel_reason
        .clone()
        .unwrap_or_else(|| english("property.cancel.default_reason", &[]));
    let had_money = money_received(state, &lease).await?.total() > 0;
    let mut refund = 0;

    // 1. The cheques: back against a receipt, or waited for.
    let held = of_version(state, &lease).await?;
    let undeposited: Vec<ChequeId> = held
        .iter()
        .filter(|c| matches!(c.status, ChequeStatus::Received | ChequeStatus::DueForDeposit))
        .map(|c| c.id)
        .collect();
    if !undeposited.is_empty() {
        match cheques(state)
            .return_undeposited(state, ReturnCheques::new(undeposited, &reason), by)
            .await
        {
            Ok(Some(back)) => {
                done.returned = back.returned;
                done.receipt = Some(back.receipt);
            }
            Ok(None) => {}
            Err(error) => done.problems.push(problem("cheques", &error)),
        }
    }
    let deposited: Vec<_> = held
        .iter()
        .filter(|c| c.status == ChequeStatus::Deposited)
        .collect();
    for cheque in &deposited {
        if CancelWait::query()
            .where_(CancelWait::LEASE_ID.eq(lease.id))
            .where_(CancelWait::CHEQUE_ID.eq(cheque.id.0))
            .count(&state.db)
            .await?
            == 0
        {
            CancelWait::create(
                &state.db,
                CancelWait {
                    lease_id: lease.id,
                    cheque_id: cheque.id.0,
                    party_id: lease.tenant_party_id,
                    outcome: WaitOutcome::Waiting,
                    ..Default::default()
                },
            )
            .await?;
        }
        done.waiting.push(cheque.id);
    }
    if !deposited.is_empty() {
        let list = deposited
            .iter()
            .map(|c| format!("{} ({}, {})", c.number, c.bank, crate::money_text(c.amount)))
            .collect::<Vec<_>>()
            .join("; ");
        let task = RoleTask::new(
            lease.subject(),
            ACCOUNTANT_ROLE,
            &english(
                "property.cancel.task_deposited",
                &[("lease", &lease.number_with_version())],
            ),
            &format!("property.cancel_cheques:{}", lease.id),
        )
        .description(&list);
        crate::tasks::open(state, &lease.scope(), task).await?;
    }
    // A cleared cheque is a payment: its amount is refunded like rent paid
    // (when it paid an invoice, the invoice step below counts it).
    let charges = lease.charges(&state.db).await?;
    for cheque in held.iter().filter(|c| c.status == ChequeStatus::Cleared) {
        let pays_an_invoice = cheque.linked.iter().any(|link| {
            charges
                .iter()
                .any(|c| c.id == link.subject_id && c.invoice_id.is_some())
        });
        if !pays_an_invoice {
            refund += cheque.amount;
        }
    }

    // 2. The invoices: drafts discarded, issued ones credited.
    for charge in charges.iter().filter(|c| c.status == ChargeStatus::Invoiced) {
        let Some(invoice_id) = charge.invoice_id else {
            continue;
        };
        match cancel_invoice(state, &lease, InvoiceId(invoice_id), &reason, by).await {
            Ok(Cancelled::Discarded) => done.discarded.push(invoice_id),
            Ok(Cancelled::Credited(paid)) => {
                done.credited.push(invoice_id);
                refund += paid;
            }
            Ok(Cancelled::Left) => {}
            Err(error) => done
                .problems
                .push(problem(&format!("invoice {invoice_id}"), &error)),
        }
    }
    LeaseCharge::query()
        .where_(LeaseCharge::LEASE_ID.eq(lease.id))
        .where_(LeaseCharge::STATUS.eq(ChargeStatus::Invoiced))
        .update(
            &state.db,
            &[
                ("status", &ChargeStatus::Cancelled),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;

    // 3. What was paid goes back from the credit balance.
    if refund > 0 {
        match ask_refund(state, &lease, refund, &reason, by).await {
            Ok(()) => done.refund = refund,
            Err(error) => done.problems.push(problem("refund", &error)),
        }
    }

    // 4. The deposit of a new lease: settlement started, deductions published.
    if !lease.is_follow_up() {
        let mut settlement = CancelSettlement::new(lease.agreement());
        if lease.cancel_forfeit > 0
            && let Some(clause) = lease.cancel_forfeit_clause.as_deref()
        {
            settlement = settlement.forfeit(lease.cancel_forfeit, clause);
        }
        match deposits(state).settle_cancelled(state, settlement, by).await {
            Ok(started) => done.deposit = started.map(|s| s.deposit.0),
            Err(error) => done.problems.push(problem("deposit", &error)),
        }
    }

    // 5. What is left by hand, the stamp, the event.
    if !done.problems.is_empty() {
        let task = RoleTask::new(
            lease.subject(),
            ACCOUNTANT_ROLE,
            &english(
                "property.cancel.task_problems",
                &[("lease", &lease.number_with_version())],
            ),
            &format!("property.cancel_problems:{}", lease.id),
        )
        .description(&done.problems.join("\n"));
        crate::tasks::open(state, &lease.scope(), task).await?;
    }
    Lease::query()
        .where_(Lease::ID.eq(lease.id))
        .update(
            &state.db,
            &[
                ("money_settled_at", &Some(renox::db::now())),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    state
        .emit(LeaseCancelled {
            lease: lease.id,
            root: lease.root_id,
            tenant: lease.tenant(),
            reason,
            forfeit: lease.cancel_forfeit,
            money_received: had_money,
        })
        .await?;
    Ok(done)
}

/// A line for the accountant: what failed and why.
fn problem(what: &str, error: &Error) -> String {
    format!("{what}: {error}")
}

/// What was done to an invoice of the cancelled lease.
enum Cancelled {
    /// A draft, deleted.
    Discarded,
    /// Credited in full; the fils that were paid and now sit in the credit
    /// balance.
    Credited(i64),
    /// Left as it is: already credited, or written off.
    Left,
}

/// Discards a draft invoice, or credits an issued one in full after taking
/// the payments off it.
async fn cancel_invoice(
    state: &AppState,
    lease: &Lease,
    id: InvoiceId,
    reason: &str,
    by: Option<i64>,
) -> Result<Cancelled> {
    let contract = invoicer(state);
    let Some(invoice) = contract.find(&state.db, id).await? else {
        return Ok(Cancelled::Left);
    };
    match invoice.status {
        InvoiceStatus::Draft => {
            contract.discard_draft(&state.db, id).await?;
            Ok(Cancelled::Discarded)
        }
        InvoiceStatus::Issued
        | InvoiceStatus::PartiallyPaid
        | InvoiceStatus::Overdue
        | InvoiceStatus::Paid => {
            let mut paid = 0;
            if invoice.gross_total > invoice.balance {
                for (_source, amount) in contract
                    .unallocate_all(state, id, ReopenReason::AllocationMoved)
                    .await?
                {
                    paid += amount;
                }
            }
            let note = format!("{} · {reason}", lease.number_with_version());
            contract.credit_in_full(state, id, note, by).await?;
            Ok(Cancelled::Credited(paid))
        }
        InvoiceStatus::WrittenOff | InvoiceStatus::Credited => Ok(Cancelled::Left),
    }
}

/// Asks `amount` back from the credit balance of the tenant through the
/// Refund flow.
async fn ask_refund(
    state: &AppState,
    lease: &Lease,
    amount: i64,
    reason: &str,
    by: Option<i64>,
) -> Result<()> {
    let entity = LegalEntity::for_scope(
        &state.db,
        Some((BUILDING_SUBJECT_TYPE, lease.building_id)),
    )
    .await?;
    let tenant: PartyId = lease.tenant();
    // Built before the await: the argument list of `english` is not `Send`.
    let refund_reason = english(
        "property.cancel.refund_reason",
        &[("lease", &lease.number_with_version()), ("reason", &reason)],
    );
    payment_recorder(state)
        .request_refund(
            state,
            RefundRequest {
                source: RefundSource::PartyCredit(tenant),
                party: tenant,
                legal_entity_id: entity.id,
                amount,
                currency: "AED".to_owned(),
                method: None,
                gateway_txn: None,
                reason: refund_reason,
            },
            by,
        )
        .await?;
    Ok(())
}

/// A payment from a cheque arrived for a tenant: a waited-for cheque of a
/// cancelled lease that cleared has its amount asked back (`PaymentReceived`
/// does not say which cheque, so the waits of the party are checked against
/// the register).
///
/// # Errors
///
/// The database's and the contracts' errors.
pub async fn on_payment_received(state: &AppState, event: &PaymentReceived) -> Result {
    if event.method != PaymentMethod::Cheque {
        return Ok(());
    }
    let waits = CancelWait::query()
        .where_(CancelWait::PARTY_ID.eq(event.party.0))
        .where_(CancelWait::OUTCOME.eq(WaitOutcome::Waiting))
        .order_by(CancelWait::ID)
        .get(&state.db)
        .await?;
    for wait in waits {
        let Some(cheque) = cheques(state)
            .find(&state.db, ChequeId(wait.cheque_id))
            .await?
        else {
            continue;
        };
        if cheque.status != ChequeStatus::Cleared {
            continue;
        }
        let lease = Lease::find(&state.db, wait.lease_id)
            .await?
            .ok_or(Error::NotFound)?;
        let reason = lease.cancel_reason.clone().unwrap_or_default();
        ask_refund(state, &lease, cheque.amount, &reason, None).await?;
        CancelWait::query()
            .where_(CancelWait::ID.eq(wait.id))
            .update(
                &state.db,
                &[
                    ("outcome", &WaitOutcome::Refunded),
                    ("updated_at", &renox::db::now()),
                ],
            )
            .await?;
    }
    Ok(())
}

/// A waited-for cheque bounced: nothing is refunded for it.
///
/// # Errors
///
/// The database's errors.
pub async fn on_cheque_bounced(state: &AppState, event: &ChequeBounced) -> Result {
    CancelWait::query()
        .where_(CancelWait::CHEQUE_ID.eq(event.cheque.0))
        .where_(CancelWait::OUTCOME.eq(WaitOutcome::Waiting))
        .update(
            &state.db,
            &[
                ("outcome", &WaitOutcome::Bounced),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

/// The user who asked for a cancellation: the actor of a direct run, else the
/// requester of the approval request that was approved.
pub(crate) async fn requester(
    state: &AppState,
    event: &Transitioned<Lease>,
) -> Result<Option<i64>> {
    if event.transition == CANCEL || event.transition == crate::lease_machine::FORCE_TERMINATE {
        return Ok(event.actor_id);
    }
    let Some(request) = event
        .meta
        .get(wl_kernel::approvals::submit::REQUEST_KEY)
        .and_then(Value::as_i64)
    else {
        return Ok(None);
    };
    Ok(ApprovalRequest::find(&state.db, request)
        .await?
        .map(|r| r.requester_id))
}

/// Registers the listeners: the money effects after `cancel` and
/// `cancel_approved`, and the two events that end a waited-for cheque.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: Transitioned<Lease>, state: AppState| async move {
        if ![CANCEL, CANCEL_APPROVED].contains(&event.transition.as_str()) {
            return Ok(());
        }
        let by = requester(&state, &event).await?;
        settle(&state, event.key, by).await.map(drop)
    });
    app.listen(|event: PaymentReceived, state: AppState| async move {
        on_payment_received(&state, &event).await
    });
    app.listen(|event: ChequeBounced, state: AppState| async move {
        on_cheque_bounced(&state, &event).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_total_is_what_the_policy_reads() {
        let nothing = MoneyReceived::default();
        assert_eq!(nothing.total(), 0);
        let some = MoneyReceived {
            deposit: 500_000,
            paid: 2_500_000,
            cheques: 7_500_000,
        };
        assert_eq!(some.total(), 10_500_000);
    }

    #[test]
    fn a_wait_starts_waiting() {
        assert_eq!(WaitOutcome::default(), WaitOutcome::Waiting);
        assert_eq!(WaitOutcome::Bounced.as_str(), "bounced");
    }
}
