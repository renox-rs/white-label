//! Unblocking a party (wl-parties §4 `unblock`): `blocked` → `active`.
//!
//! The transition needs `parties.block` and a reason. Its effect, in the
//! transition's transaction, closes every open Block record of the party
//! (unblocked by the actor, now, with the note and the basis) and clears the
//! party's `hold_reason`. The checks that need the database (separation of
//! duties against every "blocked by" user, `parties.block_reason.view` for a
//! restricted record, the sanctions basis and files) belong to the service
//! that runs the transition, as a guard has no database.

use renox::prelude::*;
use renox::state_machine::{Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::PartyStatus;

use crate::block_record::BlockRecord;
use crate::evidence::UnblockBasis;
use crate::party::Party;
use crate::permissions;
use crate::texts;

/// The transition.
pub const UNBLOCK: &str = "unblock";

/// What an unblock run carries to its guard and effect.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UnblockInput {
    /// Why the block is lifted; stored on each closed record.
    pub note: String,
    /// The statement behind lifting a `sanctions` block.
    pub basis: Option<UnblockBasis>,
}

/// The `unblock` transition, for the machine.
pub(crate) fn transition() -> Transition<Party> {
    Transition::new(UNBLOCK)
        .label("Unblock")
        .tone("success")
        .from([PartyStatus::Blocked])
        .to(PartyStatus::Active)
        .permission(permissions::BLOCK)
        .reason_required()
        .guard("note_given", |_party: &Party, ctx| {
            Box::pin(async move {
                match input(ctx) {
                    Some(input) if !input.note.trim().is_empty() => Ok(()),
                    _ => Err(texts::english("wl_parties.errors.note_required")),
                }
            })
        })
        .effect(|party: &Party, ctx, tx| {
            let party_id = party.id;
            Box::pin(async move {
                let input = input(ctx).unwrap_or_default();
                let user = ctx.actor().map(|user| user.id);
                for mut record in BlockRecord::open_for(&mut *tx, party_id).await? {
                    record.close(user, &input.note, input.basis);
                    record.save(&mut *tx).await?;
                }
                // Not the status column, so outside the machine's guard.
                renox::db::sql(format!(
                    "UPDATE {} SET hold_reason = NULL, updated_at = ? WHERE id = ?",
                    Party::TABLE
                ))
                .bind(renox::db::now())
                .bind(party_id)
                .execute(&mut *tx)
                .await?;
                Ok(())
            })
        })
}

/// The run's input, if it is an [`UnblockInput`].
fn input(ctx: &TransitionCtx) -> Option<UnblockInput> {
    ctx.input_as::<UnblockInput>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::state_machine::{StateMachine, TransitionError};

    #[test]
    fn unblock_goes_from_blocked_to_active_for_a_user() {
        let machine = Party::machine();
        let transition = machine.find(UNBLOCK).expect("transition");
        assert_eq!(transition.from_states(), ["blocked"]);
        assert_eq!(transition.to_state(), Some("active"));
        assert_eq!(transition.guard_names(), ["note_given"]);
        assert!(!transition.is_system());
    }

    #[renox::test]
    async fn the_guard_refuses_a_blank_note() {
        let machine = Party::machine();
        let unblock = machine.find(UNBLOCK).expect("transition");
        let party = Party::default();
        let run = |note: &str| {
            let input = UnblockInput {
                note: note.into(),
                basis: None,
            };
            TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).expect("json"))
        };
        assert!(
            unblock
                .check_guards(&party, &run("paid in full"))
                .await
                .is_ok()
        );
        for blank in ["", "   "] {
            assert!(matches!(
                unblock.check_guards(&party, &run(blank)).await,
                Err(TransitionError::Guard(_))
            ));
        }
    }
}
