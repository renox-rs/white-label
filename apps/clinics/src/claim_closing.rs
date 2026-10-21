//! Closing a claim that missed the payer's first-submission window, or that
//! the payer will not pay (05 §3.3 `CLN.ACL.16` and `FIN.ACL.14`, §4.3 "When
//! it goes wrong" row *First-submission window missed*, §7.1 *Claim*
//! `write_off`; BL-CLN-09).
//!
//! [`write_off`] is the clinic manager's (`claims.writeoff`). It closes the
//! claim the way its payer bill closes: the open amount is **written off**
//! (payer bill `write_off`), or **moved to the patient** (payer bill
//! `transfer_to_customer`) only when the payer contract allows billing the
//! patient for a share the clinic did not submit in time, a flag that is off
//! by default and that only the owner sets ([`set_missed_window_billing`]).
//!
//! - From `draft` or `ready` it is allowed only once the payer's
//!   first-submission window has passed (today is after its last day,
//!   Asia/Dubai) and the payer does not accept late claims; for a payer that
//!   does, `validate` takes a late reason instead. The claim is then
//!   reported as *not submitted in time* ([`not_submitted_in_time`]).
//! - From `partially_paid` or `denied` the window does not matter.
//! - Above the limit of the payer agreement (AED 1,000) the payer bill holds
//!   the write-off for the owner's approval (`FIN.ACL.14`): the claim stays as
//!   it is and shows the request ([`Closed::Held`]); when the owner approves,
//!   the payer bill closes and the claim follows with its system transition
//!   ([`on_bill_closed`]). A rejection leaves both unchanged. If the open
//!   amount changes while the request waits, the payer bill fails the
//!   request.
//!
//! Decisions:
//!
//! - **Moving to the patient** applies the same contract flag from every
//!   state the claim can be closed from, not only to a missed window: the
//!   spec's `write_off` effect reads "balance moved to the patient where the
//!   contract allows". A move needs no approval (the payer bill gives none);
//!   the patient is told by the payer bill, with the amount and without the
//!   codes. The visit's patient total rises, and its payer total falls, by
//!   what moved.
//! - **The windows** are the payer contract's; the payer bill checks the
//!   payer agreement's own (the two are kept equal) and its refusal comes
//!   back unchanged.
//! - **The owner approves**, not the clinic manager: the approval policy of
//!   the write-off is the owner's alone in this app (`register_policy`), so a
//!   second clinic manager cannot approve above the limit.

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::COMPANY_ADMIN_ROLE;
use wl_kernel::approvals::PolicyDefault;
use wl_kernel::documents::{dubai_date, today};
use wl_kernel::payer_bills::{PayerAnswer, PayerBillClosed, PayerBillOutcome, payer_bills};

use crate::claim::{Claim, ClaimStatus};
use crate::claim_machine::{ClaimInput, WRITE_OFF, WRITE_OFF_APPROVED, clean};
use crate::claims::{attempt_deadline, contract_of, dry_run};
use crate::contract::PayerContract;
use crate::permissions::{self, OWNER_ROLE};
use crate::texts::english;
use crate::visit::Visit;

/// What `held_kind` says while a write-off waits for the owner.
pub const HELD_WRITE_OFF: &str = "write_off";

/// The approval action of the payer bill's write-off (`FIN.ACL.14`), as
/// wl-invoicing names it (`invoicing.payer_write_off`).
pub const WRITE_OFF_ACTION: &str = "invoicing.payer_write_off";

/// What closing a claim came to.
#[derive(Debug, Clone, PartialEq)]
pub enum Closed {
    /// The payer bill closed and the claim is `written_off`.
    Done(Claim),
    /// The write-off is above the limit: the owner must approve first. The
    /// claim is as it was and shows the request.
    Held(Claim),
}

/// What the KPI reports as *not submitted in time*.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NotSubmitted {
    /// How many claims.
    pub claims: i64,
    /// What they were worth when closed, in fils.
    pub amount: i64,
}

/// The facts the guards of `write_off` need, gathered for `claim` as `by` at
/// `at`.
///
/// # Errors
///
/// The database's and the payer bills' errors.
pub async fn facts(
    state: &AppState,
    claim: &Claim,
    at: DateTime,
    by: Option<i64>,
) -> Result<ClaimInput> {
    let contract = contract_of(&state.db, claim).await?;
    let open = match payer_bills(state)
        .find(&state.db, claim.payer_bill())
        .await?
    {
        Some(bill) => bill.open,
        None => (claim.claimed - claim.paid).max(0),
    };
    Ok(ClaimInput {
        on: Some(dubai_date(at)),
        by,
        deadline: attempt_deadline(claim, contract.as_ref()),
        accepts_late: contract.as_ref().is_some_and(|c| c.accepts_late_claims),
        to_patient: contract.as_ref().is_some_and(|c| c.bill_missed_to_patient),
        open,
        ..ClaimInput::default()
    })
}

/// Closes claim `id` as `by` (who needs `claims.writeoff`) with `reason`: the
/// open amount is written off, or moved to the patient where the payer
/// contract allows it. Above the payer agreement's write-off limit the
/// answer is [`Closed::Held`] and nothing changes until the owner approves.
///
/// # Errors
///
/// `Forbidden` without `claims.writeoff`; `NotFrom` unless the claim is
/// `draft`, `ready`, `partially_paid` or `denied`; `Guard` (422) without a
/// reason, with nothing open, while an advice or a write-off waits, from
/// `draft` or `ready` while the first-submission window is open or when the
/// payer accepts late claims, and the payer bill's refusals (the payer
/// agreement does not allow billing the patient, a tax document that cannot
/// be made).
pub async fn write_off(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    reason: &str,
    at: DateTime,
) -> std::result::Result<Closed, TransitionError> {
    let claim = Claim::find_or_404(&state.db, id).await?;
    let mut input = facts(state, &claim, at, Some(by.id)).await?;
    input.reason = clean(&Some(reason.to_owned()));
    dry_run(&claim, WRITE_OFF, by, &input).await?;
    if claim.is_held() {
        return Err(TransitionError::Guard(english(
            "clinics.claim.problem.held",
            &[],
        )));
    }
    let text = reason.trim();
    if input.to_patient {
        payer_bills(state)
            .transfer_to_customer(state, claim.payer_bill(), text, by)
            .await?;
    } else {
        match payer_bills(state)
            .write_off(state, claim.payer_bill(), text, by)
            .await?
        {
            PayerAnswer::Done => {}
            PayerAnswer::Pending { request } => {
                Claim::query()
                    .where_(Claim::ID.eq(id))
                    .update(
                        &state.db,
                        &[
                            ("held_kind", &Some(HELD_WRITE_OFF.to_owned())),
                            ("held_request_id", &Some(request)),
                            ("held_by", &Some(by.id)),
                            ("held_date", &input.on),
                            ("held_reference", &None::<String>),
                            ("held_result", &None::<String>),
                            ("close_reason", &input.reason),
                            ("updated_at", &renox::db::now()),
                        ],
                    )
                    .await?;
                return Ok(Closed::Held(Claim::find_or_404(&state.db, id).await?));
            }
        }
    }
    let _ = claim
        .transition(WRITE_OFF)
        .by(by)
        .input(&input)
        .run(state)
        .await?;
    if input.to_patient {
        // The patient's share of the visit rises by what moved to them.
        if let Some(visit) = Visit::find(&state.db, claim.visit_id).await? {
            Visit::query()
                .where_(Visit::ID.eq(visit.id))
                .update(
                    &state.db,
                    &[
                        ("patient_total", &(visit.patient_total + input.open)),
                        ("payer_total", &(visit.payer_total - input.open).max(0)),
                        ("updated_at", &renox::db::now()),
                    ],
                )
                .await?;
        }
    }
    Ok(Closed::Done(Claim::find_or_404(&state.db, id).await?))
}

/// `PayerBillClosed` after the owner approved a write-off the claim waited
/// for: the claim follows with `write_off_approved`. A close that no
/// write-off of the claim waits for (the user's own, which the service
/// finishes, or another module's) is left alone.
///
/// # Errors
///
/// The database's errors and a refusal of the transition other than the
/// claim being in a state that does not take it.
pub async fn on_bill_closed(state: &AppState, event: &PayerBillClosed) -> Result {
    if event.outcome != PayerBillOutcome::WrittenOff {
        return Ok(());
    }
    let Some(claim) = Claim::of_payer_bill(&state.db, event.payer_bill.0).await? else {
        return Ok(());
    };
    if claim.held_kind.as_deref() != Some(HELD_WRITE_OFF) || claim.held_request_id.is_none() {
        return Ok(());
    }
    let input = ClaimInput {
        on: Some(today()),
        by: claim.held_by,
        reason: claim.close_reason.clone(),
        open: event.open,
        ..ClaimInput::default()
    };
    match claim
        .transition(WRITE_OFF_APPROVED)
        .by_system()
        .input(&input)
        .run(state)
        .await
    {
        Ok(_) | Err(TransitionError::NotFrom(_)) => Ok(()),
        Err(other) => Err(other.into_error()),
    }
}

/// Sets, on payer contract `contract`, whether the payer share of a claim the
/// clinic did not submit in time may be billed to the patient (`CLN.ACL.16`,
/// `⚠ open: lawyer L-34`). Off by default; only the owner sets it.
///
/// # Errors
///
/// `Forbidden` without `contracts.missed_window_billing`; a 404 for an
/// unknown contract.
pub async fn set_missed_window_billing(
    state: &AppState,
    by: &AuthUser,
    contract: i64,
    allowed: bool,
) -> Result<PayerContract> {
    if !by.has_permission(permissions::MISSED_WINDOW_BILLING) {
        return Err(Error::Forbidden);
    }
    let mut found = PayerContract::find_or_404(&state.db, contract).await?;
    found.bill_missed_to_patient = allowed;
    found.save(&state.db).await?;
    Ok(found)
}

/// What the denial KPI reports as *not submitted in time*: the claims closed
/// from `draft` or `ready` after the window, between `from` and `to` (the
/// day of the close, both included; open ended when `None`), with the amounts
/// written off or moved.
///
/// # Errors
///
/// The database's errors.
pub async fn not_submitted_in_time(
    db: &Db,
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
) -> Result<NotSubmitted> {
    let closed = Claim::query()
        .where_(Claim::NOT_SUBMITTED_IN_TIME.eq(true))
        .get(db)
        .await?;
    let mut found = NotSubmitted::default();
    for claim in closed {
        let Some(on) = claim.closed_on else {
            continue;
        };
        if from.is_some_and(|f| on < f) || to.is_some_and(|t| on > t) {
            continue;
        }
        found.claims += 1;
        found.amount += claim.close_amount;
    }
    Ok(found)
}

/// Whether claim `claim` can be closed now by `write_off`, by the states and
/// the window alone (the page offers the action only then).
pub fn closable(claim: &Claim, input: &ClaimInput) -> bool {
    matches!(
        claim.status,
        ClaimStatus::Draft | ClaimStatus::Ready | ClaimStatus::PartiallyPaid | ClaimStatus::Denied
    ) && crate::claim_machine::window_missed(claim, input).is_ok()
        && input.open > 0
        && !claim.is_held()
}

/// The owner's alone decides a write-off above the limit: replaces the
/// default approval policy of the payer bill's write-off (any holder of
/// `payer_bills.write_off`, the clinic manager among them) by one step for the
/// owner role and the company administrators.
pub(crate) fn register_policy() {
    PolicyDefault::new(WRITE_OFF_ACTION)
        .amount_above(0)
        .step([OWNER_ROLE, COMPANY_ADMIN_ROLE])
        .expiry_hours(Some(168))
        .register();
}

/// Registers the owner's approval policy and the listener that follows the
/// payer bill's close.
pub(crate) fn register(app: &mut Registry) {
    register_policy();
    app.listen(|event: PayerBillClosed, state: AppState| async move {
        on_bill_closed(&state, &event).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_claim_is_closable_from_four_states_and_only_with_something_open() {
        let input = ClaimInput {
            open: 60_000,
            deadline: Some("2026-10-01".parse().unwrap()),
            on: Some("2026-10-05".parse().unwrap()),
            ..ClaimInput::default()
        };
        let draft = Claim::default();
        assert!(closable(&draft, &input));
        let paid = Claim {
            status: ClaimStatus::Paid,
            ..Claim::default()
        };
        assert!(!closable(&paid, &input));
        let nothing = ClaimInput {
            open: 0,
            ..input.clone()
        };
        assert!(!closable(&draft, &nothing));
        let held = Claim {
            held_request_id: Some(3),
            ..Claim::default()
        };
        assert!(!closable(&held, &input));
        let early = ClaimInput {
            on: Some("2026-09-30".parse().unwrap()),
            ..input
        };
        assert!(!closable(&draft, &early));
    }
}
