//! Falling back, giving up and the give-up rule `NTF.RUL.01` (wl-notify
//! §4.1 `fall_back`, `give_up`; §5.4; #55 A8, A9).
//!
//! When a send fails for good on its channel ([`crate::send`]), [`plan`]
//! looks for the next channel in the type's order that is allowed for the
//! recipient, skipping the channels the fallback chain already used:
//!
//! - found: `fall_back` ends the message `abandoned` and queues a new
//!   message on that channel, with the same type, subject, recipient and
//!   data, `fallback_of` the failed message and the same `chain_id`. No
//!   reaction runs and no `MessageGaveUp` is emitted.
//! - none: `give_up` ends the message `abandoned` and, in the same
//!   transaction, queues [`RunGiveUpReaction`], so the default reaction
//!   never depends on a listener. After the commit, `MessageGaveUp` lists
//!   every channel of the chain with its last error ([`emit_gave_up`]).
//!
//! The run ([`run_reaction`]) is stored in `wl_notify_reaction_runs`, one
//! per message (the occurrence key); a second run for the same message is
//! stored `skipped` and does nothing. The type's reactions run in
//! [`reaction_order`] (`mark_contact_invalid` first), each through its
//! [`GiveUpHandler`](crate::reactions::GiveUpHandler), and each writes a
//! step. A failed attempt is retried with [`run_backoff`]; the reactions
//! that already have a step are not run again. After the last attempt the
//! run is `failed` and the kernel's `AutomationFailed` is emitted (not
//! compliance-critical); the message stays `abandoned`.

use std::time::Duration;

use renox::db::Json;
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::Outcome;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::automation::AutomationFailed;
use wl_kernel::notify::{
    ChannelTried, GiveUpReaction, MessageGaveUp, NotificationTypeDef, reaction_order,
};

use crate::machine::{FALL_BACK, GIVE_UP};
use crate::message::{Message, MessageStatus};
use crate::notifier::{compose, open_channels, type_def};
use crate::reactions::{self, GiveUpContext};
use crate::recipients::{self, Resolved};
use crate::send::{self, SendMessage};

/// The give-up rule's id (wl-notify §6).
pub const RULE: &str = "NTF.RUL.01";

/// The subject type of a message, e.g. in `AutomationFailed`.
pub const MESSAGE_SUBJECT_TYPE: &str = "notify.message";

/// Attempts of one `NTF.RUL.01` run before it is `failed`.
pub const RUN_MAX_ATTEMPTS: u32 = 5;

/// The wait before the next attempt of a run after failed attempt
/// `attempt` (1-based): 1 minute, 5 minutes, 15 minutes, then 1 hour.
pub fn run_backoff(attempt: u32) -> Duration {
    let minutes = match attempt {
        0 | 1 => 1,
        2 => 5,
        3 => 15,
        _ => 60,
    };
    Duration::from_secs(minutes * 60)
}

/// What follows a final failure on a channel.
#[derive(Debug, Clone)]
pub enum Plan {
    /// `fall_back`: a new message of the type `def` goes out on `resolved`.
    FallBack {
        /// The type as the client has it.
        def: Box<NotificationTypeDef>,
        /// The next channel and address.
        resolved: Resolved,
    },
    /// `give_up`: no channel is left.
    GiveUp,
}

/// The step after `message` failed for good on its channel: the next
/// allowed channel of its type that the chain did not use, else give up.
pub async fn plan(state: &AppState, message: &Message) -> Result<Plan> {
    let def = type_def(&state.db, &message.type_key).await?;
    let mut used: Vec<_> = Message::of_chain(&state.db, message.chain())
        .await?
        .iter()
        .map(|m| m.channel)
        .collect();
    used.push(message.channel);
    let open = open_channels(state);
    let resolved = recipients::resolve(state, &def, &message.recipient(), &open, &used).await?;
    if resolved.route.suppress.is_some() {
        return Ok(Plan::GiveUp);
    }
    Ok(Plan::FallBack {
        def: Box::new(def),
        resolved,
    })
}

/// What [`apply_in`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    /// `fall_back`: the new message's id.
    FellBack(i64),
    /// `give_up`: the rule's job is queued.
    GaveUp,
}

/// Runs `plan` on the `failed` message inside `tx`: `fall_back` with the
/// new message and its send job, or `give_up` with the rule's job. Returns
/// the transition's outcome and what was done.
pub async fn apply_in(
    state: &AppState,
    tx: &mut renox::db::Transaction,
    failed: &Message,
    plan: &Plan,
) -> Result<(Outcome<Message>, Applied)> {
    match plan {
        Plan::FallBack { def, resolved } => {
            let outcome = send::run(failed, FALL_BACK, None, tx).await?;
            let mut next = compose(
                def,
                &failed.subject(),
                &failed.recipient(),
                resolved,
                &failed.data.0,
            );
            next.chain_id = Some(failed.chain());
            next.fallback_of = Some(failed.id);
            next.insert(&mut *tx).await?;
            state
                .queue
                .dispatch_in(
                    tx,
                    SendMessage {
                        message_id: next.id,
                    },
                )
                .await?;
            Ok((outcome, Applied::FellBack(next.id)))
        }
        Plan::GiveUp => {
            let outcome = send::run(failed, GIVE_UP, None, tx).await?;
            state
                .queue
                .dispatch_in(
                    tx,
                    RunGiveUpReaction {
                        message_id: failed.id,
                    },
                )
                .await?;
            Ok((outcome, Applied::GaveUp))
        }
    }
}

/// The channels `chain` tried (oldest first), with their last errors;
/// `last` is added when the chain does not hold it.
pub fn channels_tried(chain: &[Message], last: &Message) -> Vec<ChannelTried> {
    let mut messages: Vec<&Message> = chain.iter().collect();
    if !messages.iter().any(|m| m.id == last.id) {
        messages.push(last);
    }
    messages
        .into_iter()
        .map(|m| ChannelTried {
            message: m.message_id(),
            channel: m.channel,
            last_error_code: m.last_error_code.clone(),
            last_error: m.last_error.clone(),
        })
        .collect()
}

/// The `MessageGaveUp` of `message` (the last of `chain`) of type `def`.
pub fn gave_up_event(
    def: &NotificationTypeDef,
    message: &Message,
    chain: &[Message],
) -> MessageGaveUp {
    MessageGaveUp {
        message: message.message_id(),
        type_key: message.type_key.clone(),
        subject: message.subject(),
        recipient: message.recipient(),
        contact_point: message.contact_point_id,
        channels_tried: channels_tried(chain, message),
        statutory: def.statutory,
        reactions: reaction_order(&def.give_up),
    }
}

/// Emits `MessageGaveUp` for message `id`, after its `give_up` committed.
pub async fn emit_gave_up(state: &AppState, id: i64) -> Result {
    let message = Message::find(&state.db, id)
        .await?
        .ok_or(renox::Error::NotFound)?;
    let def = type_def(&state.db, &message.type_key).await?;
    let chain = Message::of_chain(&state.db, message.chain()).await?;
    state.emit(gave_up_event(&def, &message, &chain)).await
}

/// Where a rule run is.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RunStatus {
    /// Started; an attempt failed or is under way.
    #[default]
    Running,
    /// Every reaction ran.
    Succeeded,
    /// Not run: the message already has a run, or did not give up.
    Skipped,
    /// The last attempt failed.
    Failed,
}

/// One run of `NTF.RUL.01` for one message.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_notify_reaction_runs", index(message_id))]
pub struct ReactionRun {
    /// The primary key.
    pub id: i64,
    /// The rule id, `NTF.RUL.01`.
    pub rule: String,
    /// The occurrence key: the message that gave up.
    pub message_id: i64,
    /// The queue job running it; its retries continue this run.
    pub job_id: Option<i64>,
    /// Where the run is.
    pub status: RunStatus,
    /// The attempts made.
    pub attempts: i64,
    /// The last error, or why it was skipped.
    pub error: Option<String>,
    /// The reactions, in run order.
    pub reactions: Json<Vec<GiveUpReaction>>,
    /// When it started.
    pub started_at: Option<DateTime>,
    /// When it ended.
    pub finished_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ReactionRun {
    /// The runs for message `id`, oldest first.
    pub async fn of_message(db: &Db, id: i64) -> Result<Vec<ReactionRun>> {
        ReactionRun::query()
            .where_(ReactionRun::MESSAGE_ID.eq(id))
            .order_by(ReactionRun::ID)
            .get(db)
            .await
    }

    /// Its steps, in run order.
    pub async fn steps(&self, db: &Db) -> Result<Vec<ReactionStep>> {
        ReactionStep::query()
            .where_(ReactionStep::RUN_ID.eq(self.id))
            .order_by(ReactionStep::POSITION)
            .get(db)
            .await
    }
}

/// One reaction a run applied.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_notify_reaction_steps", unique(run_id, reaction))]
pub struct ReactionStep {
    /// The primary key.
    pub id: i64,
    /// The run.
    pub run_id: i64,
    /// The reaction.
    pub reaction: GiveUpReaction,
    /// Its 1-based place in the run order.
    pub position: i64,
    /// What the handler did, e.g. `recorded`.
    pub status: String,
    /// The handler's details.
    pub detail: Json<Value>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The job that runs `NTF.RUL.01` for a message that gave up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunGiveUpReaction {
    /// The message that gave up.
    pub message_id: i64,
}

impl Job for RunGiveUpReaction {
    const NAME: &'static str = "wl-notify.run-give-up-reaction";
    const MAX_ATTEMPTS: u32 = RUN_MAX_ATTEMPTS;

    fn backoff(attempt: u32) -> Duration {
        run_backoff(attempt)
    }

    async fn handle(self, ctx: JobContext) -> Result {
        let attempt = Attempt {
            job_id: ctx.id,
            attempt: ctx.attempt,
        };
        run_reaction(&ctx.state, self.message_id, attempt)
            .await
            .map(|_| ())
    }
}

/// The queue attempt a run is part of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attempt {
    /// The job's id.
    pub job_id: i64,
    /// 1 on the first try.
    pub attempt: u32,
}

impl Attempt {
    /// Whether no retry follows a failure of this attempt.
    pub fn is_last(self) -> bool {
        self.attempt >= RUN_MAX_ATTEMPTS
    }
}

/// What one attempt of [`run_reaction`] came to (a failure is an error).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    /// Every reaction ran; the run `id` `succeeded`.
    Succeeded(i64),
    /// The run `id` was stored `skipped`: nothing ran.
    Skipped(i64),
}

/// Why a run was skipped: the message already has a run.
pub const ALREADY_RUN: &str = "the message already has a run";
/// Why a run was skipped: the message did not end through `give_up`.
pub const NOT_GIVEN_UP: &str = "the message did not give up";

/// Runs `NTF.RUL.01` for message `id`, as `attempt` of its job.
pub async fn run_reaction(state: &AppState, id: i64, attempt: Attempt) -> Result<RunOutcome> {
    let mut run = match claim(state, id, attempt).await? {
        Claim::Run(run) => run,
        Claim::Skipped(run) => return Ok(RunOutcome::Skipped(run)),
    };
    let message = Message::find(&state.db, id)
        .await?
        .ok_or(renox::Error::NotFound)?;
    run.attempts += 1;
    run.save(&state.db).await?;

    match apply_reactions(state, &run, &message).await {
        Ok(()) => {
            run.status = RunStatus::Succeeded;
            run.error = None;
            run.finished_at = Some(renox::db::now());
            run.save(&state.db).await?;
            Ok(RunOutcome::Succeeded(run.id))
        }
        Err(error) => {
            run.error = Some(error.to_string());
            let last = attempt.is_last() || error.is_permanent();
            if last {
                run.status = RunStatus::Failed;
                run.finished_at = Some(renox::db::now());
            }
            run.save(&state.db).await?;
            if last {
                state
                    .emit(AutomationFailed {
                        rule: RULE.to_owned(),
                        subject: Subject::new(MESSAGE_SUBJECT_TYPE, message.id),
                        error: error.to_string(),
                        compliance_critical: false,
                    })
                    .await?;
            }
            Err(error)
        }
    }
}

enum Claim {
    Run(ReactionRun),
    Skipped(i64),
}

/// The run this attempt works on: the job's own unfinished run on a
/// retry, else a new one; `skipped` when the message has a run already or
/// did not give up.
async fn claim(state: &AppState, id: i64, attempt: Attempt) -> Result<Claim> {
    // Read before the transaction: the type may come from the database.
    let reactions = match Message::find(&state.db, id).await? {
        Some(message) => reaction_order(&type_def(&state.db, &message.type_key).await?.give_up),
        None => Vec::new(),
    };
    let mut tx = state.db.begin().await?;
    let existing = ReactionRun::query()
        .where_(ReactionRun::RULE.eq(RULE))
        .where_(ReactionRun::MESSAGE_ID.eq(id))
        .where_(ReactionRun::STATUS.ne(RunStatus::Skipped))
        .first(&mut tx)
        .await?;
    let skip = match existing {
        Some(run)
            if run.status == RunStatus::Running
                && run.job_id == Some(attempt.job_id)
                && attempt.attempt > 1 =>
        {
            tx.commit().await?;
            return Ok(Claim::Run(run));
        }
        Some(_) => Some(ALREADY_RUN),
        None if !gave_up(&mut tx, id).await? => Some(NOT_GIVEN_UP),
        None => None,
    };
    let now = renox::db::now();
    let mut run = ReactionRun {
        rule: RULE.to_owned(),
        message_id: id,
        job_id: Some(attempt.job_id),
        started_at: Some(now),
        ..Default::default()
    };
    if let Some(reason) = skip {
        run.status = RunStatus::Skipped;
        run.error = Some(reason.to_owned());
        run.finished_at = Some(now);
        run.insert(&mut tx).await?;
        tx.commit().await?;
        return Ok(Claim::Skipped(run.id));
    }
    run.status = RunStatus::Running;
    run.reactions = Json(reactions);
    run.insert(&mut tx).await?;
    tx.commit().await?;
    Ok(Claim::Run(run))
}

/// Whether message `id` ended `abandoned` through `give_up`: abandoned,
/// and no message falls back from it.
async fn gave_up(tx: &mut renox::db::Transaction, id: i64) -> Result<bool> {
    let Some(message) = Message::find(&mut *tx, id).await? else {
        return Ok(false);
    };
    if message.status != MessageStatus::Abandoned {
        return Ok(false);
    }
    let fell_back = Message::query()
        .where_(Message::FALLBACK_OF.eq(id))
        .exists(&mut *tx)
        .await?;
    Ok(!fell_back)
}

/// Runs the run's reactions that have no step yet, in order, each writing
/// its step.
async fn apply_reactions(state: &AppState, run: &ReactionRun, message: &Message) -> Result {
    let def = type_def(&state.db, &message.type_key).await?;
    let chain = Message::of_chain(&state.db, message.chain()).await?;
    let ctx = GiveUpContext {
        run_id: run.id,
        message: message.message_id(),
        type_key: message.type_key.clone(),
        subject: message.subject(),
        recipient: message.recipient(),
        contact_point: message.contact_point_id,
        channels_tried: channels_tried(&chain, message),
        statutory: def.statutory,
        staff_role: def.staff_role.clone(),
        reactions: run.reactions.0.clone(),
    };
    let done: Vec<GiveUpReaction> = run
        .steps(&state.db)
        .await?
        .into_iter()
        .map(|s| s.reaction)
        .collect();
    let handlers = reactions::reactions(state);
    for (index, &reaction) in run.reactions.0.iter().enumerate() {
        if done.contains(&reaction) {
            continue;
        }
        let handler = handlers.for_reaction(reaction).ok_or_else(|| {
            renox::Error::permanent_message(format!(
                "no handler for the give-up reaction `{}`",
                reaction.as_str()
            ))
        })?;
        let outcome = handler.run(state, &ctx).await?;
        ReactionStep {
            run_id: run.id,
            reaction,
            position: i64::try_from(index + 1).unwrap_or(i64::MAX),
            status: outcome.status,
            detail: Json(outcome.detail),
            ..Default::default()
        }
        .insert(&state.db)
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use wl_kernel::PartyId;
    use wl_kernel::notify::{GiveUpReaction as R, NotifyChannel, NotifyRecipient};

    use super::*;

    fn failed(id: i64, channel: NotifyChannel, code: &str, text: &str) -> Message {
        Message {
            id,
            type_key: "invoice.reminder".into(),
            subject_type: "invoicing.invoice".into(),
            subject_id: 9,
            chain_id: Some(4),
            party_id: Some(3),
            contact_point_id: Some(12),
            channel,
            status: MessageStatus::Abandoned,
            last_error_code: Some(code.into()),
            last_error: Some(text.into()),
            ..Default::default()
        }
    }

    #[test]
    fn the_run_backoff_grows_to_an_hour() {
        let waits: Vec<_> = (1..=4).map(|a| run_backoff(a).as_secs() / 60).collect();
        assert_eq!(waits, [1, 5, 15, 60]);
        assert_eq!(RUN_MAX_ATTEMPTS, 5);
    }

    #[test]
    fn only_the_fifth_attempt_is_the_last() {
        let attempt = |attempt| Attempt { job_id: 1, attempt };
        assert!(!attempt(4).is_last());
        assert!(attempt(5).is_last());
    }

    #[test]
    fn the_event_lists_every_channel_of_the_chain() {
        let email = failed(4, NotifyChannel::Email, "550", "mailbox unknown");
        let sms = Message {
            fallback_of: Some(4),
            ..failed(5, NotifyChannel::Sms, "timeout", "gateway timeout")
        };
        let def = NotificationTypeDef::new("invoice.reminder", "invoicing").give_up(&[
            R::AlternativeRecipient,
            R::MarkContactInvalid,
            R::StaffTask,
        ]);
        let event = gave_up_event(&def, &sms, &[email.clone(), sms.clone()]);
        assert_eq!(event.message, sms.message_id());
        assert_eq!(event.recipient, NotifyRecipient::Party { id: PartyId(3) });
        assert_eq!(event.contact_point, Some(12));
        assert!(!event.statutory);
        let tried: Vec<_> = event
            .channels_tried
            .iter()
            .map(|t| {
                (
                    t.channel,
                    t.last_error_code.as_deref(),
                    t.last_error.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            tried,
            [
                (NotifyChannel::Email, Some("550"), Some("mailbox unknown")),
                (NotifyChannel::Sms, Some("timeout"), Some("gateway timeout")),
            ]
        );
        assert_eq!(
            event.reactions,
            [R::MarkContactInvalid, R::AlternativeRecipient, R::StaffTask]
        );
    }

    #[test]
    fn the_last_message_is_listed_when_the_chain_lacks_it() {
        let lone = failed(7, NotifyChannel::Email, "550", "mailbox unknown");
        let tried = channels_tried(&[], &lone);
        assert_eq!(tried.len(), 1);
        assert_eq!(tried[0].message, lone.message_id());
    }

    #[test]
    fn run_statuses_are_stored_in_snake_case() {
        let names: Vec<_> = RunStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(names, ["running", "succeeded", "skipped", "failed"]);
    }
}
