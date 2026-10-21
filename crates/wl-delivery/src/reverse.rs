//! A delivery marked delivered by mistake is reversed, never edited
//! (wl-delivery §4.1 `reverse_delivery`, §5.5 step 3, `WRK.ACL.06`,
//! `WRK.ACL.10`, issue #161).
//!
//! - [`request`] is the manager's: a holder of `deliveries.reverse` asks to
//!   reverse a `delivered` or `under_review` delivery with a reason (*marked
//!   by mistake*, *recipient denies*, *wrong recipient*) and a liability
//!   (*driver*, *business*, *client*, *none*). The kernel's policy
//!   `delivery.reverse_delivery` (`WRK.ACL.10`) defers it as an approval
//!   request that another holder of `deliveries.reverse` decides (never the
//!   requester, and never the driver of the run, who can neither request nor
//!   approve).
//! - [`approve`] is the second manager's. When the cash recorded as collected
//!   was already handed over (the driver's handover of the run is counted)
//!   the approver must choose *refund to payer* or *keep for next attempt*;
//!   the choice is stored like the absorber of a cash difference
//!   ([`crate::cod::ApprovalChoice`]) and read after the approval.
//! - At the final approval the kernel runs `reverse_delivery_approved`: the
//!   original proof is kept and marked reversed (why, liability, who
//!   approved), a non-counting attempt with the reason *reversed* is added,
//!   the frozen flag is cleared, the goods are traced, and the delivery is
//!   `attempted`. After the commit the target state is worked out from the
//!   run as it is then: a run still `dispatched` puts the delivery back on the
//!   road (`out_for_delivery`, the system transition `back_on_the_road`); a
//!   run that closed meanwhile leaves it `attempted`. The cash is dealt with
//!   (taken out of the driver's open custody, or refunded or kept when the
//!   handover was counted), the review's ticket is resolved with the outcome
//!   *reversed* (left open for the HR follow-up when the liability is the
//!   driver's; a delivery that was not under review gets a follow-up task),
//!   and `DeliveryReversed` is emitted. Sources react to the event: this
//!   module posts nothing of their fulfilment.
//!
//! A reversal is the whole delivery; the attempts counted so far are not
//! changed. A rejected or returned approval leaves the delivery as it was.

use renox::db::Transaction;
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{
    Outcome, StateMachine, Transition, TransitionCtx, TransitionError, Transitioned,
};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{self, ApprovalEnd, PolicyDefault, decide, submit::REQUEST_KEY};
use wl_kernel::custody::{custody, custody_provided};
use wl_kernel::delivery::DeliveryReversed;
use wl_kernel::entities::LegalEntity;
use wl_kernel::payments::{RefundRequest, RefundSource, payment_recorder};
use wl_kernel::tickets::{self, OpenTicket, TASK_CATEGORY, TaskComment, TicketId};

use crate::cod::{self, ApprovalChoice};
use crate::delivery::{Attempt, AttemptOutcome, Delivery, DeliveryStatus, Proof};
use crate::permissions;
use crate::review::source_lines;
use crate::run::{Run, RunStatus};
use crate::texts::english;
use crate::trace;

/// `reverse_delivery`: delivered or under_review → attempted, by a user.
pub const REVERSE_DELIVERY: &str = "reverse_delivery";
/// `reverse_delivery`, run by the kernel once the request is approved.
pub const REVERSE_DELIVERY_APPROVED: &str = "reverse_delivery_approved";
/// attempted → out_for_delivery, run by the system after a reversal while the
/// run is still out.
pub const BACK_ON_THE_ROAD: &str = "back_on_the_road";

/// The approval policy's action key of a reversal (`WRK.ACL.10`).
pub const ACTION: &str = "delivery.reverse_delivery";

/// The rule, as the specification names it.
pub const RULE: &str = "WRK.ACL.10";

/// Hours an approval request for a reversal may wait.
pub const EXPIRY_HOURS: i64 = 72;

/// The reasons a delivery is reversed.
pub const REASONS: [&str; 3] = ["marked_by_mistake", "recipient_denies", "wrong_recipient"];

/// Who is responsible for a reversed delivery.
pub const LIABILITIES: [&str; 4] = ["driver", "business", "client", "none"];

/// The failure reason code of the attempt a reversal adds.
pub const REVERSED_REASON: &str = "reversed";

/// What the approver does with cash that was already handed over.
pub const CASH_CHOICES: [&str; 2] = ["refund", "keep"];

/// What a reversal runs on; it is also the approval request's payload, which
/// the kernel hands to `reverse_delivery_approved`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct ReverseInput {
    /// The reason's code.
    #[serde(default)]
    pub reason_code: String,
    /// The reason in words, with the requester's note: the run's reason.
    #[serde(default)]
    pub reason: String,
    /// Who is responsible.
    #[serde(default)]
    pub liability: String,
    /// The driver of the delivery's run, or 0.
    #[serde(default)]
    pub driver_user_id: i64,
}

fn reverse_input(ctx: &TransitionCtx) -> Option<ReverseInput> {
    ctx.input_as::<ReverseInput>().ok()
}

/// The guard "reason and liability" of a reversal.
pub(crate) fn reason_and_liability(
    input: Option<&ReverseInput>,
) -> std::result::Result<(), String> {
    match input {
        None => Err("The reversal was not prepared by the reverse service.".to_owned()),
        Some(input) if !REASONS.contains(&input.reason_code.as_str()) => Err(english(
            "wl_delivery.reverse.choose_reason",
        )
        .to_owned()),
        Some(input) if !LIABILITIES.contains(&input.liability.as_str()) => Err(english(
            "wl_delivery.reverse.choose_liability",
        )
        .to_owned()),
        Some(_) => Ok(()),
    }
}

/// What both ways to reverse share: the states, the reason and liability
/// rules, and the effect on the proof, the attempts and the frozen flag.
fn reversing(transition: Transition<Delivery>) -> Transition<Delivery> {
    transition
        .from([DeliveryStatus::Delivered, DeliveryStatus::UnderReview])
        .to(DeliveryStatus::Attempted)
        .guard("reason_and_liability", |_delivery: &Delivery, ctx| {
            let checked = reason_and_liability(reverse_input(ctx).as_ref());
            Box::pin(async move { checked })
        })
        .effect(|delivery: &Delivery, ctx, tx| {
            let id = delivery.id;
            let run_id = delivery.run_id;
            let actor = ctx.actor().map(|user| user.id);
            let approver = ctx.meta().get("actor_id").and_then(Value::as_i64);
            let input = reverse_input(ctx);
            Box::pin(async move {
                let input = input.ok_or_else(|| {
                    Error::from(renox::anyhow::anyhow!("a reversal without its input"))
                })?;
                // The driver of the run cannot approve, even with the
                // permission (`WRK.ACL.10`): the decision is rolled back.
                if input.driver_user_id != 0 && approver == Some(input.driver_user_id) {
                    return Err(Error::Forbidden);
                }
                save(tx, id, run_id, approver.or(actor), &input).await
            })
        })
}

/// The effect of a reversal: the proof is marked reversed, a non-counting
/// attempt is added, the frozen flag is cleared, the goods are traced.
async fn save(
    tx: &mut Transaction,
    delivery_id: i64,
    run_id: Option<i64>,
    who: Option<i64>,
    input: &ReverseInput,
) -> Result<()> {
    let now = renox::db::now();
    let original = Proof::query()
        .where_(Proof::DELIVERY_ID.eq(delivery_id))
        .where_(Proof::REVERSED.eq(false))
        .order_by_desc(Proof::ID)
        .first(&mut *tx)
        .await?;
    if let Some(original) = original {
        let reversed = true;
        let at = Some(now);
        let reason = Some(input.reason.clone());
        let liability = Some(input.liability.clone());
        Proof::query()
            .where_(Proof::ID.eq(original.id))
            .update(
                &mut *tx,
                &[
                    ("reversed", &reversed),
                    ("reversed_at", &at),
                    ("reversal_reason", &reason),
                    ("reversal_liability", &liability),
                    ("reversed_by", &who),
                ],
            )
            .await?;
    }
    let made = Attempt::query()
        .where_(Attempt::DELIVERY_ID.eq(delivery_id))
        .count(&mut *tx)
        .await?;
    // The attempt does not count against the maximum.
    let mut attempt = Attempt {
        delivery_id,
        run_id,
        number: i64::try_from(made).unwrap_or(i64::MAX) + 1,
        outcome: AttemptOutcome::Failed,
        failure_reason: Some(REVERSED_REASON.to_owned()),
        counts: false,
        occurred_at: now,
        actor_user_id: who,
        ..Attempt::default()
    };
    attempt.insert(&mut *tx).await?;
    let frozen = false;
    Delivery::query()
        .where_(Delivery::ID.eq(delivery_id))
        .update(&mut *tx, &[("frozen", &frozen)])
        .await?;
    trace::insert_in(&mut *tx, delivery_id, trace::REVERSAL, "", "", now, who).await
}

/// The reversal transitions and `back_on_the_road`, for the machine.
pub(crate) fn transitions() -> Vec<Transition<Delivery>> {
    vec![
        reversing(
            Transition::new(REVERSE_DELIVERY)
                .label("Reverse delivery")
                .tone("danger")
                .permission(permissions::REVERSE)
                .reason_required(),
        )
        .guard("not_the_driver", |_delivery: &Delivery, ctx| {
            let driver = reverse_input(ctx).map_or(0, |i| i.driver_user_id);
            let is_driver = matches!(ctx.actor(), Some(user) if driver != 0 && user.id == driver);
            Box::pin(async move {
                if is_driver {
                    Err(english("wl_delivery.reverse.not_the_driver").to_owned())
                } else {
                    Ok(())
                }
            })
        }),
        reversing(Transition::new(REVERSE_DELIVERY_APPROVED).label("Reverse delivery (approved)").system()),
        Transition::new(BACK_ON_THE_ROAD)
            .label("Back on the road")
            .tone("info")
            .from([DeliveryStatus::Attempted])
            .to(DeliveryStatus::OutForDelivery)
            .system(),
    ]
}

fn unprocessable(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// The reason in words: the label of the code and the requester's note.
fn reason_text(code: &str, note: &str) -> String {
    let label = english(&format!("wl_delivery.reverse.reason.{code}")).to_owned();
    let note = note.trim();
    if note.is_empty() {
        label
    } else {
        format!("{label}: {note}")
    }
}

/// The driver of the run `delivery` is on, or 0.
async fn driver_of(db: &Db, delivery: &Delivery) -> Result<i64> {
    match delivery.run_id {
        Some(id) => Ok(Run::find(db, id).await?.map_or(0, |run| run.driver_user_id)),
        None => Ok(0),
    }
}

/// What a request for a reversal came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReverseRequest {
    /// No approval was needed (no policy is in force): the delivery is
    /// reversed.
    Reversed,
    /// An approval request (its id) waits for a second manager.
    Requested(i64),
}

/// A holder of `deliveries.reverse` asks to reverse `delivery` for `reason`
/// (one of [`REASONS`]) with an optional `note`, and `liability` (one of
/// [`LIABILITIES`]). The kernel's policy defers it as an approval request
/// that another holder of `deliveries.reverse` decides; the request is
/// announced after the commit.
///
/// # Errors
///
/// `Forbidden` without `deliveries.reverse`; `NotFrom` for a delivery that is
/// not `delivered` or `under_review`; `Guard` (422) with an unknown reason or
/// liability, for the run's driver, or when a request for the delivery is
/// already pending.
pub async fn request(
    state: &AppState,
    user: &AuthUser,
    delivery: &Delivery,
    reason: &str,
    liability: &str,
    note: &str,
) -> std::result::Result<ReverseRequest, TransitionError> {
    let text = reason_text(reason.trim(), note);
    let input = ReverseInput {
        reason_code: reason.trim().to_owned(),
        reason: text.clone(),
        liability: liability.trim().to_owned(),
        driver_user_id: driver_of(&state.db, delivery)
            .await
            .map_err(TransitionError::Failed)?,
    };
    let outcome = delivery
        .transition(REVERSE_DELIVERY)
        .by(user)
        .reason(&text)
        .input(&input)
        .run(state)
        .await?;
    wl_kernel::approvals::announce(state, &outcome)
        .await
        .map_err(TransitionError::Failed)?;
    Ok(match &outcome {
        Outcome::Pending(pending) => match pending.data.get(REQUEST_KEY).and_then(Value::as_i64) {
            Some(id) => ReverseRequest::Requested(id),
            None => ReverseRequest::Reversed,
        },
        _ => ReverseRequest::Reversed,
    })
}

/// Whether the cash recorded as collected on `delivery` was already handed
/// over: the driver's handover of the run is counted (or short) and the
/// delivery's proof has cash. Without a cash custody nothing is held.
///
/// # Errors
///
/// Database errors.
pub async fn cash_handed_over(state: &AppState, delivery: &Delivery) -> Result<bool> {
    if !custody_provided(state) {
        return Ok(false);
    }
    let Some(run_id) = delivery.run_id else {
        return Ok(false);
    };
    let Some(proof) = delivery.latest_proof(&state.db).await? else {
        return Ok(false);
    };
    if proof.cash_collected <= 0 {
        return Ok(false);
    }
    let run = Run::find_or_404(&state.db, run_id).await?;
    Ok(
        match close_handover(state, &run).await?.map(|summary| summary.status) {
            Some(status) => status != wl_kernel::custody::HandoverStatus::Open,
            None => false,
        },
    )
}

async fn close_handover(
    state: &AppState,
    run: &Run,
) -> Result<Option<wl_kernel::custody::HandoverSummary>> {
    crate::close::handover_of(state, run).await
}

/// `user` (a holder of `deliveries.reverse`, neither the requester nor the
/// driver of the run) approves the pending reversal of `delivery`. When the
/// cash was already handed over, `cash_treatment` must be `refund` (back to
/// the payer) or `keep` (for the next attempt). The delivery is reversed when
/// this returns.
///
/// # Errors
///
/// 403 for anyone who may not decide the request (the requester and the run's
/// driver included); 422 when no request waits, the cash choice is missing or
/// unknown, a refund cannot be made (no payments module, no payer party), or
/// the delivery can no longer be reversed.
pub async fn approve(
    state: &AppState,
    user: &AuthUser,
    delivery: &Delivery,
    cash_treatment: Option<&str>,
    comment: &str,
) -> Result<()> {
    let request = cod::pending_request(&state.db, delivery.id, ACTION)
        .await?
        .ok_or_else(|| unprocessable(english("wl_delivery.reverse.no_request")))?;
    if driver_of(&state.db, delivery).await? == user.id {
        return Err(Error::Forbidden);
    }
    decide::ensure_approver(&state.db, &request, user).await?;
    let choice = match cash_treatment.map(str::trim).filter(|c| !c.is_empty()) {
        None => None,
        Some(choice) if CASH_CHOICES.contains(&choice) => Some(choice.to_owned()),
        Some(_) => return Err(unprocessable(english("wl_delivery.reverse.choose_cash"))),
    };
    if cash_handed_over(state, delivery).await? {
        match choice.as_deref() {
            None => return Err(unprocessable(english("wl_delivery.reverse.choose_cash"))),
            Some("refund") => ensure_refundable(state, delivery)?,
            Some(_) => {}
        }
    }
    ApprovalChoice::record(&state.db, request.id, None, choice, user.id).await?;
    match approvals::approve(state, request.id, user, comment).await? {
        Some(decided) if decided.decision != ApprovalEnd::Approved => Err(unprocessable(
            english("wl_delivery.reverse.approval_failed").to_owned(),
        )),
        _ => Ok(()),
    }
}

/// 422 unless a refund to the payer can be asked: the payments module is on
/// and the delivery has a recipient party to pay back.
fn ensure_refundable(state: &AppState, delivery: &Delivery) -> Result<()> {
    let provided = state
        .provided::<std::sync::Arc<dyn wl_kernel::payments::PaymentRecorder>>()
        .is_some();
    if !provided || delivery.recipient().is_none() {
        return Err(unprocessable(english("wl_delivery.reverse.cannot_refund")));
    }
    Ok(())
}

/// `user` rejects the pending reversal of `delivery` with a `comment`
/// (required): the delivery stays as it was and its proof is unchanged.
///
/// # Errors
///
/// 403 for anyone who may not decide the request; 422 without a comment or
/// when no request waits.
pub async fn reject(
    state: &AppState,
    user: &AuthUser,
    delivery: &Delivery,
    comment: &str,
) -> Result<()> {
    let request = cod::pending_request(&state.db, delivery.id, ACTION)
        .await?
        .ok_or_else(|| unprocessable(english("wl_delivery.reverse.no_request")))?;
    if driver_of(&state.db, delivery).await? == user.id {
        return Err(Error::Forbidden);
    }
    approvals::reject(state, request.id, user, comment).await?;
    Ok(())
}

/// What became of the cash recorded as collected on the reversed `delivery`
/// (its original proof `proof`): taken out of the driver's open custody, or,
/// once the handover was counted, refunded to the payer or kept, as the
/// approver chose (`keep` when no choice was made).
async fn deal_with_cash(
    state: &AppState,
    delivery: &Delivery,
    proof: &Proof,
    choice: Option<&str>,
    approver: Option<i64>,
) -> Result<Option<String>> {
    if proof.cash_collected <= 0 || !custody_provided(state) {
        return Ok(None);
    }
    match custody(state)
        .release(state, delivery.delivery_id().subject())
        .await
    {
        Ok(true) => Ok(Some("removed".to_owned())),
        Ok(false) => Ok(None),
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _)) => {
            if choice == Some("refund") {
                refund(state, delivery, proof, approver).await?;
                Ok(Some("refunded".to_owned()))
            } else {
                Ok(Some("kept".to_owned()))
            }
        }
        Err(error) => Err(error),
    }
}

/// Asks `wl-payments` to pay the cash of the reversed delivery back to the
/// recipient (the payer); the refund waits for its own approval
/// (`FIN.ACL.04`).
async fn refund(
    state: &AppState,
    delivery: &Delivery,
    proof: &Proof,
    approver: Option<i64>,
) -> Result<()> {
    let party = delivery
        .recipient()
        .ok_or_else(|| unprocessable(english("wl_delivery.reverse.cannot_refund")))?;
    let entity = LegalEntity::default_entity(&state.db).await?;
    payment_recorder(state)
        .request_refund(
            state,
            RefundRequest {
                source: RefundSource::Order(delivery.delivery_id().subject()),
                party,
                legal_entity_id: entity.id,
                amount: proof.cash_collected,
                currency: "AED".to_owned(),
                method: None,
                gateway_txn: None,
                reason: english("wl_delivery.reverse.refund_reason")
                    .replace("{number}", &delivery.number),
            },
            approver,
        )
        .await?;
    Ok(())
}

/// After the commit of a reversal: puts the delivery back on the road when
/// its run is still out, deals with the cash, the review's ticket and the
/// driver's liability, and emits `DeliveryReversed`.
async fn after_reversed(state: &AppState, event: &Transitioned<Delivery>) -> Result {
    if ![REVERSE_DELIVERY, REVERSE_DELIVERY_APPROVED].contains(&event.transition.as_str()) {
        return Ok(());
    }
    let mut delivery = Delivery::find_or_404(&state.db, event.key).await?;
    let proof = Proof::query()
        .where_(Proof::DELIVERY_ID.eq(delivery.id))
        .where_(Proof::REVERSED.eq(true))
        .order_by_desc(Proof::ID)
        .first(&state.db)
        .await?;
    let request_id = event.meta.get("approval_request").and_then(Value::as_i64);
    let approver = event
        .meta
        .get("actor_id")
        .and_then(Value::as_i64)
        .or(event.actor_id);
    let choice = match request_id {
        Some(id) => ApprovalChoice::query()
            .where_(ApprovalChoice::REQUEST_ID.eq(id))
            .first(&state.db)
            .await?
            .and_then(|choice| choice.cash_treatment),
        None => None,
    };
    let liability = proof
        .as_ref()
        .and_then(|p| p.reversal_liability.clone())
        .unwrap_or_else(|| "none".to_owned());
    let reason = proof
        .as_ref()
        .and_then(|p| p.reversal_reason.clone())
        .unwrap_or_default();

    // The target state is worked out now, from the run as it is.
    let run = match delivery.run_id {
        Some(id) => Run::find(&state.db, id).await?,
        None => None,
    };
    if run.is_some_and(|run| run.status == RunStatus::Dispatched) {
        let _outcome = delivery
            .transition(BACK_ON_THE_ROAD)
            .by_system()
            .run(state)
            .await
            .map_err(TransitionError::into_error)?;
        delivery = Delivery::find_or_404(&state.db, event.key).await?;
    }

    let cash_treatment = match &proof {
        Some(proof) => deal_with_cash(state, &delivery, proof, choice.as_deref(), approver).await?,
        None => None,
    };

    let was_under_review = event.from == DeliveryStatus::UnderReview.as_str();
    let driver_liable = liability == "driver";
    if was_under_review && let Some(ticket) = delivery.review_ticket_id.map(TicketId) {
        if driver_liable {
            // Left open for the HR follow-up.
            tickets::task_creator(state)
                .comment(
                    state,
                    ticket,
                    TaskComment::internal(english("wl_delivery.reverse.follow_up_comment")),
                )
                .await?;
        } else {
            tickets::task_creator(state)
                .resolve_from_source(
                    state,
                    ticket,
                    &english("wl_delivery.reverse.resolved").replace("{reason}", &reason),
                )
                .await?;
        }
    } else if driver_liable {
        let title = english("wl_delivery.reverse.follow_up_title").replace("{number}", &delivery.number);
        let description = english("wl_delivery.reverse.follow_up_description")
            .replace("{reason}", &reason)
            .replace("{cash}", &crate::proof::aed(proof.as_ref().map_or(0, |p| p.cash_collected)))
            .replace("{treatment}", cash_treatment.as_deref().unwrap_or("none"));
        let ticket = OpenTicket::new(delivery.delivery_id().subject(), TASK_CATEGORY, &title)
            .description(&description)
            .source(delivery.source())
            .source_key(&format!(
                "delivery.reversed:{}:{}",
                delivery.id,
                proof.as_ref().map_or(0, |p| p.id)
            ));
        tickets::task_creator(state).open(state, ticket).await?;
    }

    state
        .emit(DeliveryReversed {
            delivery: delivery.delivery_id(),
            source: delivery.source(),
            reason,
            liability,
            proof: proof.as_ref().map(|p| p.id),
            cash_treatment,
            approver,
            lines: source_lines(&state.db, &delivery).await?,
        })
        .await
}

/// Registers the approval policy and the listener that finishes a reversal.
pub(crate) fn register(app: &mut Registry) {
    // `WRK.ACL.10`: another holder of `deliveries.reverse` approves, never the
    // requester.
    PolicyDefault::new(ACTION)
        .step_permission(permissions::REVERSE)
        .expiry_hours(Some(EXPIRY_HOURS))
        .register();
    app.listen(|event: Transitioned<Delivery>, state: AppState| async move {
        after_reversed(&state, &event).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(reason: &str, liability: &str) -> ReverseInput {
        ReverseInput {
            reason_code: reason.into(),
            reason: reason.into(),
            liability: liability.into(),
            driver_user_id: 3,
        }
    }

    #[test]
    fn a_reversal_needs_a_known_reason_and_liability() {
        assert!(reason_and_liability(None).is_err());
        assert!(reason_and_liability(Some(&input("marked_by_mistake", "driver"))).is_ok());
        assert!(reason_and_liability(Some(&input("because", "driver"))).is_err());
        assert!(reason_and_liability(Some(&input("wrong_recipient", "carrier"))).is_err());
    }

    #[test]
    fn the_reason_reads_as_words_with_the_note() {
        assert_eq!(
            reason_text("recipient_denies", ""),
            "Recipient denies the delivery"
        );
        assert_eq!(
            reason_text("recipient_denies", " Mr Ali was abroad "),
            "Recipient denies the delivery: Mr Ali was abroad"
        );
    }
}
