//! A blocked party's loyalty account is paused, not closed (wl-sales §4.7
//! *Party blocked*, wl-parties §5.6; BL-SAL-26). The account is an agreement
//! of the kind *loyalty account* with the block effect *pause*: no earning,
//! no redemption (refused at the counter with the neutral text *on hold*,
//! never the reason), out of every segment, no loyalty messages. The lots
//! keep expiring. `PartyUnblocked` clears the pause; points not earned while
//! it lasted are not earned later.
//!
//! A *pause* creates no review task, so the kind has no agreement source.

use renox::db::now;
use renox::prelude::*;
use wl_kernel::block_effects::{AgreementKind, BlockEffect};
use wl_kernel::parties::{PartyBlocked, PartyId, PartyUnblocked};

use crate::loyalty_account::{AccountStatus, LoyaltyAccount};

/// The agreement kind of wl-parties §5.6.
pub const KIND: &str = "loyalty_account";

/// The role that would decide, were the effect a review.
pub const DECIDER: &str = "sales_manager";

/// Pauses (or resumes) the open accounts of `party`.
async fn set_paused(state: &AppState, party: PartyId, paused: bool) -> Result {
    LoyaltyAccount::query()
        .where_(LoyaltyAccount::PARTY_ID.eq(party.0))
        .where_(LoyaltyAccount::STATUS.eq(AccountStatus::Active))
        .update(&state.db, &[("paused", &paused), ("updated_at", &now())])
        .await?;
    Ok(())
}

/// Registers the agreement kind and the listeners.
pub(crate) fn register(app: &mut Registry) {
    AgreementKind::new(KIND)
        .owner("wl-sales")
        .effect(BlockEffect::Pause)
        .decider(DECIDER)
        .register();
    app.listen(|event: PartyBlocked, state: AppState| async move {
        set_paused(&state, event.party, true).await
    });
    app.listen(|event: PartyUnblocked, state: AppState| async move {
        set_paused(&state, event.party, false).await
    });
}
