//! The ticket lifecycle (wl-ticketing §4.1) with the SLA in working time:
//! the acceptance check AC9 of #146. Every action runs at an explicit time,
//! on Tuesday 2026-11-03 and the days after it (Asia/Dubai; the working
//! week is Monday to Friday, 09:00 to 18:00).

mod support;

use renox::prelude::*;
use renox::state_machine::{self, StateMachine};
use wl_kernel::tickets::Assignee;
use wl_ticketing::{Category, SlaPolicy, Ticket, TicketStatus};

use support::{Booted, boot, dubai};

/// The people of the story.
struct Cast {
    coordinator: User,
    imran: User,
    omar: User,
    requester: User,
}

async fn cast(t: &Booted) -> Cast {
    Cast {
        coordinator: t.coordinator().await,
        imran: t.technician("Imran").await,
        omar: t.technician("Omar").await,
        requester: t.user("Rashid", &[wl_ticketing::permissions::VIEW]).await,
    }
}

/// A ticket of `category` asked for by Rashid, assigned to Imran as High
/// priority on Tuesday 2026-11-03 at 09:00: response due 17:00, resolution
/// due Thursday 2026-11-05 18:00.
async fn assigned(t: &Booted, cast: &Cast, category: &str) -> Ticket {
    let open = t.open(category, Some(&cast.requester)).await;
    let imran = cast.imran.id.to_string();
    t.assign(
        &cast.coordinator,
        &open,
        "2026-11-03 09:00",
        &[("user", &imran), ("priority", "high")],
    )
    .await
    .assert_ok();
    let ticket = t.reload(&open).await;
    assert_eq!(ticket.resolution_due, Some(dubai("2026-11-05", 18, 0)));
    ticket
}

/// `assigned`, started at 09:30.
async fn started(t: &Booted, cast: &Cast, category: &str) -> Ticket {
    let ticket = assigned(t, cast, category).await;
    t.run(&cast.imran, &ticket, "start", "2026-11-03 09:30", &[])
        .await
        .assert_ok();
    t.reload(&ticket).await
}

/// A category that needs the requester's sign-off.
async fn signoff_category(t: &Booted) {
    let standard = SlaPolicy::by_key(t.db(), "standard")
        .await
        .unwrap()
        .unwrap();
    Category::create(
        t.db(),
        Category {
            key: "complaint".to_owned(),
            name_en: "Complaint".to_owned(),
            sla_policy_id: Some(standard.id),
            requires_signoff: true,
            active: true,
            ..Category::default()
        },
    )
    .await
    .unwrap();
}

/// `start` by the assignee sets the response time; another technician is
/// refused.
#[renox::test]
async fn only_the_assignee_starts_the_ticket() {
    let t = boot().await;
    let cast = cast(&t).await;
    let ticket = assigned(&t, &cast, "general").await;
    assert_eq!(ticket.responded_at, None);

    t.run(&cast.omar, &ticket, "start", "2026-11-03 09:30", &[])
        .await
        .assert_status(422);
    let still = t.reload(&ticket).await;
    assert_eq!(still.status, TicketStatus::Assigned);
    assert_eq!(still.responded_at, None);

    t.run(&cast.imran, &ticket, "start", "2026-11-03 09:45", &[])
        .await
        .assert_ok();
    let started = t.reload(&ticket).await;
    assert_eq!(started.status, TicketStatus::InProgress);
    assert_eq!(started.responded_at, Some(dubai("2026-11-03", 9, 45)));
    // Starting does not move the due times.
    assert_eq!(started.response_due, ticket.response_due);
    assert_eq!(started.resolution_due, ticket.resolution_due);
}

/// A hold for a pausing reason moves the resolution due time by the working
/// time it lasted; a hold for Approval does not.
#[renox::test]
async fn hold_and_resume_move_the_resolution_due_time_by_the_pause() {
    let t = boot().await;
    let cast = cast(&t).await;

    // Parts, 10:00 to 12:00: two working hours. Thursday 18:00 plus two
    // working hours is Friday 11:00.
    let parts = started(&t, &cast, "general").await;
    t.run(
        &cast.imran,
        &parts,
        "hold",
        "2026-11-03 10:00",
        &[("hold_reason", "parts")],
    )
    .await
    .assert_ok();
    let held = t.reload(&parts).await;
    assert_eq!(held.status, TicketStatus::OnHold);
    assert_eq!(held.sla_paused_at, Some(dubai("2026-11-03", 10, 0)));
    assert_eq!(held.resolution_due, parts.resolution_due);
    t.run(&cast.imran, &held, "resume", "2026-11-03 12:00", &[])
        .await
        .assert_ok();
    let resumed = t.reload(&parts).await;
    assert_eq!(resumed.status, TicketStatus::InProgress);
    assert_eq!(resumed.sla_paused_at, None);
    assert_eq!(resumed.hold_reason_id, None);
    assert_eq!(resumed.sla_paused_minutes, 120);
    assert_eq!(resumed.resolution_due, Some(dubai("2026-11-06", 11, 0)));
    assert_eq!(resumed.response_due, parts.response_due);

    // Approval does not stop the clock.
    let approval = started(&t, &cast, "general").await;
    t.run(
        &cast.imran,
        &approval,
        "hold",
        "2026-11-03 13:00",
        &[("hold_reason", "approval")],
    )
    .await
    .assert_ok();
    let held = t.reload(&approval).await;
    assert_eq!(held.status, TicketStatus::OnHold);
    assert_eq!(held.sla_paused_at, None);
    t.run(&cast.imran, &held, "resume", "2026-11-03 15:00", &[])
        .await
        .assert_ok();
    let resumed = t.reload(&approval).await;
    assert_eq!(resumed.status, TicketStatus::InProgress);
    assert_eq!(resumed.sla_paused_minutes, 0);
    assert_eq!(resumed.resolution_due, approval.resolution_due);

    // A hold needs a known reason, and only the assignee holds.
    let other = started(&t, &cast, "general").await;
    for pairs in [&[][..], &[("hold_reason", "lunch")][..]] {
        t.run(&cast.imran, &other, "hold", "2026-11-03 14:00", pairs)
            .await
            .assert_status(422);
    }
    t.run(
        &cast.omar,
        &other,
        "hold",
        "2026-11-03 14:00",
        &[("hold_reason", "parts")],
    )
    .await
    .assert_status(422);
    assert_eq!(t.reload(&other).await.status, TicketStatus::InProgress);
}

/// `resolve`, then `confirm` by the requester: `closed`, and `TicketClosed`
/// says whether it was in time.
#[renox::test]
async fn resolve_and_confirm_close_the_ticket_and_say_if_it_was_in_time() {
    let t = boot().await;
    let cast = cast(&t).await;

    let in_time = started(&t, &cast, "general").await;
    t.run(&cast.imran, &in_time, "resolve", "2026-11-04 12:00", &[])
        .await
        .assert_ok();
    let resolved = t.reload(&in_time).await;
    assert_eq!(resolved.status, TicketStatus::Resolved);
    assert_eq!(resolved.resolved_at, Some(dubai("2026-11-04", 12, 0)));
    assert!(t.closed.all().is_empty());
    // Not the assignee, not the requester.
    t.run(&cast.omar, &resolved, "confirm", "2026-11-04 13:00", &[])
        .await
        .assert_status(422);
    t.run(
        &cast.requester,
        &resolved,
        "confirm",
        "2026-11-04 13:00",
        &[],
    )
    .await
    .assert_ok();
    assert_eq!(t.reload(&in_time).await.status, TicketStatus::Closed);
    let closed = t.closed.all();
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].ticket, in_time.ticket_id());
    assert!(closed[0].within_sla);
    // 09:00 on Tuesday to 12:00 on Wednesday: 9 + 3 working hours.
    assert_eq!(closed[0].resolution_minutes, Some(12 * 60));

    let late = started(&t, &cast, "general").await;
    t.run(&cast.imran, &late, "resolve", "2026-11-06 12:00", &[])
        .await
        .assert_ok();
    t.run(&cast.requester, &late, "confirm", "2026-11-06 13:00", &[])
        .await
        .assert_ok();
    let closed = t.closed.all();
    assert_eq!(closed.len(), 2);
    assert_eq!(closed[1].ticket, late.ticket_id());
    assert!(!closed[1].within_sla);

    let history = state_machine::history(t.db(), Ticket::TABLE, in_time.id)
        .await
        .unwrap();
    let names: Vec<_> = history.iter().map(|row| row.transition.as_str()).collect();
    assert_eq!(names, ["assign", "start", "resolve", "confirm"]);
}

/// `close` by staff: refused on a sign-off category when the ticket has a
/// requester; fine otherwise.
#[renox::test]
async fn staff_cannot_close_a_signoff_ticket_that_has_a_requester() {
    let t = boot().await;
    let cast = cast(&t).await;
    signoff_category(&t).await;

    let signoff = started(&t, &cast, "complaint").await;
    t.run(&cast.imran, &signoff, "resolve", "2026-11-04 12:00", &[])
        .await
        .assert_ok();
    t.run(
        &cast.coordinator,
        &signoff,
        "close",
        "2026-11-04 13:00",
        &[],
    )
    .await
    .assert_status(422);
    assert_eq!(t.reload(&signoff).await.status, TicketStatus::Resolved);
    assert!(t.closed.all().is_empty());

    // No sign-off needed: staff close it.
    let plain = started(&t, &cast, "general").await;
    t.run(&cast.imran, &plain, "resolve", "2026-11-04 12:00", &[])
        .await
        .assert_ok();
    // Only a holder of `tickets.assign` closes.
    t.run(&cast.imran, &plain, "close", "2026-11-04 13:00", &[])
        .await
        .assert_forbidden();
    t.run(&cast.coordinator, &plain, "close", "2026-11-04 13:00", &[])
        .await
        .assert_ok();
    assert_eq!(t.reload(&plain).await.status, TicketStatus::Closed);
    assert_eq!(t.closed.all().len(), 1);
}

/// `reopen` works on day 7 after `resolved_at` and is refused on day 8.
#[renox::test]
async fn a_resolved_ticket_is_reopened_within_seven_days() {
    let t = boot().await;
    let cast = cast(&t).await;

    let day7 = started(&t, &cast, "general").await;
    let day8 = started(&t, &cast, "general").await;
    for ticket in [&day7, &day8] {
        t.run(&cast.imran, ticket, "resolve", "2026-11-04 12:00", &[])
            .await
            .assert_ok();
    }

    // Someone who is neither the requester nor a coordinator cannot.
    t.run(&cast.omar, &day7, "reopen", "2026-11-11 12:00", &[])
        .await
        .assert_status(422);
    t.run(&cast.requester, &day8, "reopen", "2026-11-12 12:00", &[])
        .await
        .assert_status(422);
    assert_eq!(t.reload(&day8).await.status, TicketStatus::Resolved);

    t.run(&cast.requester, &day7, "reopen", "2026-11-11 12:00", &[])
        .await
        .assert_ok();
    let reopened = t.reload(&day7).await;
    assert_eq!(reopened.status, TicketStatus::InProgress);
    assert_eq!(reopened.resolved_at, None);
    // The resolution clock went on while it was resolved.
    assert!(reopened.resolution_due > day7.resolution_due);
    assert!(reopened.sla_paused_minutes > 0);
}

/// `cancel` needs a reason; it is kept; only open, assigned and held tickets
/// are cancelled.
#[renox::test]
async fn cancel_needs_a_reason() {
    let t = boot().await;
    let cast = cast(&t).await;

    let ticket = assigned(&t, &cast, "general").await;
    for reason in ["", "   "] {
        t.run(
            &cast.coordinator,
            &ticket,
            "cancel",
            "2026-11-03 10:00",
            &[("reason", reason)],
        )
        .await
        .assert_status(422);
    }
    t.run(
        &cast.imran,
        &ticket,
        "cancel",
        "2026-11-03 10:00",
        &[("reason", "x")],
    )
    .await
    .assert_forbidden();
    assert_eq!(t.reload(&ticket).await.status, TicketStatus::Assigned);
    t.run(
        &cast.coordinator,
        &ticket,
        "cancel",
        "2026-11-03 10:00",
        &[("reason", "Duplicate of TKT-2026-0001")],
    )
    .await
    .assert_ok();
    let cancelled = t.reload(&ticket).await;
    assert_eq!(cancelled.status, TicketStatus::Cancelled);
    assert_eq!(
        cancelled.cancel_reason.as_deref(),
        Some("Duplicate of TKT-2026-0001")
    );

    // Work in progress is not cancelled.
    let working = started(&t, &cast, "general").await;
    t.run(
        &cast.coordinator,
        &working,
        "cancel",
        "2026-11-03 11:00",
        &[("reason", "Not needed")],
    )
    .await
    .assert_status(422);
    assert_eq!(t.reload(&working).await.status, TicketStatus::InProgress);
    assert_eq!(
        t.reload(&working).await.assignee(),
        Some(Assignee::User { id: cast.imran.id })
    );
}

/// Every state and transition of §4.1 exists.
#[test]
fn every_state_and_transition_of_the_diagram_exists() {
    let machine = Ticket::machine();
    let states: Vec<&str> = machine.states().iter().map(|s| s.name.as_str()).collect();
    for state in [
        "open",
        "assigned",
        "in_progress",
        "on_hold",
        "resolved",
        "closed",
        "cancelled",
    ] {
        assert!(states.contains(&state), "state {state} in {states:?}");
    }
    for name in [
        "assign", "start", "hold", "resume", "resolve", "confirm", "close", "reopen", "cancel",
    ] {
        assert!(machine.find(name).is_ok(), "transition {name}");
    }
    // The system's own: resume and cancel.
    for name in ["auto_resume", "auto_cancel"] {
        assert!(machine.find(name).is_ok(), "transition {name}");
    }
}
