//! What follows an `assign` once it commits (#146 A13, A14): a listener on
//! `Transitioned<Ticket>` that emits `TicketAssigned` and sends the notices
//! through the kernel [`Notifier`](wl_kernel::notify::Notifier).
//!
//! | Case | Notice |
//! |---|---|
//! | first assignment | [`ASSIGNED`] to the assignee |
//! | reassignment | [`WITHDRAWN`] to the previous assignee, [`ASSIGNED`] to the new one |
//! | visit change | [`VISIT_RESCHEDULED`] to the requester, with the new time |
//!
//! `TicketAssigned` is emitted for the first assignment and a reassignment,
//! not for a visit change. The listener reads the committed ticket, and the
//! kind and assignees from the transition's `meta`.

use renox::db::DateTime;
use renox::prelude::*;
use renox::serde_json::{self, Value, json};
use renox::state_machine::Transitioned;
use wl_kernel::notify::{self, NotificationTypeDef, NotifyChannel, NotifyRequest};
use wl_kernel::parties::directory;
use wl_kernel::tickets::{Assignee, TicketAssigned};

use crate::assign::{ASSIGN, AssignKind, AssignMeta, reasons_of};
use crate::texts::english;
use crate::ticket::Ticket;

/// The module that sends the notices.
const MODULE: &str = "ticketing";
/// The group of the wl-notify §5.4 default table.
const GROUP: &str = "tickets";

/// To the new assignee: a ticket is yours.
pub const ASSIGNED: &str = "ticketing.assigned";
/// To the previous assignee: the job was taken away.
pub const WITHDRAWN: &str = "ticketing.withdrawn";
/// To the requester: the visit is at another time.
pub const VISIT_RESCHEDULED: &str = "ticketing.visit_rescheduled";

/// The notification type `key`, with its English text from the package.
fn type_of(key: &str, texts: &str) -> NotificationTypeDef {
    let subject = english(&format!("wl_ticketing.notice.{texts}.subject")).to_owned();
    let body = english(&format!("wl_ticketing.notice.{texts}.body")).to_owned();
    NotificationTypeDef::new(key, MODULE).group(GROUP).template(
        NotifyChannel::Email,
        "en",
        Some(&subject),
        &body,
    )
}

/// The three types this module sends.
pub fn notification_types() -> [NotificationTypeDef; 3] {
    [
        type_of(ASSIGNED, "assigned"),
        type_of(WITHDRAWN, "withdrawn"),
        type_of(VISIT_RESCHEDULED, "visit_rescheduled"),
    ]
}

/// How a time reads in a notice: Asia/Dubai, e.g. `03 Nov 2026 13:00`.
fn time_text(state: &AppState, at: Option<DateTime>) -> String {
    match at {
        Some(at) => wl_kernel::calendar::calendar(state)
            .local(at)
            .format("%d %b %Y %H:%M")
            .to_string(),
        None => english("wl_ticketing.notice.not_scheduled").to_owned(),
    }
}

/// The name of an assignee in a notice.
async fn name_of(state: &AppState, assignee: Assignee) -> Result<String> {
    Ok(match assignee {
        Assignee::User { id } => User::find(&state.db, id)
            .await?
            .map_or_else(|| format!("user {id}"), |user| user.name),
        Assignee::Party { id } => directory(state)
            .display_name(&state.db, id, "en")
            .await?
            .unwrap_or_else(|| format!("party {id}")),
    })
}

/// The reason as a notice reads it: the name of the code, then the text.
fn reason_text(state: &AppState, meta: &AssignMeta) -> String {
    let Some(code) = meta.reason_code.as_deref() else {
        return String::new();
    };
    let name = reasons_of(state)
        .get(code)
        .map_or_else(|| code.to_owned(), |r| r.name.clone());
    match meta.reason_text.as_deref() {
        Some(text) if !text.is_empty() => format!("{name} - {text}"),
        _ => name,
    }
}

/// The notices that follow the assign `meta` described on `ticket`.
pub(crate) async fn send(state: &AppState, ticket: &Ticket, meta: &AssignMeta) -> Result<()> {
    let notifier = notify::notifier(state);
    let subject = ticket.ticket_id().subject();
    let base = |extra: Value| {
        let mut data = json!({
            "number": ticket.number,
            "title": ticket.title,
            "priority": ticket.priority.name(),
            "visit": time_text(state, meta.visit_at),
        });
        if let (Some(data), Value::Object(extra)) = (data.as_object_mut(), extra) {
            data.extend(extra);
        }
        data
    };
    match meta.kind() {
        Some(AssignKind::First | AssignKind::Reassign) => {
            if let Some(previous) = meta.previous_assignee {
                let data = base(json!({
                    "assignee": name_of(state, meta.assignee).await?,
                    "reason": reason_text(state, meta),
                }));
                let request = NotifyRequest::new(WITHDRAWN, subject.clone())
                    .to(previous.recipient())
                    .data(data);
                notifier.notify(state, request).await?;
            }
            let data = base(json!({
                "response_due": time_text(state, ticket.response_due),
                "resolution_due": time_text(state, ticket.resolution_due),
            }));
            let request = NotifyRequest::new(ASSIGNED, subject)
                .to(meta.assignee.recipient())
                .data(data);
            notifier.notify(state, request).await?;
        }
        Some(AssignKind::VisitChange) => {
            if let Some(requester) = ticket.requester() {
                let request = NotifyRequest::new(VISIT_RESCHEDULED, subject)
                    .to(requester.recipient())
                    .data(base(json!({})));
                notifier.notify(state, request).await?;
            }
        }
        None => {}
    }
    Ok(())
}

/// The `TicketAssigned` event for the first assignment or reassignment
/// `meta` described on the committed `ticket`.
pub(crate) fn assigned_event(ticket: &Ticket, meta: &AssignMeta) -> Option<TicketAssigned> {
    matches!(meta.kind(), Some(AssignKind::First | AssignKind::Reassign)).then(|| TicketAssigned {
        ticket: ticket.ticket_id(),
        assignee: meta.assignee,
        previous_assignee: meta.previous_assignee,
        reassignment_reason: meta.reason_code.clone(),
        priority: ticket.priority.as_str().to_owned(),
        response_due: ticket.response_due,
        resolution_due: ticket.resolution_due,
    })
}

/// Registers the notification types and the listener that, after an
/// `assign` commits, emits `TicketAssigned` and sends the notices.
pub(crate) fn register(app: &mut Registry) {
    for def in notification_types() {
        notify::register_type(def);
    }
    app.listen(|event: Transitioned<Ticket>, state: AppState| async move {
        if event.transition != ASSIGN {
            return Ok(());
        }
        let Ok(meta) = serde_json::from_value::<AssignMeta>(event.meta.clone()) else {
            return Ok(());
        };
        // The committed row: the due times the first assignment wrote.
        let ticket = Ticket::find_or_404(&state.db, event.key).await?;
        if let Some(assigned) = assigned_event(&ticket, &meta) {
            state.emit(assigned).await?;
        }
        send(&state, &ticket, &meta).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use renox::chrono::NaiveDate;
    use renox::testing::TestApp;
    use wl_kernel::BusinessCalendar;
    use wl_kernel::PartyId;
    use wl_kernel::notify::{MessageId, Notifier, NotifyFuture, NotifyRecipient};
    use wl_kernel::tickets::TicketId;

    use crate::priority::Priority;
    use crate::ticket::TicketStatus;

    fn dubai(date: &str, hour: u32) -> DateTime {
        let local = date
            .parse::<NaiveDate>()
            .expect("a date")
            .and_hms_opt(hour, 0, 0)
            .expect("a time");
        BusinessCalendar::default().from_local(local)
    }

    const COOLAIR: Assignee = Assignee::Party { id: PartyId(5) };
    const IMRAN: Assignee = Assignee::User { id: 7 };

    #[derive(Clone, Default)]
    struct Recording(Arc<Mutex<Vec<NotifyRequest>>>);

    impl Notifier for Recording {
        fn notify<'a>(
            &'a self,
            _state: &'a AppState,
            request: NotifyRequest,
        ) -> NotifyFuture<'a, Vec<MessageId>> {
            self.0.lock().unwrap().push(request);
            Box::pin(async { Ok(Vec::new()) })
        }
    }

    fn ticket() -> Ticket {
        Ticket {
            id: 42,
            number: "TKT-2026-0042".into(),
            subject_type: "property.unit".into(),
            subject_id: 12,
            title: "Chiller".into(),
            priority: Priority::High,
            requester_user_id: Some(3),
            assignee_user_id: Some(7),
            response_due: Some(dubai("2026-11-03", 17)),
            resolution_due: Some(dubai("2026-11-05", 18)),
            status: TicketStatus::Assigned,
            ..Ticket::default()
        }
    }

    fn meta(kind: AssignKind, previous: Option<Assignee>, assignee: Assignee) -> AssignMeta {
        AssignMeta {
            kind: kind.as_str().to_owned(),
            previous_assignee: previous,
            assignee,
            reason_code: (kind == AssignKind::Reassign).then(|| "vendor_no_show".to_owned()),
            reason_text: None,
            visit_at: Some(dubai("2026-11-04", 10)),
        }
    }

    async fn app(notices: &Recording) -> TestApp {
        let notifier: Arc<dyn Notifier> = Arc::new(notices.clone());
        TestApp::new(
            App::new()
                .module(Auth::new())
                .module(crate::WlTicketing::new())
                .provide(notifier),
        )
        .await
    }

    fn sent(notices: &Recording) -> Vec<(String, NotifyRecipient)> {
        notices
            .0
            .lock()
            .unwrap()
            .iter()
            .flat_map(|r| {
                r.recipients
                    .iter()
                    .map(|to| (r.type_key.clone(), to.clone()))
            })
            .collect()
    }

    #[test]
    fn the_three_types_have_an_english_text() {
        for def in notification_types() {
            let template = def
                .template_for(NotifyChannel::Email, "en")
                .expect("a text");
            let subject = template.subject.as_deref().unwrap_or_default();
            assert!(subject.contains("{{ number }}"), "{}", def.key);
            assert!(!template.body.starts_with("wl_ticketing."), "{}", def.key);
            assert_eq!(def.group, "tickets");
        }
        let keys = notification_types().map(|d| d.key);
        assert_eq!(keys, [ASSIGNED, WITHDRAWN, VISIT_RESCHEDULED]);
    }

    #[test]
    fn ticket_assigned_is_for_a_first_assignment_or_a_reassignment_only() {
        let ticket = ticket();
        let first = assigned_event(&ticket, &meta(AssignKind::First, None, COOLAIR)).unwrap();
        assert_eq!(first.ticket, TicketId(42));
        assert_eq!(first.assignee, COOLAIR);
        assert_eq!(first.previous_assignee, None);
        assert_eq!(first.reassignment_reason, None);
        assert_eq!(first.priority, "high");
        assert_eq!(first.response_due, ticket.response_due);
        assert_eq!(first.resolution_due, ticket.resolution_due);

        let again =
            assigned_event(&ticket, &meta(AssignKind::Reassign, Some(COOLAIR), IMRAN)).unwrap();
        assert_eq!(again.previous_assignee, Some(COOLAIR));
        assert_eq!(again.assignee, IMRAN);
        assert_eq!(again.reassignment_reason.as_deref(), Some("vendor_no_show"));

        let visit = meta(AssignKind::VisitChange, Some(COOLAIR), COOLAIR);
        assert!(assigned_event(&ticket, &visit).is_none());
    }

    #[renox::test]
    async fn a_first_assignment_tells_only_the_assignee() {
        let notices = Recording::default();
        let app = app(&notices).await;
        send(
            app.state(),
            &ticket(),
            &meta(AssignKind::First, None, COOLAIR),
        )
        .await
        .unwrap();
        assert_eq!(
            sent(&notices),
            [(
                ASSIGNED.to_owned(),
                NotifyRecipient::Party { id: PartyId(5) }
            )]
        );
        let request = notices.0.lock().unwrap()[0].clone();
        assert_eq!(request.subject.subject_type, "ticketing.ticket");
        assert_eq!(request.data["number"], "TKT-2026-0042");
        assert_eq!(request.data["response_due"], "03 Nov 2026 17:00");
        assert_eq!(request.data["resolution_due"], "05 Nov 2026 18:00");
    }

    #[renox::test]
    async fn a_reassignment_withdraws_from_the_previous_and_assigns_the_new() {
        let notices = Recording::default();
        let app = app(&notices).await;
        send(
            app.state(),
            &ticket(),
            &meta(AssignKind::Reassign, Some(COOLAIR), IMRAN),
        )
        .await
        .unwrap();
        assert_eq!(
            sent(&notices),
            [
                (
                    WITHDRAWN.to_owned(),
                    NotifyRecipient::Party { id: PartyId(5) }
                ),
                (ASSIGNED.to_owned(), NotifyRecipient::User { id: 7 }),
            ]
        );
        let withdrawn = notices.0.lock().unwrap()[0].clone();
        assert_eq!(withdrawn.data["reason"], "Vendor did not turn up");
    }

    #[renox::test]
    async fn a_visit_change_tells_the_requester_the_new_time_and_nobody_else() {
        let notices = Recording::default();
        let app = app(&notices).await;
        send(
            app.state(),
            &ticket(),
            &meta(AssignKind::VisitChange, Some(COOLAIR), COOLAIR),
        )
        .await
        .unwrap();
        assert_eq!(
            sent(&notices),
            [(
                VISIT_RESCHEDULED.to_owned(),
                NotifyRecipient::User { id: 3 }
            )]
        );
        let request = notices.0.lock().unwrap()[0].clone();
        assert_eq!(request.data["visit"], "04 Nov 2026 10:00");

        // No requester, no notice.
        let internal = Ticket {
            requester_user_id: None,
            ..ticket()
        };
        let before = sent(&notices).len();
        send(
            app.state(),
            &internal,
            &meta(AssignKind::VisitChange, Some(COOLAIR), COOLAIR),
        )
        .await
        .unwrap();
        assert_eq!(sent(&notices).len(), before);
    }
}
