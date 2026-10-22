//! The transfer of an enrolment to another batch (10 §7.1 *Enrolment*
//! `transfer`, `EDU.ACL.17`, BL-EDU-02).
//!
//! The counsellor moves a student to another batch of the same course, or the
//! system does it when a batch is cancelled or terminated (BL-EDU-13,
//! BL-EDU-12). The target must have a seat: this service looks first and says
//! so to the machine (`target_has_seat`), and the machine's effect then takes
//! the seat atomically ([`crate::seats::take`]), so a seat that two people go
//! for is given to one. A refusal offers the other open batches with a free
//! seat and the waiting list ([`crate::seats::offer`]).
//!
//! What moves: a new `active` enrolment in the target batch, holding the value
//! the student had paid for the sessions not yet used
//! ([`crate::refund::carried_value`], counted as paid there) and the
//! instalments not yet invoiced; the old one is `transferred` and gives its
//! seat back. Decision: the batches have no price of their own, the fee is the
//! enrolment's, so there is no price difference to invoice or credit when the
//! target is another run of the same course.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::documents::today;

use crate::batch::{Batch, BatchStatus};
use crate::db::refused;
use crate::enrolment::{Enrolment, EnrolmentKind, EnrolmentStatus};
use crate::enrolment_machine::{EnrolmentInput, TRANSFER, TRANSFER_SYSTEM};
use crate::enrolments::paid_of;
use crate::permissions;
use crate::refund;
use crate::seats::{self, Limit};
use crate::sessions;
use crate::texts::english;

/// What the machine needs to move `enrolment` to `target`: the value that
/// moves and the sessions of the target, with the day the figures are as of.
async fn facts(
    state: &AppState,
    enrolment: &Enrolment,
    target: &Batch,
    on: NaiveDate,
) -> Result<(i64, i64)> {
    let paid = paid_of(state, enrolment).await?;
    let held = match enrolment.batch_id {
        Some(batch) => sessions::held_until(&state.db, batch, on).await?,
        None => 0,
    };
    let carried = refund::carried_value(
        paid,
        enrolment.registration_fee,
        enrolment.sessions_total,
        held,
    );
    let sessions_in_target = sessions::planned_count(&state.db, target.id).await?;
    Ok((carried, sessions_in_target))
}

/// Whether `target` has a seat for a taker with `limit`.
async fn has_seat(state: &AppState, target: &Batch, limit: Limit) -> Result<bool> {
    let held = seats::count_in(&state.db, target.id).await?;
    Ok(held < seats::ceiling(target, limit))
}

/// `transfer`: moves enrolment `id` to batch `target_batch_id`. A user needs
/// `enrolments.manage` and goes over the capacity only with
/// `batches.exceed_capacity`; `by` is `None` for the system (a batch `cancel`
/// or `terminate`), which never goes over it. Returns the new enrolment.
///
/// # Errors
///
/// The machine's: a 403 without `enrolments.manage`; a 422 unless the
/// enrolment is a course enrolment `active` or `frozen`, the target is an
/// `open` or `running` batch of the same course other than its own, and the
/// target has a seat (`EDU.ACL.17`: then with the other open batches and the
/// waiting list).
pub async fn transfer(
    state: &AppState,
    by: Option<&AuthUser>,
    id: i64,
    target_batch_id: i64,
) -> Result<Enrolment> {
    let enrolment = Enrolment::find_or_404(&state.db, id).await?;
    if let Some(user) = by
        && !user.has_permission(permissions::ENROLMENTS_MANAGE)
    {
        return Err(Error::Forbidden);
    }
    let Some(from_batch) = enrolment.batch_id.filter(|_| enrolment.kind == EnrolmentKind::Course)
    else {
        return Err(refused(english("edtech.transfer.problem.not_course", &[])));
    };
    if !matches!(
        enrolment.status,
        EnrolmentStatus::Active | EnrolmentStatus::Frozen
    ) {
        return Err(refused(english("edtech.transfer.problem.status", &[])));
    }
    let source = Batch::find_or_404(&state.db, from_batch).await?;
    let target = Batch::find_or_404(&state.db, target_batch_id).await?;
    if target.id == source.id {
        return Err(refused(english("edtech.transfer.problem.same", &[])));
    }
    if target.course_id != source.course_id {
        return Err(refused(english("edtech.transfer.problem.course", &[])));
    }
    if !matches!(target.status, BatchStatus::Open | BatchStatus::Running) {
        return Err(refused(english("edtech.transfer.problem.not_open", &[])));
    }
    let room = by.is_some_and(|user| user.has_permission(permissions::SEATS_EXCEED));
    let limit = if room { Limit::Room } else { Limit::Capacity };
    let on = today();
    let (carried, target_sessions) = facts(state, &enrolment, &target, on).await?;
    let input = EnrolmentInput {
        on: Some(on),
        by: by.map(|user| user.id),
        target_batch_id: Some(target.id),
        target_has_seat: Some(has_seat(state, &target, limit).await?),
        target_room: room,
        target_sessions,
        carried,
        ..EnrolmentInput::default()
    };
    let builder = match by {
        Some(user) => enrolment.transition(TRANSFER).by(user),
        None => enrolment.transition(TRANSFER_SYSTEM).by_system(),
    };
    let run = builder.input(&input).run(state).await;
    let _outcome = match run {
        Ok(outcome) => outcome,
        Err(error) => {
            let error = error.into_error();
            return Err(seats::offer(&state.db, target.id, error).await);
        }
    };
    let moved = Enrolment::find_or_404(&state.db, id).await?;
    match moved.transferred_to_id {
        Some(next) => Enrolment::find_or_404(&state.db, next).await,
        None => Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            english("edtech.transfer.problem.status", &[]),
        )),
    }
}
