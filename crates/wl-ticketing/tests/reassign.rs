//! Assigning and reassigning a ticket with a recorded reason: the
//! acceptance checks AC4 to AC8 and AC11 of #146. TKT-2026-0042 of the issue
//! is the first ticket here (High, "CoolAir" first, then technician
//! "Imran").

mod support;

use renox::prelude::*;
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::tickets::Assignee;
use wl_ticketing::assign::AssignKind;
use wl_ticketing::{Priority, Ticket, TicketStatus};

use support::{Booted, COOLAIR, COOLAIR_CODE, DUSTY, boot, dubai, local};

const COOLAIR_ASSIGNEE: Assignee = Assignee::Party { id: COOLAIR };

/// The people of the story.
struct Cast {
    coordinator: User,
    imran: User,
    requester: User,
}

async fn cast(t: &Booted) -> Cast {
    Cast {
        coordinator: t.coordinator().await,
        imran: t.technician("Imran").await,
        requester: t.user("Rashid", &[wl_ticketing::permissions::VIEW]).await,
    }
}

/// An open ticket, assigned to CoolAir on Tuesday 2026-11-03 at 09:00 as
/// High priority with a visit on the 4th at 10:00.
async fn assigned_to_coolair(t: &Booted, cast: &Cast) -> Ticket {
    let open = t.open("general", Some(&cast.requester)).await;
    assign_to_coolair(t, cast, open).await
}

/// `open`, assigned to CoolAir as [`assigned_to_coolair`] says.
async fn assign_to_coolair(t: &Booted, cast: &Cast, open: Ticket) -> Ticket {
    let visit = local("2026-11-04 10:00");
    t.assign(
        &cast.coordinator,
        &open,
        "2026-11-03 09:00",
        &[("party", "5"), ("priority", "high"), ("visit", &visit)],
    )
    .await
    .assert_ok();
    t.reload(&open).await
}

/// AC4: the first assignment starts the SLA in working time, writes a
/// `first` row, emits `TicketAssigned` and tells CoolAir.
#[renox::test]
async fn the_first_assignment_starts_the_sla_and_tells_the_vendor() {
    let t = boot().await;
    let cast = cast(&t).await;
    let open = t.open("general", Some(&cast.requester)).await;
    assert_eq!(open.status, TicketStatus::Open);

    let response = t
        .assign(
            &cast.coordinator,
            &open,
            "2026-11-03 09:00",
            &[("party", "5"), ("priority", "high")],
        )
        .await;
    response.assert_ok();
    assert_eq!(response.text(), "first");

    let ticket = t.reload(&open).await;
    assert_eq!(ticket.status, TicketStatus::Assigned);
    assert_eq!(ticket.priority, Priority::High);
    assert_eq!(ticket.assignee(), Some(COOLAIR_ASSIGNEE));
    assert_eq!(ticket.sla_started_at, Some(dubai("2026-11-03", 9, 0)));
    assert_eq!(ticket.response_due, Some(dubai("2026-11-03", 17, 0)));
    assert_eq!(ticket.resolution_due, Some(dubai("2026-11-05", 18, 0)));

    let history = t.history(&ticket).await;
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].kind, AssignKind::First);
    assert_eq!(history[0].previous_assignee(), None);
    assert_eq!(history[0].assignee(), Some(COOLAIR_ASSIGNEE));
    assert_eq!(history[0].reason_code, None);
    assert_eq!(history[0].by_user_id, Some(cast.coordinator.id));
    assert_eq!(history[0].at, dubai("2026-11-03", 9, 0));

    let events = t.assigned.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].ticket, ticket.ticket_id());
    assert_eq!(events[0].assignee, COOLAIR_ASSIGNEE);
    assert_eq!(events[0].previous_assignee, None);
    assert_eq!(events[0].reassignment_reason, None);
    assert_eq!(events[0].priority, "high");
    assert_eq!(events[0].resolution_due, Some(dubai("2026-11-05", 18, 0)));

    assert_eq!(
        t.notices.recipients("ticketing.assigned"),
        [NotifyRecipient::Party { id: COOLAIR }]
    );
    assert!(t.notices.of_type("ticketing.withdrawn").is_empty());
    let notice = &t.notices.of_type("ticketing.assigned")[0];
    assert_eq!(notice.subject, ticket.ticket_id().subject());
    assert_eq!(notice.data["number"], ticket.number.as_str());
    assert_eq!(notice.data["response_due"], "03 Nov 2026 17:00");
    assert_eq!(notice.data["resolution_due"], "05 Nov 2026 18:00");
}

/// AC5: the coordinator reassigns to Imran at 13:00 with `vendor_no_show`;
/// the history shows it, both are told, and the due times stay.
#[renox::test]
async fn a_reassignment_is_recorded_and_both_assignees_are_told() {
    let t = boot().await;
    let cast = cast(&t).await;
    let before = assigned_to_coolair(&t, &cast).await;
    let imran = cast.imran.id.to_string();

    let response = t
        .assign(
            &cast.coordinator,
            &before,
            "2026-11-03 13:00",
            &[
                ("user", &imran),
                ("priority", "high"),
                ("reason_code", "vendor_no_show"),
                ("reason_text", "Nobody came at noon"),
            ],
        )
        .await;
    response.assert_ok();
    assert_eq!(response.text(), "reassign");

    let ticket = t.reload(&before).await;
    assert_eq!(ticket.status, TicketStatus::Assigned);
    assert_eq!(
        ticket.assignee(),
        Some(Assignee::User { id: cast.imran.id })
    );
    assert_eq!(ticket.assignee_party_id, None);
    assert_eq!(ticket.sla_started_at, before.sla_started_at);
    assert_eq!(ticket.response_due, Some(dubai("2026-11-03", 17, 0)));
    assert_eq!(ticket.resolution_due, Some(dubai("2026-11-05", 18, 0)));

    let history = t.history(&ticket).await;
    assert_eq!(history.len(), 2);
    let row = &history[1];
    assert_eq!(row.kind, AssignKind::Reassign);
    assert_eq!(row.previous_assignee(), Some(COOLAIR_ASSIGNEE));
    assert_eq!(row.assignee(), Some(Assignee::User { id: cast.imran.id }));
    assert_eq!(row.reason_code.as_deref(), Some("vendor_no_show"));
    assert_eq!(row.reason_text.as_deref(), Some("Nobody came at noon"));
    assert_eq!(row.at, dubai("2026-11-03", 13, 0));
    assert_eq!(row.by_user_id, Some(cast.coordinator.id));

    // The state history keeps the reason code as its reason.
    let transitions = renox::state_machine::history(t.db(), Ticket::TABLE, ticket.id)
        .await
        .unwrap();
    let last = transitions.last().expect("transitions");
    assert_eq!(last.transition, "assign");
    assert_eq!(last.reason.as_deref(), Some("vendor_no_show"));

    assert_eq!(
        t.notices.recipients("ticketing.withdrawn"),
        [NotifyRecipient::Party { id: COOLAIR }]
    );
    assert_eq!(
        t.notices.recipients("ticketing.assigned"),
        [
            NotifyRecipient::Party { id: COOLAIR },
            NotifyRecipient::User { id: cast.imran.id }
        ]
    );
    let withdrawn = &t.notices.of_type("ticketing.withdrawn")[0];
    assert_eq!(withdrawn.data["assignee"], "Imran");
    assert!(
        withdrawn.data["reason"]
            .as_str()
            .unwrap()
            .starts_with("Vendor did not turn up")
    );

    let events = t.assigned.all();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].assignee, Assignee::User { id: cast.imran.id });
    assert_eq!(events[1].previous_assignee, Some(COOLAIR_ASSIGNEE));
    assert_eq!(
        events[1].reassignment_reason.as_deref(),
        Some("vendor_no_show")
    );
    assert_eq!(events[1].response_due, Some(dubai("2026-11-03", 17, 0)));
    assert_eq!(events[1].resolution_due, Some(dubai("2026-11-05", 18, 0)));
}

/// AC5: changing the priority in the same assign does not move the due
/// times.
#[renox::test]
async fn changing_the_priority_on_a_reassignment_keeps_the_due_times() {
    let t = boot().await;
    let cast = cast(&t).await;
    let before = assigned_to_coolair(&t, &cast).await;
    let imran = cast.imran.id.to_string();

    t.assign(
        &cast.coordinator,
        &before,
        "2026-11-03 13:00",
        &[
            ("user", &imran),
            ("priority", "urgent"),
            ("reason_code", "workload"),
        ],
    )
    .await
    .assert_ok();

    let ticket = t.reload(&before).await;
    assert_eq!(ticket.priority, Priority::Urgent);
    assert_eq!(ticket.sla_started_at, Some(dubai("2026-11-03", 9, 0)));
    assert_eq!(ticket.response_due, Some(dubai("2026-11-03", 17, 0)));
    assert_eq!(ticket.resolution_due, Some(dubai("2026-11-05", 18, 0)));
    assert_eq!(t.assigned.all()[1].priority, "urgent");
}

/// AC6: a reassignment without a reason, with an unknown code, or with
/// `other` and a blank text is refused; nothing changes and nobody is told.
#[renox::test]
async fn a_reassignment_without_a_valid_reason_is_refused() {
    let t = boot().await;
    let cast = cast(&t).await;
    let before = assigned_to_coolair(&t, &cast).await;
    let imran = cast.imran.id.to_string();
    let notices = t.notices.all().len();
    let events = t.assigned.all().len();

    let attempts: [&[(&str, &str)]; 5] = [
        &[],
        &[("reason_code", "  ")],
        &[("reason_code", "bored")],
        &[("reason_code", "other")],
        &[("reason_code", "other"), ("reason_text", "   ")],
    ];
    for extra in attempts {
        let mut pairs = vec![("user", imran.as_str()), ("priority", "high")];
        pairs.extend_from_slice(extra);
        t.assign(&cast.coordinator, &before, "2026-11-03 13:00", &pairs)
            .await
            .assert_status(422);
    }

    let ticket = t.reload(&before).await;
    assert_eq!(ticket.assignee(), Some(COOLAIR_ASSIGNEE));
    assert_eq!(ticket.status, TicketStatus::Assigned);
    assert_eq!(t.history(&ticket).await.len(), 1);
    assert_eq!(t.notices.all().len(), notices);
    assert_eq!(t.assigned.all().len(), events);

    // `other` with a text is fine.
    t.assign(
        &cast.coordinator,
        &before,
        "2026-11-03 13:00",
        &[
            ("user", &imran),
            ("priority", "high"),
            ("reason_code", "other"),
            ("reason_text", "Moved to another site"),
        ],
    )
    .await
    .assert_ok();
    assert_eq!(t.history(&ticket).await.len(), 2);
}

/// AC6: the assign sheet's route answers 422 for a reassignment without a
/// reason and changes nothing.
#[renox::test]
async fn the_assign_route_refuses_a_reassignment_without_a_reason() {
    let t = boot().await;
    let cast = cast(&t).await;
    let before = assigned_to_coolair(&t, &cast).await;
    let imran = cast.imran.id.to_string();

    t.app.acting_as(&cast.coordinator);
    t.app
        .htmx()
        .post(
            &format!("/tickets/{}/assign", before.id),
            &[("assignee_user_id", imran.as_str()), ("priority", "high")],
        )
        .await
        .assert_status(422);
    let ticket = t.reload(&before).await;
    assert_eq!(ticket.assignee(), Some(COOLAIR_ASSIGNEE));
    assert_eq!(t.history(&ticket).await.len(), 1);
    assert!(t.notices.of_type("ticketing.withdrawn").is_empty());

    // With the reason, the same route reassigns.
    t.app
        .htmx()
        .post(
            &format!("/tickets/{}/assign", before.id),
            &[
                ("assignee_user_id", imran.as_str()),
                ("priority", "high"),
                ("reason_code", "contract_ended"),
            ],
        )
        .await
        .assert_ok();
    let ticket = t.reload(&before).await;
    assert_eq!(
        ticket.assignee(),
        Some(Assignee::User { id: cast.imran.id })
    );
    assert_eq!(ticket.resolution_due, Some(dubai("2026-11-05", 18, 0)));
}

/// AC7: moving only CoolAir's visit adds a `visit_change` row without a
/// reason, withdraws nothing and tells the requester the new time; the same
/// assignee at the same time is refused.
#[renox::test]
async fn a_visit_change_needs_no_reason_and_tells_the_requester() {
    let t = boot().await;
    let cast = cast(&t).await;
    let before = assigned_to_coolair(&t, &cast).await;
    let moved = local("2026-11-05 14:00");

    let response = t
        .assign(
            &cast.coordinator,
            &before,
            "2026-11-03 15:00",
            &[("party", "5"), ("priority", "high"), ("visit", &moved)],
        )
        .await;
    response.assert_ok();
    assert_eq!(response.text(), "visit_change");

    let ticket = t.reload(&before).await;
    assert_eq!(ticket.assignee(), Some(COOLAIR_ASSIGNEE));
    assert_eq!(ticket.visit_at, Some(dubai("2026-11-05", 14, 0)));
    assert_eq!(ticket.resolution_due, before.resolution_due);
    let history = t.history(&ticket).await;
    assert_eq!(history.len(), 2);
    assert_eq!(history[1].kind, AssignKind::VisitChange);
    assert_eq!(history[1].reason_code, None);
    assert_eq!(history[1].visit_at, Some(dubai("2026-11-05", 14, 0)));

    assert!(t.notices.of_type("ticketing.withdrawn").is_empty());
    assert_eq!(t.notices.of_type("ticketing.assigned").len(), 1);
    let rescheduled = t.notices.of_type("ticketing.visit_rescheduled");
    assert_eq!(rescheduled.len(), 1);
    assert_eq!(
        rescheduled[0].recipients,
        [NotifyRecipient::User {
            id: cast.requester.id
        }]
    );
    assert_eq!(rescheduled[0].data["visit"], "05 Nov 2026 14:00");
    // A visit change is not an assignment event.
    assert_eq!(t.assigned.all().len(), 1);

    // The same assignee at the same time changes nothing.
    t.assign(
        &cast.coordinator,
        &ticket,
        "2026-11-03 16:00",
        &[("party", "5"), ("priority", "high"), ("visit", &moved)],
    )
    .await
    .assert_status(422)
    .assert_see("Nothing changed");
    assert_eq!(t.history(&ticket).await.len(), 2);
    assert_eq!(t.notices.of_type("ticketing.visit_rescheduled").len(), 1);
}

/// AC8: `assign` on an `in_progress` or `on_hold` ticket is refused.
#[renox::test]
async fn an_assign_is_refused_once_the_work_has_started() {
    let t = boot().await;
    let cast = cast(&t).await;
    let imran = cast.imran.id.to_string();
    let open = t.open("general", None).await;
    t.assign(
        &cast.coordinator,
        &open,
        "2026-11-03 09:00",
        &[("user", &imran), ("priority", "high")],
    )
    .await
    .assert_ok();
    t.run(&cast.imran, &open, "start", "2026-11-03 09:30", &[])
        .await
        .assert_ok();

    let reassign = [
        ("party", "5"),
        ("priority", "high"),
        ("reason_code", "workload"),
    ];
    t.assign(&cast.coordinator, &open, "2026-11-03 10:00", &reassign)
        .await
        .assert_status(422);
    assert_eq!(t.reload(&open).await.status, TicketStatus::InProgress);

    t.run(
        &cast.imran,
        &open,
        "hold",
        "2026-11-03 10:00",
        &[("hold_reason", "parts")],
    )
    .await
    .assert_ok();
    t.assign(&cast.coordinator, &open, "2026-11-03 11:00", &reassign)
        .await
        .assert_status(422);
    let ticket = t.reload(&open).await;
    assert_eq!(ticket.status, TicketStatus::OnHold);
    assert_eq!(
        ticket.assignee(),
        Some(Assignee::User { id: cast.imran.id })
    );
    assert_eq!(t.history(&ticket).await.len(), 1);
}

/// AC8: without `tickets.assign` the assign route answers 403, and the
/// service refuses the user too.
#[renox::test]
async fn only_a_user_with_the_assign_permission_assigns() {
    let t = boot().await;
    let cast = cast(&t).await;
    let open = t.open("general", None).await;

    t.app.acting_as(&cast.imran);
    t.app
        .htmx()
        .post(
            &format!("/tickets/{}/assign", open.id),
            &[("party_code", COOLAIR_CODE), ("priority", "high")],
        )
        .await
        .assert_forbidden();
    t.assign(
        &cast.imran,
        &open,
        "2026-11-03 09:00",
        &[("party", "5"), ("priority", "high")],
    )
    .await
    .assert_forbidden();
    assert_eq!(t.reload(&open).await.status, TicketStatus::Open);
    assert!(t.history(&open).await.is_empty());
}

/// AC8: a blocked or unknown vendor party is refused, and so is a user that
/// does not exist.
#[renox::test]
async fn a_blocked_or_unknown_vendor_is_refused() {
    let t = boot().await;
    let cast = cast(&t).await;
    let open = t.open("general", None).await;
    let blocked = DUSTY.0.to_string();

    for assignee in [
        ("party", blocked.as_str()),
        ("party", "99"),
        ("user", "9999"),
    ] {
        t.assign(
            &cast.coordinator,
            &open,
            "2026-11-03 09:00",
            &[assignee, ("priority", "high")],
        )
        .await
        .assert_status(422);
    }
    let ticket = t.reload(&open).await;
    assert_eq!(ticket.status, TicketStatus::Open);
    assert_eq!(ticket.sla_started_at, None);
    assert!(t.history(&ticket).await.is_empty());
    assert!(t.notices.all().is_empty());
}

/// AC11: the ticket page shows the reassignment history and the SLA due
/// times; the assign sheet asks for the reason on an `assigned` ticket only.
#[renox::test]
async fn the_ticket_page_shows_the_history_and_asks_for_the_reason_only_when_reassigning() {
    let t = boot().await;
    let cast = cast(&t).await;
    let open = t.open("general", Some(&cast.requester)).await;
    let reason_select = format!("assign-reason-{}", open.id);
    let assign_sheet = format!("assign-ticket-{}", open.id);

    // Open: the sheet assigns, and asks for no reason.
    t.app.acting_as(&cast.coordinator);
    let page = t.app.get(&format!("/tickets/{}", open.id)).await;
    page.assert_ok().assert_see(&assign_sheet);
    page.assert_dont_see(&reason_select);

    let imran = cast.imran.id.to_string();
    let first = assign_to_coolair(&t, &cast, open.clone()).await;
    t.assign(
        &cast.coordinator,
        &first,
        "2026-11-03 13:00",
        &[
            ("user", &imran),
            ("priority", "high"),
            ("reason_code", "vendor_no_show"),
        ],
    )
    .await
    .assert_ok();

    t.app.acting_as(&cast.coordinator);
    let page = t.app.get(&format!("/tickets/{}", open.id)).await;
    page.assert_ok()
        .assert_see(&assign_sheet)
        .assert_see(&reason_select)
        .assert_see("Reassignment history")
        .assert_see("CoolAir to Imran")
        .assert_see("Vendor did not turn up")
        .assert_see("03 Nov 2026 13:00")
        .assert_see("03 Nov 2026 17:00")
        .assert_see("05 Nov 2026 18:00");
}
