//! A blocked party's loyalty account is paused, not closed (wl-sales §4.7
//! *Party blocked*, wl-parties §5.6; BL-SAL-26). The account is an agreement
//! of the kind *loyalty account* with the block effect *pause*: no earning,
//! no redemption (refused at the counter with the neutral text *on hold*,
//! never the reason), out of every segment, no loyalty messages. The lots
//! keep expiring. `PartyUnblocked` clears the pause; points not earned while
//! it lasted are not earned later.
//!
//! A *pause* creates no review task, so the kind has no agreement source.
//!
//! The orders of a blocked customer that are not delivered yet are held too
//! (BL-MFG-14, 07 §7.1 *Party blocked*): while the effect of the kernel's
//! `sales_order` kind is *pause*, `PartyBlocked` puts every `confirmed`,
//! `partially_fulfilled` or `pending_acceptance` order of the party on
//! `credit_hold` with the reason *account on hold*, keeping its reservations
//! ([`hold_orders`]). Dispatched goods are not touched, and a credit releaser
//! releases the order. The work orders of the vertical are its own agreements.

use renox::auth::permissions::Scope;
use renox::db::now;
use renox::prelude::*;
use wl_kernel::block_effects::{AgreementKind, BlockEffect, effect_for};
use wl_kernel::parties::{PartyBlocked, PartyId, PartyUnblocked};
use wl_kernel::sales::CreditFailure;

use crate::loyalty_account::{AccountStatus, LoyaltyAccount};
use crate::sales_order::{SalesOrder, SalesOrderStatus};
use crate::sales_order_machine::HoldInput;

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

/// Holds the orders of a blocked customer that are not delivered yet (wl-parties
/// §5.6, 07 §7.1 *Party blocked*): when the effect of the `sales_order` kind
/// is *pause*, every order that still has goods to deliver (`confirmed`,
/// `partially_fulfilled`, `pending_acceptance`) goes on `credit_hold` with the
/// reason *account on hold*, keeping its reservations. Goods already
/// dispatched are not touched; a credit releaser releases the order when the
/// party is cleared.
async fn hold_orders(state: &AppState, party: PartyId) -> Result {
    let rule = effect_for(AGREEMENT_ORDER_KIND, &Scope::global());
    if rule.effect != BlockEffect::Pause {
        return Ok(());
    }
    for order in SalesOrder::query()
        .where_(SalesOrder::PARTY_ID.eq(party.0))
        .get(&state.db)
        .await?
    {
        if matches!(
            order.status,
            SalesOrderStatus::Confirmed
                | SalesOrderStatus::PartiallyFulfilled
                | SalesOrderStatus::PendingAcceptance
        ) {
            crate::credit::hold_order(
                state,
                &order,
                HoldInput {
                    reason: CreditFailure::AccountOnHold,
                    open_balance: 0,
                    overdue: 0,
                    limit: None,
                },
            )
            .await?;
        }
    }
    Ok(())
}

/// The kind of agreement a sales order is for the block effects.
const AGREEMENT_ORDER_KIND: &str = "sales_order";

/// Registers the agreement kind and the listeners.
pub(crate) fn register(app: &mut Registry) {
    AgreementKind::new(KIND)
        .owner("wl-sales")
        .effect(BlockEffect::Pause)
        .decider(DECIDER)
        .register();
    app.listen(|event: PartyBlocked, state: AppState| async move {
        set_paused(&state, event.party, true).await?;
        hold_orders(&state, event.party).await
    });
    app.listen(|event: PartyUnblocked, state: AppState| async move {
        set_paused(&state, event.party, false).await
    });
}
