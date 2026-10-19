//! The `alternative_recipient` give-up reaction (wl-notify §5.4; #57 A4-A8).
//!
//! A message sent to a recipient rule ("the finance contacts of Blue Dunes
//! LLC") that gave up is queued once more to another contact of the same
//! party. In this order:
//!
//! 1. The message is itself an alternative: `alternative_already_tried`.
//! 2. The chain already has an alternative (a retried run): `queued`
//!    again, naming that message, and nothing new is queued.
//! 3. The recipient is not a rule: `no_rule`.
//! 4. The rule is re-run without the persons the chain already tried and
//!    without the contacts that cannot be reached (no address on any open
//!    channel of the type, an address on the suppression list, or a party
//!    on hold). Nobody left: `none_left`.
//! 5. Else the same message (type, subject, label, data, rule) is queued
//!    for the first contact left, written in their language on the channel
//!    chosen for them, with `alternative_of` the first message of the
//!    chain: `queued`.
//!
//! When `queued`, the run's later `staff_task` is `deferred` to the
//! alternative. In every other outcome the staff task runs once for the
//! chain: through the type's own `staff_task` step when it comes later in
//! the run order, otherwise by this handler ([`open_give_up_task`]), which
//! adds the task's step under `staff_task` in the step detail.

use renox::prelude::*;
use renox::serde_json::{Value, json};
use wl_kernel::PartyId;
use wl_kernel::notify::{GiveUpReaction, NotifyRecipient};

use crate::message::{Message, RecipientKind};
use crate::notifier::{compose, open_channels, type_def};
use crate::reactions::{GiveUpContext, GiveUpHandler, HandlerFuture, ReactionOutcome, StepOutcome};
use crate::recipients::{self, Resolved};
use crate::send::SendMessage;
use crate::staff_task::{StaffTask, open_give_up_task};

/// The step status when the message was queued to another contact.
pub const QUEUED: &str = "queued";
/// The step status when no other contact can be reached.
pub const NONE_LEFT: &str = "none_left";
/// The step status when the recipient is not a recipient rule.
pub const NO_RULE: &str = "no_rule";
/// The step status when the message is itself an alternative.
pub const ALTERNATIVE_ALREADY_TRIED: &str = "alternative_already_tried";

/// The built-in `alternative_recipient` handler, registered by
/// [`crate::WlNotify`].
#[derive(Debug, Clone, Copy, Default)]
pub struct AlternativeRecipientHandler;

impl GiveUpHandler for AlternativeRecipientHandler {
    fn reaction(&self) -> GiveUpReaction {
        GiveUpReaction::AlternativeRecipient
    }

    fn run<'a>(
        &'a self,
        state: &'a AppState,
        ctx: &'a GiveUpContext,
    ) -> HandlerFuture<'a, StepOutcome> {
        Box::pin(async move { alternative(state, ctx).await.map(|o| o.step) })
    }

    fn run_outcome<'a>(
        &'a self,
        state: &'a AppState,
        ctx: &'a GiveUpContext,
    ) -> HandlerFuture<'a, ReactionOutcome> {
        Box::pin(alternative(state, ctx))
    }
}

/// Whether the run's `staff_task` comes after `alternative_recipient`.
pub fn staff_task_follows(reactions: &[GiveUpReaction]) -> bool {
    let own = reactions
        .iter()
        .position(|r| *r == GiveUpReaction::AlternativeRecipient);
    let task = reactions
        .iter()
        .position(|r| *r == GiveUpReaction::StaffTask);
    matches!((own, task), (Some(own), Some(task)) if task > own)
}

/// The persons a chain already went to, in order and without repeats.
pub fn tried_persons(chain: &[Message], last: &Message) -> Vec<PartyId> {
    let mut out: Vec<PartyId> = Vec::new();
    for message in chain.iter().chain(std::iter::once(last)) {
        if let Some(id) = message.party_id.map(PartyId)
            && !out.contains(&id)
        {
            out.push(id);
        }
    }
    out
}

/// The outcome of a give-up that queued nothing: the step, and the staff
/// task opened here unless the run's own `staff_task` step follows.
async fn then_staff_task(
    state: &AppState,
    ctx: &GiveUpContext,
    status: &str,
    mut detail: Value,
) -> Result<ReactionOutcome> {
    if !staff_task_follows(&ctx.reactions) {
        let task = StaffTask {
            role: ctx.staff_role.clone(),
            ..Default::default()
        };
        let opened = open_give_up_task(state, ctx, task).await?;
        detail["staff_task"] = json!({ "status": opened.status, "detail": opened.detail });
    }
    Ok(StepOutcome::new(status, detail).into())
}

/// The outcome of a chain whose alternative is `message`.
fn queued(ctx: &GiveUpContext, message: &Message) -> ReactionOutcome {
    let detail = json!({
        "message_id": message.id,
        "party_id": message.party_id,
        "channel": message.channel.as_str(),
    });
    let outcome = ReactionOutcome::of(StepOutcome::new(QUEUED, detail.clone()));
    if staff_task_follows(&ctx.reactions) {
        outcome.deferring(&[GiveUpReaction::StaffTask], detail)
    } else {
        outcome
    }
}

/// Runs the reaction for the give-up `ctx`.
async fn alternative(state: &AppState, ctx: &GiveUpContext) -> Result<ReactionOutcome> {
    let last = Message::find(&state.db, ctx.message.0)
        .await?
        .ok_or(renox::Error::NotFound)?;
    if let Some(root) = last.alternative_of {
        return then_staff_task(
            state,
            ctx,
            ALTERNATIVE_ALREADY_TRIED,
            json!({ "alternative_of": root }),
        )
        .await;
    }
    let root = last.chain();
    // Checked first: a retried run finds the alternative it queued.
    let existing = Message::query()
        .where_(Message::ALTERNATIVE_OF.eq(root))
        .order_by(Message::ID)
        .first(&state.db)
        .await?;
    if let Some(message) = existing {
        return Ok(queued(ctx, &message));
    }
    let NotifyRecipient::Contact { of, purpose } = &ctx.recipient else {
        return then_staff_task(state, ctx, NO_RULE, json!({})).await;
    };

    let def = type_def(&state.db, &last.type_key).await?;
    let open = open_channels(state);
    let chain = Message::of_chain(&state.db, root).await?;
    let mut excluded = tried_persons(&chain, &last);
    let tried = excluded.len();
    let mut found: Option<(PartyId, Resolved)> = None;
    // Each round excludes one more person, so the loop ends.
    while let Some(person) = recipients::choose_person(state, *of, purpose, &excluded).await? {
        if excluded.contains(&person) {
            break;
        }
        let party = NotifyRecipient::Party { id: person };
        let resolved = recipients::resolve(state, &def, &party, &open, &[]).await?;
        if resolved.route.suppress.is_none() {
            found = Some((person, resolved));
            break;
        }
        excluded.push(person);
    }
    let Some((person, mut resolved)) = found else {
        let detail = json!({
            "tried": excluded[..tried].iter().map(|p| p.0).collect::<Vec<_>>(),
            "unreachable": excluded[tried..].iter().map(|p| p.0).collect::<Vec<_>>(),
        });
        return then_staff_task(state, ctx, NONE_LEFT, detail).await;
    };
    resolved.chosen = Some(person);

    let mut next = compose(
        &def,
        &last.subject(),
        &ctx.recipient,
        &resolved,
        &last.data.0,
    );
    next.subject_label.clone_from(&last.subject_label);
    next.alternative_of = Some(root);
    debug_assert_eq!(next.recipient_kind, RecipientKind::Contact);
    let mut tx = state.db.begin().await?;
    next.insert(&mut tx).await?;
    // The first message of its own, new chain.
    next.chain_id = Some(next.id);
    next.save(&mut tx).await?;
    state
        .queue
        .dispatch_in(
            &mut tx,
            SendMessage {
                message_id: next.id,
            },
        )
        .await?;
    tx.commit().await?;
    Ok(queued(ctx, &next))
}

#[cfg(test)]
mod tests {
    use GiveUpReaction as R;

    use super::*;

    fn sent(id: i64, party: Option<i64>) -> Message {
        Message {
            id,
            party_id: party,
            ..Default::default()
        }
    }

    #[test]
    fn the_staff_task_follows_only_when_it_comes_later() {
        assert!(staff_task_follows(&[R::AlternativeRecipient, R::StaffTask]));
        assert!(staff_task_follows(&[
            R::MarkContactInvalid,
            R::AlternativeRecipient,
            R::StaffTask
        ]));
        assert!(!staff_task_follows(&[R::AlternativeRecipient]));
        assert!(!staff_task_follows(&[
            R::StaffTask,
            R::AlternativeRecipient
        ]));
        assert!(!staff_task_follows(&[R::StaffTask]));
    }

    #[test]
    fn the_persons_of_the_chain_are_tried_once_each() {
        let chain = [sent(1, Some(4)), sent(2, Some(4))];
        let last = sent(3, Some(4));
        assert_eq!(tried_persons(&chain, &last), [PartyId(4)]);
        let other = [sent(1, Some(4)), sent(2, None)];
        assert_eq!(
            tried_persons(&other, &sent(3, Some(7))),
            [PartyId(4), PartyId(7)]
        );
    }

    #[test]
    fn the_last_message_counts_when_the_chain_lacks_it() {
        assert_eq!(tried_persons(&[], &sent(9, Some(5))), [PartyId(5)]);
        assert!(tried_persons(&[], &sent(9, None)).is_empty());
    }

    fn ctx(reactions: &[GiveUpReaction]) -> GiveUpContext {
        GiveUpContext {
            run_id: 1,
            message: wl_kernel::notify::MessageId(5),
            type_key: "invoice.reminder".into(),
            subject: wl_kernel::Subject::new("invoicing.invoice", 9),
            subject_label: None,
            recipient: NotifyRecipient::Party { id: PartyId(2) },
            gave_up_at: renox::db::now(),
            contact_point: None,
            channels_tried: Vec::new(),
            statutory: false,
            staff_role: None,
            urgency: wl_kernel::notify::Urgency::Normal,
            reactions: reactions.to_vec(),
        }
    }

    #[test]
    fn a_queued_alternative_defers_the_later_staff_task() {
        let message = Message {
            id: 12,
            party_id: Some(8),
            ..Default::default()
        };
        let outcome = queued(&ctx(&[R::AlternativeRecipient, R::StaffTask]), &message);
        assert_eq!(outcome.step.status, QUEUED);
        assert_eq!(outcome.step.detail["message_id"], 12);
        assert_eq!(outcome.defer, [R::StaffTask]);
        assert_eq!(outcome.defer_detail["party_id"], 8);
        let alone = queued(&ctx(&[R::AlternativeRecipient]), &message);
        assert!(alone.defer.is_empty());
    }

    #[test]
    fn the_handler_is_for_the_alternative_recipient() {
        assert_eq!(
            AlternativeRecipientHandler.reaction(),
            GiveUpReaction::AlternativeRecipient
        );
    }
}
