//! Releasing the vehicle of a declined or cancelled job card (08 §7.1 *Job
//! card*, *Release vehicle*, `AUT.ACL.17`, `AUT.ACL.21`; BL-AUT-01).
//!
//! Releasing is an action on a `declined` or `cancelled` job card, not a
//! transition (on `delivered` it is part of `deliver`). It records when
//! (Asia/Dubai), who took the vehicle, the condition (*safe*, or *as is*, which
//! needs the customer's signature) and the signature. The vehicle leaves when
//! the invoice is paid or the customer is on account (`payment_cleared`); for an
//! insurance job `AUT.ACL.17` applies: the excess and the private items of the
//! claim are paid, or, after a total loss, the insurer's written notice is
//! enough, paid or not.
//!
//! Decisions:
//!
//! - *On account* is a flag on the job card that a holder of `job_cards.approve`
//!   sets for a customer who pays on credit terms (the credit terms themselves
//!   are wl-sales' and not a kernel contract yet); a job billed in a fleet
//!   contract's monthly invoice is on account.
//! - A job card billed nothing (a waived bill, a decline with no fee) has
//!   nothing to pay.
//! - The storage fee is invoiced at release ([`crate::storage::bill_release`],
//!   `AUT.RUL.18`, BL-AUT-05).

use renox::db::now;
use renox::prelude::*;
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus, invoicer};

use crate::claim::InsuranceClaim;
use crate::db::refused;
use crate::job_card::{JobCard, JobCardStatus};
use crate::money_text;
use crate::permissions;
use crate::texts::english;

/// The vehicle leaves drivable and safe.
pub const SAFE: &str = "safe";
/// The vehicle leaves as it is: not drivable or unsafe, with the customer's
/// signature.
pub const AS_IS: &str = "as_is";

/// What the advisor records when the vehicle is released.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Release {
    /// Who takes the vehicle.
    pub released_to: String,
    /// [`SAFE`] or [`AS_IS`].
    pub condition: String,
    /// The customer's signature, for a release as is.
    pub signature: Option<String>,
}

impl Release {
    /// The vehicle goes safe to `released_to`.
    pub fn safe(released_to: &str) -> Self {
        Release {
            released_to: released_to.trim().to_owned(),
            condition: SAFE.to_owned(),
            signature: None,
        }
    }

    /// The vehicle goes as it is to `released_to`, who signs.
    pub fn as_is(released_to: &str, signature: &str) -> Self {
        Release {
            released_to: released_to.trim().to_owned(),
            condition: AS_IS.to_owned(),
            signature: Some(signature.trim().to_owned()).filter(|s| !s.is_empty()),
        }
    }
}

/// Why the vehicle of `card` cannot leave yet, in words; `None` when payment
/// is cleared (`payment_cleared`): paid, on account, nothing to pay, or the
/// insurance rule (`AUT.ACL.17`).
///
/// # Errors
///
/// The database's and the invoicer's errors.
pub async fn payment_problem(state: &AppState, card: &JobCard) -> Result<Option<String>> {
    if card.on_account || crate::fleet_billing::bills_monthly(&state.db, card.id).await? {
        return Ok(None);
    }
    let claims = InsuranceClaim::of_job_card(&state.db, card.id).await?;
    if let Some(claim) = claims
        .into_iter()
        .find(|claim| claim.status.funds_job_card())
    {
        // After a total loss the insurer's written notice is enough.
        if claim.total_loss_notice.is_some() || claim.excess <= 0 {
            return Ok(None);
        }
        let cleared = match claim.customer_invoice_id {
            Some(invoice) => invoice_cleared(state, invoice).await?,
            None => false,
        };
        if cleared {
            return Ok(None);
        }
        let number = claim.number();
        return Ok(Some(english(
            "auto.release.problem.claim_unpaid",
            &[("claim", &number)],
        )));
    }
    match card.invoice_id {
        Some(invoice) => {
            if invoice_cleared(state, invoice).await? {
                return Ok(None);
            }
            let balance = match invoicer(state)
                .find(&state.db, InvoiceId(invoice))
                .await?
            {
                Some(summary) => summary.balance,
                None => card.billed_net,
            };
            let open = money_text(balance);
            Ok(Some(english(
                "auto.release.problem.unpaid",
                &[("balance", &open)],
            )))
        }
        None if card.billed_net <= 0 => Ok(None),
        None => Ok(Some(english("auto.release.problem.not_invoiced", &[]))),
    }
}

/// Whether invoice `id` is paid (or credited in full).
async fn invoice_cleared(state: &AppState, id: i64) -> Result<bool> {
    let found = invoicer(state).find(&state.db, InvoiceId(id)).await?;
    Ok(found.is_some_and(|summary| {
        matches!(summary.status, InvoiceStatus::Paid | InvoiceStatus::Credited)
    }))
}

/// Releases the vehicle of the declined or cancelled job card `id`.
///
/// # Errors
///
/// A 403 without `job_cards.advise`; a 422 in words for a job card that is not
/// declined or cancelled, one released already, a blank recipient, a condition
/// that is not safe or as is, a release as is without the customer's
/// signature, or an invoice that is not paid for a customer who is not on
/// account.
pub async fn release(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    request: Release,
) -> Result<JobCard> {
    if !by.has_permission(permissions::JOB_CARDS_ADVISE) {
        return Err(Error::Forbidden);
    }
    let card = JobCard::find_or_404(&state.db, id).await?;
    if !matches!(
        card.status,
        JobCardStatus::Declined | JobCardStatus::Cancelled
    ) {
        return Err(refused(english("auto.release.problem.not_closed", &[])));
    }
    if card.released_at.is_some() {
        return Err(refused(english("auto.release.problem.released", &[])));
    }
    let recipient = request.released_to.trim().to_owned();
    if recipient.is_empty() {
        return Err(refused(english("auto.release.problem.recipient", &[])));
    }
    if request.condition != SAFE && request.condition != AS_IS {
        return Err(refused(english("auto.release.problem.condition", &[])));
    }
    let signature = request
        .signature
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    if request.condition == AS_IS && signature.is_none() {
        return Err(refused(english("auto.release.problem.signature", &[])));
    }
    if let Some(problem) = payment_problem(state, &card).await? {
        return Err(refused(problem));
    }
    let stamp = now();
    let claimed = JobCard::query()
        .where_(JobCard::ID.eq(id))
        .where_(JobCard::RELEASED_AT.is_null())
        .update(
            &state.db,
            &[
                ("released_at", &Some(stamp)),
                ("released_to", &Some(recipient)),
                ("release_condition", &Some(request.condition.clone())),
                ("release_signature", &signature),
                ("released_by", &Some(by.id)),
                ("updated_at", &stamp),
            ],
        )
        .await?;
    if claimed == 0 {
        return Err(refused(english("auto.release.problem.released", &[])));
    }
    // The storage fee of the days the vehicle stayed (`AUT.RUL.18`).
    crate::storage::bill_release(state, id).await?;
    JobCard::find_or_404(&state.db, id).await
}

/// A holder of `job_cards.approve` says whether the customer of job card `id`
/// is on account: the vehicle then leaves unpaid.
///
/// # Errors
///
/// A 403 without `job_cards.approve`.
pub async fn set_on_account(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    on_account: bool,
) -> Result<JobCard> {
    if !by.has_permission(permissions::JOB_CARDS_APPROVE) {
        return Err(Error::Forbidden);
    }
    let card = JobCard::find_or_404(&state.db, id).await?;
    JobCard::query()
        .where_(JobCard::ID.eq(card.id))
        .update(
            &state.db,
            &[("on_account", &on_account), ("updated_at", &now())],
        )
        .await?;
    JobCard::find_or_404(&state.db, id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_release_is_built_safe_or_as_is() {
        let safe = Release::safe(" Mona Hassan ");
        assert_eq!(safe.released_to, "Mona Hassan");
        assert_eq!(safe.condition, SAFE);
        assert_eq!(safe.signature, None);
        let as_is = Release::as_is("Mona", " ");
        assert_eq!(as_is.condition, AS_IS);
        assert_eq!(as_is.signature, None, "a blank signature is none");
        assert_eq!(
            Release::as_is("Mona", "M. Hassan").signature.as_deref(),
            Some("M. Hassan")
        );
    }
}
