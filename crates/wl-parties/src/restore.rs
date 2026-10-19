//! Restoring an archived party (wl-parties §4 `restore`).
//!
//! `archive` is not built yet; only `restore` is, as two transitions on the
//! Party machine (a renox transition has one target state):
//!
//! - `restore`: `archived` → `active`;
//! - `restore_blocked`: `archived` → `blocked`, for a party that still has an
//!   open Block record (e.g. a confirmed sanctions match added while it was
//!   archived).
//!
//! Both need `parties.archive`. [`restore`] picks the transition. A
//! party with an open record never becomes `active` by a restore: only
//! `unblock` lifts a hold. `restore_blocked` sets the hold reason to the
//! display code of the newest open record (`on_hold` when restricted), and
//! emits [`PartyBlocked`] as well as [`PartyRestored`].

use renox::prelude::*;
use renox::state_machine::{StateMachine, Transition, TransitionError};
use wl_kernel::{PartyBlocked, PartyId, PartyRestored, PartyStatus};

use crate::block_record::BlockRecord;
use crate::party::Party;
use crate::permissions;

/// The transition to `active`.
pub const RESTORE: &str = "restore";
/// The transition to `blocked`.
pub const RESTORE_BLOCKED: &str = "restore_blocked";

/// The `restore` and `restore_blocked` transitions, for the machine.
pub(crate) fn transitions() -> [Transition<Party>; 2] {
    [
        Transition::new(RESTORE)
            .label("Restore")
            .tone("success")
            .from([PartyStatus::Archived])
            .to(PartyStatus::Active)
            .permission(permissions::ARCHIVE)
            .effect(|party: &Party, _ctx, tx| {
                let party_id = party.id;
                Box::pin(async move { set_hold_reason(tx, party_id, None).await })
            }),
        Transition::new(RESTORE_BLOCKED)
            .label("Restore (still on hold)")
            .tone("danger")
            .from([PartyStatus::Archived])
            .to(PartyStatus::Blocked)
            .permission(permissions::ARCHIVE)
            .effect(|party: &Party, _ctx, tx| {
                let party_id = party.id;
                Box::pin(async move {
                    let records = BlockRecord::open_for(&mut *tx, party_id).await?;
                    let display = records.last().map(|r| r.display_reason().to_owned());
                    set_hold_reason(tx, party_id, display.as_deref()).await
                })
            }),
    ]
}

/// Sets the hold reason; not the status column, so outside the machine's
/// guard.
async fn set_hold_reason(
    tx: &mut renox::db::Transaction,
    party_id: i64,
    reason: Option<&str>,
) -> Result<()> {
    renox::db::sql(format!(
        "UPDATE {} SET hold_reason = ?, updated_at = ? WHERE id = ?",
        Party::TABLE
    ))
    .bind(reason)
    .bind(renox::db::now())
    .bind(party_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// `user` restores the archived party `party`. Needs `parties.archive`. A
/// party with an open Block record is restored to `blocked` (`restore_blocked`),
/// else to `active` (`restore`). After the commit it emits [`PartyRestored`],
/// and for `blocked` also [`PartyBlocked`] with the display reason only.
///
/// # Errors
///
/// `Forbidden` without the permission, `NotFrom` when the party is not
/// `archived`; nothing is written then.
pub async fn restore(
    state: &AppState,
    user: &AuthUser,
    party: i64,
) -> std::result::Result<(), TransitionError> {
    if !user.has_permission(permissions::ARCHIVE) {
        return Err(TransitionError::Forbidden);
    }
    // The write lock on SQLite, the row lock on PostgreSQL.
    let mut tx = state.db.begin_immediate().await?;
    let party = Party::query()
        .where_eq("id", party)
        .lock_for_update()
        .first(&mut tx)
        .await?
        .ok_or(renox::Error::NotFound)?;
    let records = BlockRecord::open_for(&mut tx, party.id).await?;
    let name = if records.is_empty() {
        RESTORE
    } else {
        RESTORE_BLOCKED
    };
    let outcome = party.transition(name).by(user).run_in(&mut tx).await?;
    tx.commit().await?;
    outcome.emit(state).await?;
    let id = PartyId(party.id);
    state.emit(PartyRestored { party: id }).await?;
    if let Some(newest) = records.last() {
        state
            .emit(PartyBlocked {
                party: id,
                display_reason: newest.display_reason().to_owned(),
            })
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_goes_from_archived_to_active_or_blocked() {
        let machine = Party::machine();
        let active = machine.find(RESTORE).expect("restore");
        assert_eq!(active.from_states(), ["archived"]);
        assert_eq!(active.to_state(), Some("active"));
        let blocked = machine.find(RESTORE_BLOCKED).expect("restore_blocked");
        assert_eq!(blocked.from_states(), ["archived"]);
        assert_eq!(blocked.to_state(), Some("blocked"));
        assert!(!active.is_system() && !blocked.is_system());
    }
}
