//! Recording a payment (wl-payments §4 `record`).
//!
//! A payment is inserted as `pending` inside the record transaction and
//! `record` moves it to `received`, so the machine's guards, its history row
//! and its effect all run in that one transaction, and a refusal leaves no
//! row and uses no number. The machine has no app state, so [`record`]
//! gathers what the guards need into a [`RecordInput`]: whether the method
//! is enabled, whether the legal entity is active and whether the party is
//! found (through the kernel's [`PartyDirectory`](wl_kernel::PartyDirectory)).
//! A run without that input (a bare POST) is refused by the guards.
//!
//! The guards: the amount is above 0; the method is enabled; the method
//! details are complete (a transfer and a card need a reference, a cheque
//! needs its number, bank and date, cash needs nothing); the entity is
//! active; the party is found. The effect, in the transition's transaction:
//! the receipt number (incoming) or the voucher number (outgoing) from the
//! kernel's series (gapless, per entity, taken only here, so a rolled-back
//! record gives it back). The payment date is the given one, else today in
//! Asia/Dubai. `PaymentReceived` is emitted after the commit
//! ([`crate::events`]).
//!
//! `record_by_system` is the same transition for a record with no user (the
//! payment of a refund, a bank statement match): only the system may run
//! it.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::{Outcome, StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::LegalEntity;
use wl_kernel::documents::today;
use wl_kernel::drawings::PURPOSE;
use wl_kernel::payments::{
    Direction, PaymentMethod, PaymentStatus, RecordPayment, payment_voucher_series, receipt_series,
};

use crate::payment::{Payment, details_json};
use crate::permissions;

/// The `record` transition's name.
pub const RECORD: &str = "record";

/// The system's `record_by_system` transition's name.
pub const RECORD_BY_SYSTEM: &str = "record_by_system";

/// What a record run carries to its guards, gathered by [`record`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct RecordInput {
    /// Whether the payment method is enabled in the module's settings.
    pub method_enabled: bool,
    /// Whether the legal entity is found and active.
    pub entity_active: bool,
    /// Whether the party is found.
    pub party_found: bool,
    /// Whether an outgoing owner drawing was approved (`FIN.ACL.21`): set
    /// only by [`crate::drawings`], never by a caller.
    #[serde(default)]
    pub drawing_approved: bool,
}

/// The run's input, if it is a [`RecordInput`].
fn input(ctx: &TransitionCtx) -> Option<RecordInput> {
    ctx.input_as::<RecordInput>().ok()
}

/// Why the details of `payment` are incomplete for its method, if they are.
pub(crate) fn details_problem(payment: &Payment) -> Option<String> {
    match payment.method {
        PaymentMethod::Transfer | PaymentMethod::Card => {
            if payment.effective_reference().is_none() {
                return Some(format!(
                    "A {} payment needs a reference.",
                    payment.method.as_str()
                ));
            }
            None
        }
        PaymentMethod::Cheque => {
            let missing: Vec<&str> = ["number", "bank", "date"]
                .into_iter()
                .filter(|key| payment.detail(key).is_none())
                .collect();
            if !missing.is_empty() {
                return Some(format!(
                    "A cheque needs its number, bank and date (missing: {}).",
                    missing.join(", ")
                ));
            }
            let date = payment.detail("date").unwrap_or_default();
            if NaiveDate::parse_from_str(&date, "%Y-%m-%d").is_err() {
                return Some("The cheque date must be a date like 2026-11-03.".to_owned());
            }
            None
        }
        _ => None,
    }
}

/// `transition` with the guards and the effect both record transitions
/// share.
fn with_rules(transition: Transition<Payment>) -> Transition<Payment> {
    transition
        .from([PaymentStatus::Pending])
        .to(PaymentStatus::Received)
        .guard("amount_positive", |payment: &Payment, _ctx| {
            Box::pin(async move {
                if payment.amount > 0 {
                    Ok(())
                } else {
                    Err("The amount must be above 0.".to_owned())
                }
            })
        })
        .guard("method_enabled", |payment: &Payment, ctx| {
            Box::pin(async move {
                match input(ctx) {
                    Some(input) if input.method_enabled => Ok(()),
                    _ => Err(format!(
                        "The payment method {} is not enabled.",
                        payment.method.as_str()
                    )),
                }
            })
        })
        .guard("details_complete", |payment: &Payment, _ctx| {
            Box::pin(async move { details_problem(payment).map_or(Ok(()), Err) })
        })
        .guard("entity_active", |_payment: &Payment, ctx| {
            Box::pin(async move {
                match input(ctx) {
                    Some(input) if input.entity_active => Ok(()),
                    _ => Err("The legal entity is not active.".to_owned()),
                }
            })
        })
        .guard("party_found", |_payment: &Payment, ctx| {
            Box::pin(async move {
                match input(ctx) {
                    Some(input) if input.party_found => Ok(()),
                    _ => Err("The party was not found.".to_owned()),
                }
            })
        })
        .guard("drawing_approved", |payment: &Payment, ctx| {
            Box::pin(async move {
                // `FIN.ACL.21`: only the final approval records a drawing out;
                // a repayment by the owner (money in) needs none.
                if payment.purpose != PURPOSE || payment.direction != Direction::Out {
                    return Ok(());
                }
                match input(ctx) {
                    Some(input) if input.drawing_approved => Ok(()),
                    _ => Err("An owner drawing is recorded only after its approval.".to_owned()),
                }
            })
        })
        .effect(|payment: &Payment, _ctx, tx| {
            let payment = payment.clone();
            Box::pin(async move {
                let entity = LegalEntity::find_or_404(&mut *tx, payment.legal_entity_id).await?;
                let active = LegalEntity::query()
                    .where_(LegalEntity::STATUS.eq("active"))
                    .count(&mut *tx)
                    .await?;
                let active = usize::try_from(active).unwrap_or(usize::MAX);
                let series = match payment.direction {
                    Direction::In => receipt_series(active),
                    Direction::Out => payment_voucher_series(active),
                };
                // Inside the record transaction only: a rollback gives the
                // number back, so the series stays gapless.
                let number = series.next(&mut *tx, None, Some(&entity), today()).await?;
                // Not the status column, so outside the machine's guard.
                Payment::query()
                    .where_(Payment::ID.eq(payment.id))
                    .update(&mut *tx, &[("number", &number)])
                    .await?;
                Ok(())
            })
        })
}

/// `record` (a user with `payments.record`) and `record_by_system`, for the
/// machine.
pub(crate) fn transitions() -> Vec<Transition<Payment>> {
    vec![
        with_rules(
            Transition::new(RECORD)
                .label("Record")
                .tone("primary")
                .permission(permissions::RECORD),
        ),
        with_rules(
            Transition::new(RECORD_BY_SYSTEM)
                .label("Record")
                .tone("primary")
                .system(),
        ),
    ]
}

/// The unsaved `pending` payment for `request`, recorded by `recorded_by`.
fn new_payment(request: &RecordPayment, recorded_by: Option<i64>) -> Payment {
    Payment {
        direction: request.direction,
        legal_entity_id: request.legal_entity_id,
        bank_account: request
            .bank_account
            .as_deref()
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .map(str::to_owned),
        party_id: request.party.0,
        method: request.method,
        method_details: details_json(request.method_details.clone()),
        amount: request.amount,
        currency: request.currency.trim().to_ascii_uppercase(),
        payment_date: request.date.unwrap_or_else(today),
        reference: request
            .reference
            .as_deref()
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .map(str::to_owned),
        source: request.source,
        statement_line_id: request.statement_line_id,
        collected_by: request.collected_by,
        prepaid_type: request.prepaid.as_ref().map(|p| p.subject_type.clone()),
        prepaid_id: request.prepaid.as_ref().map(|p| p.subject_id),
        purpose: "none".to_owned(),
        recorded_by,
        status: PaymentStatus::Pending,
        ..Default::default()
    }
}

/// Whether `currency` is an ISO code like `AED`.
fn is_currency(currency: &str) -> bool {
    currency.len() == 3 && currency.bytes().all(|b| b.is_ascii_uppercase())
}

/// What the guards of recording `payment` need.
async fn gather(state: &AppState, payment: &Payment) -> Result<RecordInput> {
    let entity = LegalEntity::find(&state.db, payment.legal_entity_id).await?;
    let party = wl_kernel::parties::directory(state)
        .find(&state.db, payment.party())
        .await?;
    Ok(RecordInput {
        method_enabled: crate::config_of(state).method_enabled(payment.method.as_str()),
        entity_active: entity.as_ref().is_some_and(LegalEntity::is_active),
        party_found: party.is_some(),
        drawing_approved: false,
    })
}

/// Records `request` as a `received` payment with its receipt or voucher
/// number, in one transaction, by `actor` (`None` = the system), and returns
/// it. `PaymentReceived` follows after the commit.
///
/// # Errors
///
/// `Forbidden` for a user without `payments.record`. `Guard` (422) for an amount
/// of 0 or less, a disabled method, incomplete method details, an inactive
/// legal entity or an unknown party, and for a currency that is not an ISO
/// code; nothing is written then and no number is used.
pub async fn record(
    state: &AppState,
    actor: Option<&AuthUser>,
    request: &RecordPayment,
) -> std::result::Result<Payment, TransitionError> {
    run_record(state, actor, actor.map(|user| user.id), request).await
}

/// [`record`] for another module's call through the kernel's
/// `PaymentRecorder`: there is only a user id (`None` = the system), and no
/// `AuthUser` to run the user transition with. It runs `record_by_system`
/// with `recorded_by` set to `by`; the calling module has authorized its
/// user already.
pub(crate) async fn record_for(
    state: &AppState,
    by: Option<i64>,
    request: &RecordPayment,
) -> std::result::Result<Payment, TransitionError> {
    run_record(state, None, by, request).await
}

async fn run_record(
    state: &AppState,
    actor: Option<&AuthUser>,
    recorded_by: Option<i64>,
    request: &RecordPayment,
) -> std::result::Result<Payment, TransitionError> {
    let (payment, input) = prepare(state, request, recorded_by).await?;
    let mut tx = state.db.begin().await?;
    let (payment, outcome) = record_in(&mut tx, actor, payment, &input).await?;
    tx.commit().await?;
    // `Transitioned<Payment>` -> `PaymentReceived` (crate::events).
    outcome.emit(state).await.map_err(TransitionError::Failed)?;
    Payment::find_or_404(&state.db, payment.id)
        .await
        .map_err(TransitionError::Failed)
}

/// The unsaved `pending` payment for `request` and what its guards need,
/// read before any transaction opens.
///
/// # Errors
///
/// `Guard` for a currency that is not an ISO code.
pub(crate) async fn prepare(
    state: &AppState,
    request: &RecordPayment,
    recorded_by: Option<i64>,
) -> std::result::Result<(Payment, RecordInput), TransitionError> {
    let payment = new_payment(request, recorded_by);
    if !is_currency(&payment.currency) {
        return Err(TransitionError::Guard(format!(
            "`{}` is not a currency code like AED.",
            request.currency
        )));
    }
    let input = gather(state, &payment).await?;
    Ok((payment, input))
}

/// Inserts `payment` and runs `record` (a user) or `record_by_system` in
/// `tx`, which the caller commits before it emits the outcome.
pub(crate) async fn record_in(
    tx: &mut renox::db::Transaction,
    actor: Option<&AuthUser>,
    mut payment: Payment,
    input: &RecordInput,
) -> std::result::Result<(Payment, Outcome<Payment>), TransitionError> {
    payment.insert(&mut *tx).await?;
    let builder = payment.transition(if actor.is_some() {
        RECORD
    } else {
        RECORD_BY_SYSTEM
    });
    let builder = match actor {
        Some(user) => builder.by(user),
        None => builder.by_system(),
    };
    let outcome = builder.input(input).run_in(&mut *tx).await?;
    Ok((payment, outcome))
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::serde_json::json;
    use wl_kernel::PartyId;

    fn ready() -> RecordInput {
        RecordInput {
            method_enabled: true,
            entity_active: true,
            party_found: true,
            drawing_approved: false,
        }
    }

    fn run(input: &RecordInput) -> TransitionCtx {
        TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).expect("json"))
    }

    fn request(method: PaymentMethod) -> RecordPayment {
        RecordPayment::new(PartyId(7), 2, method, 250_000)
    }

    fn payment_of(request: &RecordPayment) -> Payment {
        new_payment(request, Some(3))
    }

    async fn refused(payment: &Payment, input: &RecordInput) -> Option<String> {
        let machine = Payment::machine();
        let record = machine.find(RECORD).expect("transition");
        match record.check_guards(payment, &run(input)).await {
            Err(TransitionError::Guard(message)) => Some(message),
            Ok(()) => None,
            Err(other) => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn record_goes_from_pending_to_received() {
        let machine = Payment::machine();
        let record = machine.find(RECORD).expect("transition");
        assert_eq!(record.from_states(), ["pending"]);
        assert_eq!(record.to_state(), Some("received"));
        assert!(!record.is_system());
        assert_eq!(
            record.guard_names(),
            [
                "amount_positive",
                "method_enabled",
                "details_complete",
                "entity_active",
                "party_found",
                "drawing_approved"
            ]
        );
        let system = machine.find(RECORD_BY_SYSTEM).expect("transition");
        assert!(system.is_system());
        assert_eq!(system.guard_names(), record.guard_names());
    }

    #[renox::test]
    async fn cash_and_a_transfer_with_a_reference_pass() {
        let cash = payment_of(&request(PaymentMethod::Cash));
        assert_eq!(refused(&cash, &ready()).await, None);
        let transfer = payment_of(&request(PaymentMethod::Transfer).reference("TRX-1"));
        assert_eq!(refused(&transfer, &ready()).await, None);
        let in_details = payment_of(
            &request(PaymentMethod::Card).method_details(json!({"reference": "SLIP-9"})),
        );
        assert_eq!(refused(&in_details, &ready()).await, None);
    }

    #[renox::test]
    async fn a_transfer_or_card_without_a_reference_is_refused() {
        for method in [PaymentMethod::Transfer, PaymentMethod::Card] {
            let payment = payment_of(&request(method).reference("  "));
            let message = refused(&payment, &ready()).await.expect("refused");
            assert!(message.contains("needs a reference"), "{message}");
        }
    }

    #[renox::test]
    async fn a_cheque_needs_number_bank_and_date() {
        let complete = json!({"number": "1042", "bank": "ENBD", "date": "2026-11-03"});
        let cheque = payment_of(&request(PaymentMethod::Cheque).method_details(complete));
        assert_eq!(refused(&cheque, &ready()).await, None);

        let no_bank = json!({"number": "1042", "date": "2026-11-03"});
        let payment = payment_of(&request(PaymentMethod::Cheque).method_details(no_bank));
        let message = refused(&payment, &ready()).await.expect("refused");
        assert!(message.contains("missing: bank)"), "{message}");

        let bad_date = json!({"number": "1042", "bank": "ENBD", "date": "soon"});
        let payment = payment_of(&request(PaymentMethod::Cheque).method_details(bad_date));
        assert!(refused(&payment, &ready()).await.is_some());

        let bare = payment_of(&request(PaymentMethod::Cheque));
        assert!(refused(&bare, &ready()).await.is_some());
    }

    #[renox::test]
    async fn an_outgoing_drawing_needs_its_approval_and_a_repayment_does_not() {
        let mut drawing = payment_of(&request(PaymentMethod::Cash).outgoing());
        assert_eq!(refused(&drawing, &ready()).await, None);
        drawing.purpose = PURPOSE.to_owned();
        let message = refused(&drawing, &ready()).await.expect("refused");
        assert!(message.contains("only after its approval"), "{message}");
        let approved = RecordInput {
            drawing_approved: true,
            ..ready()
        };
        assert_eq!(refused(&drawing, &approved).await, None);
        // Money in with the purpose: the owner pays back.
        let mut repayment = payment_of(&request(PaymentMethod::Cash));
        repayment.purpose = PURPOSE.to_owned();
        assert_eq!(refused(&repayment, &ready()).await, None);
    }

    #[renox::test]
    async fn the_amount_method_entity_and_party_are_checked() {
        let mut payment = payment_of(&request(PaymentMethod::Cash));
        payment.amount = 0;
        assert!(refused(&payment, &ready()).await.is_some());
        payment.amount = -5;
        assert!(refused(&payment, &ready()).await.is_some());
        payment.amount = 100;
        for input in [
            RecordInput {
                method_enabled: false,
                ..ready()
            },
            RecordInput {
                entity_active: false,
                ..ready()
            },
            RecordInput {
                party_found: false,
                ..ready()
            },
        ] {
            assert!(refused(&payment, &input).await.is_some());
        }
        // A bare run, without the gathered input.
        let machine = Payment::machine();
        let record = machine.find(RECORD).expect("transition");
        assert!(matches!(
            record
                .check_guards(&payment, &TransitionCtx::new(None))
                .await,
            Err(TransitionError::Guard(_))
        ));
    }

    #[test]
    fn the_new_payment_is_pending_with_the_requests_data() {
        let date = NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();
        let request = request(PaymentMethod::Transfer)
            .currency("aed")
            .date(date)
            .reference(" TRX-1 ")
            .collected_by(9)
            .prepaid(wl_kernel::Subject::new("property.deposit", 4));
        let payment = new_payment(&request, Some(3));
        assert_eq!(payment.status, PaymentStatus::Pending);
        assert_eq!(payment.number, None);
        assert_eq!(payment.currency, "AED");
        assert_eq!(payment.payment_date, date);
        assert_eq!(payment.reference.as_deref(), Some("TRX-1"));
        assert_eq!((payment.party_id, payment.legal_entity_id), (7, 2));
        assert_eq!(payment.recorded_by, Some(3));
        assert_eq!(payment.collected_by, Some(9));
        assert_eq!(payment.prepaid_type.as_deref(), Some("property.deposit"));
        assert_eq!(payment.prepaid_id, Some(4));
        assert_eq!(payment.purpose, "none");
        let undated = new_payment(&self::request(PaymentMethod::Cash), None);
        assert_eq!(undated.payment_date, today());
    }

    #[test]
    fn currencies_are_iso_codes() {
        assert!(is_currency("AED"));
        assert!(!is_currency("aed"));
        assert!(!is_currency("DIRHAM"));
    }
}
