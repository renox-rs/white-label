//! Blocking a party (wl-parties §4 `block`, §5.6 steps 1 and 3, `PTY.ACL.02`).
//!
//! Two transitions move a party from `active` to `blocked`, with the same
//! guards and effect:
//!
//! - `block`, run by a user holding `parties.block` ([`block`]);
//! - `block_by_system`, run by the system, e.g. screening's
//!   `confirm_match` ([`block_by_system`]).
//!
//! The guards: the reason code is configured ([`BlockReasons`]), the note
//! is not blank, and a restricted code needs `parties.block_reason.view`
//! (so a user may choose only the codes the block dialog offers them). The
//! effect writes the Block record with the real code and sets the party's
//! `hold_reason` to the display code, in the transition's transaction. The
//! history row's reason is the display code too, and nothing restricted
//! goes into its `meta`. After the commit a listener emits the kernel's
//! [`PartyBlocked`] with the display code.
//!
//! Nothing is sent to the party: no mail, no notification, no job for its
//! contacts (no tipping off).

use renox::prelude::*;
use renox::state_machine::{
    StateMachine, Transition, TransitionCtx, TransitionError, Transitioned,
};
use serde::{Deserialize, Serialize};
use wl_kernel::parties::display_block_reason;
use wl_kernel::{NEUTRAL_BLOCK_REASON, PartyBlocked, PartyId, PartyStatus};

use crate::block_record::BlockRecord;
use crate::party::Party;
use crate::permissions;
use crate::reasons::BlockReasons;
use crate::texts;

/// The user transition.
pub const BLOCK: &str = "block";
/// The system transition.
pub const BLOCK_BY_SYSTEM: &str = "block_by_system";

/// What a block run carries to its guards and effect. The service fills
/// `known` and `restricted` from the app's [`BlockReasons`], as the machine
/// has no app state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct BlockInput {
    /// The real reason code.
    pub code: String,
    /// Why, in the blocker's words.
    pub note: String,
    /// Whether the code is configured.
    pub known: bool,
    /// Whether the code is restricted.
    pub restricted: bool,
    /// For a system run, the user whose action caused it (stored as the
    /// Block record's blocked-by).
    pub ran_by: Option<i64>,
    /// For a system run made by a confirmed screening match, that screening
    /// (stored on the Block record so a rerun finds it).
    #[serde(default)]
    pub screening_id: Option<i64>,
}

/// The `block` and `block_by_system` transitions, for the machine.
pub(crate) fn transitions() -> [Transition<Party>; 2] {
    [
        rules(
            Transition::new(BLOCK)
                .label("Block")
                .tone("danger")
                .permission(permissions::BLOCK),
        ),
        rules(
            Transition::new(BLOCK_BY_SYSTEM)
                .label("Blocked by the system")
                .tone("danger")
                .system(),
        ),
    ]
}

/// The guards and the effect both block transitions share.
fn rules(transition: Transition<Party>) -> Transition<Party> {
    transition
        .from([PartyStatus::Active])
        .to(PartyStatus::Blocked)
        .guard("known_reason", |_party: &Party, ctx| {
            Box::pin(async move {
                match input(ctx) {
                    Some(input) if input.known => Ok(()),
                    _ => Err(english("errors.reason_unknown")),
                }
            })
        })
        .guard("note_given", |_party: &Party, ctx| {
            Box::pin(async move {
                match input(ctx) {
                    Some(input) if !input.note.trim().is_empty() => Ok(()),
                    _ => Err(english("errors.note_required")),
                }
            })
        })
        .guard("restricted_reason", |_party: &Party, ctx| {
            Box::pin(async move {
                let restricted = input(ctx).is_none_or(|input| input.restricted);
                // A system run (no actor) may use any code.
                let allowed = ctx
                    .actor()
                    .is_none_or(|user| user.has_permission(permissions::BLOCK_REASON_VIEW));
                if !restricted || allowed {
                    Ok(())
                } else {
                    Err(english("errors.reason_restricted"))
                }
            })
        })
        .effect(|party: &Party, ctx, tx| {
            let party_id = party.id;
            Box::pin(async move {
                let input = input(ctx).unwrap_or_default();
                let blocked_by = ctx.actor().map(|user| user.id).or(input.ran_by);
                let mut record = match input.screening_id {
                    Some(screening) => BlockRecord::open_for_screening(
                        party_id,
                        screening,
                        &input.code,
                        input.restricted,
                        &input.note,
                        blocked_by,
                    ),
                    None => BlockRecord::open(
                        party_id,
                        &input.code,
                        input.restricted,
                        &input.note,
                        blocked_by,
                    ),
                };
                record.insert(&mut *tx).await?;
                // Only the display code: never a restricted one (`PTY.ACL.02`).
                // Not the status column, so outside the machine's guard.
                renox::db::sql(format!(
                    "UPDATE {} SET hold_reason = ?, updated_at = ? WHERE id = ?",
                    Party::TABLE
                ))
                .bind(display_block_reason(&input.code, input.restricted))
                .bind(renox::db::now())
                .bind(party_id)
                .execute(&mut *tx)
                .await?;
                Ok(())
            })
        })
}

/// The run's input, if it is a [`BlockInput`].
fn input(ctx: &TransitionCtx) -> Option<BlockInput> {
    ctx.input_as::<BlockInput>().ok()
}

/// A guard message (guards have no request language).
fn english(key: &str) -> String {
    texts::english(&format!("wl_parties.{key}"))
}

/// The input for `code` and `note`, resolved against the app's reasons.
fn resolve(state: &AppState, code: &str, note: &str, ran_by: Option<i64>) -> BlockInput {
    let reasons = BlockReasons::of(state);
    let code = code.trim();
    BlockInput {
        code: code.to_owned(),
        note: note.trim().to_owned(),
        known: reasons.contains(code),
        restricted: reasons.is_restricted(code),
        ran_by,
        screening_id: None,
    }
}

/// `user` blocks `party` with the reason `code` and `note` (`block`). Needs
/// `parties.block`, and `parties.block_reason.view` for a restricted code.
///
/// # Errors
///
/// `Forbidden` without the permission, `NotFrom` when the party is not
/// `active`, `Guard` for an unknown code, a blank note or a restricted code
/// the user may not use; nothing is written then.
pub async fn block(
    state: &AppState,
    user: &AuthUser,
    party: &Party,
    code: &str,
    note: &str,
) -> std::result::Result<(), TransitionError> {
    let input = resolve(state, code, note, None);
    let display = display_block_reason(&input.code, input.restricted).to_owned();
    // `run` emits a done outcome after the commit. A pending one (an
    // approval policy on blocks; none by default, A10) moved nothing yet.
    let _outcome = party
        .transition(BLOCK)
        .by(user)
        .reason(display)
        .input(&input)
        .run(state)
        .await?;
    Ok(())
}

/// The system blocks `party` with the reason `code` and `note`
/// (`block_by_system`), e.g. on a confirmed sanctions match. `ran_by` is the
/// user whose action caused it (the reviewer who ran `confirm_match`),
/// stored as the Block record's blocked-by and in the history row's `meta`;
/// `None` for a pure system run. The history row has no actor.
///
/// # Errors
///
/// `NotFrom` when the party is not `active`, `Guard` for an unknown code or
/// a blank note.
pub async fn block_by_system(
    state: &AppState,
    party: &Party,
    code: &str,
    note: &str,
    ran_by: Option<i64>,
) -> std::result::Result<(), TransitionError> {
    block_system(state, party, None, code, note, ran_by).await
}

/// [`block_by_system`] for the confirmed match of screening `screening_id`:
/// the Block record keeps the screening id, so the confirmation can be run
/// again without a second record.
///
/// # Errors
///
/// As [`block_by_system`].
pub async fn block_by_system_for_screening(
    state: &AppState,
    party: &Party,
    screening_id: i64,
    code: &str,
    note: &str,
    ran_by: Option<i64>,
) -> std::result::Result<(), TransitionError> {
    block_system(state, party, Some(screening_id), code, note, ran_by).await
}

async fn block_system(
    state: &AppState,
    party: &Party,
    screening_id: Option<i64>,
    code: &str,
    note: &str,
    ran_by: Option<i64>,
) -> std::result::Result<(), TransitionError> {
    let mut input = resolve(state, code, note, ran_by);
    input.screening_id = screening_id;
    let display = display_block_reason(&input.code, input.restricted).to_owned();
    let meta = match ran_by {
        Some(user) => renox::serde_json::json!({ "ran_by": user }),
        None => renox::serde_json::Value::Null,
    };
    // `run` emits a done outcome after the commit. A pending one (an
    // approval policy on blocks; none by default, A10) moved nothing yet.
    let _outcome = party
        .transition(BLOCK_BY_SYSTEM)
        .by_system()
        .reason(display)
        .input(&input)
        .meta(meta)
        .run(state)
        .await?;
    Ok(())
}

/// The display code a block event carries: the run's reason (already the
/// display code), and `on_hold` for anything restricted or unknown, so a
/// restricted code can never leave in the event.
fn event_reason(reasons: &BlockReasons, reason: Option<&str>) -> String {
    match reason.map(str::trim) {
        Some(code) if !code.is_empty() && !reasons.is_restricted(code) => code.to_owned(),
        _ => NEUTRAL_BLOCK_REASON.to_owned(),
    }
}

/// Emits [`PartyBlocked`] after a block commits.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: Transitioned<Party>, state: AppState| async move {
        if event.transition != BLOCK && event.transition != BLOCK_BY_SYSTEM {
            return Ok(());
        }
        let reasons = BlockReasons::of(&state);
        state
            .emit(PartyBlocked {
                party: PartyId(event.key),
                display_reason: event_reason(&reasons, event.reason.as_deref()),
            })
            .await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_block_transitions_go_from_active_to_blocked() {
        let machine = Party::machine();
        for name in [BLOCK, BLOCK_BY_SYSTEM] {
            let transition = machine.find(name).expect("transition");
            assert_eq!(transition.from_states(), ["active"]);
            assert_eq!(transition.to_state(), Some("blocked"));
            assert_eq!(
                transition.guard_names(),
                ["known_reason", "note_given", "restricted_reason"]
            );
        }
        assert!(!machine.find(BLOCK).expect("block").is_system());
        assert!(machine.find(BLOCK_BY_SYSTEM).expect("system").is_system());
    }

    #[test]
    fn the_event_never_carries_a_restricted_code() {
        let reasons = BlockReasons::default();
        assert_eq!(event_reason(&reasons, Some("non_payment")), "non_payment");
        assert_eq!(event_reason(&reasons, Some("on_hold")), "on_hold");
        assert_eq!(event_reason(&reasons, Some("aml_concern")), "on_hold");
        assert_eq!(event_reason(&reasons, Some("sanctions")), "on_hold");
        assert_eq!(event_reason(&reasons, Some("unknown_code")), "on_hold");
        assert_eq!(event_reason(&reasons, None), "on_hold");
    }

    #[renox::test]
    async fn the_guards_refuse_a_blank_note_and_an_unknown_code() {
        let machine = Party::machine();
        let block = machine.find(BLOCK_BY_SYSTEM).expect("transition");
        let party = Party::default();
        let run = |input: BlockInput| {
            TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).expect("json"))
        };
        let good = BlockInput {
            code: "sanctions".into(),
            note: "confirmed match".into(),
            known: true,
            restricted: true,
            ran_by: None,
            screening_id: None,
        };
        assert!(block.check_guards(&party, &run(good.clone())).await.is_ok());
        let blank = BlockInput {
            note: "  ".into(),
            ..good.clone()
        };
        assert!(matches!(
            block.check_guards(&party, &run(blank)).await,
            Err(TransitionError::Guard(_))
        ));
        let unknown = BlockInput {
            known: false,
            ..good
        };
        assert!(matches!(
            block.check_guards(&party, &run(unknown)).await,
            Err(TransitionError::Guard(_))
        ));
    }
}
