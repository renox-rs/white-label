//! The give-up reactions (wl-notify §5.4): one handler per reaction key,
//! run by `NTF.RUL.01` ([`crate::give_up`]).
//!
//! `staff_task` opens a task for staff ([`crate::staff_task`]);
//! `alternative_recipient` ([`crate::alternative`]), `mark_contact_invalid`
//! ([`crate::contact_invalid`]) and `statutory_notice`
//! ([`crate::statutory_notice`]) are built in too. [`RecordingHandler`]
//! only records what it was given (status `recorded`). An app replaces a
//! handler through [`crate::WlNotify::reaction`]. `none` has no handler: it
//! runs nothing.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::prelude::*;
use renox::serde_json::{self, Value};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::notify::{ChannelTried, GiveUpReaction, MessageId, NotifyRecipient, Urgency};

/// A boxed future returned by a [`GiveUpHandler`].
pub type HandlerFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// What a give-up reaction is told about the message that gave up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GiveUpContext {
    /// The `NTF.RUL.01` run.
    pub run_id: i64,
    /// The last message of the chain (the one that gave up).
    pub message: MessageId,
    /// The type key.
    pub type_key: String,
    /// The record it is about.
    pub subject: Subject,
    /// The request's label for the subject, if any.
    pub subject_label: Option<String>,
    /// Who it was for.
    pub recipient: NotifyRecipient,
    /// When the message gave up; a retried run sees the same time.
    pub gave_up_at: DateTime,
    /// The party contact point of the last message, if any.
    pub contact_point: Option<i64>,
    /// Every channel of the chain, in order, with its last error.
    pub channels_tried: Vec<ChannelTried>,
    /// Whether the type is statutory.
    pub statutory: bool,
    /// The staff role told on give-up, e.g. `collector`.
    pub staff_role: Option<String>,
    /// The type's urgency; `urgent` also sends the in-app alert.
    pub urgency: Urgency,
    /// The run's reactions, in run order.
    pub reactions: Vec<GiveUpReaction>,
}

/// What one reaction did, stored as the run's step.
#[derive(Debug, Clone, PartialEq)]
pub struct StepOutcome {
    /// A short status, e.g. `recorded`, `task_created`.
    pub status: String,
    /// Details for the run record.
    pub detail: Value,
}

/// The status of a step a built-in handler wrote.
pub const RECORDED: &str = "recorded";

impl StepOutcome {
    /// A step with `status` and `detail`.
    pub fn new(status: &str, detail: Value) -> Self {
        StepOutcome {
            status: status.to_owned(),
            detail,
        }
    }

    /// The step of a recording handler: `recorded`, with the context.
    pub fn recorded(ctx: &GiveUpContext) -> Self {
        StepOutcome::new(RECORDED, serde_json::to_value(ctx).unwrap_or(Value::Null))
    }
}

/// The status of a step the run wrote instead of running its reaction.
pub const DEFERRED: &str = "deferred";

/// What a reaction did, and which later reactions of the run it took over.
#[derive(Debug, Clone, PartialEq)]
pub struct ReactionOutcome {
    /// The reaction's own step.
    pub step: StepOutcome,
    /// Later reactions of the run that must not run: each gets a
    /// `deferred` step carrying `defer_detail`.
    pub defer: Vec<GiveUpReaction>,
    /// The detail of the `deferred` steps.
    pub defer_detail: Value,
}

impl ReactionOutcome {
    /// An outcome that defers nothing.
    pub fn of(step: StepOutcome) -> Self {
        ReactionOutcome {
            step,
            defer: Vec::new(),
            defer_detail: Value::Null,
        }
    }

    /// Defers `reactions`, recording `detail` in their steps.
    pub fn deferring(mut self, reactions: &[GiveUpReaction], detail: Value) -> Self {
        self.defer = reactions.to_vec();
        self.defer_detail = detail;
        self
    }
}

impl From<StepOutcome> for ReactionOutcome {
    fn from(step: StepOutcome) -> Self {
        ReactionOutcome::of(step)
    }
}

/// Runs one give-up reaction for a message that gave up.
///
/// A failure fails the rule run's attempt; the run is retried with backoff
/// and the reactions that already have a step are not run again.
pub trait GiveUpHandler: Send + Sync + 'static {
    /// The reaction it runs (not `none`).
    fn reaction(&self) -> GiveUpReaction;

    /// Applies the reaction for `ctx`.
    fn run<'a>(
        &'a self,
        state: &'a AppState,
        ctx: &'a GiveUpContext,
    ) -> HandlerFuture<'a, StepOutcome>;

    /// Like [`GiveUpHandler::run`], and tells the run which later reactions
    /// to defer. The run calls this; by default it defers nothing.
    fn run_outcome<'a>(
        &'a self,
        state: &'a AppState,
        ctx: &'a GiveUpContext,
    ) -> HandlerFuture<'a, ReactionOutcome> {
        let step = self.run(state, ctx);
        Box::pin(async move { step.await.map(ReactionOutcome::of) })
    }

    /// Called by the run after the step of this reaction is stored: where
    /// a handler emits its events, so a retried run never emits for a step
    /// it has to run again. By default it does nothing.
    fn after_step<'a>(
        &'a self,
        state: &'a AppState,
        ctx: &'a GiveUpContext,
        step: &'a StepOutcome,
    ) -> HandlerFuture<'a, ()> {
        let _ = (state, ctx, step);
        Box::pin(async { Ok(()) })
    }
}

/// The built-in handler of `reaction`: records the step and does nothing
/// else, until the reaction's own issue replaces it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordingHandler(pub GiveUpReaction);

impl GiveUpHandler for RecordingHandler {
    fn reaction(&self) -> GiveUpReaction {
        self.0
    }

    fn run<'a>(
        &'a self,
        _state: &'a AppState,
        ctx: &'a GiveUpContext,
    ) -> HandlerFuture<'a, StepOutcome> {
        Box::pin(async move { Ok(StepOutcome::recorded(ctx)) })
    }
}

/// The module's give-up handlers, one per reaction, as the app holds them.
#[derive(Clone)]
pub struct Reactions {
    list: Vec<Arc<dyn GiveUpHandler>>,
}

impl Default for Reactions {
    /// The staff-task, alternative-recipient, mark-contact-invalid and
    /// statutory-notice handlers.
    fn default() -> Self {
        use GiveUpReaction::{
            AlternativeRecipient, MarkContactInvalid, StaffTask, StatutoryNotice,
        };
        Reactions {
            list: [
                StaffTask,
                AlternativeRecipient,
                MarkContactInvalid,
                StatutoryNotice,
            ]
            .into_iter()
            .map(|r| match r {
                StaffTask => {
                    Arc::new(crate::staff_task::StaffTaskHandler) as Arc<dyn GiveUpHandler>
                }
                AlternativeRecipient => Arc::new(crate::alternative::AlternativeRecipientHandler),
                StatutoryNotice => Arc::new(crate::statutory_notice::StatutoryNoticeHandler),
                MarkContactInvalid => Arc::new(crate::contact_invalid::MarkContactInvalidHandler),
                other => Arc::new(RecordingHandler(other)),
            })
            .collect(),
        }
    }
}

impl Reactions {
    /// Uses `handler` for its reaction, in place of the one there.
    pub fn replace(&mut self, handler: Arc<dyn GiveUpHandler>) {
        let reaction = handler.reaction();
        match self.list.iter_mut().find(|h| h.reaction() == reaction) {
            Some(slot) => *slot = handler,
            None => self.list.push(handler),
        }
    }

    /// The handler of `reaction`.
    pub fn for_reaction(&self, reaction: GiveUpReaction) -> Option<Arc<dyn GiveUpHandler>> {
        self.list.iter().find(|h| h.reaction() == reaction).cloned()
    }
}

/// The app's give-up handlers; the built-in ones when the module did not
/// provide any.
pub fn reactions(state: &AppState) -> Arc<Reactions> {
    state.provided::<Reactions>().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Custom;

    impl GiveUpHandler for Custom {
        fn reaction(&self) -> GiveUpReaction {
            GiveUpReaction::StaffTask
        }

        fn run<'a>(
            &'a self,
            _state: &'a AppState,
            _ctx: &'a GiveUpContext,
        ) -> HandlerFuture<'a, StepOutcome> {
            Box::pin(async { Ok(StepOutcome::new("task_created", Value::Null)) })
        }
    }

    #[test]
    fn every_reaction_but_none_has_a_built_in_handler() {
        let reactions = Reactions::default();
        for reaction in GiveUpReaction::ALL.iter() {
            let found = reactions.for_reaction(*reaction).map(|h| h.reaction());
            if *reaction == GiveUpReaction::None {
                assert_eq!(found, None);
            } else {
                assert_eq!(found, Some(*reaction));
            }
        }
    }

    #[test]
    fn a_handler_replaces_the_one_of_its_reaction() {
        let mut reactions = Reactions::default();
        reactions.replace(Arc::new(Custom));
        assert_eq!(reactions.list.len(), 4);
        let handler = reactions.for_reaction(GiveUpReaction::StaffTask).unwrap();
        assert_eq!(handler.reaction(), GiveUpReaction::StaffTask);
    }

    #[test]
    fn an_outcome_defers_the_reactions_it_names() {
        let outcome = ReactionOutcome::of(StepOutcome::new("queued", Value::Null))
            .deferring(&[GiveUpReaction::StaffTask], serde_json::json!({ "to": 4 }));
        assert_eq!(outcome.defer, [GiveUpReaction::StaffTask]);
        assert_eq!(outcome.defer_detail["to"], 4);
        let plain = ReactionOutcome::from(StepOutcome::new("recorded", Value::Null));
        assert!(plain.defer.is_empty());
    }

    #[test]
    fn a_recorded_step_keeps_the_context() {
        let ctx = GiveUpContext {
            run_id: 1,
            message: MessageId(5),
            type_key: "invoice.reminder".into(),
            subject: Subject::new("invoicing.invoice", 9),
            subject_label: None,
            gave_up_at: renox::db::now(),
            recipient: NotifyRecipient::User { id: 2 },
            contact_point: None,
            channels_tried: Vec::new(),
            statutory: false,
            staff_role: Some("collector".into()),
            urgency: Urgency::Normal,
            reactions: vec![GiveUpReaction::StaffTask],
        };
        let step = StepOutcome::recorded(&ctx);
        assert_eq!(step.status, RECORDED);
        assert_eq!(step.detail["type_key"], "invoice.reminder");
        assert_eq!(step.detail["staff_role"], "collector");
    }
}
