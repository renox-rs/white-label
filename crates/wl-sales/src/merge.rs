//! A party merge moves the points (wl-sales §4.7 *Party merge*, §6 consumed
//! `PartiesMerged`; BL-SAL-26). Duplicate parties are merged by staff; the
//! member's points must not be lost by it.
//!
//! For each open account of the merged party:
//!
//! - the survivor has an open account in the same programme: every open lot
//!   moves with its expiry date (a `transfer` movement out of the merged
//!   account and one into the survivor's, the same points), a negative
//!   balance moves too, the visits add up, and the merged account closes
//!   *merged into* the survivor's code. No `adjust`, no approval, nothing
//!   forfeited;
//! - the survivor has none in that programme: the account itself moves to the
//!   survivor and stays open. Two programmes (two brands) stay separate, one
//!   per programme.
//!
//! A blocked party cannot be merged (wl-parties allows `merge` from `active`
//! or `archived` only), so a pause never moves. When the survivor is blocked
//! the accounts it ends up with are paused with it until `PartyUnblocked`.
//!
//! Decisions:
//! - Points held for a sale in progress are released first.
//! - The earnings of the merged account stay with it: a later return of one
//!   of its sales finds a closed account and takes nothing back, as for any
//!   closed account.

use renox::db::now;
use renox::prelude::*;
use wl_kernel::parties::{PartiesMerged, PartyStatus, directory};
use wl_kernel::sales::PointsTransferred;

use crate::loyalty_account::{AccountStatus, LoyaltyAccount};
use crate::loyalty_points::{CAUSE_TRANSFER, PointsKind, PointsMovement};
use crate::loyalty_programme::LoyaltyProgramme;
use crate::redeem;
use crate::texts::english;

/// The open account of `party` in `programme`, if any.
async fn open_in(db: &Db, programme: i64, party: i64) -> Result<Option<LoyaltyAccount>> {
    LoyaltyAccount::query()
        .where_(LoyaltyAccount::PROGRAMME_ID.eq(programme))
        .where_(LoyaltyAccount::PARTY_ID.eq(party))
        .where_(LoyaltyAccount::STATUS.eq(AccountStatus::Active))
        .first(db)
        .await
}

/// Moves everything of `from` to `to` and closes `from`. Returns the points
/// moved.
async fn transfer(
    state: &AppState,
    from: &LoyaltyAccount,
    to: &LoyaltyAccount,
    survivor_blocked: bool,
) -> Result<i64> {
    redeem::release_account_holds(state, from.id, "account_merged").await?;
    let from = LoyaltyAccount::find_or_404(&state.db, from.id).await?;
    let programme = LoyaltyProgramme::find_or_404(&state.db, from.programme_id).await?;
    let value_of = |points: i64| points * programme.point_value;
    let from_subject = from.subject();
    let to_subject = to.subject();
    let mut tx = state.db.begin().await?;
    let mut moved: i64 = 0;
    for lot in PointsMovement::lots(&mut tx, from.id, None).await? {
        let left = lot.lot_remaining();
        if left <= 0 {
            continue;
        }
        PointsMovement {
            account_id: from.id,
            kind: PointsKind::Transfer,
            points: -left,
            value: -value_of(left),
            legal_entity_id: lot.legal_entity_id,
            source_type: to_subject.subject_type.clone(),
            source_id: to_subject.subject_id,
            earning_id: lot.earning_id,
            cause: CAUSE_TRANSFER.to_owned(),
            lot_id: Some(lot.id),
            lot_expires_on: lot.lot_expires_on,
            ..PointsMovement::default()
        }
        .insert(&mut tx)
        .await?;
        PointsMovement::query()
            .where_(PointsMovement::ID.eq(lot.id))
            .increment(&mut tx, "consumed", left)
            .await?;
        PointsMovement {
            account_id: to.id,
            kind: PointsKind::Transfer,
            points: left,
            value: value_of(left),
            legal_entity_id: lot.legal_entity_id,
            source_type: from_subject.subject_type.clone(),
            source_id: from_subject.subject_id,
            cause: CAUSE_TRANSFER.to_owned(),
            lot_expires_on: lot.lot_expires_on,
            ..PointsMovement::default()
        }
        .insert(&mut tx)
        .await?;
        moved += left;
    }
    // What the lots do not hold, a negative balance after a reversal, moves
    // too, without an expiry.
    let rest = from.points_balance - moved;
    if rest != 0 {
        PointsMovement {
            account_id: from.id,
            kind: PointsKind::Transfer,
            points: -rest,
            value: -value_of(rest),
            source_type: to_subject.subject_type.clone(),
            source_id: to_subject.subject_id,
            cause: CAUSE_TRANSFER.to_owned(),
            ..PointsMovement::default()
        }
        .insert(&mut tx)
        .await?;
        PointsMovement {
            account_id: to.id,
            kind: PointsKind::Transfer,
            points: rest,
            value: value_of(rest),
            source_type: from_subject.subject_type.clone(),
            source_id: from_subject.subject_id,
            cause: CAUSE_TRANSFER.to_owned(),
            ..PointsMovement::default()
        }
        .insert(&mut tx)
        .await?;
    }
    let total = moved + rest;
    LoyaltyAccount::query()
        .where_(LoyaltyAccount::ID.eq(to.id))
        .increment(&mut tx, "points_balance", total)
        .await?;
    LoyaltyAccount::query()
        .where_(LoyaltyAccount::ID.eq(to.id))
        .increment(&mut tx, "visits", from.visits)
        .await?;
    if survivor_blocked {
        LoyaltyAccount::query()
            .where_(LoyaltyAccount::ID.eq(to.id))
            .update(&mut tx, &[("paused", &true)])
            .await?;
    }
    let closed_reason = Some(english(
        "wl_sales.merge.closed_reason",
        &[("code", &to.code)],
    ));
    LoyaltyAccount::query()
        .where_(LoyaltyAccount::ID.eq(from.id))
        .update(
            &mut tx,
            &[
                ("points_balance", &0_i64),
                ("visits", &0_i64),
                ("status", &AccountStatus::Closed),
                ("closed_reason", &closed_reason),
                ("closed_at", &Some(now())),
                ("updated_at", &now()),
            ],
        )
        .await?;
    tx.commit().await?;
    if total != 0 {
        state
            .emit(PointsTransferred {
                from: from.account_id(),
                to: to.account_id(),
                party: to.party(),
                points: total,
                value: value_of(total),
            })
            .await?;
    }
    Ok(total)
}

/// `PartiesMerged`: the merged party's accounts go to the survivor.
async fn on_merged(state: &AppState, event: &PartiesMerged) -> Result {
    let survivor_blocked =
        directory(state).status(&state.db, event.survivor).await? == Some(PartyStatus::Blocked);
    let accounts = LoyaltyAccount::query()
        .where_(LoyaltyAccount::PARTY_ID.eq(event.merged.0))
        .where_(LoyaltyAccount::STATUS.eq(AccountStatus::Active))
        .order_by(LoyaltyAccount::ID)
        .get(&state.db)
        .await?;
    for from in accounts {
        match open_in(&state.db, from.programme_id, event.survivor.0).await? {
            Some(to) => {
                transfer(state, &from, &to, survivor_blocked).await?;
            }
            None => {
                // The account itself moves, paused when the survivor is on
                // hold.
                redeem::release_account_holds(state, from.id, "account_merged").await?;
                LoyaltyAccount::query()
                    .where_(LoyaltyAccount::ID.eq(from.id))
                    .update(
                        &state.db,
                        &[
                            ("party_id", &event.survivor.0),
                            ("paused", &survivor_blocked),
                            ("updated_at", &now()),
                        ],
                    )
                    .await?;
            }
        }
    }
    Ok(())
}

/// Registers the listener.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: PartiesMerged, state: AppState| async move { on_merged(&state, &event).await },
    );
}
