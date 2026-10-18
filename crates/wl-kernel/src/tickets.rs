//! The ticket contract (wl-kernel §8, wl-ticketing §6): how every module
//! opens a ticket (a task) on any record and follows it, without depending
//! on the `wl-ticketing` crate.
//!
//! `wl-ticketing` provides a [`TaskCreator`] with [`provide_task_creator`]
//! and emits [`TicketAssigned`] and [`TicketClosed`] after the commit. Other
//! modules open tickets through [`task_creator`], which falls back to
//! [`NoTickets`] when the `ticketing` module is off:
//!
//! ```ignore
//! use wl_kernel::tickets::{self, Assignee, OpenTicket};
//!
//! let ticket = OpenTicket::new(unit.subject(), "general", "Leaking tap")
//!     .description("The kitchen tap drips.")
//!     .requester_party(tenant)
//!     .assign_to(Assignee::user(coordinator));
//! match tickets::task_creator(&state).open(&state, ticket).await? {
//!     Some(ticket) => { /* link ticket.id, show ticket.number */ }
//!     None => { /* ticketing is off: no task */ }
//! }
//! ```
//!
//! [`OpenTicket`] is built with its builder methods only, so the contract can
//! gain optional fields without breaking callers; the same holds for the [`TaskCreator`] methods, which new ones
//! join with default bodies.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::notify::NotifyRecipient;
use crate::numbering::{Reset, Series};
use crate::parties::PartyId;
use crate::subject::Subject;

/// The subject type of a ticket, for [`Subject`] references.
pub const TICKET_SUBJECT_TYPE: &str = "ticketing.ticket";

/// The category key of internal staff tasks other modules open.
pub const TASK_CATEGORY: &str = "task";

/// The page of ticket `id`: `/tickets/{id}`.
pub fn ticket_path(id: TicketId) -> String {
    format!("/tickets/{}", id.0)
}

/// Ticket numbers: `TKT-2026-0001`, across the deployment, reset yearly;
/// gaps allowed (take them with [`Series::next_allowing_gaps`] on the
/// Asia/Dubai date).
pub const TICKET_SERIES: Series = Series {
    key: "ticket",
    pattern: "{prefix}-{yyyy}-{0000}",
    prefix: "TKT",
    reset: Reset::Yearly,
    gapless: false,
    per_entity: false,
};

/// The id of a ticket.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct TicketId(pub i64);

impl TicketId {
    /// The ticket as a [`Subject`]: `ticketing.ticket:42`.
    pub fn subject(self) -> Subject {
        Subject::new(TICKET_SUBJECT_TYPE, self.0)
    }
}

impl fmt::Display for TicketId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<i64> for TicketId {
    fn from(id: i64) -> Self {
        TicketId(id)
    }
}

/// A ticket another module opened: its id and number.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TicketRef {
    /// The id.
    pub id: TicketId,
    /// The number, e.g. `TKT-2026-0001`.
    pub number: String,
}

/// Who works on a ticket: exactly one staff user or one vendor party.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Assignee {
    /// A staff user (a technician, a coordinator), by user id.
    User {
        /// The user's id.
        id: i64,
    },
    /// A vendor party, reached through its contact points.
    Party {
        /// The party.
        id: PartyId,
    },
}

impl Assignee {
    /// The staff user `id`.
    pub fn user(id: i64) -> Self {
        Assignee::User { id }
    }

    /// The vendor `party`.
    pub fn party(party: PartyId) -> Self {
        Assignee::Party { id: party }
    }

    /// Whom to send the assignee's notices to.
    pub fn recipient(self) -> NotifyRecipient {
        match self {
            Assignee::User { id } => NotifyRecipient::User { id },
            Assignee::Party { id } => NotifyRecipient::Party { id },
        }
    }
}

/// Who asked for a ticket: a user (staff or portal) or a party.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Requester {
    /// A user, by user id.
    User {
        /// The user's id.
        id: i64,
    },
    /// A party (a tenant, a customer).
    Party {
        /// The party.
        id: PartyId,
    },
}

impl Requester {
    /// Whom to send the requester's notices to.
    pub fn recipient(self) -> NotifyRecipient {
        match self {
            Requester::User { id } => NotifyRecipient::User { id },
            Requester::Party { id } => NotifyRecipient::Party { id },
        }
    }
}

/// How a ticket came in.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TicketChannel {
    /// The customer portal.
    Portal,
    /// Staff, in the back office.
    Staff,
    /// An e-mail.
    Email,
    /// A phone call.
    Phone,
    /// Another module or a rule.
    #[default]
    System,
}

/// A ticket another module asks for: [`TaskCreator::open`]. Built with
/// [`OpenTicket::new`] and the builder methods.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct OpenTicket {
    /// The record it is about, e.g. `property.unit:12`.
    pub subject: Subject,
    /// The category key, e.g. `general`; an unknown key is refused.
    pub category_key: String,
    /// The title.
    pub title: String,
    /// The description; empty when none.
    pub description: String,
    /// Who asked for it, if anyone (an internal task has none).
    pub requester: Option<Requester>,
    /// How it came in; [`TicketChannel::System`] unless set.
    pub channel: TicketChannel,
    /// The record that opened it (a dispute, a document, a message…), when
    /// not the subject.
    pub source: Option<Subject>,
    /// Who works on it: `wl-ticketing` runs the first `assign` as the system
    /// when opening, so the SLA starts as for any first assignment. `None`
    /// leaves it `open`.
    pub assignee: Option<Assignee>,
    /// When it is due, if it has a due time of its own.
    pub due_at: Option<DateTime>,
    /// A key the opener can find the ticket by while it is open
    /// ([`TaskCreator::find_open`]), e.g. `notify.give_up:…`.
    pub source_key: Option<String>,
}

impl OpenTicket {
    /// A ticket of `category_key` titled `title` about `subject`, from the
    /// system, with no requester, source or assignee.
    pub fn new(subject: Subject, category_key: &str, title: &str) -> Self {
        OpenTicket {
            subject,
            category_key: category_key.to_owned(),
            title: title.to_owned(),
            description: String::new(),
            requester: None,
            channel: TicketChannel::System,
            source: None,
            assignee: None,
            due_at: None,
            source_key: None,
        }
    }

    /// With this description.
    pub fn description(mut self, description: &str) -> Self {
        self.description = description.to_owned();
        self
    }

    /// Asked for by `requester`.
    pub fn requester(mut self, requester: Requester) -> Self {
        self.requester = Some(requester);
        self
    }

    /// Asked for by the user `id`.
    pub fn requester_user(self, id: i64) -> Self {
        self.requester(Requester::User { id })
    }

    /// Asked for by `party`.
    pub fn requester_party(self, party: PartyId) -> Self {
        self.requester(Requester::Party { id: party })
    }

    /// Come in through `channel`.
    pub fn channel(mut self, channel: TicketChannel) -> Self {
        self.channel = channel;
        self
    }

    /// Opened by `source`.
    pub fn source(mut self, source: Subject) -> Self {
        self.source = Some(source);
        self
    }

    /// Assigned to `assignee` from the start.
    pub fn assign_to(mut self, assignee: Assignee) -> Self {
        self.assignee = Some(assignee);
        self
    }

    /// Due at `due_at`.
    pub fn due_at(mut self, due_at: DateTime) -> Self {
        self.due_at = Some(due_at);
        self
    }

    /// Findable by `key` while open.
    pub fn source_key(mut self, key: &str) -> Self {
        self.source_key = Some(key.to_owned());
        self
    }
}

/// A comment on a ticket: [`TaskCreator::comment`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskComment {
    /// The text.
    pub text: String,
    /// Whether only staff see it (not the requester).
    pub internal: bool,
}

impl TaskComment {
    /// An internal comment with `text`.
    pub fn internal(text: &str) -> Self {
        TaskComment {
            text: text.to_owned(),
            internal: true,
        }
    }

    /// A comment with `text` the requester sees too.
    pub fn visible(text: &str) -> Self {
        TaskComment {
            text: text.to_owned(),
            internal: false,
        }
    }
}

/// The future a [`TaskCreator`] method returns.
pub type TicketFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Opens tickets for other modules. `wl-ticketing` provides it; without that
/// module, [`task_creator`] gives [`NoTickets`].
pub trait TaskCreator: Send + Sync + 'static {
    /// Opens a ticket from `ticket` (status `open`, or `assigned` with an
    /// assignee) and returns its id and number; `None` when ticketing is
    /// off. Fails for an unknown category key.
    fn open<'a>(
        &'a self,
        state: &'a AppState,
        ticket: OpenTicket,
    ) -> TicketFuture<'a, Option<TicketRef>>;

    /// The open ticket opened with `source_key`, if any. A ticket is open in
    /// any state except `resolved`, `closed` and `cancelled`. The default
    /// finds nothing.
    fn find_open<'a>(
        &'a self,
        _state: &'a AppState,
        _source_key: &'a str,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        Box::pin(async { Ok(None) })
    }

    /// Adds `comment` to `ticket`. The default does nothing.
    fn comment<'a>(
        &'a self,
        _state: &'a AppState,
        _ticket: TicketId,
        _comment: TaskComment,
    ) -> TicketFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

/// The task creator when the `ticketing` module is off: no ticket is ever
/// opened, so callers carry on without one.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoTickets;

impl TaskCreator for NoTickets {
    fn open<'a>(
        &'a self,
        _state: &'a AppState,
        _ticket: OpenTicket,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        Box::pin(async { Ok(None) })
    }
}

/// Makes `creator` the app's [`TaskCreator`]; `wl-ticketing` calls it in
/// `register`.
pub fn provide_task_creator(app: &mut Registry, creator: impl TaskCreator) {
    let creator: Arc<dyn TaskCreator> = Arc::new(creator);
    app.provide(creator);
}

/// The app's [`TaskCreator`], or [`NoTickets`] when none is provided.
pub fn task_creator(state: &AppState) -> Arc<dyn TaskCreator> {
    match state.provided::<Arc<dyn TaskCreator>>() {
        Some(creator) => Arc::clone(&creator),
        None => Arc::new(NoTickets),
    }
}

/// A ticket was assigned (`assign`): a first assignment, or a reassignment
/// to someone else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketAssigned {
    /// The ticket.
    pub ticket: TicketId,
    /// Who works on it now.
    pub assignee: Assignee,
    /// Who worked on it before; `None` for the first assignment.
    pub previous_assignee: Option<Assignee>,
    /// The reassignment reason code, e.g. `vendor_no_show`; `None` for the
    /// first assignment.
    pub reassignment_reason: Option<String>,
    /// The priority key: `urgent`, `high`, `normal` or `low`.
    pub priority: String,
    /// When the first response is due, if the ticket has an SLA. A
    /// reassignment never moves it.
    pub response_due: Option<DateTime>,
    /// When the resolution is due, if the ticket has an SLA.
    pub resolution_due: Option<DateTime>,
}

impl Event for TicketAssigned {}

/// A ticket was closed (`confirm` or `close`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketClosed {
    /// The ticket.
    pub ticket: TicketId,
    /// The working minutes from the SLA start (the first assignment) to its
    /// resolution, paused time left out; `None` when the SLA never started.
    pub resolution_minutes: Option<i64>,
    /// Whether it was resolved by its resolution due time; `true` when it
    /// had no due time.
    pub within_sla: bool,
}

impl Event for TicketClosed {}

#[cfg(test)]
mod tests {
    use renox::serde_json::json;

    use super::*;

    #[test]
    fn a_ticket_id_is_a_subject() {
        let ticket = TicketId(42);
        assert_eq!(ticket.subject().to_string(), "ticketing.ticket:42");
        assert_eq!(ticket.to_string(), "42");
        assert_eq!(TicketId::from(7), TicketId(7));
    }

    #[test]
    fn the_ticket_series_is_tkt_year_and_four_digits() {
        assert_eq!(TICKET_SERIES.key, "ticket");
        assert_eq!(TICKET_SERIES.pattern, "{prefix}-{yyyy}-{0000}");
        assert_eq!(TICKET_SERIES.prefix, "TKT");
        assert_eq!(TICKET_SERIES.reset, Reset::Yearly);
        assert_eq!(
            (TICKET_SERIES.gapless, TICKET_SERIES.per_entity),
            (false, false)
        );
    }

    #[test]
    fn a_new_ticket_comes_from_the_system_with_nobody_set() {
        let unit = Subject::new("property.unit", 12);
        let ticket = OpenTicket::new(unit.clone(), "general", "Leaking tap");
        assert_eq!(ticket.subject, unit);
        assert_eq!(ticket.category_key, "general");
        assert_eq!(ticket.title, "Leaking tap");
        assert_eq!(ticket.description, "");
        assert_eq!(ticket.channel, TicketChannel::System);
        assert_eq!(ticket.requester, None);
        assert_eq!(ticket.source, None);
        assert_eq!(ticket.assignee, None);
        assert_eq!(ticket.due_at, None);
        assert_eq!(ticket.source_key, None);
    }

    #[test]
    fn the_builder_sets_the_due_time_and_the_source_key() {
        let due = "2026-11-06T06:00:00Z".parse::<DateTime>().unwrap();
        let ticket = OpenTicket::new(Subject::new("property.unit", 1), TASK_CATEGORY, "x")
            .due_at(due)
            .source_key("notify.give_up:a:1:user:2");
        assert_eq!(ticket.category_key, "task");
        assert_eq!(ticket.due_at, Some(due));
        assert_eq!(
            ticket.source_key.as_deref(),
            Some("notify.give_up:a:1:user:2")
        );
    }

    #[test]
    fn a_ticket_has_a_page() {
        assert_eq!(ticket_path(TicketId(42)), "/tickets/42");
    }

    #[test]
    fn comments_are_internal_or_visible() {
        assert!(TaskComment::internal("a").internal);
        assert!(!TaskComment::visible("a").internal);
        assert_eq!(TaskComment::visible("a").text, "a");
    }

    #[test]
    fn the_builder_sets_every_optional_field() {
        let dispute = Subject::new("invoicing.dispute", 3);
        let ticket = OpenTicket::new(Subject::new("invoicing.invoice", 9), "general", "Dispute")
            .description("Line 2 is wrong.")
            .requester_party(PartyId(5))
            .channel(TicketChannel::Portal)
            .source(dispute.clone())
            .assign_to(Assignee::user(11));
        assert_eq!(ticket.description, "Line 2 is wrong.");
        assert_eq!(ticket.requester, Some(Requester::Party { id: PartyId(5) }));
        assert_eq!(ticket.channel, TicketChannel::Portal);
        assert_eq!(ticket.source, Some(dispute));
        assert_eq!(ticket.assignee, Some(Assignee::User { id: 11 }));

        let by_user = OpenTicket::new(Subject::new("property.unit", 1), "general", "x")
            .requester_user(4)
            .assign_to(Assignee::party(PartyId(8)));
        assert_eq!(by_user.requester, Some(Requester::User { id: 4 }));
        assert_eq!(by_user.assignee, Some(Assignee::Party { id: PartyId(8) }));
    }

    #[test]
    fn channels_are_stored_in_snake_case() {
        let names: Vec<_> = TicketChannel::ALL.iter().map(|c| c.as_str()).collect();
        assert_eq!(names, ["portal", "staff", "email", "phone", "system"]);
        assert_eq!(
            "phone".parse::<TicketChannel>().unwrap(),
            TicketChannel::Phone
        );
        assert_eq!(TicketChannel::default(), TicketChannel::System);
    }

    #[test]
    fn assignees_and_requesters_are_notice_recipients() {
        assert_eq!(
            Assignee::user(3).recipient(),
            NotifyRecipient::User { id: 3 }
        );
        assert_eq!(
            Assignee::party(PartyId(4)).recipient(),
            NotifyRecipient::Party { id: PartyId(4) }
        );
        assert_eq!(
            Requester::Party { id: PartyId(6) }.recipient(),
            NotifyRecipient::Party { id: PartyId(6) }
        );
        assert_eq!(
            renox::serde_json::to_value(Assignee::party(PartyId(4))).unwrap(),
            json!({ "kind": "party", "id": 4 })
        );
    }

    #[test]
    fn a_reassignment_event_carries_the_previous_assignee_and_the_reason() {
        let due = "2026-11-03T13:00:00Z".parse::<DateTime>().unwrap();
        let event = TicketAssigned {
            ticket: TicketId(42),
            assignee: Assignee::user(7),
            previous_assignee: Some(Assignee::party(PartyId(2))),
            reassignment_reason: Some("vendor_no_show".into()),
            priority: "high".into(),
            response_due: Some(due),
            resolution_due: None,
        };
        let json = renox::serde_json::to_value(&event).unwrap();
        assert_eq!(json["ticket"], 42);
        assert_eq!(json["assignee"], json!({ "kind": "user", "id": 7 }));
        assert_eq!(
            json["previous_assignee"],
            json!({ "kind": "party", "id": 2 })
        );
        assert_eq!(json["reassignment_reason"], "vendor_no_show");
        let back: TicketAssigned = renox::serde_json::from_value(json).unwrap();
        assert_eq!(back, event);
    }

    #[test]
    fn a_closed_event_says_whether_it_was_within_the_sla() {
        let event = TicketClosed {
            ticket: TicketId(1),
            resolution_minutes: Some(540),
            within_sla: true,
        };
        let json = renox::serde_json::to_string(&event).unwrap();
        assert_eq!(
            json,
            r#"{"ticket":1,"resolution_minutes":540,"within_sla":true}"#
        );
    }
}
