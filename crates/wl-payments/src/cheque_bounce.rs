//! Recording a bounced cheque (wl-payments §4 `bounce`, §5.3 step 3,
//! `FIN.RUL.04`, issue #102).
//!
//! When the bank returns a `deposited` cheque, a user with `cheques.deposit`
//! records the bank's return reason, the reference of its return certificate
//! (required for a received cheque, none for an issued one) and the part the
//! bank paid from the available balance (the honoured amount, 0 unless
//! entered; it must stay below what was presented).
//!
//! In the transition's transaction: the return is kept ([`ChequeReturn`], one
//! per presentation), the cheque's honoured and open amounts, return reason,
//! certificate and day are set (open = open before − honoured), and the
//! honoured part is recorded as a payment of the cheque (money in for a
//! received cheque, out for an issued one) and, after the commit, allocated
//! to the documents the cheque pays, oldest first.
//!
//! After the commit ([`after_bounce`], from the listener in
//! [`crate::cheque_events`]):
//!
//! - **received cheque:** the bounce fee charge is added when a fee is set
//!   (§7), due on the bounce date, on the agreement linked to the cheque or as
//!   a standalone charge for the party; when invoicing cannot schedule
//!   charges, the accountant gets a task to raise it. A new fee is added at
//!   each bounce. Then `ChequeBounced` is emitted (wl-sales puts the customer
//!   on credit hold), the party is told (unless blocked) and the accountant
//!   gets a follow-up task (`FIN.RUL.04`).
//! - **issued cheque:** none of these; the accountant gets a task to replace
//!   it.
//!
//! Decisions on what the spec leaves open: the certificate is recorded as a
//! reference (a document id or number from wl-documents), since the kernel
//! has no contract to upload a file from this flow yet; the reason is free
//! text, compared without case against the criminal-grounds list.

use renox::prelude::*;
use renox::state_machine::{StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::cheques::{BounceFeeCharge, ChequeBounced, ChequeStatus, bounce_fees};
use wl_kernel::documents::today;
use wl_kernel::payments::PaymentMethod;

use crate::cheque::{Cheque, ChequeReturn};
use crate::cheque_money;
use crate::config_of;
use crate::followup::FollowupTask;
use crate::notices;
use crate::permissions;

/// The `bounce` transition's name.
pub const BOUNCE: &str = "bounce";

/// What a user records when a cheque bounced.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BounceRequest {
    /// The bank's return reason, e.g. "insufficient funds".
    pub reason: String,
    /// The reference of the bank's return certificate (required for a
    /// received cheque).
    pub certificate: Option<String>,
    /// What the bank paid from the available balance, in fils; 0 when
    /// nothing.
    pub honoured: i64,
}

/// What a `bounce` run carries to its guards and effect: the request,
/// trimmed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct BounceInput {
    /// The return reason.
    pub reason: String,
    /// The certificate reference, if given.
    pub certificate: Option<String>,
    /// The honoured amount, in fils.
    pub honoured: i64,
}

impl BounceInput {
    fn of(request: &BounceRequest) -> Self {
        BounceInput {
            reason: request.reason.trim().to_owned(),
            certificate: request
                .certificate
                .as_deref()
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .map(str::to_owned),
            honoured: request.honoured,
        }
    }
}

fn bounce_input(ctx: &TransitionCtx) -> Option<BounceInput> {
    ctx.input_as::<BounceInput>().ok()
}

/// Why `input` cannot bounce `cheque`, if it cannot: no reason, a received
/// cheque without its certificate, a negative honoured amount, or one that
/// is not below what was presented (that is `clear`).
pub(crate) fn input_problem(cheque: &Cheque, input: Option<&BounceInput>) -> Option<String> {
    let Some(input) = input else {
        return Some("A return reason is required.".to_owned());
    };
    if input.reason.trim().is_empty() {
        return Some("The bank's return reason is required.".to_owned());
    }
    if cheque.is_received()
        && input
            .certificate
            .as_deref()
            .is_none_or(|c| c.trim().is_empty())
    {
        return Some("The bank's return certificate must be attached.".to_owned());
    }
    if input.honoured < 0 {
        return Some("The honoured amount cannot be negative.".to_owned());
    }
    if input.honoured >= cheque.open_amount {
        return Some(format!(
            "The honoured amount must be below the {} fils presented: a cheque the bank paid in full is cleared.",
            cheque.open_amount
        ));
    }
    None
}

/// `bounce`, for the machine.
pub(crate) fn transitions() -> Vec<Transition<Cheque>> {
    vec![
        Transition::new(BOUNCE)
            .label("Bounced")
            .tone("danger")
            .permission(permissions::CHEQUES_DEPOSIT)
            .from([ChequeStatus::Deposited])
            .to(ChequeStatus::Bounced)
            .guard("return_recorded", |cheque: &Cheque, ctx| {
                let problem = input_problem(cheque, bounce_input(ctx).as_ref());
                Box::pin(async move { problem.map_or(Ok(()), Err) })
            })
            .effect(|cheque: &Cheque, ctx, tx| {
                let cheque = cheque.clone();
                let input = bounce_input(ctx);
                let by = ctx.actor().map(|user| user.id);
                Box::pin(async move {
                    let input = input.ok_or_else(|| {
                        Error::from(renox::anyhow::anyhow!("a bounce without its input"))
                    })?;
                    let day = today();
                    let honoured_total = cheque.honoured_amount + input.honoured;
                    let open = cheque.open_amount - input.honoured;
                    // The bank's certificate belongs to a received cheque.
                    let certificate = if cheque.is_received() {
                        input.certificate.clone()
                    } else {
                        None
                    };
                    let mut bounce = ChequeReturn {
                        cheque_id: cheque.id,
                        presentation: cheque.presentations,
                        reason: input.reason.clone(),
                        certificate: certificate.clone(),
                        honoured_amount: input.honoured,
                        returned_on: day,
                        recorded_by: by,
                        ..Default::default()
                    };
                    bounce.insert(&mut *tx).await?;
                    // Not the status column, so outside the machine's guard.
                    Cheque::query()
                        .where_(Cheque::ID.eq(cheque.id))
                        .update(
                            &mut *tx,
                            &[
                                ("honoured_amount", &honoured_total),
                                ("open_amount", &open),
                                ("return_reason", &Some(input.reason)),
                                ("return_certificate", &certificate),
                                ("bounced_on", &Some(day)),
                                ("updated_at", &renox::db::now()),
                            ],
                        )
                        .await?;
                    Ok(())
                })
            }),
    ]
}

async fn reload(state: &AppState, cheque: &Cheque) -> std::result::Result<Cheque, TransitionError> {
    Cheque::find_or_404(&state.db, cheque.id)
        .await
        .map_err(TransitionError::Failed)
}

/// `user` records that the bank returned the `deposited` `cheque`
/// ([`BounceRequest`]). The cheque becomes `bounced`; a honoured part is
/// recorded as a payment and allocated to the documents the cheque pays; the
/// fee, the event, the notices and the tasks follow the commit.
///
/// # Errors
///
/// `Forbidden` without `cheques.deposit`; `NotFrom` unless `deposited`;
/// `Guard` (422) without a reason, for a received cheque without its
/// certificate, for a honoured amount below 0 or not below the presented
/// amount, or when the payment of the honoured part is refused.
pub async fn bounce(
    state: &AppState,
    user: &AuthUser,
    cheque: &Cheque,
    request: &BounceRequest,
) -> std::result::Result<Cheque, TransitionError> {
    let cheque = reload(state, cheque).await?;
    let input = BounceInput::of(request);
    // The guards run in the transition; the payment is prepared first only
    // when there is a honoured part to record.
    let prepared = if input.honoured > 0 && input_problem(&cheque, Some(&input)).is_none() {
        let payment =
            cheque_money::payment_for(&cheque, input.honoured, PaymentMethod::Cheque, None);
        Some(cheque_money::prepare(state, &payment, Some(user.id)).await?)
    } else {
        None
    };
    let mut tx = state.db.begin().await?;
    let outcome = cheque
        .transition(BOUNCE)
        .by(user)
        .input(&input)
        .run_in(&mut tx)
        .await?;
    let mut honoured = None;
    if let Some(prepared) = prepared {
        let (payment, received) = cheque_money::record_in(&mut tx, prepared).await?;
        ChequeReturn::query()
            .where_(ChequeReturn::CHEQUE_ID.eq(cheque.id))
            .where_(ChequeReturn::PRESENTATION.eq(cheque.presentations))
            .update(&mut tx, &[("honoured_payment_id", &Some(payment.id))])
            .await?;
        honoured = Some((payment, received));
    }
    tx.commit().await?;
    if let Some((payment, received)) = honoured {
        received
            .emit(state)
            .await
            .map_err(TransitionError::Failed)?;
        cheque_money::allocate_to_links(state, &cheque, &payment, payment.amount, Some(user.id))
            .await?;
    }
    // `Transitioned<Cheque>` -> the fee, `ChequeBounced`, the notices and the
    // tasks (crate::cheque_events).
    outcome.emit(state).await.map_err(TransitionError::Failed)?;
    reload(state, &cheque).await
}

/// Adds the bounce fee for `bounce` of the received `cheque`, when a fee is
/// set (§7): the charge through wl-invoicing, due on the bounce date; the
/// accountant gets a task when none could be made. The return keeps the fee,
/// and the cheque the total.
async fn add_fee(state: &AppState, cheque: &Cheque, bounce: &ChequeReturn) -> Result<()> {
    let config = config_of(state);
    let Some(fee) = config.bounce_fee.clone() else {
        return Ok(());
    };
    let charge = BounceFeeCharge {
        cheque: cheque.cheque_id(),
        party: cheque.party(),
        legal_entity_id: cheque.legal_entity_id,
        agreement: cheque.agreement(),
        charge_type: fee.charge_type.clone(),
        amount: fee.amount,
        currency: cheque.currency.clone(),
        vat_category: config.bounce_fee_vat,
        due: bounce.returned_on,
    };
    let made = bounce_fees(state).add(state, charge).await?;
    let now = renox::db::now();
    ChequeReturn::query()
        .where_(ChequeReturn::ID.eq(bounce.id))
        .update(
            &state.db,
            &[
                ("fee_charge_type", &Some(fee.charge_type.clone())),
                ("fee_amount", &fee.amount),
                (
                    "fee_subject_type",
                    &made.as_ref().map(|c| c.subject_type.clone()),
                ),
                ("fee_subject_id", &made.as_ref().map(|c| c.subject_id)),
                ("updated_at", &now),
            ],
        )
        .await?;
    Cheque::query()
        .where_(Cheque::ID.eq(cheque.id))
        .update(
            &state.db,
            &[
                ("bounce_fee_total", &(cheque.bounce_fee_total + fee.amount)),
                ("updated_at", &now),
            ],
        )
        .await?;
    if made.is_none() {
        // Invoicing cannot schedule the charge: the accountant raises it.
        let task = FollowupTask::raise_bounce_fee(cheque, fee.amount);
        if let Some(task) = FollowupTask::create_once(&state.db, task).await? {
            notices::cheque_task(state, cheque, &task).await?;
        }
    }
    Ok(())
}

/// Creates `task` unless the cheque has an open one of its kind, and tells
/// the staff.
async fn make_task(state: &AppState, cheque: &Cheque, task: FollowupTask) -> Result<()> {
    if let Some(task) = FollowupTask::create_once(&state.db, task).await? {
        notices::cheque_task(state, cheque, &task).await?;
    }
    Ok(())
}

/// After a committed `bounce` of `cheque_id`: see the module's documentation.
///
/// # Errors
///
/// The database's errors, or a notice that failed.
pub(crate) async fn after_bounce(state: &AppState, cheque_id: i64) -> Result<()> {
    let cheque = Cheque::find_or_404(&state.db, cheque_id).await?;
    let Some(bounce) = ChequeReturn::latest(&state.db, cheque.id).await? else {
        return Ok(());
    };
    if !cheque.is_received() {
        // An issued cheque is replaced: no certificate, fee, event or hold.
        return make_task(state, &cheque, FollowupTask::replace_cheque(&cheque)).await;
    }
    add_fee(state, &cheque, &bounce).await?;
    let cheque = Cheque::find_or_404(&state.db, cheque_id).await?;
    let linked = cheque.linked(&state.db).await?;
    state
        .emit(ChequeBounced {
            cheque: cheque.cheque_id(),
            party: cheque.party(),
            credit_group: None,
            amount: cheque.amount,
            honoured_amount: cheque.honoured_amount,
            open_amount: cheque.open_amount,
            currency: cheque.currency.clone(),
            reason: bounce.reason.clone(),
            presentation: bounce.presentation,
            linked,
        })
        .await?;
    notices::cheque_bounced(state, &cheque, &bounce).await?;
    make_task(state, &cheque, FollowupTask::follow_up_cheque(&cheque)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deposited() -> Cheque {
        Cheque {
            id: 1,
            amount: 2_000_000,
            open_amount: 2_000_000,
            presentations: 1,
            status: ChequeStatus::Deposited,
            ..Default::default()
        }
    }

    fn request(reason: &str, certificate: Option<&str>, honoured: i64) -> BounceInput {
        BounceInput::of(&BounceRequest {
            reason: reason.into(),
            certificate: certificate.map(str::to_owned),
            honoured,
        })
    }

    #[test]
    fn bounce_leaves_deposited_only() {
        let machine = Cheque::machine();
        let found = machine.find(BOUNCE).expect("transition");
        assert!(found.check_from(ChequeStatus::Deposited).is_ok());
        for other in [
            ChequeStatus::Received,
            ChequeStatus::DueForDeposit,
            ChequeStatus::Bounced,
            ChequeStatus::Cleared,
        ] {
            assert!(found.check_from(other).is_err(), "{other:?}");
        }
        assert_eq!(found.guard_names(), ["return_recorded"]);
    }

    #[test]
    fn a_received_cheque_needs_a_reason_and_its_certificate() {
        let cheque = deposited();
        let ok = request("insufficient funds", Some("CERT-77"), 600_000);
        assert_eq!(input_problem(&cheque, Some(&ok)), None);
        assert!(input_problem(&cheque, None).is_some());
        let blank = request("  ", Some("CERT-77"), 0);
        assert!(input_problem(&cheque, Some(&blank)).is_some());
        let no_certificate = request("insufficient funds", None, 0);
        let message = input_problem(&cheque, Some(&no_certificate)).expect("refused");
        assert!(message.contains("certificate"), "{message}");
        let blank_certificate = request("insufficient funds", Some("   "), 0);
        assert!(input_problem(&cheque, Some(&blank_certificate)).is_some());
    }

    #[test]
    fn an_issued_cheque_needs_the_reason_only() {
        let issued = Cheque {
            direction: wl_kernel::payments::Direction::Out,
            ..deposited()
        };
        let no_certificate = request("insufficient funds", None, 0);
        assert_eq!(input_problem(&issued, Some(&no_certificate)), None);
        assert!(input_problem(&issued, Some(&request("", None, 0))).is_some());
    }

    #[test]
    fn the_honoured_amount_stays_below_what_was_presented() {
        let cheque = deposited();
        let at = |honoured| request("insufficient funds", Some("C"), honoured);
        assert_eq!(input_problem(&cheque, Some(&at(0))), None);
        assert_eq!(input_problem(&cheque, Some(&at(1_999_999))), None);
        assert!(input_problem(&cheque, Some(&at(2_000_000))).is_some());
        assert!(input_problem(&cheque, Some(&at(2_500_000))).is_some());
        assert!(input_problem(&cheque, Some(&at(-1))).is_some());
        // After a re-presentation only the open amount was presented.
        let again = Cheque {
            open_amount: 1_400_000,
            ..cheque
        };
        assert!(input_problem(&again, Some(&at(1_400_000))).is_some());
        assert_eq!(input_problem(&again, Some(&at(1_000_000))), None);
    }

    #[test]
    fn the_request_is_trimmed() {
        let input = request("  account closed ", Some("  CERT "), 0);
        assert_eq!(input.reason, "account closed");
        assert_eq!(input.certificate.as_deref(), Some("CERT"));
        assert_eq!(request("x", Some("  "), 0).certificate, None);
    }

    #[renox::test]
    async fn the_guard_reads_the_run_input() {
        let machine = Cheque::machine();
        let found = machine.find(BOUNCE).expect("transition");
        let ctx = |input: &BounceInput| {
            TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).expect("json"))
        };
        let cheque = deposited();
        let good = request("insufficient funds", Some("C"), 0);
        assert!(found.check_guards(&cheque, &ctx(&good)).await.is_ok());
        let bad = request("insufficient funds", None, 0);
        assert!(matches!(
            found.check_guards(&cheque, &ctx(&bad)).await,
            Err(TransitionError::Guard(_))
        ));
        assert!(matches!(
            found.check_guards(&cheque, &TransitionCtx::new(None)).await,
            Err(TransitionError::Guard(_))
        ));
    }
}
