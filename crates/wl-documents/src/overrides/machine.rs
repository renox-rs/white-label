//! The BlockOverride state machine (wl-documents §4.3, #50).
//!
//! ```text
//! [*] --> requested --grant_approved--> approved --use--> used
//!         requested --refuse--> refused   approved <--unuse-- used
//! ```
//!
//! All four transitions are the system's: `grant_approved` is run by the
//! kernel on the last approval (the actor in the run's metadata is the
//! approver), `refuse` by the `ApprovalDecided` listener, `use` and `unuse`
//! by the guard when a guarded transition claims the override or fails.
//! The row is inserted as `requested` by the request service.

use renox::db::{DateTime, now};
use renox::prelude::*;
use renox::state_machine::{Machine, Outcome, StateMachine, Transition};
use wl_kernel::approvals::ApprovalEnd;

use super::OVERRIDE_VALIDITY_HOURS;
use super::model::{BlockOverride, BlockOverrideStatus};

/// The transition the kernel runs on the last approval.
pub const GRANT_APPROVED: &str = "grant_approved";
/// The transition for an approval that ended without a grant.
pub const REFUSE: &str = "refuse";
/// The transition that stamps the use.
pub const USE: &str = "use";
/// The transition that gives the override back when the guarded run failed.
pub const UNUSE: &str = "unuse";

impl StateMachine for BlockOverride {
    type State = BlockOverrideStatus;
    const COLUMN: renox::db::Col<BlockOverride, BlockOverrideStatus> = BlockOverride::STATUS;

    fn machine() -> Machine<Self> {
        use BlockOverrideStatus::{Approved, Refused, Requested, Used};
        Machine::new()
            // Inserted by the request service.
            .initial([Requested])
            .state(Requested)
            .tone("warning")
            .state(Approved)
            .tone("success")
            .state(Refused)
            .tone("error")
            .terminal()
            .state(Used)
            .tone("neutral")
            .transition(grant_approved())
            .transition(refuse())
            .transition(use_override())
            .transition(unuse())
    }
}

/// The end of the validity of an override approved at `at`.
pub fn valid_until(at: DateTime) -> DateTime {
    at + renox::chrono::Duration::hours(OVERRIDE_VALIDITY_HOURS)
}

/// `grant_approved`: requested → approved. Stamps who approved (the actor
/// in the run's metadata), when, and the end of the validity.
fn grant_approved() -> Transition<BlockOverride> {
    Transition::new(GRANT_APPROVED)
        .label("Approve")
        .tone("success")
        .system()
        .from([BlockOverrideStatus::Requested])
        .to(BlockOverrideStatus::Approved)
        .effect(|o: &BlockOverride, ctx, tx| {
            let id = o.id;
            let by = ctx.meta().get("actor_id").and_then(|v| v.as_i64());
            Box::pin(async move {
                let at = now();
                let until = valid_until(at);
                BlockOverride::query()
                    .where_(BlockOverride::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("approved_by", &by),
                            ("approved_at", &at),
                            ("valid_until", &until),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// `refuse`: requested → refused.
fn refuse() -> Transition<BlockOverride> {
    Transition::new(REFUSE)
        .label("Refuse")
        .tone("error")
        .system()
        .from([BlockOverrideStatus::Requested])
        .to(BlockOverrideStatus::Refused)
}

/// `use`: approved → used, while still valid. Stamps `used_at`.
fn use_override() -> Transition<BlockOverride> {
    Transition::new(USE)
        .label("Use")
        .tone("neutral")
        .system()
        .from([BlockOverrideStatus::Approved])
        .to(BlockOverrideStatus::Used)
        .guard("still_valid", |o: &BlockOverride, _ctx| {
            let valid = o.usable_at(now());
            Box::pin(async move {
                if valid {
                    Ok(())
                } else {
                    Err("The override has expired or was already used.".to_owned())
                }
            })
        })
        .effect(|o: &BlockOverride, _ctx, tx| {
            let id = o.id;
            Box::pin(async move {
                BlockOverride::query()
                    .where_(BlockOverride::ID.eq(id))
                    .update(&mut *tx, &[("used_at", &Some(now()))])
                    .await?;
                Ok(())
            })
        })
}

/// `unuse`: used → approved, with `used_at` empty again, when the guarded
/// run failed.
fn unuse() -> Transition<BlockOverride> {
    Transition::new(UNUSE)
        .label("Give back")
        .tone("neutral")
        .system()
        .from([BlockOverrideStatus::Used])
        .to(BlockOverrideStatus::Approved)
        .effect(|o: &BlockOverride, _ctx, tx| {
            let id = o.id;
            Box::pin(async move {
                BlockOverride::query()
                    .where_(BlockOverride::ID.eq(id))
                    .update(&mut *tx, &[("used_at", &None::<DateTime>)])
                    .await?;
                Ok(())
            })
        })
}

/// Runs `refuse` on override `id` after its approval ended as `end`. An
/// override that is no longer `requested` is left as it is.
pub(crate) async fn refuse_override(state: &AppState, id: i64, end: ApprovalEnd) -> Result {
    let Some(record) = BlockOverride::find(&state.db, id).await? else {
        return Ok(());
    };
    if record.status != BlockOverrideStatus::Requested {
        return Ok(());
    }
    match record
        .transition(REFUSE)
        .by_system()
        .reason(format!("The approval ended: {end:?}."))
        .run(state)
        .await
    {
        Ok(outcome @ Outcome::Done(_)) => outcome.emit(state).await,
        // Moved meanwhile: nothing to refuse.
        Ok(_) | Err(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_has_the_four_system_transitions() {
        let machine = BlockOverride::machine();
        for name in [GRANT_APPROVED, REFUSE, USE, UNUSE] {
            let transition = machine.find(name).expect(name);
            assert!(transition.is_system(), "{name}");
        }
    }

    #[test]
    fn the_transitions_connect_the_states() {
        let machine = BlockOverride::machine();
        let ends = |name: &str| {
            let t = machine.find(name).expect(name);
            (t.from_states().to_vec(), t.to_state().map(str::to_owned))
        };
        assert_eq!(
            ends(GRANT_APPROVED),
            (vec!["requested".to_owned()], Some("approved".to_owned()))
        );
        assert_eq!(
            ends(REFUSE),
            (vec!["requested".to_owned()], Some("refused".to_owned()))
        );
        assert_eq!(
            ends(USE),
            (vec!["approved".to_owned()], Some("used".to_owned()))
        );
        assert_eq!(
            ends(UNUSE),
            (vec!["used".to_owned()], Some("approved".to_owned()))
        );
    }

    #[test]
    fn validity_is_72_hours() {
        let at = now();
        assert_eq!(valid_until(at) - at, renox::chrono::Duration::hours(72));
    }
}
