//! Closing a chargeback (wl-payments §4 `win_chargeback`, `lose_chargeback`
//! and `accept_chargeback`, §5.2a steps 3 and 4, `FIN.ACL.18`).
//!
//! ```text
//! charged_back --win_chargeback--> received
//! charged_back --lose_chargeback--> reversed
//! charged_back --accept_chargeback--> reversed
//! ```
//!
//! - **Won** (the gateway's `DisputeWon`, [`gateway_dispute_won`]): the funds
//!   are back. The allocations the chargeback removed are applied again where
//!   their targets are still open, each up to its open balance (a target
//!   written off or paid meanwhile counts as not open); the rest stays as the
//!   party's credit and the accountants are told. `PaymentReceived` follows,
//!   with the allocations. The dispute fee is not given back unless the
//!   gateway does it.
//! - **Lost** (the gateway's `DisputeLost`, [`gateway_dispute_lost`], or the
//!   evidence deadline passed without a submission, [`run_deadlines`], or a
//!   user with `payments.chargeback` for N-Genius, [`lose_by_hand`]): the
//!   payment is `reversed`. The invoices it paid stay open (collect the party
//!   again, or write them off through invoice `write_off`: nothing is closed
//!   here). `ChargebackClosed` (lost); the accountants are told.
//! - **Accepted** ([`accept`]): the accountant decides not to contest. It
//!   needs the approval of another holder of `payments.chargeback`
//!   (`FIN.ACL.18`, always, segregation of duties): the payment stays
//!   `charged_back` meanwhile. At the final approval the gateway is told
//!   (Stripe's `accept_dispute`; N-Genius: accepted in its portal and
//!   confirmed here), and when it refuses, the request `fail`s, the payment
//!   stays `charged_back` and the accountants are told. Then the kernel runs
//!   `accept_chargeback_approved`, which ends like a lost dispute
//!   (`ChargebackClosed`, accepted).
//!
//! A dispute whose evidence was noted submitted ([`crate::chargeback::evidence_submitted`])
//! is never closed by the deadline rule; only a gateway outcome closes it.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{
    Outcome, StateMachine, Transition, TransitionCtx, TransitionError, Transitioned,
};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{self, ApprovalDecided, ApprovalEnd, ApprovalRequest, PolicyDefault};
use wl_kernel::documents::today;
use wl_kernel::invoicing::invoicer;
use wl_kernel::payments::{
    AllocationTarget, ChargebackClosed, ChargebackOutcome, PAYMENT_SUBJECT_TYPE, PaymentAllocation,
    PaymentStatus, ReversalReason,
};

use crate::allocate;
use crate::allocation::Allocation;
use crate::chargeback::REMOVED_REASON;
use crate::credit;
use crate::followup::{FollowupKind, FollowupStatus, FollowupTask, money_text};
use crate::gateway;
use crate::notices;
use crate::online::received_event;
use crate::payment::Payment;
use crate::permissions;

/// The `win_chargeback` transition's name.
pub const WIN_CHARGEBACK: &str = "win_chargeback";

/// The `lose_chargeback` transition's name: a user records that the dispute
/// was lost (N-Genius).
pub const LOSE_CHARGEBACK: &str = "lose_chargeback";

/// The `lose_chargeback` transition run by the system.
pub const LOSE_CHARGEBACK_BY_SYSTEM: &str = "lose_chargeback_by_system";

/// The `accept_chargeback` transition's name.
pub const ACCEPT_CHARGEBACK: &str = "accept_chargeback";

/// The transition the kernel runs when the acceptance is approved.
pub const ACCEPT_CHARGEBACK_APPROVED: &str = "accept_chargeback_approved";

/// The approval policies' action key of `accept_chargeback` (`FIN.ACL.18`).
pub const ACCEPT_ACTION: &str = "payments.accept_chargeback";

/// The reason the removed allocations carry once a won dispute applied them
/// again.
pub const WON_REASON: &str = "chargeback_won";

/// The scheduled task and command of the deadline rule.
pub const DEADLINE_TASK: &str = "payments:chargeback_deadlines";

/// What a closing run carries to its effects.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct CloseInput {
    /// The user who closes it; `None` for the system.
    pub by: Option<i64>,
    /// The reason given with an acceptance.
    pub reason: Option<String>,
}

fn input(ctx: &TransitionCtx) -> Option<CloseInput> {
    ctx.input_as::<CloseInput>().ok()
}

/// `win_chargeback`, `lose_chargeback` (a user and the system) and
/// `accept_chargeback` with its approved twin, for the machine.
pub(crate) fn transitions() -> Vec<Transition<Payment>> {
    vec![
        win(),
        ending(
            Transition::new(LOSE_CHARGEBACK)
                .label("Record lost dispute")
                .tone("danger")
                .permission(permissions::CHARGEBACK),
            "lost",
        ),
        ending(
            Transition::new(LOSE_CHARGEBACK_BY_SYSTEM)
                .label("Dispute lost")
                .tone("danger")
                .system(),
            "lost",
        ),
        ending(
            Transition::new(ACCEPT_CHARGEBACK)
                .label("Accept chargeback")
                .tone("danger")
                .permission(permissions::CHARGEBACK)
                .reason_required(),
            "accepted",
        ),
        ending(
            Transition::new(ACCEPT_CHARGEBACK_APPROVED)
                .label("Accept chargeback (approved)")
                .tone("danger")
                .system(),
            "accepted",
        ),
    ]
}

/// `win_chargeback`: the funds are back.
fn win() -> Transition<Payment> {
    Transition::new(WIN_CHARGEBACK)
        .label("Dispute won")
        .tone("success")
        .system()
        .from([PaymentStatus::ChargedBack])
        .to(PaymentStatus::Received)
        .effect(|payment: &Payment, _ctx, tx| {
            let id = payment.id;
            Box::pin(async move {
                let status = Some("won".to_owned());
                Payment::query()
                    .where_(Payment::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("dispute_status", &status),
                            ("dispute_outcome", &status),
                            ("updated_at", &renox::db::now()),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// A transition that ends the dispute with `outcome` (`lost` or `accepted`):
/// the payment is `reversed` for the reason chargeback.
fn ending(transition: Transition<Payment>, outcome: &'static str) -> Transition<Payment> {
    transition
        .from([PaymentStatus::ChargedBack])
        .to(PaymentStatus::Reversed)
        .effect(move |payment: &Payment, ctx, tx| {
            let id = payment.id;
            let given = input(ctx).unwrap_or_default();
            let reason = ctx
                .reason()
                .map(str::to_owned)
                .or(given.reason)
                .filter(|r| !r.trim().is_empty());
            Box::pin(async move {
                let status = Some(outcome.to_owned());
                // Not the status column, so outside the machine's guard.
                Payment::query()
                    .where_(Payment::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            (
                                "reversal_reason",
                                &Some(ReversalReason::Chargeback.as_str().to_owned()),
                            ),
                            ("reversal_note", &reason),
                            ("reversed_by", &given.by),
                            ("reversed_on", &Some(today())),
                            ("dispute_status", &status),
                            ("dispute_outcome", &status),
                            ("updated_at", &renox::db::now()),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// The payment whose dispute has the gateway's `reference`, if any.
async fn by_dispute(db: &Db, reference: &str) -> Result<Option<Payment>> {
    Payment::query()
        .where_(Payment::DISPUTE_REFERENCE.eq(reference.trim()))
        .first(db)
        .await
}

async fn reload(state: &AppState, id: i64) -> std::result::Result<Payment, TransitionError> {
    Payment::find_or_404(&state.db, id)
        .await
        .map_err(TransitionError::Failed)
}

/// The gateway reports that the dispute `reference` was won (`DisputeWon`):
/// the payment is `received` again and its allocations are applied again
/// where their targets are open. A dispute that is not open (a replayed
/// webhook, an unknown reference) changes nothing and answers `None`.
///
/// # Errors
///
/// The refusals of the machine other than "not charged back".
pub async fn gateway_dispute_won(
    state: &AppState,
    reference: &str,
) -> std::result::Result<Option<Payment>, TransitionError> {
    let Some(payment) = by_dispute(&state.db, reference).await? else {
        return Ok(None);
    };
    if payment.status != PaymentStatus::ChargedBack {
        return Ok(None);
    }
    match payment
        .transition(WIN_CHARGEBACK)
        .by_system()
        .input(&CloseInput::default())
        .run(state)
        .await
    {
        Ok(_) => {}
        Err(TransitionError::NotFrom(_)) => return Ok(None),
        Err(other) => return Err(other),
    }
    reload(state, payment.id).await.map(Some)
}

/// Runs `lose_chargeback` on `payment` as the system.
async fn lose_by_system(
    state: &AppState,
    payment: &Payment,
) -> std::result::Result<Option<Payment>, TransitionError> {
    match payment
        .transition(LOSE_CHARGEBACK_BY_SYSTEM)
        .by_system()
        .input(&CloseInput::default())
        .run(state)
        .await
    {
        Ok(_) => {}
        Err(TransitionError::NotFrom(_)) => return Ok(None),
        Err(other) => return Err(other),
    }
    reload(state, payment.id).await.map(Some)
}

/// The gateway reports that the dispute `reference` was lost
/// (`DisputeLost`): the payment is `reversed`, the invoices stay open. A
/// dispute that is not open changes nothing and answers `None`.
///
/// # Errors
///
/// The refusals of the machine other than "not charged back".
pub async fn gateway_dispute_lost(
    state: &AppState,
    reference: &str,
) -> std::result::Result<Option<Payment>, TransitionError> {
    let Some(payment) = by_dispute(&state.db, reference).await? else {
        return Ok(None);
    };
    if payment.status != PaymentStatus::ChargedBack {
        return Ok(None);
    }
    lose_by_system(state, &payment).await
}

/// `user` records that the dispute of `payment` was lost, from the acquirer's
/// notice (N-Genius).
///
/// # Errors
///
/// `Forbidden` without `payments.chargeback`; `NotFrom` unless the payment is
/// `charged_back`.
pub async fn lose_by_hand(
    state: &AppState,
    user: &AuthUser,
    payment: &Payment,
) -> std::result::Result<Payment, TransitionError> {
    if !user.has_permission(permissions::CHARGEBACK) {
        return Err(TransitionError::Forbidden);
    }
    let given = CloseInput {
        by: Some(user.id),
        reason: None,
    };
    let _outcome = payment
        .transition(LOSE_CHARGEBACK)
        .by(user)
        .input(&given)
        .run(state)
        .await?;
    reload(state, payment.id).await
}

/// `FIN.ACL.18`: what asking to accept a chargeback came to.
#[derive(Debug, Clone)]
pub struct Accepted {
    /// The payment as it is now: `charged_back` while the approval waits.
    pub payment: Payment,
    /// The approval request that waits for another person, if there is one;
    /// `None` when the acceptance ran at once (no policy is installed).
    pub request: Option<i64>,
}

/// `user` asks to accept the chargeback of `payment` (not contest it), with a
/// `reason`. It waits for the approval of another holder of
/// `payments.chargeback` (`FIN.ACL.18`); the payment stays `charged_back`.
///
/// # Errors
///
/// `Forbidden` without `payments.chargeback`; `NotFrom` unless the payment is
/// `charged_back`; `ReasonRequired` without a reason.
pub async fn accept(
    state: &AppState,
    user: &AuthUser,
    payment: &Payment,
    reason: &str,
) -> std::result::Result<Accepted, TransitionError> {
    if !user.has_permission(permissions::CHARGEBACK) {
        return Err(TransitionError::Forbidden);
    }
    let reason = reason.trim();
    // Before an approval request is made: a request carries a reason.
    if reason.is_empty() {
        return Err(TransitionError::ReasonRequired);
    }
    let given = CloseInput {
        by: Some(user.id),
        reason: Some(reason.to_owned()),
    };
    let outcome = payment
        .transition(ACCEPT_CHARGEBACK)
        .by(user)
        .input(&given)
        .reason(reason.to_owned())
        .run(state)
        .await?;
    // The approvers are told once the request is stored.
    approvals::announce(state, &outcome)
        .await
        .map_err(TransitionError::Failed)?;
    let request = match &outcome {
        Outcome::Pending(pending) => pending
            .data
            .get(approvals::submit::REQUEST_KEY)
            .and_then(Value::as_i64),
        Outcome::Done(_) => None,
    };
    Ok(Accepted {
        payment: reload(state, payment.id).await?,
        request,
    })
}

/// The removed allocations a chargeback took from `payment`, oldest first.
async fn removed_by_chargeback(db: &Db, payment_id: i64) -> Result<Vec<Allocation>> {
    Ok(Allocation::all_of(db, payment_id)
        .await?
        .into_iter()
        .filter(|a| !a.is_active() && a.removed_reason.as_deref() == Some(REMOVED_REASON))
        .collect())
}

/// Marks the open evidence task of `payment_id` done: the dispute is closed.
async fn close_evidence_tasks(db: &Db, payment_id: i64) -> Result {
    let open = FollowupTask::query()
        .where_(FollowupTask::KIND.eq(FollowupKind::ChargebackEvidence))
        .where_(FollowupTask::PAYMENT_ID.eq(payment_id))
        .where_(FollowupTask::STATUS.eq(FollowupStatus::Open))
        .get(db)
        .await?;
    for mut task in open {
        task.status = FollowupStatus::Done;
        task.done_at = Some(renox::db::now());
        task.save(db).await?;
    }
    Ok(())
}

/// After a committed `win_chargeback` of payment `id`: apply the removed
/// allocations again where their targets are open, keep the rest as credit,
/// emit `PaymentReceived`, tell the accountants when something stayed credit.
async fn after_win(state: &AppState, id: i64) -> Result {
    let payment = Payment::find_or_404(&state.db, id).await?;
    close_evidence_tasks(&state.db, payment.id).await?;
    let removed = removed_by_chargeback(&state.db, payment.id).await?;
    let invoices = invoicer(state);
    let mut left = credit::unallocated(&state.db, &payment).await?;
    let mut applied = Vec::new();
    let mut kept = 0;
    for row in &removed {
        let mut target = None;
        let mut amount = 0;
        if let Some(AllocationTarget::Invoice(invoice_id)) = row.allocation_target() {
            target = Some(invoice_id);
            if let Some(invoice) = invoices.find(&state.db, invoice_id).await?
                && invoice.status.is_open()
            {
                amount = row.amount.min(invoice.balance).min(left);
            }
        }
        let mut done = 0;
        if let Some(invoice_id) = target
            && amount > 0
            && allocate::allocate(
                state,
                payment.payment_id(),
                AllocationTarget::Invoice(invoice_id),
                amount,
                None,
            )
            .await
            .is_ok()
        {
            done = amount;
            left -= amount;
            applied.push(PaymentAllocation {
                target: row.target(),
                amount,
            });
        }
        kept += row.amount - done;
        // The row stays; it no longer waits for a won dispute.
        Allocation::query()
            .where_(Allocation::ID.eq(row.id))
            .update(
                &state.db,
                &[
                    ("removed_reason", &Some(WON_REASON.to_owned())),
                    ("updated_at", &renox::db::now()),
                ],
            )
            .await?;
    }
    state.emit(received_event(&payment, applied)).await?;
    if kept > 0 {
        let text = format!(
            "Chargeback {} on payment {} was won; {} could not be applied again (the invoice was written off or paid meanwhile) and stays as the party's credit",
            payment.dispute_reference.clone().unwrap_or_default(),
            payment.number.clone().unwrap_or_default(),
            money_text(&payment.currency, kept)
        );
        notices::accountants_told(state, &payment, &text).await?;
    }
    Ok(())
}

/// After a committed lost or accepted dispute of payment `id`:
/// `ChargebackClosed`, and the accountants are told that the invoices stay
/// open.
async fn after_end(state: &AppState, id: i64, outcome: ChargebackOutcome) -> Result {
    let payment = Payment::find_or_404(&state.db, id).await?;
    close_evidence_tasks(&state.db, payment.id).await?;
    let reopened: Vec<PaymentAllocation> = removed_by_chargeback(&state.db, payment.id)
        .await?
        .iter()
        .map(|a| PaymentAllocation {
            target: a.target(),
            amount: a.amount,
        })
        .collect();
    state
        .emit(ChargebackClosed {
            payment: payment.payment_id(),
            party: payment.party(),
            amount: payment.amount,
            currency: payment.currency.clone(),
            outcome,
            reopened,
        })
        .await?;
    let text = format!(
        "Chargeback {} on payment {} of {} ended {}; the invoices it paid stay open: collect the amount from the party again, or write them off",
        payment.dispute_reference.clone().unwrap_or_default(),
        payment.number.clone().unwrap_or_default(),
        money_text(&payment.currency, payment.amount),
        match outcome {
            ChargebackOutcome::Lost => "lost",
            ChargebackOutcome::Accepted => "accepted",
        }
    );
    notices::accountants_told(state, &payment, &text).await
}

/// The deadline rule: a chargeback whose evidence deadline passed before
/// `on` without a submission is lost (`lose_chargeback` by the system).
/// Returns how many were closed.
///
/// # Errors
///
/// The database's errors.
pub async fn run_deadlines(state: &AppState, on: NaiveDate) -> Result<usize> {
    let late = Payment::query()
        .where_(Payment::STATUS.eq(PaymentStatus::ChargedBack))
        .where_(Payment::DISPUTE_EVIDENCE_DUE.lt(on))
        .where_(Payment::DISPUTE_EVIDENCE_SUBMITTED_ON.is_null())
        .order_by(Payment::ID)
        .get(&state.db)
        .await?;
    let mut closed = 0;
    for payment in late {
        match lose_by_system(state, &payment).await {
            Ok(Some(_)) => closed += 1,
            Ok(None) => {}
            Err(other) => return Err(other.into_error()),
        }
    }
    Ok(closed)
}

/// The final re-check of an approval about a payment, run before the kernel
/// runs `<transition>_approved`. It keeps the money-out guard of a set-off
/// ([`wl_kernel::block_guard::recheck`], which this hook replaces) and, for
/// the acceptance of a chargeback, tells the gateway: a gateway that refuses
/// ends the request with `fail`, and the payment stays `charged_back`.
async fn recheck(
    tx: &mut renox::db::Transaction,
    request: &ApprovalRequest,
) -> Result<Option<String>> {
    if let Some(message) = wl_kernel::block_guard::recheck(tx, request).await? {
        return Ok(Some(message));
    }
    if request.transition != ACCEPT_CHARGEBACK {
        return Ok(None);
    }
    let Some(payment) = Payment::find(&mut *tx, request.subject_id).await? else {
        return Ok(None);
    };
    if payment.status != PaymentStatus::ChargedBack {
        return Ok(Some("the payment is no longer charged back".to_owned()));
    }
    let (Some(account), Some(txn)) = (
        payment.gateway.as_deref(),
        payment.gateway_transaction.as_deref(),
    ) else {
        return Ok(None);
    };
    let Some(gateway) = gateway::by_account(account) else {
        return Ok(None);
    };
    let dispute = payment.dispute_reference.clone().unwrap_or_default();
    match gateway.accept_dispute(txn, &dispute).await {
        Ok(()) => Ok(None),
        Err(error) => Ok(Some(format!("the gateway rejected the acceptance: {error}"))),
    }
}

/// Registers the approval policy of `accept_chargeback` (always, by another
/// holder of `payments.chargeback`, `FIN.ACL.18`), the page the approval card
/// links to and the final re-check. Call it **after** the Payment subject
/// machine is registered: the re-check replaces the one it registers.
pub(crate) fn register_approval() {
    PolicyDefault::new(ACCEPT_ACTION)
        .step_permission(permissions::CHARGEBACK)
        .register();
    approvals::register_subject_link(PAYMENT_SUBJECT_TYPE, |id| format!("/payments/{id}"));
    approvals::register_final_check(PAYMENT_SUBJECT_TYPE, |tx, request| {
        Box::pin(recheck(tx, request))
    });
}

/// Registers the listeners of the closing transitions and of a refused
/// acceptance, and the daily task of the deadline rule.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: Transitioned<Payment>, state: AppState| async move {
        match event.transition.as_str() {
            WIN_CHARGEBACK => after_win(&state, event.key).await,
            LOSE_CHARGEBACK | LOSE_CHARGEBACK_BY_SYSTEM => {
                after_end(&state, event.key, ChargebackOutcome::Lost).await
            }
            ACCEPT_CHARGEBACK | ACCEPT_CHARGEBACK_APPROVED => {
                after_end(&state, event.key, ChargebackOutcome::Accepted).await
            }
            _ => Ok(()),
        }
    });
    // The gateway refused the acceptance: the payment stays `charged_back`.
    app.listen(|event: ApprovalDecided, state: AppState| async move {
        if event.decision != ApprovalEnd::Failed || event.subject.subject_type != PAYMENT_SUBJECT_TYPE
        {
            return Ok(());
        }
        let Some(request) = ApprovalRequest::find(&state.db, event.request).await? else {
            return Ok(());
        };
        if request.transition != ACCEPT_CHARGEBACK {
            return Ok(());
        }
        let payment = Payment::find_or_404(&state.db, event.subject.subject_id).await?;
        let text = format!(
            "The acceptance of chargeback {} on payment {} failed ({}): the payment stays charged back",
            payment.dispute_reference.clone().unwrap_or_default(),
            payment.number.clone().unwrap_or_default(),
            "the gateway refused it or the payment changed"
        );
        notices::accountants_told(&state, &payment, &text).await
    });
    app.schedule()
        .daily_at("01:00", DEADLINE_TASK, |state: AppState| async move {
            run_deadlines(&state, today()).await.map(|_| ())
        });
    app.command(
        DEADLINE_TASK,
        "Lose the chargebacks whose evidence deadline passed without a submission",
        |_args, state: AppState| async move {
            let closed = run_deadlines(&state, today()).await?;
            println!("{closed} chargebacks lost.");
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chargeback_leaves_charged_back_and_the_acceptance_waits_for_approval() {
        let machine = Payment::machine();
        for (name, to, system) in [
            (WIN_CHARGEBACK, "received", true),
            (LOSE_CHARGEBACK, "reversed", false),
            (LOSE_CHARGEBACK_BY_SYSTEM, "reversed", true),
            (ACCEPT_CHARGEBACK, "reversed", false),
            (ACCEPT_CHARGEBACK_APPROVED, "reversed", true),
        ] {
            let found = machine.find(name).expect("transition");
            assert_eq!(found.from_states(), ["charged_back"], "{name}");
            assert_eq!(found.to_state(), Some(to), "{name}");
            assert_eq!(found.is_system(), system, "{name}");
        }
        // The kernel runs `<transition>_approved` by name.
        assert_eq!(
            ACCEPT_CHARGEBACK_APPROVED,
            format!("{ACCEPT_CHARGEBACK}_approved")
        );
        assert_eq!(ACCEPT_ACTION, "payments.accept_chargeback");
    }

    #[renox::test]
    async fn an_acceptance_needs_a_reason() {
        let machine = Payment::machine();
        let accept = machine.find(ACCEPT_CHARGEBACK).expect("transition");
        assert!(accept.reason_required());
        let lose = machine.find(LOSE_CHARGEBACK).expect("transition");
        assert!(!lose.reason_required());
    }
}
