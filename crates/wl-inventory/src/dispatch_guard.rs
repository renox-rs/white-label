//! The vertical guards of `pick` and `dispatch` (wl-inventory §4 *Dispatch*
//! rows `pick` and `dispatch`, STK.DSP.01; 07 `MFG.ACL.11`): the checks of a
//! dispatch that are not stock's own.
//!
//! Two things are asked, in this order, and the first refusal is a 422 with
//! its words (the dispatch stays as it is and can be `cancel`led):
//!
//! 1. the kernel's [`DispatchGuard`] of the vertical (the item flags: 07
//!    refuses an *export only* item for a domestic delivery);
//! 2. the document guard of wl-documents for the step, `dispatch.pick` and
//!    `dispatch.dispatch`: the kinds that list the step in their `blocks`
//!    (07: the conformity certificates of a regulated item) and the
//!    [`BlockSubjects`](wl_kernel::documents::BlockSubjects) the vertical
//!    registered for it. A kind that allows an override shows its link; a
//!    legal block gives none.
//!
//! Without a vertical and without wl-documents nothing is blocked, so a
//! deployment that has neither works as before. The date is today
//! (Asia/Dubai): `pick` checks the pick date, `dispatch` checks again on the
//! dispatch date, so a certificate that expired in between refuses it.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::TransitionError;
use wl_kernel::documents::{self, BlockRefusal};
use wl_kernel::inventory::{dispatch_directory, dispatch_guard};

use crate::dispatch::Dispatch;

/// The guarded step `pick`.
pub const PICK: &str = "dispatch.pick";

/// The guarded step `dispatch`.
pub const DISPATCH: &str = "dispatch.dispatch";

/// The refusal text: one line per failing document, then, for each kind that
/// allows an override, the link that requests it.
fn refusal_text(refusal: &BlockRefusal) -> String {
    let mut text = refusal.message();
    for link in refusal.override_links() {
        text.push_str("\nRequest an override: ");
        text.push_str(&link);
    }
    text
}

/// Checks the step `guarded` of `dispatch` on `on` for user `actor`.
///
/// # Errors
///
/// `Guard` (422) with the first refusal; the database's errors.
pub async fn check(
    state: &AppState,
    guarded: &str,
    dispatch: &Dispatch,
    actor: Option<i64>,
    on: NaiveDate,
) -> std::result::Result<(), TransitionError> {
    if let Some(facts) = dispatch_directory(state)
        .facts(state, &dispatch.subject())
        .await?
        && let Some(problem) = dispatch_guard(state)
            .blocked(state, &facts, guarded, on)
            .await?
    {
        return Err(TransitionError::Guard(problem));
    }
    let checked = documents::guard(state)
        .check_as(state, guarded, dispatch.id, on, actor)
        .await?;
    match checked {
        Ok(_) => Ok(()),
        Err(refusal) => Err(TransitionError::Guard(refusal_text(&refusal))),
    }
}
