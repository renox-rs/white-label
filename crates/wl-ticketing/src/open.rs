//! Opening a ticket (wl-ticketing §6, kernel §8): [`Tickets::open`].
//!
//! A new ticket is `open`, numbered `TKT-{YYYY}-{0000}` from the kernel's
//! `ticket` series ([`TICKET_SERIES`](wl_kernel::tickets::TICKET_SERIES);
//! gaps allowed, taken on the Asia/Dubai date of the opening), with its
//! priority, SLA policy, `confidential` and `chargeable` copied from its
//! category. With an assignee, the first `assign` runs as the system right
//! after, so the ticket starts `assigned` with its SLA running (#56 A6).
//! `Tickets` is also the app's [`TaskCreator`]:
//! [`WlTicketing`](crate::WlTicketing) provides it in `register`, so other
//! modules open tickets through `wl_kernel::tickets::task_creator`.

use renox::db::DateTime;
use renox::prelude::*;
use wl_kernel::calendar::calendar;
use wl_kernel::tickets::{
    OpenTicket, Requester, TICKET_SERIES, TaskComment, TaskCreator, TicketFuture, TicketId,
    TicketRef,
};

use crate::assign::{assign_as_system, check_assignee};
use crate::comments::{Comment, CommentVisibility};
use crate::config::Category;
use crate::ticket::{Ticket, TicketStatus};
use crate::work::resolve_from_source;
use renox::state_machine::TransitionError;

/// The ticket service: opens tickets, and is the app's [`TaskCreator`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Tickets;

impl Tickets {
    /// Opens the ticket `request` asks for, now.
    ///
    /// # Errors
    ///
    /// 422 for an unknown or inactive category key, a blank title, or an
    /// assignee who is not a user or an active vendor.
    pub async fn open(state: &AppState, request: OpenTicket) -> Result<TicketRef> {
        Self::open_at(state, request, renox::db::now()).await
    }

    /// Opens the ticket `request` asks for, at `at` (which picks the year of
    /// its number).
    ///
    /// # Errors
    ///
    /// As [`open`](Self::open).
    pub async fn open_at(state: &AppState, request: OpenTicket, at: DateTime) -> Result<TicketRef> {
        let unprocessable = |message: String| abort(StatusCode::UNPROCESSABLE_ENTITY, message);
        let title = request.title.trim();
        if title.is_empty() {
            return Err(unprocessable("A ticket needs a title.".to_owned()));
        }
        let category = match Category::by_key(&state.db, request.category_key.trim()).await? {
            Some(category) if category.active => category,
            _ => {
                return Err(unprocessable(format!(
                    "`{}` is not a ticket category.",
                    request.category_key
                )));
            }
        };
        if let Some(assignee) = request.assignee {
            check_assignee(state, assignee)
                .await
                .map_err(|error| unprocessable(error.to_string()))?;
        }
        let source_key = request
            .source_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_owned);
        let on = calendar(state).local(at).date_naive();
        let number = TICKET_SERIES
            .next_allowing_gaps(&state.db, None, None, on)
            .await?;
        let (requester_user_id, requester_party_id) = match request.requester {
            Some(Requester::User { id }) => (Some(id), None),
            Some(Requester::Party { id }) => (None, Some(id.0)),
            None => (None, None),
        };
        let mut ticket = Ticket {
            number,
            subject_type: request.subject.subject_type,
            subject_id: request.subject.subject_id,
            source_type: request.source.as_ref().map(|s| s.subject_type.clone()),
            source_id: request.source.as_ref().map(|s| s.subject_id),
            category_id: category.id,
            priority: category.default_priority,
            requester_user_id,
            requester_party_id,
            channel: request.channel,
            title: title.to_owned(),
            description: request.description.trim().to_owned(),
            sla_policy_id: category.sla_policy_id,
            confidential: category.confidential,
            chargeable: category.chargeable,
            due_at: request.due_at,
            source_key,
            status: TicketStatus::Open,
            ..Ticket::default()
        };
        ticket.insert(&state.db).await?;
        if let Some(assignee) = request.assignee {
            // The first assignment, as the system. The transition has its
            // own transaction, so a refusal takes the new ticket back.
            if let Err(error) =
                assign_as_system(state, &ticket, assignee, category.default_priority, at).await
            {
                ticket.delete(&state.db).await?;
                return Err(unprocessable(error.to_string()));
            }
        }
        Ok(ticket.reference())
    }

    /// The open ticket opened with `source_key`, if any: any state except
    /// `resolved`, `closed` and `cancelled`. The newest one when several.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn find_open(state: &AppState, source_key: &str) -> Result<Option<TicketRef>> {
        let found = Ticket::query()
            .where_(Ticket::SOURCE_KEY.eq(source_key))
            .order_by_desc(Ticket::ID)
            .get(&state.db)
            .await?;
        Ok(found
            .into_iter()
            .find(Ticket::is_open)
            .map(|ticket| ticket.reference()))
    }

    /// Adds `comment` to ticket `id`, written by the system.
    ///
    /// # Errors
    ///
    /// 404 for an unknown ticket.
    pub async fn comment(state: &AppState, id: TicketId, comment: TaskComment) -> Result<()> {
        let ticket = Ticket::find_or_404(&state.db, id.0).await?;
        let mut row = Comment {
            ticket_id: ticket.id,
            author_user_id: None,
            visibility: if comment.internal {
                CommentVisibility::Internal
            } else {
                CommentVisibility::Requester
            },
            text: comment.text.trim().to_owned(),
            ..Comment::default()
        };
        row.insert(&state.db).await?;
        Ok(())
    }
}

impl Tickets {
    /// Resolves ticket `id` for the record that opened it, with `note` as the
    /// resolution. `false` when the ticket is unknown or already resolved,
    /// closed or cancelled.
    ///
    /// # Errors
    ///
    /// 422 for a blank note.
    pub async fn resolve_from_source(state: &AppState, id: TicketId, note: &str) -> Result<bool> {
        let Some(ticket) = Ticket::find(&state.db, id.0).await? else {
            return Ok(false);
        };
        if !ticket.is_open() {
            return Ok(false);
        }
        match resolve_from_source(state, &ticket, note, renox::db::now()).await {
            Ok(()) => Ok(true),
            Err(TransitionError::NotFrom(_)) => Ok(false),
            Err(error) => Err(abort(StatusCode::UNPROCESSABLE_ENTITY, error.to_string())),
        }
    }
}

impl TaskCreator for Tickets {
    fn open<'a>(
        &'a self,
        state: &'a AppState,
        ticket: OpenTicket,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        Box::pin(async move { Tickets::open(state, ticket).await.map(Some) })
    }

    fn find_open<'a>(
        &'a self,
        state: &'a AppState,
        source_key: &'a str,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        Box::pin(Tickets::find_open(state, source_key))
    }

    fn resolve_from_source<'a>(
        &'a self,
        state: &'a AppState,
        ticket: TicketId,
        note: &'a str,
    ) -> TicketFuture<'a, bool> {
        Box::pin(Tickets::resolve_from_source(state, ticket, note))
    }

    fn comment<'a>(
        &'a self,
        state: &'a AppState,
        ticket: TicketId,
        comment: TaskComment,
    ) -> TicketFuture<'a, ()> {
        Box::pin(Tickets::comment(state, ticket, comment))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::testing::TestApp;
    use wl_kernel::Subject;
    use wl_kernel::tickets::{Assignee, TicketChannel, task_creator};

    use crate::seed;

    async fn app() -> TestApp {
        let app = TestApp::new(
            App::new()
                .module(Auth::new())
                .module(crate::WlTicketing::new()),
        )
        .await;
        seed::install(app.db()).await.expect("seeded");
        app
    }

    fn at(text: &str) -> DateTime {
        text.parse().expect("an instant")
    }

    fn refused(error: &Error) -> bool {
        matches!(error, Error::Status(code, _) if *code == StatusCode::UNPROCESSABLE_ENTITY)
    }

    #[renox::test]
    async fn tickets_are_numbered_in_order_and_start_open_from_the_category() {
        let app = app().await;
        let unit = Subject::new("property.unit", 12);
        let now = at("2026-11-03T05:00:00Z");
        let first = Tickets::open_at(
            app.state(),
            OpenTicket::new(unit.clone(), "general", "Leaking tap").description("  Drips. "),
            now,
        )
        .await
        .unwrap();
        let second = Tickets::open_at(
            app.state(),
            OpenTicket::new(unit, "general", "Broken door"),
            now,
        )
        .await
        .unwrap();
        assert_eq!(first.number, "TKT-2026-0001");
        assert_eq!(second.number, "TKT-2026-0002");

        let ticket = Ticket::find_or_404(app.db(), first.id.0).await.unwrap();
        assert_eq!(ticket.status, TicketStatus::Open);
        assert_eq!(ticket.priority, crate::Priority::Normal);
        assert_eq!(ticket.channel, TicketChannel::System);
        assert_eq!(ticket.subject(), Subject::new("property.unit", 12));
        assert_eq!(ticket.description, "Drips.");
        assert_eq!(ticket.assignee(), None);
        assert_eq!(ticket.requester(), None);
        let general = Category::by_key(app.db(), "general")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ticket.category_id, general.id);
        assert_eq!(ticket.sla_policy_id, general.sla_policy_id);
        assert!(ticket.sla_started_at.is_none());
    }

    #[renox::test]
    async fn the_number_restarts_with_the_dubai_year() {
        let app = app().await;
        let unit = Subject::new("property.unit", 1);
        // 21:00 UTC on New Year's Eve is already 2027 in Dubai.
        let late = at("2026-12-31T21:00:00Z");
        let ticket = Tickets::open_at(app.state(), OpenTicket::new(unit, "general", "x"), late)
            .await
            .unwrap();
        assert_eq!(ticket.number, "TKT-2027-0001");
    }

    #[renox::test]
    async fn the_requester_source_and_channel_are_kept() {
        let app = app().await;
        let request = OpenTicket::new(Subject::new("property.unit", 12), "general", "Noise")
            .requester_party(wl_kernel::PartyId(5))
            .channel(TicketChannel::Portal)
            .source(Subject::new("invoicing.dispute", 3));
        let opened = Tickets::open(app.state(), request).await.unwrap();
        let ticket = Ticket::find_or_404(app.db(), opened.id.0).await.unwrap();
        assert_eq!(ticket.requester_party_id, Some(5));
        assert_eq!(ticket.requester_user_id, None);
        assert_eq!(ticket.channel, TicketChannel::Portal);
        assert_eq!(ticket.source_type.as_deref(), Some("invoicing.dispute"));
        assert_eq!(ticket.source_id, Some(3));
    }

    #[renox::test]
    async fn an_unknown_category_a_blank_title_or_an_unknown_assignee_is_refused() {
        let app = app().await;
        let unit = Subject::new("property.unit", 12);
        let state = app.state();
        let unknown = Tickets::open(state, OpenTicket::new(unit.clone(), "nope", "x")).await;
        assert!(refused(&unknown.unwrap_err()));
        let blank = Tickets::open(state, OpenTicket::new(unit.clone(), "general", "  ")).await;
        assert!(refused(&blank.unwrap_err()));
        let assigned = Tickets::open(
            state,
            OpenTicket::new(unit.clone(), "general", "x").assign_to(Assignee::user(999)),
        )
        .await;
        assert!(refused(&assigned.unwrap_err()));

        let mut general = Category::by_key(app.db(), "general")
            .await
            .unwrap()
            .unwrap();
        general.active = false;
        general.save(app.db()).await.unwrap();
        let inactive = Tickets::open(state, OpenTicket::new(unit, "general", "x")).await;
        assert!(refused(&inactive.unwrap_err()));
        // Nothing was stored.
        assert_eq!(Ticket::query().count(app.db()).await.unwrap(), 0);
    }

    #[renox::test]
    async fn other_modules_open_tickets_through_the_task_creator() {
        let app = app().await;
        let request = OpenTicket::new(Subject::new("property.unit", 12), "general", "Leaking tap");
        let opened = task_creator(app.state())
            .open(app.state(), request)
            .await
            .unwrap()
            .expect("ticketing is on");
        assert_eq!(opened.number, "TKT-2026-0001");
    }

    #[renox::test]
    async fn the_task_creator_resolves_from_the_source_once() {
        let app = app().await;
        let opened = Tickets::open(
            app.state(),
            OpenTicket::new(Subject::new("property.unit", 12), "general", "Leaking tap"),
        )
        .await
        .unwrap();
        let creator = task_creator(app.state());
        let state = app.state();
        assert!(
            creator
                .resolve_from_source(state, opened.id, "Done")
                .await
                .unwrap()
        );
        let ticket = Ticket::find_or_404(app.db(), opened.id.0).await.unwrap();
        assert_eq!(ticket.status, TicketStatus::Resolved);
        assert!(ticket.resolved_at.is_some());
        // Already resolved, or unknown: nothing happens.
        assert!(
            !creator
                .resolve_from_source(state, opened.id, "Again")
                .await
                .unwrap()
        );
        assert!(
            !creator
                .resolve_from_source(state, TicketId(999), "x")
                .await
                .unwrap()
        );
        // A blank note is refused and leaves the ticket as it is.
        let other = Tickets::open(
            state,
            OpenTicket::new(Subject::new("property.unit", 12), "general", "Door"),
        )
        .await
        .unwrap();
        let blank = creator.resolve_from_source(state, other.id, "  ").await;
        assert!(refused(&blank.unwrap_err()));
    }

    #[renox::test]
    async fn a_ticket_starts_open_and_its_status_is_saved_only_by_a_transition() {
        let app = app().await;
        let opened = Tickets::open(
            app.state(),
            OpenTicket::new(Subject::new("property.unit", 12), "general", "Leaking tap"),
        )
        .await
        .unwrap();
        let mut ticket = Ticket::find_or_404(app.db(), opened.id.0).await.unwrap();
        ticket.status = TicketStatus::Closed;
        assert!(ticket.save(app.db()).await.is_err());
        let stored = Ticket::find_or_404(app.db(), opened.id.0).await.unwrap();
        assert_eq!(stored.status, TicketStatus::Open);

        // Nor can a ticket be born in another state.
        let mut born = Ticket {
            number: "TKT-2026-0099".into(),
            subject_type: "property.unit".into(),
            subject_id: 1,
            category_id: stored.category_id,
            title: "x".into(),
            status: TicketStatus::Resolved,
            ..Ticket::default()
        };
        assert!(born.insert(app.db()).await.is_err());
    }

    #[renox::test]
    async fn a_ticket_opened_with_an_assignee_is_assigned_by_the_system() {
        let app = app().await;
        let user =
            renox::auth::User::register(app.db(), "Imran", "imran@example.com", "password123")
                .await
                .unwrap();
        let due = at("2026-11-06T06:00:00Z");
        let request = OpenTicket::new(Subject::new("property.unit", 12), "general", "Call back")
            .assign_to(Assignee::user(user.id))
            .due_at(due)
            .source_key("test:1");
        let opened = Tickets::open_at(app.state(), request, at("2026-11-05T06:00:00Z"))
            .await
            .unwrap();
        let ticket = Ticket::find_or_404(app.db(), opened.id.0).await.unwrap();
        assert_eq!(ticket.status, TicketStatus::Assigned);
        assert_eq!(ticket.assignee(), Some(Assignee::user(user.id)));
        assert_eq!(ticket.due_at, Some(due));
        assert_eq!(ticket.source_key.as_deref(), Some("test:1"));
        assert!(ticket.sla_started_at.is_some());
        assert!(ticket.resolution_due.is_some());
        let history = crate::Assignment::history(app.db(), ticket.id)
            .await
            .unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].by_user_id, None);
    }

    #[renox::test]
    async fn find_open_finds_a_task_by_its_key_until_it_is_closed() {
        let app = app().await;
        let unit = Subject::new("property.unit", 12);
        let creator = task_creator(app.state());
        assert!(
            creator
                .find_open(app.state(), "k:1")
                .await
                .unwrap()
                .is_none()
        );
        let opened = Tickets::open(
            app.state(),
            OpenTicket::new(unit.clone(), "general", "x").source_key("k:1"),
        )
        .await
        .unwrap();
        let found = creator.find_open(app.state(), "k:1").await.unwrap();
        assert_eq!(found, Some(opened.clone()));
        assert!(
            creator
                .find_open(app.state(), "k:2")
                .await
                .unwrap()
                .is_none()
        );

        // A cancelled ticket is no longer open; a newer one is found instead.
        let stored = Ticket::find_or_404(app.db(), opened.id.0).await.unwrap();
        crate::work::auto_cancel(app.state(), &stored, "Not needed")
            .await
            .unwrap();
        assert!(
            creator
                .find_open(app.state(), "k:1")
                .await
                .unwrap()
                .is_none()
        );
        let again = Tickets::open(
            app.state(),
            OpenTicket::new(unit, "general", "y").source_key("k:1"),
        )
        .await
        .unwrap();
        let found = creator.find_open(app.state(), "k:1").await.unwrap();
        assert_eq!(found, Some(again));
    }

    #[renox::test]
    async fn a_comment_is_stored_as_the_system_with_its_visibility() {
        let app = app().await;
        let opened = Tickets::open(
            app.state(),
            OpenTicket::new(Subject::new("property.unit", 12), "general", "x"),
        )
        .await
        .unwrap();
        let creator = task_creator(app.state());
        creator
            .comment(
                app.state(),
                opened.id,
                TaskComment::internal(" Tried again. "),
            )
            .await
            .unwrap();
        creator
            .comment(app.state(), opened.id, TaskComment::visible("Update"))
            .await
            .unwrap();
        let comments = Comment::of_ticket(app.db(), opened.id.0).await.unwrap();
        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0].text, "Tried again.");
        assert_eq!(comments[0].visibility, CommentVisibility::Internal);
        assert_eq!(comments[0].author_user_id, None);
        assert_eq!(comments[1].visibility, CommentVisibility::Requester);
        let missing = creator
            .comment(app.state(), TicketId(999), TaskComment::internal("x"))
            .await;
        assert!(missing.is_err());
    }
}
