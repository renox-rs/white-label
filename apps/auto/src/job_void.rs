//! A job card opened in error is voidable (08 §4.4 last row, §7.1 *Job card*
//! `void`, `PLT.ACL.09`; BL-AUT-02): a duplicate check-in is voided from
//! `checked_in`, before any labour or part is recorded, and its booking is
//! cancelled with work done *none*. The kernel's void pattern gives the `void`
//! and `void_approved` transitions, the reason, the Void record that keeps the
//! number, and the `RecordVoided` event.
//!
//! Decisions:
//!
//! - Nothing is posted by a job card in `checked_in` (no stock, no money), so a
//!   void needs no approval; the default policy of the kernel matches only a
//!   record with posted effects.
//! - The refusals are the reverse handler's (a guard cannot read the database):
//!   a job card with a labour or part line (a line taken off the estimate
//!   before it was sent does not count), or linked to an open insurance claim,
//!   is refused in words, and the whole void rolls back. There is no action to
//!   unlink a claim: void the claim first when nothing was sent to the insurer
//!   (it unlinks the job card, BL-AUT-10), or cancel it, which declines the job
//!   card (BL-AUT-08).
//! - The booking is cancelled after the commit, for the user who voided, with
//!   the reason `duplicate_check_in` ([`on_voided`]); it asks the job card what
//!   was done and hears *none* ([`crate::job_stops::work_done`]).
//! - The check-in photos and documents stay attached; the vehicle's odometer
//!   is not changed by the void.

use renox::db::Transaction;
use renox::prelude::*;
use wl_kernel::machines::BoxFuture;
use wl_kernel::voids::{Effect, EffectKind, RecordVoided, Reverse, ReverseError, Voidable};

use crate::claim::InsuranceClaim;
use crate::job_card::{JOB_CARD_SUBJECT_TYPE, JobCard};
use crate::labour_line::{LabourLine, LineStatus};
use crate::part_line::PartLine;
use crate::permissions;
use crate::reasons::DUPLICATE_CHECK_IN;
use crate::texts::english;

/// What there is to reverse: nothing (a task is never posted).
fn nothing<'a>(_tx: &'a mut Transaction, _id: i64) -> BoxFuture<'a, Result<Vec<Effect>>> {
    Box::pin(async { Ok(Vec::new()) })
}

/// Refuses the void of a job card with a labour or part line or an open claim.
fn refuse_if_recorded<'a>(
    tx: &'a mut Transaction,
    id: i64,
) -> BoxFuture<'a, Result<Vec<Effect>, ReverseError>> {
    Box::pin(async move {
        let labour = LabourLine::query()
            .where_(LabourLine::JOB_CARD_ID.eq(id))
            .get(&mut *tx)
            .await?
            .into_iter()
            .any(|line| line.approval != LineStatus::Removed);
        let parts = PartLine::query()
            .where_(PartLine::JOB_CARD_ID.eq(id))
            .get(&mut *tx)
            .await?
            .into_iter()
            .any(|line| line.approval != LineStatus::Removed);
        if labour || parts {
            return Err(ReverseError::Failed(english(
                "auto.void.problem.recorded",
                &[],
            )));
        }
        let claim = InsuranceClaim::query()
            .where_(InsuranceClaim::JOB_CARD_ID.eq(Some(id)))
            .get(&mut *tx)
            .await?
            .into_iter()
            .find(|claim| !claim.status.is_closed());
        if let Some(claim) = claim {
            let number = claim.number();
            return Err(ReverseError::Failed(english(
                "auto.void.problem.claim",
                &[("claim", &number)],
            )));
        }
        Ok(Vec::new())
    })
}

/// The job card as a voidable record (permission `job_cards.advise`, the
/// number kept). The machine is registered with the kernel first, so the void
/// takes the approval action key of the machine.
pub(crate) fn register() {
    Voidable::of::<JobCard>(JOB_CARD_SUBJECT_TYPE)
        .permission(permissions::JOB_CARDS_ADVISE)
        .number("job_card", |card: &JobCard| Some(card.number()))
        .reverse(
            EffectKind::Task,
            Reverse::new(nothing, refuse_if_recorded),
        )
        .register();
}

/// `RecordVoided` of a job card: its booking is cancelled for the user who
/// voided, with the reason `duplicate_check_in`.
///
/// # Errors
///
/// The database's and the scheduling contract's errors.
pub async fn on_voided(state: &AppState, event: &RecordVoided) -> Result<bool> {
    if event.subject.subject_type != JOB_CARD_SUBJECT_TYPE {
        return Ok(false);
    }
    let card = JobCard::find_or_404(&state.db, event.subject.subject_id).await?;
    let Some(booking) = card.booking_id else {
        return Ok(false);
    };
    let cancelled = wl_kernel::scheduling::booking_actions(state)
        .cancel_booking_by_staff(
            state,
            wl_kernel::scheduling::BookingId(booking),
            DUPLICATE_CHECK_IN,
            event.user_id,
        )
        .await?;
    if cancelled {
        JobCard::query()
            .where_(JobCard::ID.eq(card.id))
            .update(
                &state.db,
                &[
                    ("booking_cancelled_at", &Some(renox::db::now())),
                    ("updated_at", &renox::db::now()),
                ],
            )
            .await?;
    }
    Ok(cancelled)
}

/// Registers the listener of the void.
pub(crate) fn listen(app: &mut Registry) {
    app.listen(|event: RecordVoided, state: AppState| async move {
        on_voided(&state, &event).await.map(|_| ())
    });
}
