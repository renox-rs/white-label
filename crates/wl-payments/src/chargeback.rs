//! Recording a chargeback (wl-payments §4 `charge_back`, §5.2a steps 1 and 2,
//! `FIN.RUL.20`).
//!
//! The card holder disputes a gateway payment and the gateway takes the funds
//! back. The payment goes from `received` to `charged_back`:
//!
//! - **by the system**, when the gateway reports the dispute
//!   ([`gateway_dispute_opened`], Stripe's `DisputeOpened`, with the evidence
//!   deadline the gateway gives), `charge_back_by_system`;
//! - **by hand**, by a user with `payments.chargeback` ([`charge_back_by_hand`])
//!   for N-Genius, which has no disputes API: the acquirer's reference, the
//!   deadline and the fee are entered from its notice, `charge_back`.
//!
//! A dispute reference is recorded once: a replayed webhook does nothing, and
//! a reference already recorded on another payment is refused.
//!
//! In the transition's transaction every active allocation is removed (the
//! rows stay, with the reason `chargeback`) and the dispute is stored on the
//! payment. After the commit [`after_charge_back`] emits `PaymentReversed`
//! with the reason `chargeback` and the allocations (wl-invoicing runs
//! `unallocate` on each target, so the invoices reopen), records the gateway's
//! dispute fee as an outgoing payment to the gateway provider (a supplier
//! party, [`WlPayments::gateway_party`](crate::WlPayments::gateway_party);
//! purpose *bank charges*, allocated to no document), makes the accountant's
//! evidence task with the gateway's deadline and the links to what the
//! platform holds (the receipt, the invoices, the prepaid record), and tells
//! the accountants and the staff member who took the payment.
//!
//! `FIN.RUL.20` reminds the accountant K days before the deadline at 09:00 in
//! Asia/Dubai, once, and not when the evidence was already submitted
//! ([`evidence_submitted`]). Nothing is sent to the gateway and no service,
//! booking or order is undone.
//!
//! Decisions: a webhook without a deadline gets one 7 days from the day it is
//! recorded; a fee in another currency than the payment's is recorded in the
//! payment's currency as entered (no exchange rate is held here).

use renox::chrono::{NaiveDate, TimeDelta};
use renox::db::DateTime;
use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::{
    StateMachine, Transition, TransitionCtx, TransitionError, Transitioned,
};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::{DUBAI_OFFSET_SECONDS, dubai_date, today};
use wl_kernel::invoicing::invoicer;
use wl_kernel::payments::{
    AllocationTarget, Direction, PaymentMethod, PaymentReversed, PaymentSource, PaymentStatus,
    RecordPayment, RemovedAllocation, ReversalReason,
};

use crate::allocation::Allocation;
use crate::config_of;
use crate::followup::{FollowupKind, FollowupStatus, FollowupTask, money_text};
use crate::notices;
use crate::payment::Payment;
use crate::permissions;
use crate::record;

/// The `charge_back` transition's name: staff record a chargeback by hand.
pub const CHARGE_BACK: &str = "charge_back";

/// The `charge_back_by_system` transition's name: the gateway reports it.
pub const CHARGE_BACK_BY_SYSTEM: &str = "charge_back_by_system";

/// The rule that makes the evidence task and reminds of its deadline.
pub const RULE: &str = "FIN.RUL.20";

/// The scheduled task and command that send the reminders.
pub const REMINDER_TASK: &str = "payments:chargeback_reminders";

/// The purpose of the outgoing payment that records the dispute fee: the
/// category *bank charges*.
pub const BANK_CHARGES: &str = "bank_charges";

/// The reason stored on the allocations a chargeback removed.
pub const REMOVED_REASON: &str = "chargeback";

/// Days a deadline is set ahead when the gateway gives none.
pub const DEFAULT_DEADLINE_DAYS: i64 = 7;

/// What a chargeback run carries to its guards and effects.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct ChargebackInput {
    /// The gateway's (or the acquirer's) dispute reference.
    pub reference: Option<String>,
    /// The day the evidence is due.
    pub evidence_due: Option<NaiveDate>,
    /// The gateway's dispute fee, in fils.
    pub fee: Option<i64>,
    /// Whether another payment already carries the reference.
    pub reference_taken: bool,
    /// The user who records it; `None` for the system.
    pub by: Option<i64>,
}

fn input(ctx: &TransitionCtx) -> Option<ChargebackInput> {
    ctx.input_as::<ChargebackInput>().ok()
}

/// The shared rules of both transitions.
fn rules(transition: Transition<Payment>) -> Transition<Payment> {
    transition
        .from([PaymentStatus::Received])
        .to(PaymentStatus::ChargedBack)
        .guard("gateway_payment", |payment: &Payment, _ctx| {
            let fits =
                payment.direction == Direction::In && payment.method == PaymentMethod::Gateway;
            Box::pin(async move {
                if fits {
                    Ok(())
                } else {
                    Err("Only a payment through the gateway can be charged back.".to_owned())
                }
            })
        })
        .guard("reference_given", |_payment: &Payment, ctx| {
            let given = input(ctx)
                .and_then(|i| i.reference)
                .is_some_and(|r| !r.trim().is_empty());
            Box::pin(async move {
                if given {
                    Ok(())
                } else {
                    Err("The dispute reference is required.".to_owned())
                }
            })
        })
        .guard("reference_unused", |_payment: &Payment, ctx| {
            let taken = input(ctx).is_some_and(|i| i.reference_taken);
            Box::pin(async move {
                if taken {
                    Err("This dispute reference is already recorded.".to_owned())
                } else {
                    Ok(())
                }
            })
        })
        .guard("deadline_given", |_payment: &Payment, ctx| {
            let given = input(ctx).is_some_and(|i| i.evidence_due.is_some());
            Box::pin(async move {
                if given {
                    Ok(())
                } else {
                    Err("The evidence deadline is required.".to_owned())
                }
            })
        })
        .guard("fee_valid", |_payment: &Payment, ctx| {
            let fee = input(ctx).and_then(|i| i.fee);
            Box::pin(async move {
                match fee {
                    Some(fee) if fee <= 0 => Err("The dispute fee must be above 0.".to_owned()),
                    _ => Ok(()),
                }
            })
        })
        .effect(|payment: &Payment, ctx, tx| {
            let id = payment.id;
            let given = input(ctx).unwrap_or_default();
            Box::pin(async move {
                let now = renox::db::now();
                // The rows stay; they are marked removed, so a won dispute
                // knows what to apply again.
                Allocation::query()
                    .where_(Allocation::PAYMENT_ID.eq(id))
                    .where_(Allocation::REMOVED_AT.is_null())
                    .update(
                        &mut *tx,
                        &[
                            ("removed_at", &Some(now)),
                            ("removed_reason", &Some(REMOVED_REASON.to_owned())),
                            ("removed_by", &given.by),
                            ("updated_at", &now),
                        ],
                    )
                    .await?;
                let reference = given.reference.as_deref().map(|r| r.trim().to_owned());
                // Not the status column, so outside the machine's guard.
                Payment::query()
                    .where_(Payment::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("dispute_reference", &reference),
                            ("dispute_status", &Some("open".to_owned())),
                            ("dispute_evidence_due", &given.evidence_due),
                            ("dispute_opened_on", &Some(today())),
                            ("dispute_fee", &given.fee),
                            ("updated_at", &now),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// `charge_back` and `charge_back_by_system`, for the machine.
pub(crate) fn transitions() -> Vec<Transition<Payment>> {
    vec![
        rules(
            Transition::new(CHARGE_BACK)
                .label("Record chargeback")
                .tone("danger")
                .permission(permissions::CHARGEBACK),
        ),
        rules(
            Transition::new(CHARGE_BACK_BY_SYSTEM)
                .label("Record chargeback")
                .tone("danger")
                .system(),
        ),
    ]
}

/// What the gateway reports of a dispute it opened (`DisputeOpened`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisputeOpened {
    /// The gateway's dispute reference.
    pub reference: String,
    /// The day the evidence is due, if the gateway says.
    pub evidence_due: Option<NaiveDate>,
    /// The gateway's dispute fee, in fils, if it says.
    pub fee: Option<i64>,
}

impl DisputeOpened {
    /// A dispute `reference` with nothing else known.
    pub fn new(reference: &str) -> Self {
        DisputeOpened {
            reference: reference.trim().to_owned(),
            evidence_due: None,
            fee: None,
        }
    }

    /// With the evidence due on `day`.
    pub fn evidence_due(mut self, day: NaiveDate) -> Self {
        self.evidence_due = Some(day);
        self
    }

    /// With the gateway's fee of `fils`.
    pub fn fee(mut self, fils: i64) -> Self {
        self.fee = Some(fils);
        self
    }
}

/// Whether another payment than `except` already carries the dispute
/// `reference`.
async fn reference_taken(db: &Db, reference: &str, except: i64) -> Result<bool> {
    let taken = Payment::query()
        .where_(Payment::DISPUTE_REFERENCE.eq(reference.trim()))
        .where_(Payment::ID.ne(except))
        .count(db)
        .await?;
    Ok(taken > 0)
}

/// Runs the chargeback on `payment` as `actor` (the system when `None`).
async fn run(
    state: &AppState,
    actor: Option<&AuthUser>,
    payment: &Payment,
    mut given: ChargebackInput,
) -> std::result::Result<Payment, TransitionError> {
    given.by = actor.map(|user| user.id);
    given.reference_taken = match given.reference.as_deref() {
        Some(reference) if !reference.trim().is_empty() => {
            reference_taken(&state.db, reference, payment.id).await?
        }
        _ => false,
    };
    let builder = payment.transition(if actor.is_some() {
        CHARGE_BACK
    } else {
        CHARGE_BACK_BY_SYSTEM
    });
    let builder = match actor {
        Some(user) => builder.by(user),
        None => builder.by_system(),
    };
    let _outcome = builder.input(&given).run(state).await?;
    Payment::find_or_404(&state.db, payment.id)
        .await
        .map_err(TransitionError::Failed)
}

/// The gateway reports the dispute `dispute` on the online payment
/// `gateway_txn` (`DisputeOpened`): the payment is `charged_back`. A dispute
/// already recorded (a replayed webhook), a payment that is not `received` or
/// a transaction nobody knows changes nothing and answers `None`.
///
/// # Errors
///
/// The refusals of the machine, e.g. a reference already recorded on another
/// payment, or a payment that is not a gateway payment.
pub async fn gateway_dispute_opened(
    state: &AppState,
    gateway_txn: &str,
    dispute: &DisputeOpened,
) -> std::result::Result<Option<Payment>, TransitionError> {
    let Some(payment) = crate::online::by_transaction(&state.db, gateway_txn).await? else {
        return Ok(None);
    };
    if payment.dispute_reference.as_deref() == Some(dispute.reference.trim())
        || payment.status != PaymentStatus::Received
    {
        return Ok(None);
    }
    let given = ChargebackInput {
        reference: Some(dispute.reference.clone()),
        evidence_due: Some(
            dispute
                .evidence_due
                .unwrap_or_else(|| today() + TimeDelta::days(DEFAULT_DEADLINE_DAYS)),
        ),
        fee: dispute.fee,
        ..Default::default()
    };
    run(state, None, &payment, given).await.map(Some)
}

/// `user` records a chargeback of `payment` by hand, from the acquirer's
/// notice (N-Genius has no disputes API): its `reference`, the evidence
/// `deadline` and the `fee` in fils, if known.
///
/// # Errors
///
/// `Forbidden` without `payments.chargeback`; `NotFrom` unless the payment is
/// `received`; `Guard` (422) without a reference or a deadline, for a
/// reference already recorded, for a fee of 0 or less and for a payment that
/// did not go through the gateway.
pub async fn charge_back_by_hand(
    state: &AppState,
    user: &AuthUser,
    payment: &Payment,
    reference: &str,
    deadline: Option<NaiveDate>,
    fee: Option<i64>,
) -> std::result::Result<Payment, TransitionError> {
    if !user.has_permission(permissions::CHARGEBACK) {
        return Err(TransitionError::Forbidden);
    }
    let given = ChargebackInput {
        reference: Some(reference.to_owned()),
        evidence_due: deadline,
        fee,
        ..Default::default()
    };
    run(state, Some(user), payment, given).await
}

/// `date` as `10 Feb 2027`.
fn date_text(date: NaiveDate) -> String {
    date.format("%-d %b %Y").to_string()
}

/// The links to what the platform holds as evidence of `payment`: the
/// receipt, the invoices it paid and the record it was paid in advance for.
/// They are links only; each page keeps its own access rules.
async fn evidence_pack(
    state: &AppState,
    payment: &Payment,
    removed: &[&Allocation],
) -> Result<Value> {
    let mut links = vec![json!({
        "label": format!("Receipt {}", payment.number.clone().unwrap_or_default()),
        "href": format!("/payments/{}", payment.id),
    })];
    let invoices = invoicer(state);
    for allocation in removed {
        if let Some(AllocationTarget::Invoice(id)) = allocation.allocation_target() {
            let number = invoices
                .find(&state.db, id)
                .await?
                .and_then(|invoice| invoice.number)
                .unwrap_or_else(|| format!("#{id}"));
            links.push(json!({
                "label": format!("Invoice {number}"),
                "href": format!("/invoices/{id}"),
            }));
        }
    }
    if let Some(prepaid) = payment.prepaid() {
        links.push(
            json!({ "label": format!("Paid in advance for {prepaid}"), "href": Value::Null }),
        );
    }
    Ok(json!({
        "gateway_reference": payment.dispute_reference,
        "links": links,
    }))
}

/// After a committed chargeback of payment `id`: `PaymentReversed`, the fee,
/// the evidence task and the notices.
pub(crate) async fn after_charge_back(state: &AppState, id: i64) -> Result {
    let payment = Payment::find_or_404(&state.db, id).await?;
    let allocations = Allocation::all_of(&state.db, payment.id).await?;
    let removed: Vec<&Allocation> = allocations
        .iter()
        .filter(|a| !a.is_active() && a.removed_reason.as_deref() == Some(REMOVED_REASON))
        .collect();
    state
        .emit(PaymentReversed {
            payment: payment.payment_id(),
            party: payment.party(),
            legal_entity_id: payment.legal_entity_id,
            reason: ReversalReason::Chargeback,
            source: payment.source,
            statement_line: payment.statement_line_id,
            removed: removed
                .iter()
                .map(|allocation| RemovedAllocation {
                    allocation: allocation.id,
                    target: allocation.target(),
                    amount: allocation.amount,
                })
                .collect(),
            prepaid: payment.prepaid(),
            reversed_on: today(),
        })
        .await?;
    record_fee(state, &payment).await?;
    let Some(due) = payment.dispute_evidence_due else {
        return Ok(());
    };
    let reference = payment.dispute_reference.clone().unwrap_or_default();
    let mut text = format!(
        "Chargeback {reference} on payment {} of {}: submit the evidence in the gateway by {}",
        payment.number.clone().unwrap_or_default(),
        money_text(&payment.currency, payment.amount),
        date_text(due)
    );
    if payment.dispute_fee.is_none() {
        text.push_str(". Enter the dispute fee from the gateway's notice");
    }
    let evidence = evidence_pack(state, &payment, &removed).await?;
    let task = FollowupTask::chargeback_evidence(&payment, due, evidence, text.clone());
    if FollowupTask::create_once(&state.db, task).await?.is_some() {
        notices::accountants_told(state, &payment, &text).await?;
    }
    if let Some(taker) = payment.collected_by.or(payment.recorded_by) {
        let note = format!(
            "Payment {} of {} was charged back by the card holder (dispute {reference})",
            payment.number.clone().unwrap_or_default(),
            money_text(&payment.currency, payment.amount)
        );
        notices::online_notice(state, &payment, &[taker], &note).await?;
    }
    Ok(())
}

/// Records the gateway's dispute fee of `payment` as an outgoing payment to
/// the gateway provider (the party the module is set up with), purpose *bank
/// charges*, allocated to no document, and stores it on the dispute. Nothing
/// when the gateway gave no fee, or when no provider party is set (the fee
/// stays on the payment and the evidence task asks for it).
async fn record_fee(state: &AppState, payment: &Payment) -> Result {
    let (Some(fee), Some(provider)) = (payment.dispute_fee, config_of(state).gateway_party) else {
        return Ok(());
    };
    if payment.dispute_fee_payment_id.is_some() {
        return Ok(());
    }
    let mut request = RecordPayment::new(
        wl_kernel::PartyId(provider),
        payment.legal_entity_id,
        PaymentMethod::Gateway,
        fee,
    )
    .outgoing()
    .currency(&payment.currency)
    .source(PaymentSource::System);
    if let Some(reference) = payment.dispute_reference.as_deref() {
        request = request.reference(reference);
    }
    let paid = record::record_applied(state, None, &request)
        .await
        .map_err(TransitionError::into_error)?;
    let now = renox::db::now();
    Payment::query()
        .where_(Payment::ID.eq(paid.id))
        .update(
            &state.db,
            &[("purpose", &BANK_CHARGES.to_owned()), ("updated_at", &now)],
        )
        .await?;
    Payment::query()
        .where_(Payment::ID.eq(payment.id))
        .update(
            &state.db,
            &[
                ("dispute_fee_payment_id", &Some(paid.id)),
                ("updated_at", &now),
            ],
        )
        .await?;
    Ok(())
}

/// `user` notes that the evidence of the chargeback on `payment` was
/// submitted in the gateway: the deadline rule no longer closes it and the
/// reminder is not sent; only a gateway outcome closes the dispute.
///
/// # Errors
///
/// `Forbidden` without `payments.chargeback`; `NotFrom` unless the payment is
/// `charged_back`.
pub async fn evidence_submitted(
    state: &AppState,
    user: &AuthUser,
    payment: &Payment,
) -> std::result::Result<Payment, TransitionError> {
    if !user.has_permission(permissions::CHARGEBACK) {
        return Err(TransitionError::Forbidden);
    }
    let payment = Payment::find_or_404(&state.db, payment.id)
        .await
        .map_err(TransitionError::Failed)?;
    if payment.status != PaymentStatus::ChargedBack {
        return Err(TransitionError::NotFrom(payment.status.as_str().to_owned()));
    }
    let now = renox::db::now();
    Payment::query()
        .where_(Payment::ID.eq(payment.id))
        .update(
            &state.db,
            &[
                ("dispute_evidence_submitted_on", &Some(today())),
                ("updated_at", &now),
            ],
        )
        .await?;
    let open = FollowupTask::query()
        .where_(FollowupTask::KIND.eq(FollowupKind::ChargebackEvidence))
        .where_(FollowupTask::PAYMENT_ID.eq(payment.id))
        .where_(FollowupTask::STATUS.eq(FollowupStatus::Open))
        .get(&state.db)
        .await?;
    for mut task in open {
        task.status = FollowupStatus::Done;
        task.done_by = Some(user.id);
        task.done_at = Some(now);
        task.save(&state.db).await?;
    }
    Payment::find_or_404(&state.db, payment.id)
        .await
        .map_err(TransitionError::Failed)
}

/// The moment `day` begins its 09:00 in Dubai.
pub fn nine_in_dubai(day: NaiveDate) -> Option<DateTime> {
    let local = day.and_hms_opt(9, 0, 0)?;
    Some(local.and_utc() - TimeDelta::seconds(i64::from(DUBAI_OFFSET_SECONDS)))
}

/// The moment the reminder of a deadline on `due` is sent: K days before it,
/// at 09:00 in Dubai.
pub fn reminder_at(due: NaiveDate, days_before: i64) -> Option<DateTime> {
    nine_in_dubai(due - TimeDelta::days(days_before))
}

/// `FIN.RUL.20`: reminds the accountants, K days before the deadline at 09:00
/// in Dubai, of the open evidence task of a chargeback that is still
/// `charged_back` and whose evidence was not submitted. Once per task.
/// Returns how many reminders were sent.
///
/// # Errors
///
/// The database's errors.
pub async fn run_reminders(state: &AppState, now: DateTime) -> Result<usize> {
    let days = config_of(state).chargeback_reminder_days;
    let tasks = FollowupTask::query()
        .where_(FollowupTask::KIND.eq(FollowupKind::ChargebackEvidence))
        .where_(FollowupTask::STATUS.eq(FollowupStatus::Open))
        .where_(FollowupTask::REMINDED_ON.is_null())
        .order_by(FollowupTask::ID)
        .get(&state.db)
        .await?;
    let mut sent = 0;
    for mut task in tasks {
        let (Some(due), Some(payment_id)) = (task.due_on, task.payment_id) else {
            continue;
        };
        let Some(at) = reminder_at(due, days) else {
            continue;
        };
        if now < at {
            continue;
        }
        let Some(payment) = Payment::find(&state.db, payment_id).await? else {
            continue;
        };
        if payment.status != PaymentStatus::ChargedBack
            || payment.dispute_evidence_submitted_on.is_some()
        {
            continue;
        }
        let text = format!(
            "Reminder: the evidence for chargeback {} on payment {} is due on {}",
            payment.dispute_reference.clone().unwrap_or_default(),
            payment.number.clone().unwrap_or_default(),
            date_text(due)
        );
        notices::accountants_told(state, &payment, &text).await?;
        task.reminded_on = Some(dubai_date(now));
        task.save(&state.db).await?;
        sent += 1;
    }
    Ok(sent)
}

/// Registers the listener of the two transitions, and the hourly task and
/// command of the reminders.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: Transitioned<Payment>, state: AppState| async move {
        match event.transition.as_str() {
            CHARGE_BACK | CHARGE_BACK_BY_SYSTEM => after_charge_back(&state, event.key).await,
            _ => Ok(()),
        }
    });
    app.schedule()
        .hourly(REMINDER_TASK, |state: AppState| async move {
            run_reminders(&state, renox::db::now()).await.map(|_| ())
        });
    app.command(
        REMINDER_TASK,
        "Remind the accountant of the evidence deadline of a chargeback (FIN.RUL.20)",
        |_args, state: AppState| async move {
            let sent = run_reminders(&state, renox::db::now()).await?;
            println!("{sent} chargeback reminders sent.");
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_with(input: ChargebackInput) -> TransitionCtx {
        TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).expect("json"))
    }

    fn gateway_payment() -> Payment {
        Payment {
            direction: Direction::In,
            method: PaymentMethod::Gateway,
            status: PaymentStatus::Received,
            amount: 80_000,
            ..Default::default()
        }
    }

    fn complete() -> ChargebackInput {
        ChargebackInput {
            reference: Some("dp_1".into()),
            evidence_due: NaiveDate::from_ymd_opt(2027, 2, 10),
            fee: Some(6_000),
            reference_taken: false,
            by: None,
        }
    }

    async fn refused(payment: &Payment, input: ChargebackInput) -> Option<String> {
        let machine = Payment::machine();
        let found = machine.find(CHARGE_BACK).expect("transition");
        match found.check_guards(payment, &run_with(input)).await {
            Err(TransitionError::Guard(message)) => Some(message),
            Ok(()) => None,
            Err(other) => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn a_chargeback_leaves_received_and_the_gateway_has_its_own_twin() {
        let machine = Payment::machine();
        let user = machine.find(CHARGE_BACK).expect("transition");
        let system = machine.find(CHARGE_BACK_BY_SYSTEM).expect("transition");
        assert!(!user.is_system());
        assert!(system.is_system());
        for found in [&user, &system] {
            assert_eq!(found.from_states(), ["received"]);
            assert_eq!(found.to_state(), Some("charged_back"));
        }
        assert_eq!(user.guard_names(), system.guard_names());
    }

    #[renox::test]
    async fn a_complete_chargeback_of_a_gateway_payment_passes() {
        assert_eq!(refused(&gateway_payment(), complete()).await, None);
        // The fee is optional.
        let no_fee = ChargebackInput {
            fee: None,
            ..complete()
        };
        assert_eq!(refused(&gateway_payment(), no_fee).await, None);
    }

    #[renox::test]
    async fn the_reference_the_deadline_and_the_fee_are_checked() {
        let payment = gateway_payment();
        for bad in [
            ChargebackInput {
                reference: None,
                ..complete()
            },
            ChargebackInput {
                reference: Some("  ".into()),
                ..complete()
            },
            ChargebackInput {
                evidence_due: None,
                ..complete()
            },
            ChargebackInput {
                reference_taken: true,
                ..complete()
            },
            ChargebackInput {
                fee: Some(0),
                ..complete()
            },
        ] {
            assert!(refused(&payment, bad).await.is_some());
        }
    }

    #[renox::test]
    async fn only_money_in_through_the_gateway_is_charged_back() {
        let mut payment = gateway_payment();
        payment.method = PaymentMethod::Cash;
        let message = refused(&payment, complete()).await.expect("refused");
        assert!(message.contains("through the gateway"), "{message}");
        let mut out = gateway_payment();
        out.direction = Direction::Out;
        assert!(refused(&out, complete()).await.is_some());
    }

    #[test]
    fn the_reminder_comes_k_days_before_the_deadline_at_nine_in_dubai() {
        let due = NaiveDate::from_ymd_opt(2027, 2, 10).unwrap();
        let at = reminder_at(due, 2).expect("a moment");
        // 09:00 on 8 Feb in Dubai is 05:00 UTC.
        assert_eq!(at.to_rfc3339(), "2027-02-08T05:00:00+00:00");
        assert_eq!(dubai_date(at), NaiveDate::from_ymd_opt(2027, 2, 8).unwrap());
    }

    #[test]
    fn a_dispute_is_built_with_its_builders() {
        let dispute = DisputeOpened::new(" dp_9 ")
            .evidence_due(NaiveDate::from_ymd_opt(2027, 2, 10).unwrap())
            .fee(6_000);
        assert_eq!(dispute.reference, "dp_9");
        assert_eq!(dispute.fee, Some(6_000));
        assert!(dispute.evidence_due.is_some());
        assert!(DisputeOpened::new("x").evidence_due.is_none());
    }
}
