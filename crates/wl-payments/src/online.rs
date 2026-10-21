//! Online payments (wl-payments §4 Payment `start_online`, `confirm`, `fail`
//! and `late_confirm`, §5.2, `FIN.RUL.19`).
//!
//! ```text
//! [*] --start_online--> pending --confirm--> received
//! pending --fail--> failed --late_confirm--> received
//! ```
//!
//! The party opens a payment link ([`crate::payment_link`]) on a public page
//! and starts a payment on it ([`start_online`]): a `pending` payment with the
//! gateway's transaction. What the gateway says later reaches the module
//! through [`gateway_succeeded`] and [`gateway_failed`] (the webhooks of the
//! gateway adapter; a payment the module no longer waits for is ignored, so a
//! replayed webhook does nothing), and [`check_pending`] is `FIN.RUL.19`: M
//! minutes after a payment became `pending` it asks the gateway and runs
//! `confirm`, or `fail` with the reason `timeout` when the gateway declines,
//! says it still processes the payment or does not answer.
//!
//! All three transitions are the system's. The follow-up of a committed one
//! runs in a listener on `Transitioned<Payment>`:
//!
//! - `confirm`: the money is allocated to the documents of the link that are
//!   still open, the link is `paid`, `PaymentReceived` is emitted;
//! - `fail`: `PaymentFailed` is emitted (the link stays `open`: the party sees
//!   the reason and a retry button); nothing is confirmed, no task is made;
//! - `late_confirm`: see [`crate::late`].

use renox::chrono::TimeDelta;
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::{
    StateMachine, Transition, TransitionCtx, TransitionError, Transitioned,
};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;
use wl_kernel::invoicing::invoicer;
use wl_kernel::payments::{
    AllocationTarget, PaymentAllocation, PaymentFailed, PaymentFailureKind, PaymentMethod,
    PaymentReceived, PaymentSource, PaymentStatus,
};
use wl_kernel::{LegalEntity, SUBSCRIPTION_SUBJECT_TYPE, Subject};

use crate::allocate;
use crate::config_of;
use crate::credit;
use crate::followup::FollowupTask;
use crate::gateway::{Checkout, GatewayPaymentStatus, gateway_of, installed_gateway};
use crate::notices;
use crate::payment::Payment;
use crate::payment_link::{self, LinkStatus, PaymentLink};
use crate::record::assign_number;

/// The history reason of the birth of an online payment.
pub const START_ONLINE: &str = "start_online";

/// The `confirm` transition's name.
pub const CONFIRM: &str = "confirm";

/// The `fail` transition's name.
pub const FAIL: &str = "fail";

/// The `late_confirm` transition's name.
pub const LATE_CONFIRM: &str = "late_confirm";

/// The rule that checks a payment that stayed `pending`.
pub const RULE: &str = "FIN.RUL.19";

/// The scheduled task and command of the rule.
pub const TASK: &str = "payments:pending_timeout";

/// The message of a confirmation whose amount is not the payment's.
pub const AMOUNT_MISMATCH: &str = "The amount the gateway reports does not match the payment.";

/// What a run carries to its guards and effects.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct OnlineInput {
    /// The amount the gateway reports, in fils (`confirm`, `late_confirm`).
    pub amount: Option<i64>,
    /// Why the payment failed (`fail`).
    pub kind: Option<PaymentFailureKind>,
    /// The words for it.
    pub reason: Option<String>,
}

fn input(ctx: &TransitionCtx) -> Option<OnlineInput> {
    ctx.input_as::<OnlineInput>().ok()
}

/// The words stored and shown for a failure: the gateway's own text when it
/// gave one, else a text for the kind.
pub(crate) fn failure_text(kind: PaymentFailureKind, given: &str) -> String {
    let given = given.trim();
    if !given.is_empty() {
        return given.to_owned();
    }
    match kind {
        PaymentFailureKind::Declined => "Declined by the gateway",
        PaymentFailureKind::Abandoned => "Cancelled by the payer",
        PaymentFailureKind::Timeout => "No final answer from the gateway in time",
    }
    .to_owned()
}

/// The three transitions, for the machine.
pub(crate) fn transitions() -> Vec<Transition<Payment>> {
    vec![confirm(), fail(), late_confirm()]
}

/// `confirm`: pending → received, when the gateway says it was paid.
fn confirm() -> Transition<Payment> {
    Transition::new(CONFIRM)
        .label("Confirm")
        .tone("primary")
        .system()
        .from([PaymentStatus::Pending])
        .to(PaymentStatus::Received)
        .guard("amount_matches", |payment: &Payment, ctx| {
            let expected = payment.amount;
            let given = input(ctx).and_then(|i| i.amount);
            Box::pin(async move {
                if given == Some(expected) {
                    Ok(())
                } else {
                    Err(AMOUNT_MISMATCH.to_owned())
                }
            })
        })
        .effect(|payment: &Payment, _ctx, tx| {
            let payment = payment.clone();
            Box::pin(async move {
                assign_number(&mut *tx, &payment).await?;
                // The money moved today, not when the payment was started.
                Payment::query()
                    .where_(Payment::ID.eq(payment.id))
                    .update(
                        &mut *tx,
                        &[
                            ("payment_date", &today()),
                            ("updated_at", &renox::db::now()),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// `fail`: pending → failed, with the reason.
fn fail() -> Transition<Payment> {
    Transition::new(FAIL)
        .label("Fail")
        .tone("danger")
        .system()
        .from([PaymentStatus::Pending])
        .to(PaymentStatus::Failed)
        .guard("reason_given", |_payment: &Payment, ctx| {
            let kind = input(ctx).and_then(|i| i.kind);
            Box::pin(async move {
                if kind.is_some() {
                    Ok(())
                } else {
                    Err("A failed payment needs its reason.".to_owned())
                }
            })
        })
        .effect(|payment: &Payment, ctx, tx| {
            let id = payment.id;
            let given = input(ctx).unwrap_or_default();
            Box::pin(async move {
                let kind = given.kind.unwrap_or_default();
                let reason = failure_text(kind, given.reason.as_deref().unwrap_or_default());
                let now = renox::db::now();
                // Not the status column, so outside the machine's guard.
                Payment::query()
                    .where_(Payment::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("failure_kind", &Some(kind.as_str().to_owned())),
                            ("failure_reason", &Some(reason)),
                            ("failed_at", &Some(now)),
                            ("updated_at", &now),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// `late_confirm`: failed → received, when the gateway confirms after the
/// failure.
fn late_confirm() -> Transition<Payment> {
    Transition::new(LATE_CONFIRM)
        .label("Confirm late")
        .tone("primary")
        .system()
        .from([PaymentStatus::Failed])
        .to(PaymentStatus::Received)
        .guard("amount_matches", |payment: &Payment, ctx| {
            let expected = payment.amount;
            let given = input(ctx).and_then(|i| i.amount);
            Box::pin(async move {
                if given == Some(expected) {
                    Ok(())
                } else {
                    Err(AMOUNT_MISMATCH.to_owned())
                }
            })
        })
        .effect(|payment: &Payment, _ctx, tx| {
            let payment = payment.clone();
            Box::pin(async move {
                assign_number(&mut *tx, &payment).await?;
                let now = renox::db::now();
                Payment::query()
                    .where_(Payment::ID.eq(payment.id))
                    .update(
                        &mut *tx,
                        &[
                            ("payment_date", &today()),
                            ("late_confirmed_at", &Some(now)),
                            ("updated_at", &now),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// What starting an online payment gave.
#[derive(Debug, Clone)]
pub struct Started {
    /// The `pending` payment.
    pub payment: Payment,
    /// The hosted page to send the party to.
    pub checkout: Checkout,
}

fn guard(message: &str) -> TransitionError {
    TransitionError::Guard(message.to_owned())
}

/// Starts an online payment on the link `link_id` (`start_online`): the
/// gateway makes a hosted page, and a `pending` payment of the link's amount
/// is stored with the gateway's transaction. The failed payments of the link
/// stay as they are.
///
/// # Errors
///
/// `Guard` (422) when the link is not `open` or its expiry passed, when a
/// payment on it is still `pending`, when the party or the legal entity is
/// not found or active, and while no gateway is installed. `Failed` when the
/// gateway cannot make the page. Nothing is written then.
pub async fn start_online(
    state: &AppState,
    link_id: i64,
) -> std::result::Result<Started, TransitionError> {
    let link = PaymentLink::find(&state.db, link_id)
        .await?
        .ok_or_else(|| guard("The payment link was not found."))?;
    if link.status != LinkStatus::Open {
        return Err(guard("This payment link can no longer be paid."));
    }
    if renox::db::now() >= link.expires_at {
        return Err(guard("This payment link has expired."));
    }
    let waiting = Payment::query()
        .where_(Payment::PAYMENT_LINK_ID.eq(link.id))
        .where_(Payment::STATUS.eq(PaymentStatus::Pending))
        .count(&state.db)
        .await?;
    if waiting > 0 {
        return Err(guard("A payment on this link is already in progress."));
    }
    let entity = LegalEntity::find(&state.db, link.legal_entity_id).await?;
    if !entity.as_ref().is_some_and(LegalEntity::is_active) {
        return Err(guard("The legal entity is not active."));
    }
    let party = wl_kernel::parties::directory(state)
        .find(&state.db, link.party())
        .await?;
    if party.is_none() {
        return Err(guard("The party was not found."));
    }
    let Some(gateway) = installed_gateway(state) else {
        return Err(guard("Online payment is not available."));
    };
    let checkout = gateway
        .checkout(link.amount, &link.currency, &format!("link-{}", link.id))
        .await
        .map_err(TransitionError::Failed)?;
    let mut payment = Payment {
        legal_entity_id: link.legal_entity_id,
        party_id: link.party_id,
        method: PaymentMethod::Gateway,
        amount: link.amount,
        currency: link.currency.clone(),
        payment_date: today(),
        source: PaymentSource::Gateway,
        gateway: Some(gateway.account().to_owned()),
        gateway_transaction: Some(checkout.transaction.clone()),
        collected_by: link.created_by,
        prepaid_type: link.prepaid_type.clone(),
        prepaid_id: link.prepaid_id,
        purpose: "none".to_owned(),
        payment_link_id: Some(link.id),
        subscription_id: link.subscription_id,
        status: PaymentStatus::Pending,
        ..Default::default()
    };
    let mut tx = state.db.begin().await?;
    payment
        .insert_in_state(&mut tx, PaymentStatus::Pending, START_ONLINE)
        .await?;
    tx.commit().await?;
    Ok(Started { payment, checkout })
}

/// The payment the gateway transaction `gateway_txn` belongs to, if any.
pub(crate) async fn by_transaction(db: &Db, gateway_txn: &str) -> Result<Option<Payment>> {
    Payment::query()
        .where_(Payment::GATEWAY_TRANSACTION.eq(gateway_txn))
        .first(db)
        .await
}

/// The gateway says the online payment `gateway_txn` was paid, `amount` fils
/// (`PaymentSucceeded`): a `pending` payment is confirmed, a `failed` one is
/// confirmed late. A payment in any other state (a replayed webhook, an
/// unknown transaction) is left alone and `None` is returned.
///
/// An amount that is not the payment's is not confirmed: the accountant gets
/// a task and a notice, and the call is refused.
///
/// # Errors
///
/// `Guard` (422) for an amount that does not match.
pub async fn gateway_succeeded(
    state: &AppState,
    gateway_txn: &str,
    amount: i64,
) -> std::result::Result<Option<Payment>, TransitionError> {
    let Some(payment) = by_transaction(&state.db, gateway_txn).await? else {
        return Ok(None);
    };
    let name = match payment.status {
        PaymentStatus::Pending => CONFIRM,
        PaymentStatus::Failed => LATE_CONFIRM,
        _ => return Ok(None),
    };
    if amount != payment.amount {
        let task = FollowupTask::amount_mismatch(&payment, amount);
        if let Some(task) = FollowupTask::create_once(&state.db, task).await? {
            notices::accountants_told(state, &payment, &task.text).await?;
        }
        return Err(guard(AMOUNT_MISMATCH));
    }
    let run = OnlineInput {
        amount: Some(amount),
        ..Default::default()
    };
    match payment
        .transition(name)
        .by_system()
        .input(&run)
        .run(state)
        .await
    {
        Ok(_) => {}
        // The other one of two answers that came together won.
        Err(TransitionError::NotFrom(_)) => return Ok(None),
        Err(other) => return Err(other),
    }
    Ok(Some(
        Payment::find_or_404(&state.db, payment.id)
            .await
            .map_err(TransitionError::Failed)?,
    ))
}

/// The gateway says the online payment `gateway_txn` failed (`declined`, with
/// its `reason`) or that the payer cancelled the hosted session
/// (`abandoned`). Only a `pending` payment fails; the first of two answers
/// that come together wins and the second finds the payment `failed` and
/// does nothing (`None`).
///
/// # Errors
///
/// The refusals of the machine other than "not pending".
pub async fn gateway_failed(
    state: &AppState,
    gateway_txn: &str,
    kind: PaymentFailureKind,
    reason: &str,
) -> std::result::Result<Option<Payment>, TransitionError> {
    let Some(payment) = by_transaction(&state.db, gateway_txn).await? else {
        return Ok(None);
    };
    if payment.status != PaymentStatus::Pending {
        return Ok(None);
    }
    fail_payment(state, &payment, kind, reason).await
}

/// Runs `fail` on `payment`.
async fn fail_payment(
    state: &AppState,
    payment: &Payment,
    kind: PaymentFailureKind,
    reason: &str,
) -> std::result::Result<Option<Payment>, TransitionError> {
    let run = OnlineInput {
        amount: None,
        kind: Some(kind),
        reason: Some(reason.to_owned()),
    };
    match payment
        .transition(FAIL)
        .by_system()
        .input(&run)
        .run(state)
        .await
    {
        Ok(_) => {}
        Err(TransitionError::NotFrom(_)) => return Ok(None),
        Err(other) => return Err(other),
    }
    Ok(Some(
        Payment::find_or_404(&state.db, payment.id)
            .await
            .map_err(TransitionError::Failed)?,
    ))
}

/// What a run of `FIN.RUL.19` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PendingReport {
    /// Payments the gateway said were paid: confirmed.
    pub confirmed: usize,
    /// Payments failed: declined, or no final answer (`timeout`).
    pub failed: usize,
}

/// Whether a payment that became `pending` at `since` is checked at `now`,
/// `minutes` later.
pub fn check_due(since: DateTime, minutes: i64, now: DateTime) -> bool {
    since + TimeDelta::minutes(minutes) <= now
}

/// `FIN.RUL.19`: of the online payments still `pending` M minutes after they
/// became so, asks the gateway the final state and runs `confirm` when it was
/// paid, `fail` (declined) when it was declined, and `fail` with the reason
/// `timeout` when the answer is not final (still processing) or the query
/// fails. A later webhook can still confirm such a payment late.
///
/// # Errors
///
/// The database's errors. A payment whose transition errors does not stop
/// the run: the others are handled, and the first error is returned after
/// the batch.
pub async fn check_pending(state: &AppState, now: DateTime) -> Result<PendingReport> {
    let minutes = config_of(state).pending_timeout_minutes;
    let pending = Payment::query()
        .where_(Payment::STATUS.eq(PaymentStatus::Pending))
        .where_(Payment::METHOD.eq(PaymentMethod::Gateway))
        .order_by(Payment::ID)
        .get(&state.db)
        .await?;
    let gateway = gateway_of(state);
    let mut report = PendingReport::default();
    let mut first_error = None;
    for payment in pending {
        let Some(since) = payment.created_at else {
            continue;
        };
        if !check_due(since, minutes, now) {
            continue;
        }
        // Decision: a payment with no gateway transaction cannot be asked
        // about; it times out like one the gateway gives no final answer for.
        let Some(txn) = payment.gateway_transaction.clone() else {
            match fail_payment(state, &payment, PaymentFailureKind::Timeout, "").await {
                Ok(Some(_)) => report.failed += 1,
                Ok(None) => {}
                Err(other) => note_error(&mut first_error, payment.id, other.into_error()),
            }
            continue;
        };
        let answer = gateway.payment_status(&txn).await;
        let result = match answer {
            Ok(GatewayPaymentStatus::Paid) => {
                match gateway_succeeded(state, &txn, payment.amount).await {
                    Ok(Some(_)) => {
                        report.confirmed += 1;
                        continue;
                    }
                    Ok(None) => continue,
                    Err(other) => Err(other),
                }
            }
            Ok(GatewayPaymentStatus::Declined { reason }) => {
                fail_payment(state, &payment, PaymentFailureKind::Declined, &reason).await
            }
            Ok(GatewayPaymentStatus::Processing) | Err(_) => {
                fail_payment(state, &payment, PaymentFailureKind::Timeout, "").await
            }
        };
        match result {
            Ok(Some(_)) => report.failed += 1,
            Ok(None) => {}
            Err(other) => note_error(&mut first_error, payment.id, other.into_error()),
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(report),
    }
}

/// Logs the error of payment `payment` and keeps the first one of the run,
/// returned once the whole batch was handled: one payment that errors must
/// not stop (or starve) the others.
fn note_error(first: &mut Option<renox::Error>, payment: i64, error: renox::Error) {
    eprintln!("payments:pending_timeout: payment {payment} failed: {error}");
    first.get_or_insert(error);
}

/// Allocates the unallocated money of the confirmed `payment` to the
/// documents of its link that are still open, each up to its open balance and
/// the amount the link gave it; a document that refuses is skipped and the
/// money stays as the party's credit. Returns what was allocated.
///
/// # Errors
///
/// The database's errors.
pub(crate) async fn allocate_to_link(
    state: &AppState,
    payment: &Payment,
) -> Result<Vec<PaymentAllocation>> {
    let Some(link_id) = payment.payment_link_id else {
        return Ok(Vec::new());
    };
    let Some(link) = PaymentLink::find(&state.db, link_id).await? else {
        return Ok(Vec::new());
    };
    let invoices = invoicer(state);
    let mut left = credit::unallocated(&state.db, payment).await?;
    let mut done = Vec::new();
    for target in link.targets.0.iter() {
        let Some(AllocationTarget::Invoice(id)) = AllocationTarget::from_subject(&target.subject())
        else {
            continue;
        };
        let Some(invoice) = invoices.find(&state.db, id).await? else {
            continue;
        };
        if !invoice.status.is_open() {
            continue;
        }
        let amount = target.amount.min(invoice.balance).min(left);
        if amount <= 0 {
            continue;
        }
        let allocated = allocate::allocate(
            state,
            payment.payment_id(),
            AllocationTarget::Invoice(id),
            amount,
            None,
        )
        .await;
        if allocated.is_ok() {
            left -= amount;
            done.push(PaymentAllocation {
                target: target.subject(),
                amount,
            });
        }
    }
    Ok(done)
}

/// The event of a payment the gateway confirmed.
pub(crate) fn received_event(
    payment: &Payment,
    allocations: Vec<PaymentAllocation>,
) -> PaymentReceived {
    PaymentReceived {
        payment: payment.payment_id(),
        party: payment.party(),
        amount: payment.amount,
        method: payment.method,
        allocations,
    }
}

/// After a committed `confirm`: allocate, `pay` the link, `PaymentReceived`.
async fn after_confirm(state: &AppState, id: i64) -> Result {
    let payment = Payment::find_or_404(&state.db, id).await?;
    let allocations = allocate_to_link(state, &payment).await?;
    if let Some(link_id) = payment.payment_link_id {
        payment_link::mark_paid(state, link_id, payment.id).await?;
    }
    state.emit(received_event(&payment, allocations)).await
}

/// The event of a failed online payment.
pub(crate) fn failed_event(payment: &Payment) -> PaymentFailed {
    PaymentFailed {
        payment: payment.payment_id(),
        party: payment.party(),
        amount: payment.amount,
        kind: payment.failure_kind.unwrap_or_default(),
        reason: payment.failure_reason.clone().unwrap_or_default(),
        subscription: payment
            .subscription_id
            .map(|id| Subject::new(SUBSCRIPTION_SUBJECT_TYPE, id)),
        payment_link: payment.payment_link(),
        prepaid: payment.prepaid(),
    }
}

/// After a committed `fail`: `PaymentFailed`. The link stays `open`.
async fn after_fail(state: &AppState, id: i64) -> Result {
    let payment = Payment::find_or_404(&state.db, id).await?;
    state.emit(failed_event(&payment)).await
}

/// Registers the listener of the three transitions, and the scheduled task
/// and command of `FIN.RUL.19`.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: Transitioned<Payment>, state: AppState| async move {
        match event.transition.as_str() {
            CONFIRM => after_confirm(&state, event.key).await,
            FAIL => after_fail(&state, event.key).await,
            LATE_CONFIRM => crate::late::after_late_confirm(&state, event.key).await,
            _ => Ok(()),
        }
    });
    app.schedule()
        .every_minutes(5, TASK, |state: AppState| async move {
            check_pending(&state, renox::db::now()).await.map(|_| ())
        });
    app.command(
        TASK,
        "Ask the gateway about the online payments that stayed pending (FIN.RUL.19)",
        |_args, state: AppState| async move {
            let report = check_pending(&state, renox::db::now()).await?;
            println!(
                "{} payments confirmed, {} failed.",
                report.confirmed, report.failed
            );
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(amount: i64) -> Payment {
        Payment {
            amount,
            status: PaymentStatus::Pending,
            ..Default::default()
        }
    }

    fn run(input: OnlineInput) -> TransitionCtx {
        TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).expect("json"))
    }

    #[test]
    fn the_online_transitions_are_the_systems_and_leave_their_states() {
        let machine = Payment::machine();
        for (name, from, to) in [
            (CONFIRM, "pending", "received"),
            (FAIL, "pending", "failed"),
            (LATE_CONFIRM, "failed", "received"),
        ] {
            let found = machine.find(name).expect("transition");
            assert!(found.is_system(), "{name}");
            assert_eq!(found.from_states(), [from], "{name}");
            assert_eq!(found.to_state(), Some(to), "{name}");
        }
        assert_eq!(
            machine.find(CONFIRM).unwrap().guard_names(),
            ["amount_matches"]
        );
        assert_eq!(machine.find(FAIL).unwrap().guard_names(), ["reason_given"]);
    }

    #[renox::test]
    async fn confirm_and_late_confirm_need_the_amount_of_the_payment() {
        let machine = Payment::machine();
        for name in [CONFIRM, LATE_CONFIRM] {
            let found = machine.find(name).expect("transition");
            let payment = pending(25_000);
            let right = OnlineInput {
                amount: Some(25_000),
                ..Default::default()
            };
            assert!(found.check_guards(&payment, &run(right)).await.is_ok());
            for given in [Some(24_999), Some(25_001), None] {
                let wrong = OnlineInput {
                    amount: given,
                    ..Default::default()
                };
                assert!(matches!(
                    found.check_guards(&payment, &run(wrong)).await,
                    Err(TransitionError::Guard(_))
                ));
            }
        }
    }

    #[renox::test]
    async fn fail_needs_a_reason_kind() {
        let machine = Payment::machine();
        let found = machine.find(FAIL).expect("transition");
        let payment = pending(100);
        let with = OnlineInput {
            kind: Some(PaymentFailureKind::Abandoned),
            ..Default::default()
        };
        assert!(found.check_guards(&payment, &run(with)).await.is_ok());
        assert!(matches!(
            found
                .check_guards(&payment, &run(OnlineInput::default()))
                .await,
            Err(TransitionError::Guard(_))
        ));
    }

    #[test]
    fn a_failure_reads_the_gateways_words_or_a_text_for_its_kind() {
        assert_eq!(
            failure_text(PaymentFailureKind::Declined, "  Insufficient funds "),
            "Insufficient funds"
        );
        assert_eq!(
            failure_text(PaymentFailureKind::Declined, ""),
            "Declined by the gateway"
        );
        assert_eq!(
            failure_text(PaymentFailureKind::Abandoned, " "),
            "Cancelled by the payer"
        );
        assert!(failure_text(PaymentFailureKind::Timeout, "").contains("in time"));
    }

    #[test]
    fn a_pending_payment_is_checked_after_the_timeout() {
        let since = renox::db::now();
        assert!(!check_due(since, 30, since + TimeDelta::minutes(29)));
        assert!(check_due(since, 30, since + TimeDelta::minutes(30)));
        assert!(check_due(since, 30, since + TimeDelta::hours(2)));
    }
}
