//! Opening a ticket (wl-ticketing §6, kernel §8): [`Tickets::open`].
//!
//! A new ticket is `open`, numbered `TKT-{YYYY}-{0000}` from the kernel's
//! `ticket` series ([`TICKET_SERIES`](wl_kernel::tickets::TICKET_SERIES);
//! gaps allowed, taken on the Asia/Dubai date of the opening), with its
//! priority, SLA policy, `confidential` and `chargeable` copied from its
//! category. `Tickets` is also the app's [`TaskCreator`]:
//! [`WlTicketing`](crate::WlTicketing) provides it in `register`, so other
//! modules open tickets through `wl_kernel::tickets::task_creator`.

use renox::db::DateTime;
use renox::prelude::*;
use wl_kernel::calendar::calendar;
use wl_kernel::tickets::{
    OpenTicket, Requester, TICKET_SERIES, TaskCreator, TicketFuture, TicketRef,
};

use crate::config::Category;
use crate::ticket::{Ticket, TicketStatus};

/// The ticket service: opens tickets, and is the app's [`TaskCreator`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Tickets;

impl Tickets {
    /// Opens the ticket `request` asks for, now.
    ///
    /// # Errors
    ///
    /// 422 for an unknown or inactive category key, a blank title, or an
    /// `assignee` (assigning when opening comes with `assign`, which a
    /// ticket needs to start its SLA).
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
        if request.assignee.is_some() {
            return Err(unprocessable(
                "A ticket is opened unassigned; assign it afterwards.".to_owned(),
            ));
        }
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
            status: TicketStatus::Open,
            ..Ticket::default()
        };
        ticket.insert(&state.db).await?;
        Ok(ticket.reference())
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::testing::TestApp;
    use wl_kernel::Subject;
    use wl_kernel::tickets::{Assignee, TicketChannel, task_creator};

    use crate::seed;

    async fn app() -> TestApp {
        let app = TestApp::new(App::new().module(crate::WlTicketing::new())).await;
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
    async fn an_unknown_category_a_blank_title_or_an_assignee_is_refused() {
        let app = app().await;
        let unit = Subject::new("property.unit", 12);
        let state = app.state();
        let unknown = Tickets::open(state, OpenTicket::new(unit.clone(), "nope", "x")).await;
        assert!(refused(&unknown.unwrap_err()));
        let blank = Tickets::open(state, OpenTicket::new(unit.clone(), "general", "  ")).await;
        assert!(refused(&blank.unwrap_err()));
        let assigned = Tickets::open(
            state,
            OpenTicket::new(unit.clone(), "general", "x").assign_to(Assignee::user(1)),
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
}
