//! The `staff_task` give-up reaction (wl-notify §5.4; #56): when a message
//! could not be delivered, staff get a task through the kernel's
//! [`TaskCreator`](wl_kernel::tickets::TaskCreator).
//!
//! - **Holder**: the lowest-id holder of the type's staff role in the
//!   subject's scope ([`wl_kernel::scopes::scope_of`]); the company admin
//!   ([`COMPANY_ADMIN_ROLE`]) when nobody holds it or the type has no
//!   staff role; nobody (the task is opened unassigned) when neither
//!   exists.
//! - **Due**: one working day after `gave_up_at`, at the same Asia/Dubai
//!   clock time. The give-up time is the message's, never the time the job
//!   runs, so a retry gets the same due time.
//! - **Source key**: `notify.give_up:{type}:{id}:{recipient}`. While a task
//!   with that key is open, a later give-up becomes a comment on it.
//! - **Text**: the recipient, the notification type and one line per
//!   channel tried, with the masked address and the last error. Never the
//!   message's subject or body.
//!
//! The work sits in [`open_give_up_task`], so the other reactions' "then
//! `staff_task`" fallbacks can call it.

use renox::auth::permissions::users_with_role_in;
use renox::prelude::*;
use renox::serde_json::json;
use wl_kernel::notify::{ChannelTried, GiveUpReaction, NotifyRecipient};
use wl_kernel::tickets::{self, Assignee, OpenTicket, TASK_CATEGORY, TaskComment, TicketRef};
use wl_kernel::{COMPANY_ADMIN_ROLE, PartyId, Subject, calendar, parties, scopes};

use crate::alert::{self, GiveUpAlert};
use crate::mask;
use crate::message::Message;
use crate::reactions::{GiveUpContext, GiveUpHandler, HandlerFuture, StepOutcome};
use crate::recipients::default_language;
use crate::texts::{english, english_with};

/// The step status when a task was opened.
pub const TASK_CREATED: &str = "task_created";
/// The step status when the give-up became a comment on an open task.
pub const TASK_COMMENTED: &str = "task_commented";
/// The step status when ticketing is off, so no task could be opened.
pub const NO_TASK_CREATOR: &str = "no_task_creator";

/// What a task is for: who is told.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StaffTask {
    /// The staff role whose holders in the subject's scope get the task;
    /// `None` goes to the company admin.
    pub role: Option<String>,
    /// The task's title; `None` is "Could not reach {recipient} about
    /// {subject}".
    pub title: Option<String>,
    /// The task's description; `None` lists the channels tried.
    pub description: Option<String>,
    /// The key the task is found by while open; `None` is
    /// [`source_key`]. A statutory notice sets its own (#59 A7).
    pub source_key: Option<String>,
}

/// The built-in `staff_task` handler, registered by [`crate::WlNotify`].
#[derive(Debug, Clone, Copy, Default)]
pub struct StaffTaskHandler;

impl GiveUpHandler for StaffTaskHandler {
    fn reaction(&self) -> GiveUpReaction {
        GiveUpReaction::StaffTask
    }

    fn run<'a>(
        &'a self,
        state: &'a AppState,
        ctx: &'a GiveUpContext,
    ) -> HandlerFuture<'a, StepOutcome> {
        Box::pin(async move {
            let task = StaffTask {
                role: ctx.staff_role.clone(),
                ..Default::default()
            };
            open_give_up_task(state, ctx, task).await
        })
    }
}

/// Where the holder of a task came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HolderSource {
    /// A holder of the type's staff role.
    Role,
    /// The company admin, because nobody holds the role.
    CompanyAdmin,
    /// Nobody holds either role.
    Nobody,
}

impl HolderSource {
    fn as_str(self) -> &'static str {
        match self {
            HolderSource::Role => "role",
            HolderSource::CompanyAdmin => "company_admin",
            HolderSource::Nobody => "no_holder",
        }
    }
}

/// The user ids who hold the task: the role's holders in the subject's
/// scope, else the company admin's, ordered by id.
pub async fn holders(
    db: &Db,
    role: Option<&str>,
    subject: &Subject,
) -> Result<(Vec<i64>, HolderSource)> {
    let scope = scopes::scope_of(db, subject).await?;
    if let Some(role) = role {
        let ids = sorted_ids(users_with_role_in(db, role, &scope).await?);
        if !ids.is_empty() {
            return Ok((ids, HolderSource::Role));
        }
    }
    let ids = sorted_ids(users_with_role_in(db, COMPANY_ADMIN_ROLE, &scope).await?);
    let source = if ids.is_empty() {
        HolderSource::Nobody
    } else {
        HolderSource::CompanyAdmin
    };
    Ok((ids, source))
}

fn sorted_ids(users: Vec<User>) -> Vec<i64> {
    let mut ids: Vec<i64> = users.into_iter().map(|u| u.id).collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// The assignee among `holders` (sorted): the lowest user id.
pub fn pick_assignee(holders: &[i64]) -> Option<Assignee> {
    holders.iter().min().copied().map(Assignee::user)
}

/// The key a give-up's task is found by while it is open:
/// `notify.give_up:{subject_type}:{subject_id}:{recipient}`, the recipient
/// being `party:{id}`, `user:{id}` or `address:{channel}:{address}`.
pub fn source_key(subject: &Subject, recipient: &NotifyRecipient) -> String {
    let who = match recipient {
        NotifyRecipient::Party { id } => format!("party:{}", id.0),
        NotifyRecipient::User { id } => format!("user:{id}"),
        NotifyRecipient::Contact { of, purpose } => format!(
            "contact:{}:{}:{}",
            of.0,
            purpose.relation.as_str(),
            purpose.position.as_deref().unwrap_or("")
        ),
        NotifyRecipient::Address { channel, address } => {
            format!("address:{}:{}", channel.as_str(), address.trim())
        }
    };
    format!(
        "notify.give_up:{}:{}:{who}",
        subject.subject_type, subject.subject_id
    )
}

/// The subject as staff read it: the request's label, else the subject's
/// own text (`ticketing.ticket 42`).
pub fn subject_text(subject: &Subject, label: Option<&str>) -> String {
    match label.map(str::trim).filter(|l| !l.is_empty()) {
        Some(label) => label.to_owned(),
        None => format!("{} {}", subject.subject_type, subject.subject_id),
    }
}

/// One channel the message went through, as the task lists it: the channel,
/// the masked address and the last error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelLine {
    /// The channel's label, e.g. `Email`.
    pub channel: String,
    /// The masked address, or "no address".
    pub address: String,
    /// The error code and text, or "no error recorded".
    pub error: String,
    /// Whom it went to, when the task covers more than one recipient.
    pub who: Option<String>,
}

impl ChannelLine {
    /// The line for `tried` whose message went to `address`.
    pub fn of(tried: &ChannelTried, address: Option<&str>) -> Self {
        let address = match address.map(str::trim).filter(|a| !a.is_empty()) {
            Some(address) => mask::address(tried.channel, address),
            None => english("wl_notify.task.no_address"),
        };
        let error = [
            tried.last_error_code.as_deref(),
            tried.last_error.as_deref(),
        ]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
        ChannelLine {
            channel: english(&format!("wl_notify.channel.{}", tried.channel.as_str())),
            address,
            error: if error.is_empty() {
                english("wl_notify.task.no_error")
            } else {
                error
            },
            who: None,
        }
    }

    /// The line marked with the recipient it went to.
    #[must_use]
    pub fn for_who(mut self, who: &str) -> Self {
        self.who = Some(who.to_owned());
        self
    }

    fn text(&self) -> String {
        let params = [
            ("channel", self.channel.as_str()),
            ("address", self.address.as_str()),
            ("error", self.error.as_str()),
        ];
        match &self.who {
            Some(who) => {
                let mut all = vec![("who", who.as_str())];
                all.extend(params);
                english_with("wl_notify.task.channel_line_for", &all)
            }
            None => english_with("wl_notify.task.channel_line", &params),
        }
    }
}

/// The task title: "Could not reach {recipient} about {subject}".
pub fn title(recipient: &str, subject: &str) -> String {
    english_with(
        "wl_notify.task.title",
        &[("recipient", recipient), ("subject", subject)],
    )
}

/// The task description (or, with `gave_up_at`, the comment of a later
/// give-up): the recipient, the type and the channels. Never the message.
pub fn description(
    recipient: &str,
    type_key: &str,
    lines: &[ChannelLine],
    gave_up_at: Option<&str>,
) -> String {
    let mut out = vec![
        english_with("wl_notify.task.recipient", &[("recipient", recipient)]),
        english_with("wl_notify.task.type", &[("type", type_key)]),
        english("wl_notify.task.channels"),
    ];
    out.extend(lines.iter().map(ChannelLine::text));
    if let Some(time) = gave_up_at {
        out.push(english_with("wl_notify.task.gave_up_at", &[("time", time)]));
    }
    out.join("\n")
}

/// A party's name in `language`, else "Party {id}".
async fn party_name(state: &AppState, id: PartyId, language: &str) -> Result<String> {
    let name = parties::directory(state)
        .display_name(&state.db, id, language)
        .await?;
    Ok(name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| english_with("wl_notify.task.party", &[("id", &id.0.to_string())])))
}

/// Who the message was for, as staff read it: a party's name in the
/// message language, a user's name, else the masked address. For a
/// recipient rule it is the person the last message went to.
pub(crate) async fn recipient_name(
    state: &AppState,
    ctx: &GiveUpContext,
    last: Option<&Message>,
    language: &str,
) -> Result<String> {
    Ok(match &ctx.recipient {
        NotifyRecipient::Contact { of, .. } => {
            let person = last.and_then(|m| m.party_id).map(PartyId);
            party_name(state, person.unwrap_or(*of), language).await?
        }
        NotifyRecipient::Party { id } => party_name(state, *id, language).await?,
        NotifyRecipient::User { id } => match User::find(&state.db, *id).await? {
            Some(user) if !user.name.trim().is_empty() => user.name,
            _ => english_with("wl_notify.task.user", &[("id", &id.to_string())]),
        },
        NotifyRecipient::Address { channel, address } => mask::address(*channel, address),
    })
}

/// The channel lines of `ctx`, with each message's address masked. When
/// the last message is the alternative of an earlier chain, the lines of
/// that chain come first and every line names its recipient.
pub(crate) async fn channel_lines(
    state: &AppState,
    ctx: &GiveUpContext,
    last: Option<&Message>,
    language: &str,
) -> Result<Vec<ChannelLine>> {
    let mut earlier = Vec::new();
    if let Some(root) = last.and_then(|m| m.alternative_of) {
        earlier = Message::of_chain(&state.db, root).await?;
    }
    let named = !earlier.is_empty();
    let mut lines = Vec::with_capacity(earlier.len() + ctx.channels_tried.len());
    for message in &earlier {
        let tried = ChannelTried {
            message: message.message_id(),
            channel: message.channel,
            last_error_code: message.last_error_code.clone(),
            last_error: message.last_error.clone(),
        };
        let line = ChannelLine::of(&tried, message.address.as_deref());
        lines.push(line.for_who(&who(state, message, language).await?));
    }
    for tried in &ctx.channels_tried {
        let message = Message::find(&state.db, tried.message.0).await?;
        let address = message.as_ref().and_then(|m| m.address.as_deref());
        let line = ChannelLine::of(tried, address);
        lines.push(match &message {
            Some(m) if named => line.for_who(&who(state, m, language).await?),
            _ => line,
        });
    }
    Ok(lines)
}

/// The person `message` went to, as a line names them.
async fn who(state: &AppState, message: &Message, language: &str) -> Result<String> {
    match message.party_id {
        Some(id) => party_name(state, PartyId(id), language).await,
        None => Ok(english("wl_notify.task.no_address")),
    }
}

/// Opens the staff task for the give-up `ctx`, or comments on the one that
/// is still open for the same subject and recipient. Returns the step to
/// store: `task_created`, `task_commented`, or `no_task_creator` when
/// ticketing is off.
pub async fn open_give_up_task(
    state: &AppState,
    ctx: &GiveUpContext,
    task: StaffTask,
) -> Result<StepOutcome> {
    let creator = tickets::task_creator(state);
    let key = task
        .source_key
        .clone()
        .unwrap_or_else(|| source_key(&ctx.subject, &ctx.recipient));
    let language = default_language(state);
    let last = Message::find(&state.db, ctx.message.0).await?;
    let lines = channel_lines(state, ctx, last.as_ref(), &language).await?;
    let recipient = recipient_name(state, ctx, last.as_ref(), &language).await?;
    let subject = subject_text(&ctx.subject, ctx.subject_label.as_deref());
    let calendar = calendar::calendar(state);

    let (holders, source) = holders(&state.db, task.role.as_deref(), &ctx.subject).await?;
    let alert_for = |ticket: &TicketRef| GiveUpAlert {
        recipient: recipient.clone(),
        subject: subject.clone(),
        type_key: ctx.type_key.clone(),
        ticket: ticket.clone(),
    };

    if let Some(open) = creator.find_open(state, &key).await? {
        let time = calendar
            .local(ctx.gave_up_at)
            .format("%Y-%m-%d %H:%M")
            .to_string();
        let text = description(&recipient, &ctx.type_key, &lines, Some(&time));
        creator
            .comment(state, open.id, TaskComment::internal(&text))
            .await?;
        // Every urgent give-up alerts, also when it only became a comment.
        let alerted = alert_if_urgent(state, ctx, &holders, alert_for(&open)).await?;
        return Ok(StepOutcome::new(
            TASK_COMMENTED,
            step_detail(&open, "commented", None, None, alerted),
        ));
    }

    let assignee = pick_assignee(&holders);
    let task_title = task
        .title
        .clone()
        .unwrap_or_else(|| title(&recipient, &subject));
    let task_text = task
        .description
        .clone()
        .unwrap_or_else(|| description(&recipient, &ctx.type_key, &lines, None));
    let mut ticket = OpenTicket::new(ctx.subject.clone(), TASK_CATEGORY, &task_title)
        .description(&task_text)
        .due_at(calendar.add_working_days(ctx.gave_up_at, 1))
        .source_key(&key);
    if let Some(assignee) = assignee {
        ticket = ticket.assign_to(assignee);
    }
    match creator.open(state, ticket).await? {
        Some(opened) => {
            let alerted = alert_if_urgent(state, ctx, &holders, alert_for(&opened)).await?;
            Ok(StepOutcome::new(
                TASK_CREATED,
                step_detail(&opened, "opened", assignee, Some(source), alerted),
            ))
        }
        None => Ok(StepOutcome::new(
            NO_TASK_CREATOR,
            json!({ "source_key": key }),
        )),
    }
}

/// Sends the in-app alert to `holders` when the type is urgent; returns
/// whether anyone got it.
async fn alert_if_urgent(
    state: &AppState,
    ctx: &GiveUpContext,
    holders: &[i64],
    alert: GiveUpAlert,
) -> Result<bool> {
    if !alert::is_urgent(ctx.urgency) {
        return Ok(false);
    }
    Ok(alert::send(state, holders, &alert).await? > 0)
}

fn step_detail(
    ticket: &TicketRef,
    action: &str,
    assignee: Option<Assignee>,
    source: Option<HolderSource>,
    alert_sent: bool,
) -> renox::serde_json::Value {
    json!({
        "ticket_id": ticket.id.0,
        "ticket_number": ticket.number,
        "action": action,
        "assignee": assignee.map(|a| match a {
            Assignee::User { id } => json!({ "kind": "user", "id": id }),
            Assignee::Party { id } => json!({ "kind": "party", "id": id.0 }),
        }),
        "holder": source.map(HolderSource::as_str),
        "no_holder": source == Some(HolderSource::Nobody),
        "alert_sent": alert_sent,
    })
}

#[cfg(test)]
mod tests {
    use wl_kernel::PartyId;
    use wl_kernel::notify::{MessageId, NotifyChannel};

    use super::*;

    fn tried(channel: NotifyChannel, code: Option<&str>, text: Option<&str>) -> ChannelTried {
        ChannelTried {
            message: MessageId(1),
            channel,
            last_error_code: code.map(str::to_owned),
            last_error: text.map(str::to_owned),
        }
    }

    #[test]
    fn the_lowest_user_id_is_the_assignee() {
        assert_eq!(pick_assignee(&[9, 4, 7]), Some(Assignee::user(4)));
        assert_eq!(pick_assignee(&[]), None);
    }

    #[test]
    fn the_source_key_names_subject_and_recipient() {
        let subject = Subject::new("ticketing.ticket", 42);
        assert_eq!(
            source_key(&subject, &NotifyRecipient::Party { id: PartyId(7) }),
            "notify.give_up:ticketing.ticket:42:party:7"
        );
        assert_eq!(
            source_key(&subject, &NotifyRecipient::User { id: 3 }),
            "notify.give_up:ticketing.ticket:42:user:3"
        );
        assert_eq!(
            source_key(
                &subject,
                &NotifyRecipient::Address {
                    channel: NotifyChannel::Sms,
                    address: " +971501234567 ".into()
                }
            ),
            "notify.give_up:ticketing.ticket:42:address:sms:+971501234567"
        );
    }

    #[test]
    fn another_recipient_or_subject_has_another_key() {
        let a = Subject::new("ticketing.ticket", 42);
        let b = Subject::new("ticketing.ticket", 43);
        let party = |id| NotifyRecipient::Party { id: PartyId(id) };
        assert_ne!(source_key(&a, &party(1)), source_key(&a, &party(2)));
        assert_ne!(source_key(&a, &party(1)), source_key(&b, &party(1)));
    }

    #[test]
    fn the_subject_text_prefers_the_label() {
        let subject = Subject::new("ticketing.ticket", 42);
        assert_eq!(
            subject_text(&subject, Some("TKT-2026-0042")),
            "TKT-2026-0042"
        );
        assert_eq!(subject_text(&subject, Some("  ")), "ticketing.ticket 42");
        assert_eq!(subject_text(&subject, None), "ticketing.ticket 42");
    }

    #[test]
    fn a_channel_line_masks_the_address_and_shows_the_error() {
        let email = ChannelLine::of(
            &tried(NotifyChannel::Email, Some("550"), Some("mailbox unknown")),
            Some("sara@example.com"),
        );
        assert_eq!(
            email.text(),
            "- Email s***@example.com: 550 mailbox unknown"
        );
        let sms = ChannelLine::of(
            &tried(NotifyChannel::Sms, None, Some("timeout")),
            Some("+971501234567"),
        );
        assert_eq!(sms.text(), "- SMS ****4567: timeout");
    }

    #[test]
    fn a_channel_without_address_or_error_says_so() {
        let line = ChannelLine::of(&tried(NotifyChannel::Whatsapp, None, None), None);
        assert_eq!(line.text(), "- WhatsApp no address: no error recorded");
    }

    #[test]
    fn a_line_of_a_longer_chain_names_its_recipient() {
        let line = ChannelLine::of(
            &tried(NotifyChannel::Sms, None, Some("timeout")),
            Some("+971501234567"),
        )
        .for_who("Mona");
        assert_eq!(line.text(), "- Mona, SMS ****4567: timeout");
    }

    #[test]
    fn the_title_names_recipient_and_subject() {
        assert_eq!(
            title("Sara M.", "TKT-2026-0042"),
            "Could not reach Sara M. about TKT-2026-0042"
        );
    }

    #[test]
    fn the_description_lists_the_channels_and_nothing_of_the_message() {
        let lines = [
            ChannelLine::of(
                &tried(NotifyChannel::Email, Some("550"), Some("mailbox unknown")),
                Some("sara@example.com"),
            ),
            ChannelLine::of(
                &tried(NotifyChannel::Sms, None, Some("timeout")),
                Some("+971501234567"),
            ),
        ];
        let text = description("Sara M.", "ticket.update", &lines, None);
        assert_eq!(
            text,
            "Recipient: Sara M.\nNotification type: ticket.update\nChannels tried:\n\
             - Email s***@example.com: 550 mailbox unknown\n- SMS ****4567: timeout"
        );
        assert!(!text.contains("sara@example.com"));
        let later = description("Sara M.", "ticket.update", &lines, Some("2026-11-05 10:00"));
        assert!(later.ends_with("Gave up at 2026-11-05 10:00 (Asia/Dubai)"));
    }

    #[test]
    fn a_step_detail_records_ticket_assignee_action_and_alert() {
        let ticket = TicketRef {
            id: 5.into(),
            number: "TKT-2026-0005".into(),
        };
        let detail = step_detail(
            &ticket,
            "opened",
            Some(Assignee::user(4)),
            Some(HolderSource::CompanyAdmin),
            false,
        );
        assert_eq!(detail["ticket_number"], "TKT-2026-0005");
        assert_eq!(detail["assignee"]["id"], 4);
        assert_eq!(detail["holder"], "company_admin");
        assert_eq!(detail["no_holder"], false);
        assert_eq!(detail["alert_sent"], false);
        let nobody = step_detail(&ticket, "opened", None, Some(HolderSource::Nobody), false);
        assert_eq!(nobody["no_holder"], true);
        assert!(nobody["assignee"].is_null());
    }
}
