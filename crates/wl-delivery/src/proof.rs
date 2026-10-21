//! The stop outcome and its proof (wl-delivery §4.1 `deliver`, `fail`; §5.2;
//! `WRK.POD.01`, issue #158).
//!
//! The driver records one outcome per visit:
//!
//! - [`deliver`] hands the delivery over: the required proof is captured
//!   (photo, signature, OTP; time and, when the phone gave it, location), and
//!   for every line the delivered and refused quantity are entered, each
//!   refused quantity with a refusal reason (a photo when it is *damaged*).
//!   The delivery is `delivered` with those quantities; the attempt, the
//!   proof and the line quantities are saved in the transition's transaction
//!   and never edited afterwards (`WRK.ACL.06`). `DeliveryDelivered` carries
//!   the quantities per line, the cash expected and collected, and the COD
//!   difference.
//! - [`fail`] records a failed visit with a failure reason and the location;
//!   it counts against the maximum attempts when the reason says so.
//!
//! The rules of `deliver` are one pure function, [`evaluate`], over the
//! driver's entries and the facts the service reads (the lines, the proof
//! required, whether partial acceptance is on for the source type, the amount
//! to collect): the machine's guard and effect both call it, so the checks
//! and what is saved cannot differ.
//!
//! **Cash.** The expected cash is the delivery's amount to collect, or, when
//! the source gives an amount per line, the sum of the accepted quantities'
//! amounts (a part of a line pro rata to its quantity), so refused lines
//! reduce it without a difference. Cash above the expected amount is refused
//! (change is given back). Cash below it needs a difference reason; beyond
//! the tolerance it also needs an approver's agreement (`WRK.ACL.09`,
//! [`crate::cod`]): the driver's request is the transition `deliver_short`,
//! which the kernel defers as an approval request, and the delivery is
//! delivered by `deliver_short_approved` once an approver agrees. Without
//! that the driver fails the attempt with *refused to pay* or *no cash*.
//!
//! **Cash custody.** After the commit of a delivery the cash collected is
//! added to the driver's cash custody for the run (`FIN.CSH.01`, the kernel's
//! `CashCustody`): the cash actually collected, except when the driver
//! absorbs the difference, then the full cash expected (the missing part is
//! the shortage at the count).

use renox::chrono::Duration;
use renox::db::{DateTime, Transaction};
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{
    StateMachine, Transition, TransitionCtx, TransitionError, Transitioned,
};
use serde::{Deserialize, Serialize};
use wl_kernel::custody::{CashCollection, HandoverPeriod, custody};
use wl_kernel::delivery::{DeliveredLine, DeliveryAttemptFailed, DeliveryDelivered, PodMethod};

use crate::cod::{self, ApprovalChoice};
use crate::delivery::{Attempt, AttemptOutcome, Delivery, DeliveryLine, DeliveryStatus, Proof};
use crate::permissions;
use crate::reasons::{CodReason, DAMAGED, FailureReason, RefusalReason};
use crate::run::Run;
use crate::settings::settings;
use crate::texts::english;

/// `deliver`: out_for_delivery → delivered.
pub const DELIVER: &str = "deliver";
/// `fail`: out_for_delivery → attempted.
pub const FAIL: &str = "fail";
/// `deliver` with a cash difference above the tolerance: the user's request,
/// which an approval policy defers (`WRK.ACL.09`).
pub const DELIVER_SHORT: &str = "deliver_short";
/// `deliver_short`, run by the kernel once the difference is approved.
pub const DELIVER_SHORT_APPROVED: &str = "deliver_short_approved";
/// The failure reason of a forced failed attempt.
pub const REFUSED_TO_PAY: &str = "refused_to_pay";

/// What the driver enters for one line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineEntry {
    /// The delivery line.
    pub line_id: i64,
    /// The quantity accepted, in thousandths.
    pub delivered_milli: i64,
    /// The quantity refused, in thousandths.
    pub refused_milli: i64,
    /// The refusal reason's code, for a refused quantity.
    #[serde(default)]
    pub reason: Option<String>,
    /// The photo of the refused goods (needed for *damaged*).
    #[serde(default)]
    pub photo: Option<String>,
}

impl LineEntry {
    /// A line delivered in full.
    pub fn full(line: &DeliveryLine) -> Self {
        LineEntry {
            line_id: line.id,
            delivered_milli: line.quantity_milli,
            refused_milli: 0,
            reason: None,
            photo: None,
        }
    }
}

/// What the driver captures at the stop for `deliver`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DeliverForm {
    /// Who received the goods.
    pub recipient_name: String,
    /// The photo's reference.
    #[serde(default)]
    pub photo: Option<String>,
    /// The signature image's reference.
    #[serde(default)]
    pub signature: Option<String>,
    /// Whether the recipient's one-time code was verified.
    #[serde(default)]
    pub otp_verified: bool,
    /// The latitude in millionths of a degree, captured at that moment.
    #[serde(default)]
    pub latitude_micro: Option<i64>,
    /// The longitude in millionths of a degree.
    #[serde(default)]
    pub longitude_micro: Option<i64>,
    /// One entry per line.
    pub lines: Vec<LineEntry>,
    /// The cash collected, in fils.
    #[serde(default)]
    pub cash_collected: i64,
    /// The reference of a cheque collected.
    #[serde(default)]
    pub cheque: Option<String>,
    /// The driver's notes.
    #[serde(default)]
    pub notes: String,
    /// The code of the reason the cash collected is below the cash expected
    /// (`WRK.ACL.09`).
    #[serde(default)]
    pub difference_reason: Option<String>,
}

/// What the service reads of a line for the rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LineFacts {
    pub id: i64,
    pub description: String,
    pub quantity_milli: i64,
    pub amount: Option<i64>,
}

/// Everything `deliver` runs on: the driver's entries and the facts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DeliverInput {
    pub at: DateTime,
    pub form: DeliverForm,
    pub driver_user_id: i64,
    pub attempt_number: i64,
    pub facts: Vec<LineFacts>,
    pub required: Vec<PodMethod>,
    pub any_method: bool,
    pub partial_allowed: bool,
    pub amount_to_collect: i64,
    pub tolerance: i64,
    pub refusal_reasons: Vec<String>,
    /// The codes of the active difference reasons.
    #[serde(default)]
    pub cod_reasons: Vec<String>,
    /// Whether a short payment above the tolerance may be approved.
    #[serde(default)]
    pub approval_allowed: bool,
    /// The absorbers that may be chosen.
    #[serde(default)]
    pub absorbers: Vec<String>,
}

/// What the rules work out for a valid `deliver`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Evaluated {
    /// The cash expected, in fils.
    pub cash_expected: i64,
    /// The methods the driver captured.
    pub captured: Vec<PodMethod>,
    /// Expected less collected, in fils.
    pub difference: i64,
    /// The difference reason's code, when the cash is short.
    pub difference_reason: Option<String>,
    /// Whether the difference is above the tolerance and needs an approval.
    pub needs_approval: bool,
}

/// `quantity_milli` as text: `5`, `2.5`, `0.125`.
pub(crate) fn milli_text(milli: i64) -> String {
    let whole = milli / 1000;
    let part = (milli % 1000).abs();
    if part == 0 {
        return whole.to_string();
    }
    let fraction = format!("{part:03}");
    format!("{whole}.{}", fraction.trim_end_matches('0'))
}

/// `fils` as `AED 130.00`.
pub(crate) fn aed(fils: i64) -> String {
    format!("AED {}.{:02}", fils / 100, (fils % 100).abs())
}

/// `amount` of a line of `quantity` for the part `accepted`, pro rata and
/// rounded to the nearest fils.
fn pro_rata(amount: i64, accepted: i64, quantity: i64) -> i64 {
    if quantity <= 0 {
        return 0;
    }
    let scaled = i128::from(amount) * i128::from(accepted);
    let quantity = i128::from(quantity);
    i64::try_from((scaled + quantity / 2) / quantity).unwrap_or(i64::MAX)
}

fn captured_methods(form: &DeliverForm) -> Vec<PodMethod> {
    let present = |value: &Option<String>| value.as_deref().is_some_and(|v| !v.trim().is_empty());
    let mut captured = Vec::new();
    if present(&form.photo) {
        captured.push(PodMethod::Photo);
    }
    if present(&form.signature) {
        captured.push(PodMethod::Signature);
    }
    if form.otp_verified {
        captured.push(PodMethod::Otp);
    }
    captured
}

/// The rules of `deliver` (wl-delivery §4.1, issue #158): each line's
/// delivered plus refused equals its quantity exactly; a refused quantity
/// has a known refusal reason, and a photo when it is *damaged*; refused
/// quantities only where partial acceptance is on; at least one unit is
/// delivered; the required proof is captured; the cash is the expected
/// amount.
///
/// # Errors
///
/// The first rule that fails, as a message safe to show.
pub(crate) fn evaluate(input: &DeliverInput) -> std::result::Result<Evaluated, String> {
    let form = &input.form;
    let mut delivered_total = 0i64;
    let mut expected_from_lines = 0i64;
    let mut has_line_amounts = false;
    for facts in &input.facts {
        let entries: Vec<&LineEntry> = form
            .lines
            .iter()
            .filter(|e| e.line_id == facts.id)
            .collect();
        let [entry] = entries.as_slice() else {
            return Err(format!(
                "{}: enter the delivered and refused quantity of this line.",
                facts.description
            ));
        };
        if entry.delivered_milli < 0 || entry.refused_milli < 0 {
            return Err(format!(
                "{}: quantities cannot be negative.",
                facts.description
            ));
        }
        if entry.delivered_milli + entry.refused_milli != facts.quantity_milli {
            return Err(format!(
                "{}: delivered plus refused must equal the quantity ordered ({}).",
                facts.description,
                milli_text(facts.quantity_milli)
            ));
        }
        if entry.refused_milli > 0 {
            if !input.partial_allowed {
                return Err(
                    "Partial acceptance is off for this delivery: deliver every line in full, or fail the attempt with the reason Refused."
                        .to_owned(),
                );
            }
            let reason = entry.reason.as_deref().map(str::trim).unwrap_or_default();
            if reason.is_empty() {
                return Err(format!(
                    "{}: a refused quantity needs a refusal reason.",
                    facts.description
                ));
            }
            if !input.refusal_reasons.iter().any(|known| known == reason) {
                return Err(format!(
                    "{}: {reason} is not a refusal reason.",
                    facts.description
                ));
            }
            let has_photo = entry.photo.as_deref().is_some_and(|p| !p.trim().is_empty());
            if reason == DAMAGED && !has_photo {
                return Err(format!(
                    "{}: a damaged quantity needs a photo.",
                    facts.description
                ));
            }
        }
        delivered_total += entry.delivered_milli;
        if let Some(amount) = facts.amount {
            has_line_amounts = true;
            expected_from_lines += pro_rata(amount, entry.delivered_milli, facts.quantity_milli);
        }
    }
    if form
        .lines
        .iter()
        .any(|e| !input.facts.iter().any(|f| f.id == e.line_id))
    {
        return Err("A quantity was entered for a line that is not on this delivery.".to_owned());
    }
    if delivered_total <= 0 {
        return Err("Nothing was accepted: fail the attempt with the reason Refused.".to_owned());
    }
    let captured = captured_methods(form);
    let missing: Vec<&str> = input
        .required
        .iter()
        .filter(|method| !captured.contains(method))
        .map(|method| method.as_str())
        .collect();
    let satisfied = if input.any_method {
        input.required.is_empty() || missing.len() < input.required.len()
    } else {
        missing.is_empty()
    };
    if !satisfied {
        return Err(format!(
            "The proof is incomplete: {} needed.",
            missing.join(", ")
        ));
    }
    let cash_expected = if has_line_amounts {
        expected_from_lines
    } else {
        input.amount_to_collect
    };
    if form.cash_collected < 0 {
        return Err("The cash collected cannot be negative.".to_owned());
    }
    if form.cash_collected > cash_expected {
        return Err(format!(
            "The cash collected is above the {} expected: give the change back.",
            aed(cash_expected)
        ));
    }
    let difference = cash_expected - form.cash_collected;
    let mut difference_reason = None;
    let mut needs_approval = false;
    if difference > 0 {
        if form.cash_collected == 0 {
            return Err(format!(
                "Nothing was collected of the {} expected: fail the attempt with the reason Refused to pay or No cash.",
                aed(cash_expected)
            ));
        }
        let above = difference > input.tolerance;
        if above && !input.approval_allowed {
            return Err(format!(
                "The cash collected is {} short of the {} expected and a short payment cannot be approved: fail the attempt with the reason Refused to pay.",
                aed(difference),
                aed(cash_expected)
            ));
        }
        let given = form
            .difference_reason
            .as_deref()
            .map(str::trim)
            .unwrap_or_default();
        if given.is_empty() {
            return Err(format!(
                "The cash collected is {} short of the {} expected: record the difference with a reason{}, or fail the attempt with the reason Refused to pay.",
                aed(difference),
                aed(cash_expected),
                if above { " and ask for approval" } else { "" }
            ));
        }
        if !input.cod_reasons.iter().any(|known| known == given) {
            return Err(format!("{given} is not a reason for a cash difference."));
        }
        needs_approval = above;
        difference_reason = Some(given.to_owned());
    }
    Ok(Evaluated {
        cash_expected,
        captured,
        difference,
        difference_reason,
        needs_approval,
    })
}

fn deliver_input(ctx: &TransitionCtx) -> std::result::Result<DeliverInput, String> {
    ctx.input_as::<DeliverInput>()
        .map_err(|_| "The outcome was not prepared by the proof service.".to_owned())
}

/// Whether the actor is the driver the input names.
fn is_driver(ctx: &TransitionCtx, driver: Option<i64>) -> bool {
    matches!((ctx.actor(), driver), (Some(user), Some(driver)) if user.id == driver)
}

/// What `fail` runs on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct FailInput {
    pub at: DateTime,
    pub driver_user_id: i64,
    pub attempt_number: i64,
    pub reason: String,
    pub counts: bool,
    pub latitude_micro: Option<i64>,
    pub longitude_micro: Option<i64>,
}

fn fail_input(ctx: &TransitionCtx) -> Option<FailInput> {
    ctx.input_as::<FailInput>().ok()
}

/// The delivery transitions of the stop outcome, for the machine.
pub(crate) fn transitions() -> Vec<Transition<Delivery>> {
    vec![
        deliver_transition(),
        deliver_short_transition(DELIVER_SHORT, false),
        deliver_short_transition(DELIVER_SHORT_APPROVED, true),
        fail_transition(),
    ]
}

/// What a `deliver` saves, read from the delivery and the run's context before
/// the future starts.
struct Job {
    delivery_id: i64,
    run_id: Option<i64>,
    actor: Option<i64>,
    /// The approval request the transition runs for, when it is the
    /// approved twin of a difference.
    request: Option<i64>,
    input: std::result::Result<DeliverInput, String>,
}

impl Job {
    fn of(delivery: &Delivery, ctx: &TransitionCtx) -> Self {
        Job {
            delivery_id: delivery.id,
            run_id: delivery.run_id,
            actor: ctx.actor().map(|user| user.id),
            request: ctx.meta().get("approval_request").and_then(Value::as_i64),
            input: deliver_input(ctx),
        }
    }

    /// Saves the attempt, the proof and the line quantities. `short` is a
    /// delivery with a difference above the tolerance: its absorber must be
    /// known (the approver's choice, else the reason's default that is
    /// offered).
    async fn save(self, tx: &mut Transaction, short: bool) -> Result<()> {
        let invalid = |message: String| Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message);
        let input = self.input.map_err(invalid)?;
        let evaluated = evaluate(&input).map_err(invalid)?;
        let form = &input.form;
        let actor = self.actor.or(Some(input.driver_user_id));
        let mut attempt = Attempt {
            delivery_id: self.delivery_id,
            run_id: self.run_id,
            number: input.attempt_number,
            outcome: AttemptOutcome::Delivered,
            failure_reason: None,
            counts: true,
            occurred_at: input.at,
            latitude_micro: form.latitude_micro,
            longitude_micro: form.longitude_micro,
            actor_user_id: actor,
            ..Attempt::default()
        };
        attempt.insert(&mut *tx).await?;
        let absorbed_by = match &evaluated.difference_reason {
            Some(reason) => {
                let chosen = match self.request {
                    Some(request) => ApprovalChoice::in_tx(&mut *tx, request)
                        .await?
                        .and_then(|choice| choice.absorbed_by),
                    None => None,
                };
                let absorber = absorber_for(&mut *tx, &input, reason, chosen).await?;
                if short && absorber.is_none() {
                    return Err(invalid(
                        english("wl_delivery.cod.choose_absorber").to_owned(),
                    ));
                }
                absorber
            }
            None => None,
        };
        let methods: Vec<&str> = evaluated.captured.iter().map(|m| m.as_str()).collect();
        let mut proof = Proof {
            delivery_id: self.delivery_id,
            attempt_id: attempt.id,
            methods: methods.join(","),
            recipient_name: form.recipient_name.trim().to_owned(),
            photo_ref: form.photo.clone().filter(|p| !p.trim().is_empty()),
            signature_ref: form.signature.clone().filter(|p| !p.trim().is_empty()),
            otp_verified: form.otp_verified,
            cash_expected: evaluated.cash_expected,
            cash_collected: form.cash_collected,
            cheque_reference: form.cheque.clone().filter(|c| !c.trim().is_empty()),
            notes: form.notes.trim().to_owned(),
            reversed: false,
            cod_difference_reason: evaluated.difference_reason.clone(),
            absorbed_by,
            approval_request_id: self.request,
            ..Proof::default()
        };
        proof.insert(&mut *tx).await?;
        for entry in &form.lines {
            let reason = if entry.refused_milli > 0 {
                entry.reason.as_deref().map(|r| r.trim().to_owned())
            } else {
                None
            };
            let photo = if entry.refused_milli > 0 {
                entry.photo.clone().filter(|p| !p.trim().is_empty())
            } else {
                None
            };
            DeliveryLine::query()
                .where_(DeliveryLine::ID.eq(entry.line_id))
                .update(
                    &mut *tx,
                    &[
                        ("delivered_milli", &entry.delivered_milli),
                        ("refused_milli", &entry.refused_milli),
                        ("refusal_reason", &reason),
                        ("refusal_photo_ref", &photo),
                    ],
                )
                .await?;
        }
        let attempts = counted_attempts(&mut *tx, self.delivery_id).await?;
        Delivery::query()
            .where_(Delivery::ID.eq(self.delivery_id))
            .update(&mut *tx, &[("attempts", &attempts)])
            .await?;
        Ok(())
    }
}

/// Who absorbs the difference of reason `reason`: the approver's `chosen`
/// absorber, else the reason's default; `None` when it is not one of the
/// absorbers the settings offer (03 leaves out the customer).
async fn absorber_for(
    tx: &mut Transaction,
    input: &DeliverInput,
    reason: &str,
    chosen: Option<String>,
) -> Result<Option<String>> {
    let candidate = match chosen {
        Some(chosen) => Some(chosen),
        None => CodReason::query()
            .where_(CodReason::CODE.eq(reason))
            .first(&mut *tx)
            .await?
            .map(|row| row.default_absorbed_by),
    };
    Ok(candidate.filter(|key| input.absorbers.iter().any(|offered| offered == key)))
}

fn deliver_transition() -> Transition<Delivery> {
    Transition::new(DELIVER)
        .label("Deliver")
        .tone("success")
        .from([DeliveryStatus::OutForDelivery])
        .to(DeliveryStatus::Delivered)
        .permission(permissions::COMPLETE)
        .guard("by_assigned_driver", |_delivery: &Delivery, ctx| {
            let allowed = is_driver(ctx, deliver_input(ctx).ok().map(|i| i.driver_user_id));
            Box::pin(async move {
                if allowed {
                    Ok(())
                } else {
                    Err("Only the driver of the run completes its stops.".to_owned())
                }
            })
        })
        .guard("proof_and_quantities", |_delivery: &Delivery, ctx| {
            let checked = deliver_input(ctx).and_then(|input| evaluate(&input).map(|_| ()));
            Box::pin(async move { checked })
        })
        .guard("no_approval_needed", |_delivery: &Delivery, ctx| {
            let checked = deliver_input(ctx)
                .and_then(|input| evaluate(&input))
                .and_then(|evaluated| {
                    if evaluated.needs_approval {
                        Err(english("wl_delivery.cod.needs_approval").to_owned())
                    } else {
                        Ok(())
                    }
                });
            Box::pin(async move { checked })
        })
        .effect(|delivery: &Delivery, ctx, tx| {
            let job = Job::of(delivery, ctx);
            Box::pin(async move { job.save(tx, false).await })
        })
}

/// `deliver_short` (the driver's request) and `deliver_short_approved` (run
/// by the kernel): a delivery whose cash is short by more than the tolerance.
fn deliver_short_transition(name: &'static str, approved: bool) -> Transition<Delivery> {
    let transition = Transition::new(name)
        .label(if approved {
            "Deliver with a cash difference (approved)"
        } else {
            "Deliver with a cash difference"
        })
        .tone("warning")
        .from([DeliveryStatus::OutForDelivery])
        .to(DeliveryStatus::Delivered);
    let transition = if approved {
        transition.system()
    } else {
        transition
            .permission(permissions::COMPLETE)
            .guard("by_assigned_driver", |_delivery: &Delivery, ctx| {
            let allowed = is_driver(ctx, deliver_input(ctx).ok().map(|i| i.driver_user_id));
            Box::pin(async move {
                if allowed {
                    Ok(())
                } else {
                    Err("Only the driver of the run completes its stops.".to_owned())
                }
            })
        })
    };
    transition
        .guard("proof_and_quantities", |_delivery: &Delivery, ctx| {
            let checked = deliver_input(ctx).and_then(|input| evaluate(&input).map(|_| ()));
            Box::pin(async move { checked })
        })
        .guard("approval_needed", |_delivery: &Delivery, ctx| {
            let checked = deliver_input(ctx)
                .and_then(|input| evaluate(&input))
                .and_then(|evaluated| {
                    if evaluated.needs_approval {
                        Ok(())
                    } else {
                        Err(english("wl_delivery.cod.needs_no_approval").to_owned())
                    }
                });
            Box::pin(async move { checked })
        })
        .effect(|delivery: &Delivery, ctx, tx| {
            let job = Job::of(delivery, ctx);
            Box::pin(async move { job.save(tx, true).await })
        })
}

/// The attempts of the delivery that count against the maximum, the one of
/// this transition included (it is saved before the count).
async fn counted_attempts(tx: &mut renox::db::Transaction, delivery_id: i64) -> Result<i64> {
    let counted = Attempt::query()
        .where_(Attempt::DELIVERY_ID.eq(delivery_id))
        .where_(Attempt::COUNTS.eq(true))
        .count(&mut *tx)
        .await?;
    Ok(i64::try_from(counted).unwrap_or(i64::MAX))
}

fn fail_transition() -> Transition<Delivery> {
    Transition::new(FAIL)
        .label("Fail")
        .tone("warning")
        .from([DeliveryStatus::OutForDelivery])
        .to(DeliveryStatus::Attempted)
        .permission(permissions::COMPLETE)
        .guard("by_assigned_driver", |_delivery: &Delivery, ctx| {
            let allowed = is_driver(ctx, fail_input(ctx).map(|i| i.driver_user_id));
            Box::pin(async move {
                if allowed {
                    Ok(())
                } else {
                    Err("Only the driver of the run completes its stops.".to_owned())
                }
            })
        })
        .guard("reason_and_location", |_delivery: &Delivery, ctx| {
            let checked = match fail_input(ctx) {
                None => Err("The outcome was not prepared by the proof service.".to_owned()),
                Some(input) if input.reason.trim().is_empty() => {
                    Err("Choose the reason the visit failed.".to_owned())
                }
                Some(input)
                    if input.latitude_micro.is_none() || input.longitude_micro.is_none() =>
                {
                    Err("The location of the visit is needed.".to_owned())
                }
                Some(_) => Ok(()),
            };
            Box::pin(async move { checked })
        })
        .effect(|delivery: &Delivery, ctx, tx| {
            let delivery_id = delivery.id;
            let run_id = delivery.run_id;
            let actor = ctx.actor().map(|user| user.id);
            let input = fail_input(ctx);
            Box::pin(async move {
                let input = input.ok_or_else(|| {
                    Error::from(renox::anyhow::anyhow!("a fail without its input"))
                })?;
                let mut attempt = Attempt {
                    delivery_id,
                    run_id,
                    number: input.attempt_number,
                    outcome: AttemptOutcome::Failed,
                    failure_reason: Some(input.reason.clone()),
                    counts: input.counts,
                    occurred_at: input.at,
                    latitude_micro: input.latitude_micro,
                    longitude_micro: input.longitude_micro,
                    actor_user_id: actor,
                    ..Attempt::default()
                };
                attempt.insert(&mut *tx).await?;
                let attempts = counted_attempts(&mut *tx, delivery_id).await?;
                Delivery::query()
                    .where_(Delivery::ID.eq(delivery_id))
                    .update(&mut *tx, &[("attempts", &attempts)])
                    .await?;
                Ok(())
            })
        })
}

fn guard_error(message: impl Into<String>) -> TransitionError {
    TransitionError::Guard(message.into())
}

/// The driver of the run `delivery` is on.
async fn driver_of(db: &Db, delivery: &Delivery) -> Result<Option<i64>> {
    match delivery.run_id {
        Some(id) => Ok(crate::run::Run::find(db, id)
            .await?
            .map(|run| run.driver_user_id)),
        None => Ok(None),
    }
}

/// What a driver's `deliver` came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliverOutcome {
    /// The delivery is `delivered`.
    Delivered,
    /// The cash collected is short by more than the tolerance: an approval
    /// request (its id) waits for an approver; the delivery stays
    /// `out_for_delivery` (`WRK.ACL.09`).
    Requested(i64),
}

/// Records the outcome *delivered* of `delivery` for `user`, the driver of
/// its run, at `at`.
///
/// A second submit for a delivery that is already `delivered` is refused and
/// the answer shows the saved proof; the proof is never edited. When the
/// cash collected is below the cash expected by more than the tolerance, the
/// outcome is a request for approval ([`DeliverOutcome::Requested`]) if the
/// driver gave a difference reason, the setting allows it and no earlier
/// request of this visit lapsed or was rejected.
///
/// # Errors
///
/// `Guard` (422) with the first rule that fails (see [`evaluate`]);
/// `Forbidden` without `deliveries.complete`; `NotFrom` unless the delivery
/// is `out_for_delivery`.
pub async fn deliver(
    state: &AppState,
    user: &AuthUser,
    delivery: &Delivery,
    form: DeliverForm,
    at: DateTime,
) -> std::result::Result<DeliverOutcome, TransitionError> {
    if delivery.status == DeliveryStatus::Delivered {
        return Err(guard_error(saved_proof_text(&state.db, delivery).await));
    }
    cod::lapse_overdue(state, delivery.id)
        .await
        .map_err(TransitionError::Failed)?;
    let settings = settings(state);
    let lines = delivery
        .lines(&state.db)
        .await
        .map_err(TransitionError::Failed)?;
    let (required, any_method) = if delivery.pod_methods.trim().is_empty() {
        (Vec::new(), false)
    } else {
        (delivery.required_methods(), delivery.pod_any)
    };
    let input = DeliverInput {
        at,
        form,
        driver_user_id: driver_of(&state.db, delivery)
            .await
            .map_err(TransitionError::Failed)?
            .unwrap_or_default(),
        attempt_number: next_attempt_number(&state.db, delivery)
            .await
            .map_err(TransitionError::Failed)?,
        facts: lines
            .iter()
            .map(|line| LineFacts {
                id: line.id,
                description: line.description.clone(),
                quantity_milli: line.quantity_milli,
                amount: line.amount_to_collect,
            })
            .collect(),
        required,
        any_method,
        partial_allowed: delivery.partial_acceptance,
        amount_to_collect: delivery.amount_to_collect,
        tolerance: settings.cod_tolerance(),
        refusal_reasons: RefusalReason::active_codes(&state.db)
            .await
            .map_err(TransitionError::Failed)?,
        cod_reasons: CodReason::active_ones(&state.db)
            .await
            .map_err(TransitionError::Failed)?
            .into_iter()
            .map(|reason| reason.code)
            .collect(),
        approval_allowed: settings.cod_approval(),
        absorbers: settings
            .cod_absorbers()
            .iter()
            .map(|absorber| absorber.as_str().to_owned())
            .collect(),
    };
    if evaluate(&input).is_ok_and(|evaluated| evaluated.needs_approval) {
        return request_difference(state, user, delivery, &input).await;
    }
    let _outcome = delivery
        .transition(DELIVER)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    after_delivered(state, delivery.id)
        .await
        .map_err(TransitionError::Failed)?;
    Ok(DeliverOutcome::Delivered)
}

/// The driver's request for a cash difference above the tolerance: the
/// transition `deliver_short`, deferred by the approval policy; the request
/// is announced after the commit and given the approval timeout.
async fn request_difference(
    state: &AppState,
    user: &AuthUser,
    delivery: &Delivery,
    input: &DeliverInput,
) -> std::result::Result<DeliverOutcome, TransitionError> {
    if let Some(message) = cod::blocked(&state.db, delivery)
        .await
        .map_err(TransitionError::Failed)?
    {
        return Err(guard_error(message));
    }
    let outcome = delivery
        .transition(DELIVER_SHORT)
        .by(user)
        .input(input)
        .run(state)
        .await?;
    wl_kernel::approvals::announce(state, &outcome)
        .await
        .map_err(TransitionError::Failed)?;
    match cod::request_id(&outcome) {
        Some(id) => {
            cod::set_timeout(state, id)
                .await
                .map_err(TransitionError::Failed)?;
            Ok(DeliverOutcome::Requested(id))
        }
        // No policy is in force: the difference was delivered at once.
        None => {
            after_delivered(state, delivery.id)
                .await
                .map_err(TransitionError::Failed)?;
            Ok(DeliverOutcome::Delivered)
        }
    }
}

/// The saved proof of a delivered delivery, as a sentence for the refusal of
/// a second submit.
async fn saved_proof_text(db: &Db, delivery: &Delivery) -> String {
    let proof = delivery.latest_proof(db).await.ok().flatten();
    match proof {
        Some(proof) => format!(
            "{} is already delivered. Saved proof: received by {} ({}).",
            delivery.number,
            proof.recipient_name,
            if proof.methods.is_empty() {
                "no method".to_owned()
            } else {
                proof.methods.replace(',', ", ")
            }
        ),
        None => format!("{} is already delivered.", delivery.number),
    }
}

/// The number the next visit of `delivery` gets.
pub(crate) async fn next_attempt_number(db: &Db, delivery: &Delivery) -> Result<i64> {
    let made = Attempt::query()
        .where_(Attempt::DELIVERY_ID.eq(delivery.id))
        .count(db)
        .await?;
    Ok(i64::try_from(made).unwrap_or(i64::MAX) + 1)
}

/// After the commit of a delivery (by `deliver`, or by the kernel when a
/// difference was approved): the cash goes to the driver's custody, then
/// `DeliveryDelivered` is emitted.
pub(crate) async fn after_delivered(state: &AppState, id: i64) -> Result<()> {
    let delivery = Delivery::find_or_404(&state.db, id).await?;
    let proof = delivery
        .latest_proof(&state.db)
        .await?
        .ok_or(Error::NotFound)?;
    collect_cash(state, &delivery, &proof).await?;
    emit_delivered(state, &delivery, &proof).await
}

/// Adds the cash of `proof` to the custody of the driver of the run
/// (`FIN.CSH.01`): the cash collected, or, when the driver absorbs the
/// difference, the full cash expected, so the missing part is the shortage at
/// the count. The kernel adds one source's cash once; the call is refused
/// once the driver's handover of the run was counted (it cannot be: the run
/// is back only after every stop has an outcome).
async fn collect_cash(state: &AppState, delivery: &Delivery, proof: &Proof) -> Result<()> {
    let Some(run_id) = delivery.run_id else {
        return Ok(());
    };
    let amount = if proof.absorbed_by.as_deref() == Some("driver") {
        proof.cash_expected
    } else {
        proof.cash_collected
    };
    if amount <= 0 {
        return Ok(());
    }
    let run = Run::find_or_404(&state.db, run_id).await?;
    let planned_end = match (run.planned_start, run.estimated_minutes) {
        (Some(start), Some(minutes)) => Some(start + Duration::minutes(minutes)),
        _ => None,
    };
    let mut period = HandoverPeriod::run(run.subject(), run.run_date, planned_end);
    if settings(state).replace_cash_escalation() {
        period = period.replaced_escalation();
    }
    custody(state)
        .collect(
            state,
            CashCollection::new(
                run.driver_user_id,
                period,
                amount,
                delivery.delivery_id().subject(),
            ),
        )
        .await?;
    Ok(())
}

/// Emits `DeliveryDelivered` for the committed state of `delivery`.
async fn emit_delivered(state: &AppState, delivery: &Delivery, proof: &Proof) -> Result<()> {
    let attempt = Attempt::find_or_404(&state.db, proof.attempt_id).await?;
    let lines = delivery
        .lines(&state.db)
        .await?
        .into_iter()
        .map(|line| DeliveredLine {
            line: line.source_line(),
            description: line.description.clone(),
            quantity_milli: line.quantity_milli,
            delivered_milli: line.delivered_milli,
            refused_milli: line.refused_milli,
            refusal_reason: line.refusal_reason.clone(),
        })
        .collect();
    state
        .emit(DeliveryDelivered {
            delivery: delivery.delivery_id(),
            source: delivery.source(),
            attempt: attempt.number,
            proof: proof.id,
            lines,
            cash_expected: proof.cash_expected,
            cash_collected: proof.cash_collected,
            cod_difference: proof.cash_expected - proof.cash_collected,
            cod_difference_reason: proof.cod_difference_reason.clone(),
            absorbed_by: proof.absorbed_by.clone(),
            cheque: proof.cheque_reference.clone(),
        })
        .await
}

/// Registers the listener that finishes a delivery the kernel ran after an
/// approver agreed to a cash difference.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: Transitioned<Delivery>, state: AppState| async move {
        if event.transition == DELIVER_SHORT_APPROVED {
            after_delivered(&state, event.key).await
        } else {
            Ok(())
        }
    });
}

/// Records a failed visit of `delivery` for `user`, the driver of its run:
/// the failure reason's code and the location captured at that moment.
///
/// # Errors
///
/// `Guard` (422) without a known reason or a location; `Forbidden` without
/// `deliveries.complete`; `NotFrom` unless the delivery is
/// `out_for_delivery`.
pub async fn fail(
    state: &AppState,
    user: &AuthUser,
    delivery: &Delivery,
    reason_code: &str,
    location: (Option<i64>, Option<i64>),
    at: DateTime,
) -> std::result::Result<(), TransitionError> {
    cod::lapse_overdue(state, delivery.id)
        .await
        .map_err(TransitionError::Failed)?;
    let reason = FailureReason::active_by_code(&state.db, reason_code)
        .await
        .map_err(TransitionError::Failed)?
        .ok_or_else(|| guard_error("Choose the reason the visit failed."))?;
    let input = FailInput {
        at,
        driver_user_id: driver_of(&state.db, delivery)
            .await
            .map_err(TransitionError::Failed)?
            .unwrap_or_default(),
        attempt_number: next_attempt_number(&state.db, delivery)
            .await
            .map_err(TransitionError::Failed)?,
        reason: reason.code.clone(),
        counts: reason.counts,
        latitude_micro: location.0,
        longitude_micro: location.1,
    };
    let _outcome = delivery
        .transition(FAIL)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    // A cash difference the driver was waiting on is withdrawn (*no answer:
    // fail attempt*).
    cod::withdraw_pending(state, user, delivery.id)
        .await
        .map_err(TransitionError::Failed)?;
    state
        .emit(DeliveryAttemptFailed {
            delivery: delivery.delivery_id(),
            source: delivery.source(),
            reason: reason.code,
            attempt: input.attempt_number,
        })
        .await
        .map_err(TransitionError::Failed)
}

/// *No answer: fail attempt*: the driver gives up waiting for an approver. The
/// pending request is withdrawn and the visit fails with the reason *refused
/// to pay* (`WRK.ACL.09`).
///
/// # Errors
///
/// As [`fail`].
pub async fn fail_no_answer(
    state: &AppState,
    user: &AuthUser,
    delivery: &Delivery,
    location: (Option<i64>, Option<i64>),
    at: DateTime,
) -> std::result::Result<(), TransitionError> {
    fail(state, user, delivery, REFUSED_TO_PAY, location, at).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> Vec<LineFacts> {
        vec![
            LineFacts {
                id: 1,
                description: "Tiles".into(),
                quantity_milli: 10_000,
                amount: Some(10_000),
            },
            LineFacts {
                id: 2,
                description: "Grout".into(),
                quantity_milli: 5_000,
                amount: Some(5_000),
            },
        ]
    }

    fn input(lines: Vec<LineEntry>, cash: i64) -> DeliverInput {
        DeliverInput {
            at: DateTime::default(),
            form: DeliverForm {
                recipient_name: "Mariam".into(),
                photo: Some("photo-1".into()),
                signature: Some("sig-1".into()),
                lines,
                cash_collected: cash,
                ..DeliverForm::default()
            },
            driver_user_id: 1,
            attempt_number: 1,
            facts: facts(),
            required: vec![PodMethod::Photo, PodMethod::Signature],
            any_method: false,
            partial_allowed: true,
            amount_to_collect: 15_000,
            tolerance: 0,
            refusal_reasons: vec!["damaged".into(), "other".into()],
            cod_reasons: vec!["recipient_short".into()],
            approval_allowed: true,
            absorbers: vec!["business".into(), "driver".into()],
        }
    }

    fn entry(line: i64, delivered: i64, refused: i64) -> LineEntry {
        LineEntry {
            line_id: line,
            delivered_milli: delivered,
            refused_milli: refused,
            reason: None,
            photo: None,
        }
    }

    #[test]
    fn quantities_read_as_decimals() {
        assert_eq!(milli_text(5_000), "5");
        assert_eq!(milli_text(2_500), "2.5");
        assert_eq!(milli_text(125), "0.125");
    }

    #[test]
    fn money_reads_as_aed() {
        assert_eq!(aed(13_000), "AED 130.00");
        assert_eq!(aed(5), "AED 0.05");
    }

    #[test]
    fn an_amount_is_taken_pro_rata_to_the_accepted_quantity() {
        assert_eq!(pro_rata(5_000, 3_000, 5_000), 3_000);
        assert_eq!(pro_rata(100, 1, 3), 33);
        assert_eq!(pro_rata(100, 0, 0), 0);
    }

    #[test]
    fn a_line_delivered_in_full_and_one_partly_accepted_is_valid() {
        let mut b = entry(2, 3_000, 2_000);
        b.reason = Some("damaged".into());
        b.photo = Some("dmg".into());
        let evaluated = evaluate(&input(vec![entry(1, 10_000, 0), b], 13_000)).unwrap();
        assert_eq!(evaluated.cash_expected, 13_000);
    }

    #[test]
    fn the_rules_refuse_what_the_issue_lists() {
        let mut no_reason = entry(2, 3_000, 2_000);
        let error =
            evaluate(&input(vec![entry(1, 10_000, 0), no_reason.clone()], 13_000)).unwrap_err();
        assert!(error.contains("refusal reason"), "{error}");

        no_reason.reason = Some("damaged".into());
        let error =
            evaluate(&input(vec![entry(1, 10_000, 0), no_reason.clone()], 13_000)).unwrap_err();
        assert!(error.contains("photo"), "{error}");

        let error = evaluate(&input(
            vec![entry(1, 10_000, 0), entry(2, 2_000, 2_000)],
            15_000,
        ))
        .unwrap_err();
        assert!(error.contains("must equal the quantity"), "{error}");

        let mut all_refused_a = entry(1, 0, 10_000);
        all_refused_a.reason = Some("other".into());
        let mut all_refused_b = entry(2, 0, 5_000);
        all_refused_b.reason = Some("other".into());
        let error = evaluate(&input(vec![all_refused_a, all_refused_b], 0)).unwrap_err();
        assert!(error.contains("Nothing was accepted"), "{error}");
    }

    #[test]
    fn the_proof_must_be_complete_and_any_mode_needs_one_method() {
        let mut only_photo = input(vec![entry(1, 10_000, 0), entry(2, 5_000, 0)], 15_000);
        only_photo.form.signature = None;
        let error = evaluate(&only_photo).unwrap_err();
        assert!(error.contains("signature"), "{error}");
        only_photo.any_method = true;
        assert!(evaluate(&only_photo).is_ok());
    }

    #[test]
    fn partial_acceptance_off_refuses_a_refused_quantity() {
        let mut b = entry(2, 3_000, 2_000);
        b.reason = Some("other".into());
        let mut off = input(vec![entry(1, 10_000, 0), b], 13_000);
        off.partial_allowed = false;
        let error = evaluate(&off).unwrap_err();
        assert!(error.contains("Partial acceptance is off"), "{error}");
    }

    #[test]
    fn without_line_amounts_the_expected_cash_stays_the_full_amount() {
        let mut b = entry(2, 3_000, 2_000);
        b.reason = Some("other".into());
        let mut plain = input(vec![entry(1, 10_000, 0), b], 15_000);
        for facts in &mut plain.facts {
            facts.amount = None;
        }
        assert_eq!(evaluate(&plain).unwrap().cash_expected, 15_000);
        plain.form.cash_collected = 13_000;
        let error = evaluate(&plain).unwrap_err();
        assert!(error.contains("short"), "{error}");
    }

    fn short_by_fifty() -> DeliverInput {
        let lines = vec![entry(1, 10_000, 0), entry(2, 5_000, 0)];
        let mut short = input(lines, 14_500);
        short.form.difference_reason = Some("recipient_short".into());
        short.tolerance = 0;
        short
    }

    #[test]
    fn a_short_payment_above_the_tolerance_needs_a_reason_and_an_approval() {
        let mut short = short_by_fifty();
        let evaluated = evaluate(&short).unwrap();
        assert_eq!(evaluated.difference, 500);
        assert_eq!(evaluated.difference_reason.as_deref(), Some("recipient_short"));
        assert!(evaluated.needs_approval);

        short.form.difference_reason = None;
        let error = evaluate(&short).unwrap_err();
        assert!(error.contains("short of the AED 150.00 expected"), "{error}");
        assert!(error.contains("ask for approval"), "{error}");

        short.form.difference_reason = Some("made_up".into());
        let error = evaluate(&short).unwrap_err();
        assert!(error.contains("not a reason"), "{error}");
    }

    #[test]
    fn a_difference_within_the_tolerance_needs_a_reason_but_no_approval() {
        let mut short = short_by_fifty();
        short.tolerance = 500;
        let evaluated = evaluate(&short).unwrap();
        assert!(!evaluated.needs_approval);
        assert_eq!(evaluated.difference_reason.as_deref(), Some("recipient_short"));
        short.form.difference_reason = None;
        assert!(evaluate(&short).is_err());
    }

    #[test]
    fn with_approval_off_only_a_failed_attempt_is_left() {
        let mut short = short_by_fifty();
        short.approval_allowed = false;
        let error = evaluate(&short).unwrap_err();
        assert!(error.contains("cannot be approved"), "{error}");
    }

    #[test]
    fn nothing_collected_is_not_a_difference() {
        let mut nothing = short_by_fifty();
        nothing.form.cash_collected = 0;
        let error = evaluate(&nothing).unwrap_err();
        assert!(error.contains("Nothing was collected"), "{error}");
    }
}
