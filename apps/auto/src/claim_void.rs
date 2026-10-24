//! A claim registered in error is voidable (08 §4.7 last row, §7.1 *Insurance
//! repair claim* `void`, `PLT.ACL.09`; BL-AUT-10): a claim for the wrong
//! vehicle, or registered twice, is voided from `registered` while nothing was
//! sent to the insurer. The kernel's void pattern gives the `void` and
//! `void_approved` transitions, the reason, the Void record that keeps the
//! claim's number, and the `RecordVoided` event.
//!
//! Decisions:
//!
//! - **Nothing sent** is the machine's guard `nothing_sent`: any message the
//!   workshop queued to the insurer for the claim ([`crate::insurer_mail`]), the
//!   survey that was booked and the estimate that was submitted stamp
//!   `sent_to_insurer_at`, and a stamped claim is cancelled, not voided.
//! - A claim posts no money and no stock, so the void needs no approval (the
//!   kernel's default policy matches only a record with posted effects).
//! - **The job card is unlinked** by the reverse handler, in the void's
//!   transaction: the claim forgets the job card (`job_card_id` is cleared and
//!   the card is kept as `unlinked_job_card_id` for the record), and every line
//!   of the job card is the customer's again (`payer`). A claim is voided only
//!   from `registered`, before any estimate was sent to the insurer or approved,
//!   so the job card has no line the insurer approved; its lines need the
//!   customer's own approval through the job card's `send_estimate` and
//!   `approve`. The job card keeps its accident flag; the police report
//!   attached to the voided claim no longer counts for it (BL-AUT-07), because
//!   the report is found through the claim that is linked to the job card.
//! - The claim's documents (Emirates ID, licence) stay attached to the voided
//!   claim, under `AUT.ACL.18`.

use renox::db::{Transaction, now};
use renox::prelude::*;
use wl_kernel::machines::BoxFuture;
use wl_kernel::voids::{Effect, EffectKind, Reverse, ReverseError, Voidable};

use crate::claim::{INSURANCE_CLAIM_SUBJECT_TYPE, InsuranceClaim};
use crate::permissions;

/// What there is to reverse: nothing (a claim posts neither money nor stock).
fn nothing<'a>(_tx: &'a mut Transaction, _id: i64) -> BoxFuture<'a, Result<Vec<Effect>>> {
    Box::pin(async { Ok(Vec::new()) })
}

/// Unlinks the job card of claim `id` and makes its lines the customer's.
fn unlink<'a>(
    tx: &'a mut Transaction,
    id: i64,
) -> BoxFuture<'a, Result<Vec<Effect>, ReverseError>> {
    Box::pin(async move {
        let claim = InsuranceClaim::query()
            .where_(InsuranceClaim::ID.eq(id))
            .first(&mut *tx)
            .await?;
        let Some(job_id) = claim.and_then(|claim| claim.job_card_id) else {
            return Ok(Vec::new());
        };
        let none: Option<i64> = None;
        InsuranceClaim::query()
            .where_(InsuranceClaim::ID.eq(id))
            .update(
                &mut *tx,
                &[
                    ("job_card_id", &none),
                    ("unlinked_job_card_id", &Some(job_id)),
                    ("updated_at", &now()),
                ],
            )
            .await?;
        crate::payer::make_private(&mut *tx, job_id).await?;
        Ok(Vec::new())
    })
}

/// The claim as a voidable record (permission `insurance.manage`, the number
/// kept). The claim's machine is registered with the kernel first, so the void
/// takes the claim's own approval action key.
pub(crate) fn register() {
    Voidable::of::<InsuranceClaim>(INSURANCE_CLAIM_SUBJECT_TYPE)
        .permission(permissions::INSURANCE_MANAGE)
        .number("insurance_claim", |claim: &InsuranceClaim| {
            Some(claim.number())
        })
        .reverse(EffectKind::Task, Reverse::new(nothing, unlink))
        .register();
}
