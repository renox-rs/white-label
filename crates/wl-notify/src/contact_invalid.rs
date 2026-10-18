//! The `mark_contact_invalid` give-up reaction (wl-notify §5.4; #58 A3, A4).
//!
//! When a chain gives up, every message of it whose `error_kind` is a hard
//! kind (`hard_bounce`, `invalid_number`, `unknown_account`) and that went
//! to a party contact point marks that point invalid through the kernel's
//! `PartyDirectory::invalidate_contact`, with the give-up time as since and
//! the kind as reason. Transient and permanent errors never mark anything.
//!
//! The step ends with one outcome, recorded in the run:
//!
//! - `marked`: at least one point went from valid to invalid; the detail
//!   lists them under `marked`;
//! - `already_invalid`: hard failures with points, but none changed (the
//!   point was invalid already, or no party directory knows it);
//! - `no_contact_point`: hard failures, but none of them went to a party
//!   contact point (a user or a bare address);
//! - `no_hard_failure`: the chain has no hard error.
//!
//! `ContactPointInvalidated` is emitted once per marked point after the
//! step is stored ([`GiveUpHandler::after_step`]).

use renox::prelude::*;
use renox::serde_json::{self, json};
use wl_kernel::notify::{ContactPointInvalidated, GiveUpReaction};

use crate::message::Message;
use crate::reactions::{GiveUpContext, GiveUpHandler, HandlerFuture, StepOutcome};

/// The step status when at least one point was marked invalid.
pub const MARKED: &str = "marked";
/// The step status when every point was invalid already.
pub const ALREADY_INVALID: &str = "already_invalid";
/// The step status when no hard failure went to a party contact point.
pub const NO_CONTACT_POINT: &str = "no_contact_point";
/// The step status when the chain has no hard error.
pub const NO_HARD_FAILURE: &str = "no_hard_failure";

/// The `error_kind`s that mark a contact point invalid.
pub const HARD_KINDS: [&str; 3] = ["hard_bounce", "invalid_number", "unknown_account"];

/// Whether `kind` is a hard error kind.
pub fn is_hard(kind: Option<&str>) -> bool {
    kind.is_some_and(|k| HARD_KINDS.contains(&k))
}

/// The built-in `mark_contact_invalid` handler, registered by
/// [`crate::WlNotify`].
#[derive(Debug, Clone, Copy, Default)]
pub struct MarkContactInvalidHandler;

impl GiveUpHandler for MarkContactInvalidHandler {
    fn reaction(&self) -> GiveUpReaction {
        GiveUpReaction::MarkContactInvalid
    }

    fn run<'a>(
        &'a self,
        state: &'a AppState,
        ctx: &'a GiveUpContext,
    ) -> HandlerFuture<'a, StepOutcome> {
        Box::pin(mark(state, ctx))
    }

    fn after_step<'a>(
        &'a self,
        state: &'a AppState,
        _ctx: &'a GiveUpContext,
        step: &'a StepOutcome,
    ) -> HandlerFuture<'a, ()> {
        Box::pin(async move {
            for event in marked_events(step) {
                state.emit(event).await?;
            }
            Ok(())
        })
    }
}

/// The points a `marked` step marked, as the events to emit.
pub fn marked_events(step: &StepOutcome) -> Vec<ContactPointInvalidated> {
    if step.status != MARKED {
        return Vec::new();
    }
    step.detail
        .get("marked")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

/// The step for a chain, given what was marked, how many hard failures it
/// had and how many of those went to a contact point.
pub fn outcome(marked: &[ContactPointInvalidated], hard: usize, with_point: usize) -> StepOutcome {
    if !marked.is_empty() {
        return StepOutcome::new(MARKED, json!({ "marked": marked }));
    }
    let status = if hard == 0 {
        NO_HARD_FAILURE
    } else if with_point == 0 {
        NO_CONTACT_POINT
    } else {
        ALREADY_INVALID
    };
    StepOutcome::new(status, json!({}))
}

/// Marks the contact points of the chain's hard failures invalid.
async fn mark(state: &AppState, ctx: &GiveUpContext) -> Result<StepOutcome> {
    let last = Message::find(&state.db, ctx.message.0)
        .await?
        .ok_or(renox::Error::NotFound)?;
    let chain = Message::of_chain(&state.db, last.chain()).await?;
    let directory = wl_kernel::parties::directory(state);
    let mut marked: Vec<ContactPointInvalidated> = Vec::new();
    let (mut hard, mut with_point) = (0, 0);
    let mut seen: Vec<i64> = Vec::new();
    for message in &chain {
        let Some(kind) = message.error_kind.as_deref().filter(|k| is_hard(Some(k))) else {
            continue;
        };
        hard += 1;
        let Some(point) = message.contact_point_id else {
            continue;
        };
        with_point += 1;
        if seen.contains(&point) {
            continue;
        }
        seen.push(point);
        if let Some(found) = directory
            .invalidate_contact(&state.db, point, ctx.gave_up_at, kind)
            .await?
        {
            marked.push(ContactPointInvalidated {
                contact_point: point,
                party: found.party,
                channel: message.channel,
                reason: kind.to_owned(),
            });
        }
    }
    Ok(outcome(&marked, hard, with_point))
}

#[cfg(test)]
mod tests {
    use wl_kernel::PartyId;
    use wl_kernel::notify::NotifyChannel;

    use renox::serde_json::Value;

    use super::*;

    fn point(id: i64, channel: NotifyChannel) -> ContactPointInvalidated {
        ContactPointInvalidated {
            contact_point: id,
            party: PartyId(3),
            channel,
            reason: "hard_bounce".into(),
        }
    }

    #[test]
    fn only_the_hard_kinds_count() {
        for kind in ["hard_bounce", "invalid_number", "unknown_account"] {
            assert!(is_hard(Some(kind)), "{kind}");
        }
        for kind in ["transient", "permanent", ""] {
            assert!(!is_hard(Some(kind)), "{kind}");
        }
        assert!(!is_hard(None));
    }

    #[test]
    fn the_hard_kinds_are_the_ones_the_sender_stores() {
        use crate::channels::{HardError, SendErrorKind};
        for hard in [
            HardError::HardBounce,
            HardError::InvalidNumber,
            HardError::UnknownAccount,
        ] {
            assert!(is_hard(Some(SendErrorKind::Hard(hard).as_str())));
        }
        assert!(!is_hard(Some(SendErrorKind::Transient.as_str())));
        assert!(!is_hard(Some(SendErrorKind::Permanent.as_str())));
    }

    #[test]
    fn the_outcome_names_what_happened() {
        let one = [point(7, NotifyChannel::Email)];
        assert_eq!(outcome(&one, 1, 1).status, MARKED);
        assert_eq!(outcome(&[], 0, 0).status, NO_HARD_FAILURE);
        assert_eq!(outcome(&[], 2, 0).status, NO_CONTACT_POINT);
        assert_eq!(outcome(&[], 2, 1).status, ALREADY_INVALID);
    }

    #[test]
    fn a_marked_step_gives_back_its_events() {
        let points = [point(7, NotifyChannel::Email), point(9, NotifyChannel::Sms)];
        let step = outcome(&points, 2, 2);
        assert_eq!(marked_events(&step), points);
        assert_eq!(step.detail["marked"][0]["contact_point"], 7);
        assert_eq!(step.detail["marked"][0]["reason"], "hard_bounce");
    }

    #[test]
    fn a_step_that_marked_nothing_emits_nothing() {
        assert!(marked_events(&outcome(&[], 1, 1)).is_empty());
        assert!(marked_events(&StepOutcome::new(MARKED, Value::Null)).is_empty());
    }

    #[test]
    fn the_handler_is_for_mark_contact_invalid() {
        assert_eq!(
            MarkContactInvalidHandler.reaction(),
            GiveUpReaction::MarkContactInvalid
        );
    }
}
