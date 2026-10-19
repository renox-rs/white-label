//! The `statutory_notice` give-up reaction (wl-notify §5.4; #59 A7, A8).
//!
//! A statutory notice that gave up has legal effect only once it is
//! delivered, so staff must deliver it another way and record the proof.
//! For a statutory type this handler:
//!
//! 1. keeps one [`StatutoryNotice`] record per chain that gave up (a
//!    retried run finds the record, it does not add a second);
//! 2. opens one task for the type's staff role (else the company admin),
//!    titled "Deliver {subject} notice to {recipient} by another means",
//!    due one working day after the give-up, with the urgent alert of
//!    `staff_task` when the type is urgent. The task lists the channels
//!    tried (masked, never the message), the proof methods offered and the
//!    link to the proof form. Its source key is `notify.statutory:{id}`, so
//!    a retried run finds the open task instead of opening another;
//! 3. takes over a later `staff_task` of the run: the staff get one task.
//!
//! The step ends with one status: the task's (`task_created`,
//! `task_open` when a retried run found it, `no_task_creator` when
//! ticketing is off, the record is still created) or `not_statutory` when
//! the type is not statutory (no record, no task). The detail names the
//! `notice_id`.

use renox::prelude::*;
use renox::serde_json::{Value, json};
use wl_kernel::notify::{GiveUpReaction, ProofMethod};
use wl_kernel::tickets;

use crate::message::Message;
use crate::notices::StatutoryNotice;
use crate::notifier::type_def;
use crate::reactions::{GiveUpContext, GiveUpHandler, HandlerFuture, ReactionOutcome, StepOutcome};
use crate::recipients::default_language;
use crate::staff_task::{
    ChannelLine, StaffTask, channel_lines, description, open_give_up_task, recipient_name,
    subject_text,
};
use crate::texts::{english, english_with};

/// The step status when the type is not statutory.
pub const NOT_STATUTORY: &str = "not_statutory";
/// The step status when a retried run found the notice's task still open.
pub const TASK_OPEN: &str = "task_open";

/// The built-in `statutory_notice` handler, registered by
/// [`crate::WlNotify`].
#[derive(Debug, Clone, Copy, Default)]
pub struct StatutoryNoticeHandler;

impl GiveUpHandler for StatutoryNoticeHandler {
    fn reaction(&self) -> GiveUpReaction {
        GiveUpReaction::StatutoryNotice
    }

    fn run<'a>(
        &'a self,
        state: &'a AppState,
        ctx: &'a GiveUpContext,
    ) -> HandlerFuture<'a, StepOutcome> {
        Box::pin(async move { notice(state, ctx).await.map(|o| o.step) })
    }

    fn run_outcome<'a>(
        &'a self,
        state: &'a AppState,
        ctx: &'a GiveUpContext,
    ) -> HandlerFuture<'a, ReactionOutcome> {
        Box::pin(notice(state, ctx))
    }
}

/// The key the notice's task is found by while it is open.
pub fn source_key(notice: i64) -> String {
    format!("notify.statutory:{notice}")
}

/// The task title: "Deliver {subject} notice to {recipient} by another
/// means".
pub fn title(subject: &str, recipient: &str) -> String {
    english_with(
        "wl_notify.statutory.title",
        &[("subject", subject), ("recipient", recipient)],
    )
}

/// The label of a proof method, as the task lists it.
pub fn method_label(method: ProofMethod) -> String {
    english(&format!("wl_notify.proof_method.{}", method.as_str()))
}

/// The task description: what `staff_task` lists, then the proof methods
/// offered and the link to the proof form.
pub fn task_description(
    recipient: &str,
    type_key: &str,
    lines: &[ChannelLine],
    methods: &[ProofMethod],
    proof_path: &str,
) -> String {
    let labels = methods
        .iter()
        .map(|m| method_label(*m))
        .collect::<Vec<_>>()
        .join(", ");
    [
        description(recipient, type_key, lines, None),
        english_with("wl_notify.statutory.methods", &[("methods", &labels)]),
        english_with("wl_notify.statutory.proof_link", &[("path", proof_path)]),
    ]
    .join("\n")
}

async fn notice(state: &AppState, ctx: &GiveUpContext) -> Result<ReactionOutcome> {
    if !ctx.statutory {
        return Ok(StepOutcome::new(NOT_STATUTORY, json!({})).into());
    }
    let message = Message::find(&state.db, ctx.message.0)
        .await?
        .ok_or_else(|| {
            renox::Error::permanent_message(format!("message {} does not exist", ctx.message.0))
        })?;
    let mut record = match StatutoryNotice::by_chain(&state.db, message.chain()).await? {
        Some(record) => record,
        None => {
            let mut record = StatutoryNotice::for_message(&message, ctx.gave_up_at);
            record.insert(&state.db).await?;
            record
        }
    };
    let key = source_key(record.id);
    let creator = tickets::task_creator(state);
    let step = if let Some(open) = creator.find_open(state, &key).await? {
        StepOutcome::new(
            TASK_OPEN,
            json!({
                "ticket_id": open.id.0,
                "ticket_number": open.number,
                "action": "found",
            }),
        )
    } else {
        let language = default_language(state);
        let lines = channel_lines(state, ctx, Some(&message), &language).await?;
        let recipient = recipient_name(state, ctx, Some(&message), &language).await?;
        let subject = subject_text(&ctx.subject, ctx.subject_label.as_deref());
        let methods = type_def(&state.db, &ctx.type_key).await?.proof_methods;
        let task = StaffTask {
            role: ctx.staff_role.clone(),
            title: Some(title(&subject, &recipient)),
            description: Some(task_description(
                &recipient,
                &ctx.type_key,
                &lines,
                &methods,
                &record.proof_path(),
            )),
            source_key: Some(key.clone()),
        };
        open_give_up_task(state, ctx, task).await?
    };
    if let Some(ticket) = step.detail["ticket_id"].as_i64()
        && record.ticket_id != Some(ticket)
    {
        record.ticket_id = Some(ticket);
        record.save(&state.db).await?;
    }
    let mut detail = step.detail;
    if !detail.is_object() {
        detail = json!({});
    }
    detail["notice_id"] = json!(record.id);
    detail["source_key"] = Value::String(key);
    let defer = json!({ "to": GiveUpReaction::StatutoryNotice.as_str(), "notice_id": record.id });
    Ok(ReactionOutcome::of(StepOutcome::new(&step.status, detail))
        .deferring(&[GiveUpReaction::StaffTask], defer))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_source_key_names_the_notice() {
        assert_eq!(source_key(12), "notify.statutory:12");
    }

    #[test]
    fn the_title_names_subject_and_recipient() {
        assert_eq!(
            title("Lease 7", "Sara M."),
            "Deliver Lease 7 notice to Sara M. by another means"
        );
    }

    #[test]
    fn the_description_adds_methods_and_link_and_nothing_of_the_message() {
        let text = task_description(
            "Sara M.",
            "lease.notice",
            &[],
            &[ProofMethod::Courier, ProofMethod::Call],
            "/statutory-notices/3/proof",
        );
        assert_eq!(
            text,
            "Recipient: Sara M.\nNotification type: lease.notice\nChannels tried:\n\
             Accepted proof of delivery: courier, call\n\
             Record the proof: /statutory-notices/3/proof"
        );
    }

    #[test]
    fn every_method_has_a_label() {
        for method in ProofMethod::ALL {
            assert!(!method_label(*method).starts_with("wl_notify."));
        }
    }
}
