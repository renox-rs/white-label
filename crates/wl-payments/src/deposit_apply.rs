//! Forfeiting a deposit by applying it to the deductions (wl-payments §4
//! *Deposit* `forfeit`, 01 Property §4.3 row "Deductions exceed the deposit",
//! issue #267).
//!
//! When the approved deductions of a lease exceed its deposit, the vertical
//! asks [`apply`] (through the kernel's
//! [`Deposits::apply_to_deductions`](wl_kernel::deposits::Deposits)) to keep the
//! deposit in full. The deposit is forfeited with the reason *applied to
//! deductions* like any forfeiture: the same waits apply (no dispute open, the
//! window of the published version passed) and the approver is another user.
//! Then the deposit is applied to the unpaid invoices the vertical named, in
//! their order (oldest first), each taking what it is open for until the
//! deposit is used, each as a payment of method *deposit applied* (recorded as a set-off,
//! by the system, whatever the enabled methods are) allocated to the invoice.
//! What is left of the held amount covers the damage lines; the vertical
//! invoices the damages that the deposit did not cover.
//!
//! The total deductions (the invoices' open amounts and the damage lines of the
//! latest published version) must reach the deposit; below it the reason is
//! refused and the settlement goes through `refund`.
//!
//! The invoices are checked before the deposit is forfeited, so a refusal
//! changes nothing. The forfeiture and the payments are separate transactions:
//! if an allocation fails after the forfeiture, the deposit stays `forfeited`
//! and the error is returned, because a forfeiture cannot be undone; the
//! payments recorded so far stay and the accountant allocates the rest by hand.

use renox::prelude::*;
use renox::state_machine::TransitionError;
use wl_kernel::deposits::{APPLIED_TO_DEDUCTIONS, ApplyDeposit, ApplyToInvoice, DepositApplied};
use wl_kernel::invoicing::invoicer;
use wl_kernel::payments::{
    AllocationTarget, PaymentId, PaymentMethod, PaymentSource, RecordPayment,
};

use crate::deposit::{Deposit, DepositStatus, ForfeitRequest, forfeit};
use crate::deposit_deductions::DepositDeduction;

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// The shares of a deposit holding `held` fils over the invoices of `request`
/// in their order: each takes what it is open for, until the deposit is used.
pub(crate) fn shares_of(request: &ApplyDeposit, held: i64) -> Vec<ApplyToInvoice> {
    let mut left = held.max(0);
    let mut shares = Vec::new();
    for invoice in &request.rent {
        let take = left.min(invoice.amount);
        if take > 0 {
            shares.push(ApplyToInvoice {
                invoice: invoice.invoice,
                amount: take,
            });
            left -= take;
        }
    }
    shares
}

/// Why `request` cannot be applied to a deposit holding `held` fils with
/// damage lines of `damages` fils, if it cannot: every invoice is open for
/// something, and the total deductions (what the invoices are open for and the
/// damage lines) reach the deposit.
pub(crate) fn request_problem(request: &ApplyDeposit, held: i64, damages: i64) -> Option<String> {
    if request.rent.iter().any(|invoice| invoice.amount <= 0) {
        return Some("Each invoice must be open for more than 0.".to_owned());
    }
    let unpaid: i64 = request.rent.iter().map(|invoice| invoice.amount).sum();
    if unpaid + damages < held {
        return Some(
            "The total deductions are below the deposit: refund the rest instead of applying it to the deductions."
                .to_owned(),
        );
    }
    None
}

/// `user` forfeits the deposit being settled for `request.source` and applies
/// it to the invoices of `request`.
///
/// # Errors
///
/// 422 when the source has no deposit being settled, for a request as
/// [`request_problem`] says, for an invoice that is not the party's or whose
/// balance is below its share, and the refusals of `forfeit` (no reason is
/// needed: the reason is fixed; no dispute may be open and the window of the
/// published version must have passed). Nothing is written then.
pub(crate) async fn apply(
    state: &AppState,
    user: &AuthUser,
    request: ApplyDeposit,
) -> Result<DepositApplied> {
    let deposit = Deposit::query()
        .where_(Deposit::SOURCE_TYPE.eq(request.source.subject_type.as_str()))
        .where_(Deposit::SOURCE_ID.eq(request.source.subject_id))
        .where_(Deposit::STATUS.eq(DepositStatus::SettlementPending))
        .order_by_desc(Deposit::ID)
        .first(&state.db)
        .await?
        .ok_or_else(|| {
            refused(format!(
                "{} has no deposit being settled.",
                request.source
            ))
        })?;
    let damages: i64 = match DepositDeduction::latest(&state.db, deposit.id).await? {
        Some(latest) => latest
            .lines(&state.db)
            .await?
            .iter()
            .map(|line| line.amount)
            .sum(),
        None => 0,
    };
    if let Some(problem) = request_problem(&request, deposit.held_amount, damages) {
        return Err(refused(problem));
    }
    for share in &request.rent {
        let invoice = invoicer(state)
            .find(&state.db, share.invoice)
            .await?
            .ok_or(Error::NotFound)?;
        if invoice.party != deposit.party() {
            return Err(refused(format!(
                "Invoice {} is not the deposit holder's.",
                share.invoice
            )));
        }
        if invoice.balance < share.amount {
            return Err(refused(format!(
                "Invoice {} is open for less than the request says.",
                share.invoice
            )));
        }
    }
    let forfeited = forfeit(
        state,
        user,
        &deposit,
        &ForfeitRequest {
            reason: APPLIED_TO_DEDUCTIONS.to_owned(),
            approver: Some(request.approver),
            approver_vetted: true,
        },
    )
    .await
    .map_err(TransitionError::into_error)?;
    let shares = shares_of(&request, forfeited.held_amount);
    let mut applied = 0;
    for share in &shares {
        let payment = crate::record::record_applied(
            state,
            Some(user.id),
            &RecordPayment::new(
                deposit.party(),
                deposit.legal_entity_id,
                PaymentMethod::SetOff,
                share.amount,
            )
            .currency(&deposit.currency)
            .reference("Deposit applied")
            .source(PaymentSource::System),
        )
        .await
        .map_err(TransitionError::into_error)?;
        crate::allocate::allocate(
            state,
            PaymentId(payment.id),
            AllocationTarget::Invoice(share.invoice),
            share.amount,
            Some(user.id),
        )
        .await?;
        applied += share.amount;
    }
    let version = DepositDeduction::latest(&state.db, forfeited.id)
        .await?
        .map_or(0, |latest| latest.version);
    Ok(DepositApplied {
        deposit: forfeited.deposit_id(),
        party: forfeited.party(),
        legal_entity_id: forfeited.legal_entity_id,
        held: forfeited.held_amount,
        to_invoices: applied,
        shares,
        kept: forfeited.held_amount - applied,
        version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::Subject;
    use wl_kernel::invoicing::InvoiceId;

    fn request(amounts: &[i64]) -> ApplyDeposit {
        ApplyDeposit {
            source: Subject::new("property.lease", 9),
            approver: 4,
            rent: amounts
                .iter()
                .enumerate()
                .map(|(index, amount)| ApplyToInvoice {
                    invoice: InvoiceId(i64::try_from(index).unwrap_or(0) + 1),
                    amount: *amount,
                })
                .collect(),
        }
    }

    #[test]
    fn the_deposit_pays_the_oldest_invoice_first() {
        let shares = shares_of(&request(&[350_000, 350_000]), 500_000);
        let amounts: Vec<i64> = shares.iter().map(|s| s.amount).collect();
        assert_eq!(amounts, vec![350_000, 150_000]);
        let shares = shares_of(&request(&[200_000]), 500_000);
        assert_eq!(shares.len(), 1);
        assert_eq!(shares[0].amount, 200_000);
        assert!(shares_of(&request(&[]), 500_000).is_empty());
    }

    #[test]
    fn the_total_deductions_must_reach_the_deposit() {
        assert_eq!(request_problem(&request(&[200_000]), 500_000, 450_000), None);
        assert_eq!(request_problem(&request(&[]), 500_000, 500_000), None);
        assert!(request_problem(&request(&[]), 500_000, 120_000).is_some());
        assert!(request_problem(&request(&[0]), 500_000, 600_000).is_some());
    }
}
