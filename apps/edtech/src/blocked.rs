//! A blocked payer or student (10 §4.1 row *The payer is blocked*, §7.1
//! *Enrolment* `freeze`, `unfreeze`, `withdraw`, wl-parties §5.6, BL-EDU-04,
//! `PTY.ACL.03`).
//!
//! The block effect of an enrolment is *pause*: the agreement stays, the
//! service stops and it resumes on unblock. The kind is `enrolment`, decided
//! by the centre manager ([`register`]).
//!
//! - **`PartyBlocked`** freezes the `active` enrolments whose payer or student
//!   is the party with the cause [`FreezeCause::Block`] ([`freeze_for_block`]):
//!   no freeze-day limit and no days counted against the allowance, the seat
//!   is kept, and nothing is sent to the party. A `frozen` enrolment keeps its
//!   freeze (the student's own) and gets the block as a second cause. A
//!   `pending_payment` enrolment is left alone to lapse at its seat hold
//!   (`EDU.RUL.11`); one that ended is not touched.
//! - **`PartyUnblocked`** removes only the cause of the block
//!   ([`unfreeze_for_unblock`]); an enrolment the student froze for longer
//!   stays `frozen`. When the other party of the enrolment (the student, or the
//!   payer) is still blocked, the cause stays.
//! - **`withdraw`** of an enrolment whose payer or student is blocked is the
//!   centre manager's alone ([`check_withdraw`]: `enrolments.approve`); the
//!   credit note is issued as for any withdrawal and the refund request waits
//!   in wl-payments until the party is unblocked or an exception is approved
//!   (`PTY.ACL.03`, [`crate::settlement`]).
//!
//! The reason staff see is *On hold*: the real reason never leaves wl-parties
//! (the page shows the cause [`FreezeCause::Block`] as *On hold* to everyone).
//! The membership's subscription is paused by wl-invoicing itself when its
//! party is blocked (its own listener, cause `party_block`), so a freeze for
//! the block does not pause it a second time ([`crate::frozen`]).

use std::collections::BTreeSet;

use renox::prelude::*;
use wl_kernel::block_effects::{AgreementKind, BlockEffect};
use wl_kernel::parties::{PartyBlocked, PartyStatus, PartyUnblocked, directory};
use wl_kernel::PartyId;

use crate::db::refused;
use crate::enrolment::{Enrolment, EnrolmentFreeze, EnrolmentStatus, FreezeCause};
use crate::enrolments;
use crate::permissions;
use crate::texts::english;

/// The agreement kind of wl-parties §5.6 an enrolment is.
pub const KIND: &str = "enrolment";

/// Registers the block effect of the enrolment: *pause*, decided by the centre
/// manager (the built-in default of wl-parties §5.6, stated by the vertical).
pub(crate) fn register() {
    AgreementKind::new(KIND)
        .owner("edtech")
        .effect(BlockEffect::Pause)
        .decider(permissions::CENTRE_MANAGER_ROLE)
        .register();
}

/// The enrolments whose payer or student is `party`, oldest first.
///
/// # Errors
///
/// The database's errors.
pub async fn enrolments_of(db: &Db, party: PartyId) -> Result<Vec<Enrolment>> {
    let mut found = Enrolment::query()
        .where_(Enrolment::PAYER_PARTY_ID.eq(party.0))
        .order_by(Enrolment::ID)
        .get(db)
        .await?;
    let as_student = Enrolment::query()
        .where_(Enrolment::STUDENT_PARTY_ID.eq(party.0))
        .order_by(Enrolment::ID)
        .get(db)
        .await?;
    for enrolment in as_student {
        if !found.iter().any(|known| known.id == enrolment.id) {
            found.push(enrolment);
        }
    }
    found.sort_by_key(|enrolment| enrolment.id);
    Ok(found)
}

/// Whether `party` is blocked now.
async fn party_blocked(state: &AppState, party: PartyId) -> Result<bool> {
    Ok(directory(state).status(&state.db, party).await? == Some(PartyStatus::Blocked))
}

/// Whether the payer or the student of `enrolment` is blocked.
///
/// # Errors
///
/// The directory's errors.
pub async fn is_blocked(state: &AppState, enrolment: &Enrolment) -> Result<bool> {
    for party in [enrolment.payer(), enrolment.student()] {
        if party_blocked(state, party).await? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Refuses `user`'s withdrawal of `enrolment` while its payer or student is
/// blocked, unless `user` is the centre manager (`enrolments.approve`).
///
/// # Errors
///
/// A 422 naming the hold, without the reason of the block.
pub async fn check_withdraw(state: &AppState, user: &AuthUser, enrolment: &Enrolment) -> Result {
    if user.has_permission(permissions::ENROLMENTS_APPROVE) {
        return Ok(());
    }
    if is_blocked(state, enrolment).await? {
        return Err(refused(english("edtech.blocked.problem.withdraw", &[])));
    }
    Ok(())
}

/// Freezes the enrolments of the blocked `party`. Answers the numbers frozen
/// now; one already held by a block is left as it is. Every enrolment is tried;
/// the first failure is returned at the end.
///
/// # Errors
///
/// The first error of the freezes.
pub async fn freeze_for_block(state: &AppState, party: PartyId) -> Result<Vec<i64>> {
    let mut frozen = Vec::new();
    let mut failure = None;
    for enrolment in enrolments_of(&state.db, party).await? {
        if !matches!(
            enrolment.status,
            EnrolmentStatus::Active | EnrolmentStatus::Frozen
        ) {
            continue;
        }
        let open = EnrolmentFreeze::open_of(&state.db, enrolment.id).await?;
        if open.iter().any(|row| row.cause == FreezeCause::Block) {
            continue;
        }
        match enrolments::freeze(state, None, enrolment.id, FreezeCause::Block, None).await {
            Ok(_) => frozen.push(enrolment.id),
            Err(error) => {
                eprintln!("edtech: enrolment {} is not frozen for the block: {error:?}", enrolment.id);
                failure.get_or_insert(error);
            }
        }
    }
    failure.map_or(Ok(frozen), Err)
}

/// Removes the cause of the block from the enrolments of `party`, now unblocked.
/// An enrolment whose other party is still blocked keeps it. Answers the
/// numbers unfrozen or relieved of the cause.
///
/// # Errors
///
/// The first error of the unfreezes.
pub async fn unfreeze_for_unblock(state: &AppState, party: PartyId) -> Result<Vec<i64>> {
    let mut released = Vec::new();
    let mut failure = None;
    let mut seen = BTreeSet::new();
    for enrolment in enrolments_of(&state.db, party).await? {
        if enrolment.status != EnrolmentStatus::Frozen || !seen.insert(enrolment.id) {
            continue;
        }
        let open = EnrolmentFreeze::open_of(&state.db, enrolment.id).await?;
        if !open.iter().any(|row| row.cause == FreezeCause::Block) {
            continue;
        }
        if is_blocked(state, &enrolment).await? {
            continue;
        }
        match enrolments::unfreeze(state, None, enrolment.id, FreezeCause::Block).await {
            Ok(_) => released.push(enrolment.id),
            Err(error) => {
                eprintln!("edtech: enrolment {} keeps its block freeze: {error:?}", enrolment.id);
                failure.get_or_insert(error);
            }
        }
    }
    failure.map_or(Ok(released), Err)
}

/// The listeners of the block and the unblock.
pub(crate) fn listen(app: &mut Registry) {
    app.listen(|event: PartyBlocked, state: AppState| async move {
        freeze_for_block(&state, event.party).await.map(|_| ())
    });
    app.listen(|event: PartyUnblocked, state: AppState| async move {
        unfreeze_for_unblock(&state, event.party).await.map(|_| ())
    });
}
