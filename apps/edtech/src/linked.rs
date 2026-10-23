//! A membership and its enrolment stay in step (10 §4.3 row *The membership is
//! paused*, §7.1 *Subscription* `pause`, `resume`, *Enrolment* `freeze`,
//! `unfreeze`, §7.2 `EDU.RUL.14`, BL-EDU-05).
//!
//! The two **belong together** when the subscription was started by the
//! enrolment (a membership). Pausing the subscription freezes the enrolment
//! and freezing the enrolment pauses the subscription, and the same for resume
//! and unfreeze, with no loop and no transition run twice:
//!
//! - A user pausing the subscription (staff, or the guardian) is heard as
//!   `SubscriptionPaused` with the cause `user`: the enrolment is frozen with
//!   the cause [`FreezeCause::Linked`] (a freeze on a `frozen` enrolment only
//!   adds the cause). The cause is not counted against the allowance: the
//!   subscription counted the pause, once, on the side the user acted on.
//!   `SubscriptionResumed` for that cause removes the `linked` cause; the
//!   enrolment unfreezes when none is left. A subscription that resumes into
//!   `past_due` still unfreezes it (`suspended` is not a pause, and is not
//!   heard).
//! - Staff freezing the enrolment for the student ([`after_freeze`], after the
//!   commit) pause the subscription with the cause `agreement`, which wl-invoicing
//!   does not count either. A subscription already `paused` (by a block, say) is
//!   not paused a second time. When the last cause of the freeze ends
//!   ([`after_unfreeze`]) the agreement's cause is removed; the subscription
//!   resumes when no other cause is left.
//! - **Loop guard.** The enrolment ignores the pause and resume it asked for
//!   itself (cause `agreement`) and the block's (cause `party_block`: the block
//!   freezes the enrolment on its own, [`crate::blocked`]); the subscription side
//!   runs only for the causes `student`, the enrolment's own. A freeze with the
//!   cause `linked` or `block` pauses nothing.
//!
//! A step that fails after the commit does not undo the freeze or the unfreeze;
//! it is reported on stderr, as the package days are ([`crate::frozen`]).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::subscriptions::{
    PauseCause, PauseSubscription, SubscriptionPaused, SubscriptionResumed, SubscriptionStatus,
    SubscriptionSummary, subscriptions,
};

use crate::enrolment::{
    ENROLMENT_SUBJECT_TYPE, Enrolment, EnrolmentFreeze, EnrolmentKind, EnrolmentStatus, FreezeCause,
};
use crate::enrolments;
use crate::texts::english;

/// The live subscription of a membership `enrolment`: the latest that is not
/// `cancelled`.
///
/// # Errors
///
/// The contract's errors.
pub async fn membership_of(
    state: &AppState,
    enrolment: &Enrolment,
) -> Result<Option<SubscriptionSummary>> {
    if enrolment.kind != EnrolmentKind::Membership {
        return Ok(None);
    }
    let all = subscriptions(state)
        .of_agreement(&state.db, &enrolment.subject())
        .await?;
    Ok(all
        .into_iter()
        .rev()
        .find(|summary| summary.status != SubscriptionStatus::Cancelled))
}

/// The membership enrolment a subscription event is about, when the
/// subscription belongs to an enrolment of this app.
async fn enrolment_of(state: &AppState, source: &wl_kernel::Subject) -> Result<Option<Enrolment>> {
    if source.subject_type != ENROLMENT_SUBJECT_TYPE {
        return Ok(None);
    }
    Ok(Enrolment::find(&state.db, source.subject_id)
        .await?
        .filter(|enrolment| enrolment.kind == EnrolmentKind::Membership))
}

/// `SubscriptionPaused`: a user paused the membership; the enrolment follows.
///
/// # Errors
///
/// The freeze's errors.
pub async fn on_paused(state: &AppState, event: &SubscriptionPaused) -> Result {
    // Its own pause and the block's are not a user's.
    if event.cause != PauseCause::User {
        return Ok(());
    }
    let Some(enrolment) = enrolment_of(state, &event.source).await? else {
        return Ok(());
    };
    if !matches!(
        enrolment.status,
        EnrolmentStatus::Active | EnrolmentStatus::Frozen
    ) {
        return Ok(());
    }
    let open = EnrolmentFreeze::open_of(&state.db, enrolment.id).await?;
    if open.iter().any(|row| row.cause == FreezeCause::Linked) {
        return Ok(());
    }
    enrolments::freeze(state, None, enrolment.id, FreezeCause::Linked, event.until)
        .await
        .map(|_| ())
}

/// `SubscriptionResumed`: a user's pause of the membership ended; the
/// enrolment follows. Whether the subscription went back to `past_due` or
/// `active` makes no difference.
///
/// # Errors
///
/// The unfreeze's errors.
pub async fn on_resumed(state: &AppState, event: &SubscriptionResumed) -> Result {
    if event.cause != PauseCause::User {
        return Ok(());
    }
    let Some(enrolment) = enrolment_of(state, &event.source).await? else {
        return Ok(());
    };
    if enrolment.status != EnrolmentStatus::Frozen {
        return Ok(());
    }
    let open = EnrolmentFreeze::open_of(&state.db, enrolment.id).await?;
    if !open.iter().any(|row| row.cause == FreezeCause::Linked) {
        return Ok(());
    }
    enrolments::unfreeze(state, None, enrolment.id, FreezeCause::Linked)
        .await
        .map(|_| ())
}

/// After a freeze for `cause` on `on`: the membership is paused for the
/// agreement. A cause that is not the student's pauses nothing, and a
/// subscription that is already `paused` is left as it is.
pub(crate) async fn after_freeze(
    state: &AppState,
    enrolment: &Enrolment,
    cause: Option<FreezeCause>,
    on: NaiveDate,
) {
    if cause != Some(FreezeCause::Student) {
        return;
    }
    if let Err(error) = pause_membership(state, enrolment, on).await {
        eprintln!(
            "edtech: the membership of {} is not paused with its freeze: {error:?}",
            enrolment.number()
        );
    }
}

async fn pause_membership(state: &AppState, enrolment: &Enrolment, on: NaiveDate) -> Result {
    let Some(membership) = membership_of(state, enrolment).await? else {
        return Ok(());
    };
    if membership.status == SubscriptionStatus::Paused
        || membership.status == SubscriptionStatus::Suspended
    {
        return Ok(());
    }
    let reason = english(
        "edtech.linked.pause_reason",
        &[("enrolment", &enrolment.number())],
    );
    let _paused = subscriptions(state)
        .pause(
            state,
            &enrolment.subject(),
            PauseSubscription::new(PauseCause::Agreement, on).reason(reason),
        )
        .await?;
    Ok(())
}

/// After the last cause of a freeze ended on `on`: the agreement's cause is
/// removed from the membership, which resumes when no other cause is left. A
/// membership the agreement did not pause is not touched.
pub(crate) async fn after_unfreeze(state: &AppState, enrolment: &Enrolment, on: NaiveDate) {
    if let Err(error) = resume_membership(state, enrolment, on).await {
        eprintln!(
            "edtech: the membership of {} is not resumed with its unfreeze: {error:?}",
            enrolment.number()
        );
    }
}

async fn resume_membership(state: &AppState, enrolment: &Enrolment, on: NaiveDate) -> Result {
    let Some(membership) = membership_of(state, enrolment).await? else {
        return Ok(());
    };
    if !membership.pause_causes.contains(&PauseCause::Agreement) {
        return Ok(());
    }
    let _resumed = subscriptions(state)
        .resume(state, &enrolment.subject(), PauseCause::Agreement, on)
        .await?;
    Ok(())
}
