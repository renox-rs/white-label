//! A blocked client (03 Logistics §3.3 `PTY.ACL.03`, §4.1 row *Client
//! blocked*, checklist column *Party blocked*; BL-LOG-19).
//!
//! When a client is blocked (for example for non-payment or a legal dispute):
//!
//! - **New business is refused** with one neutral text, never the reason: a
//!   booking (the portal, the CSV import and the staff booking for that
//!   client, [`crate::booking::book`]) and a new freight job
//!   ([`crate::freight::open`]) ask the kernel's block guard for
//!   [`ActionKind::Start`], as the system, so the answer cannot carry a reason.
//!   A rate card renewal or extension would ask for `Renew` or `Extend`
//!   ([`refuse`]; rate cards are F2's, see the deferred list).
//! - **Work in progress finishes.** Shipments already booked may still be
//!   picked up (the dispatcher sees *client on hold* on the list,
//!   [`on_hold`]), picked up ones are delivered or returned, and open freight
//!   jobs continue. A shipment is not an agreement, so nothing else of the
//!   wl-parties block effect applies to it.
//! - **Money out waits.** The weekly rule drafts the COD remittance and it can
//!   be approved, but `pay` and `repay` wait for the unblock or an approved
//!   exception ([`crate::remit`]); invoices and receipts continue.

use renox::prelude::*;
use renox::state_machine::TransitionError;
use wl_kernel::PartyId;
use wl_kernel::block_guard::{self, ActionKind, GuardOutcome};
use wl_kernel::parties::{self, PartyStatus};

use crate::texts::english;

/// The neutral text of a refused booking or freight job: the same whatever the
/// reason, and for the portal and the back office alike (the text of
/// `BL-POR-01`).
pub fn neutral() -> String {
    english("logistics.blocked.neutral", &[])
}

/// Whether the client `party` is blocked now, as the party directory says.
///
/// # Errors
///
/// The directory's errors.
pub async fn on_hold(state: &AppState, party: PartyId) -> Result<bool> {
    Ok(parties::directory(state).status(&state.db, party).await? == Some(PartyStatus::Blocked))
}

/// Refuses the action `kind` for `party` when it is blocked, with the neutral
/// text: the kernel's block guard is asked in a transaction of its own, as the
/// system (no viewer), so no reason can leak.
///
/// # Errors
///
/// `Guard` (422) with [`neutral`] for a blocked party; database errors.
pub async fn refuse(
    state: &AppState,
    party: PartyId,
    kind: ActionKind,
) -> std::result::Result<(), TransitionError> {
    let mut tx = state.db.begin().await?;
    let outcome = block_guard::guard(state)
        .check_in(&mut tx, None, party, kind)
        .await?;
    match outcome {
        GuardOutcome::Allowed => Ok(()),
        GuardOutcome::Blocked(_) => Err(TransitionError::Guard(neutral())),
    }
}

/// Refuses a new booking or freight job for `party` when it is blocked.
///
/// # Errors
///
/// As [`refuse`].
pub async fn refuse_start(
    state: &AppState,
    party: PartyId,
) -> std::result::Result<(), TransitionError> {
    refuse(state, party, ActionKind::Start).await
}
