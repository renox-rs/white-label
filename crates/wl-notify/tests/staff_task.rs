//! The `staff_task` give-up reaction (wl-notify §5.4): the acceptance checks
//! AC1 to AC6 of #56. A recording task creator stands for wl-ticketing, and
//! Tower A, Tower B and Sara M. are the fixtures of the plan.
//!
//! The date checks run `open_give_up_task` on the real `MessageGaveUp` of a
//! real give-up with the give-up time set, so they do not depend on today's
//! date.

mod support;

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use wl_kernel::notify::{GiveUpReaction, MessageGaveUp, Urgency};
use wl_kernel::tickets::{Assignee, TASK_CATEGORY, TicketId};
use wl_kernel::{BusinessCalendar, Subject};
use wl_notify::staff_task::{NO_TASK_CREATOR, TASK_COMMENTED, TASK_CREATED};
use wl_notify::{GiveUpContext, ReactionRun, RunStatus, StaffTask, open_give_up_task};

use support::{
    BLUE_DUNES, Booted, COORDINATOR, SARA_EMAIL, SARA_MOBILE, SECRET_BODY, SECRET_SUBJECT,
    TICKET_DIGEST, TICKET_EMERGENCY, TICKET_UPDATE, TOWER_A_OTHER_TICKET, TOWER_A_TICKET, boot,
    boot_staff, boot_without_tickets, tower_a, tower_b,
};

const ALERT: &str = "notify-give-up-alert";

/// Asia/Dubai `date` (`2026-11-05`) at `hour:minute`, as an instant.
fn dubai(date: &str, hour: u32, minute: u32) -> DateTime {
    let local = date
        .parse::<NaiveDate>()
        .expect("a date")
        .and_hms_opt(hour, minute, 0)
        .expect("a time");
    BusinessCalendar::default().from_local(local)
}

/// The context of `event` as the rule hands it to a handler, about
/// `ticket`, given up at `at`.
fn context(event: &MessageGaveUp, ticket: i64, at: DateTime, urgency: Urgency) -> GiveUpContext {
    GiveUpContext {
        run_id: 1,
        message: event.message,
        type_key: event.type_key.clone(),
        subject: Subject::new("ticketing.ticket", ticket),
        subject_label: event.subject_label.clone(),
        recipient: event.recipient.clone(),
        gave_up_at: at,
        contact_point: event.contact_point,
        channels_tried: event.channels_tried.clone(),
        statutory: event.statutory,
        staff_role: Some(COORDINATOR.to_owned()),
        urgency,
        reactions: vec![GiveUpReaction::StaffTask],
    }
}

fn coordinator_task() -> StaffTask {
    StaffTask {
        role: Some(COORDINATOR.to_owned()),
    }
}

/// The local time of a task's due time, as `2026-11-06 10:00`.
fn shown(t: &Booted, at: Option<DateTime>) -> String {
    wl_kernel::calendar::calendar(t.app.state())
        .local(at.expect("a due time"))
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

/// AC1: a give-up on Thursday 10:00 about ticket 42 in Tower A, to Sara M.
/// after e-mail and SMS failed, opens one task for Tower A's coordinator,
/// due Friday 10:00, that lists both channels and carries nothing of the
/// message.
#[renox::test]
async fn a_give_up_opens_a_task_for_the_towers_coordinator() {
    let t = boot().await;
    // Tower B's coordinator has the lower id and must not get it.
    let _bilal = t.holder("Bilal", COORDINATOR, Some(&tower_b())).await;
    let aisha = t.holder("Aisha", COORDINATOR, Some(&tower_a())).await;
    let event = t
        .give_up_on_sara(TICKET_UPDATE, TOWER_A_TICKET, "TKT-2026-0042")
        .await;

    let ctx = context(
        &event,
        TOWER_A_TICKET,
        dubai("2026-11-05", 10, 0),
        Urgency::Normal,
    );
    let step = open_give_up_task(t.app.state(), &ctx, coordinator_task())
        .await
        .unwrap();
    assert_eq!(step.status, TASK_CREATED);
    assert_eq!(step.detail["ticket_number"], "TKT-2026-0001");
    assert_eq!(step.detail["action"], "opened");
    assert_eq!(step.detail["assignee"]["id"], aisha.id);
    assert_eq!(step.detail["alert_sent"], false);

    let opened = t.tasks.opened();
    assert_eq!(opened.len(), 1, "one task");
    let task = &opened[0];
    assert_eq!(task.title, "Could not reach Sara M. about TKT-2026-0042");
    assert_eq!(task.category_key, TASK_CATEGORY);
    assert_eq!(task.subject, Subject::new("ticketing.ticket", 42));
    assert_eq!(task.assignee, Some(Assignee::user(aisha.id)));
    assert_eq!(shown(&t, task.due_at), "2026-11-06 10:00");
    assert_eq!(
        task.source_key.as_deref(),
        Some("notify.give_up:ticketing.ticket:42:party:5")
    );
    assert_eq!(
        task.description,
        "Recipient: Sara M.\nNotification type: ticket.update\nChannels tried:\n\
         - Email s***@example.com: 550 mailbox unknown\n- SMS ****4567: 504 timeout"
    );
    for text in [&task.title, &task.description] {
        for secret in [SECRET_SUBJECT, SECRET_BODY, SARA_EMAIL, SARA_MOBILE] {
            assert!(!text.contains(secret), "`{secret}` leaked into: {text}");
        }
    }
    // A normal type sends no alert.
    assert!(t.notices(&aisha, ALERT).await.is_empty());
}

/// AC2: the due time is the next working day at the same clock time:
/// Friday 16:00 is due Monday 16:00; the day before a holiday of
/// `client.toml` (Wednesday 2 December) is due the day after it.
#[renox::test]
async fn the_due_time_skips_the_weekend_and_the_holidays() {
    let t = boot().await;
    t.holder("Aisha", COORDINATOR, Some(&tower_a())).await;
    let event = t
        .give_up_on_sara(TICKET_UPDATE, TOWER_A_TICKET, "TKT-2026-0042")
        .await;

    // Each case about its own ticket, so none comments on another's task.
    let cases = [
        ("2026-11-05", 10, "2026-11-06 10:00"),
        ("2026-11-06", 16, "2026-11-09 16:00"),
        ("2026-11-07", 10, "2026-11-09 10:00"),
        ("2026-12-01", 10, "2026-12-03 10:00"),
    ];
    for (n, (date, hour, due)) in cases.into_iter().enumerate() {
        let ticket = 200 + n as i64;
        let ctx = context(&event, ticket, dubai(date, hour, 0), Urgency::Normal);
        let step = open_give_up_task(t.app.state(), &ctx, coordinator_task())
            .await
            .unwrap();
        assert_eq!(step.status, TASK_CREATED);
        assert_eq!(
            shown(&t, t.tasks.opened()[n].due_at),
            due,
            "given up {date} {hour}:00"
        );
    }
}

/// AC3: a give-up of an urgent type gives the coordinator the task and one
/// database notification that links to the ticket; the same give-up of a
/// normal type gives the task and none.
#[renox::test]
async fn an_urgent_type_also_alerts_the_coordinator() {
    let t = boot_staff().await;
    let aisha = t.holder("Aisha", COORDINATOR, Some(&tower_a())).await;

    t.give_up_on_sara(TICKET_UPDATE, TOWER_A_OTHER_TICKET, "TKT-2026-0044")
        .await;
    assert_eq!(t.tasks.opened().len(), 1);
    assert!(t.notices(&aisha, ALERT).await.is_empty());

    let event = t
        .give_up_on_sara(TICKET_EMERGENCY, TOWER_A_TICKET, "TKT-2026-0042")
        .await;
    let opened = t.tasks.opened();
    assert_eq!(opened.len(), 2);
    assert_eq!(opened[1].assignee, Some(Assignee::user(aisha.id)));
    let notices = t.notices(&aisha, ALERT).await;
    assert_eq!(notices.len(), 1);
    assert!(
        notices[0].contains("Urgent: could not reach Sara M. about TKT-2026-0042"),
        "{}",
        notices[0]
    );
    assert!(notices[0].contains("/tickets/2"), "{}", notices[0]);
    assert!(!notices[0].contains(SECRET_BODY));

    // The step of the rule's run records the ticket and the alert.
    let runs = ReactionRun::of_message(t.db(), event.message.0)
        .await
        .unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, RunStatus::Succeeded);
    let steps = runs[0].steps(t.db()).await.unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].status, TASK_CREATED);
    assert_eq!(steps[0].detail.0["ticket_number"], "TKT-2026-0002");
    assert_eq!(steps[0].detail.0["alert_sent"], true);
}

/// AC4: a second give-up for the same subject and recipient while the task
/// is open comments on it (and, for an urgent type, alerts again); once the
/// task is closed a new task opens; another recipient has its own task.
#[renox::test]
async fn a_later_give_up_comments_on_the_open_task() {
    let t = boot_staff().await;
    let aisha = t.holder("Aisha", COORDINATOR, Some(&tower_a())).await;

    t.give_up_on_sara(TICKET_EMERGENCY, TOWER_A_TICKET, "TKT-2026-0042")
        .await;
    assert_eq!(t.tasks.opened().len(), 1);
    assert!(t.tasks.comments().is_empty());
    assert_eq!(t.notices(&aisha, ALERT).await.len(), 1);

    let event = t
        .give_up_on_sara(TICKET_EMERGENCY, TOWER_A_TICKET, "TKT-2026-0042")
        .await;
    assert_eq!(t.tasks.opened().len(), 1, "no second task");
    let comments = t.tasks.comments();
    assert_eq!(comments.len(), 1, "one comment");
    assert_eq!(comments[0].0, TicketId(1));
    assert!(comments[0].1.internal);
    assert!(
        comments[0]
            .1
            .text
            .contains("- Email s***@example.com: 550 mailbox unknown"),
        "{}",
        comments[0].1.text
    );
    assert!(comments[0].1.text.contains("Gave up at "));
    assert!(!comments[0].1.text.contains(SECRET_BODY));
    // Every urgent give-up alerts, also when it only became a comment.
    assert_eq!(t.notices(&aisha, ALERT).await.len(), 2);
    let runs = ReactionRun::of_message(t.db(), event.message.0)
        .await
        .unwrap();
    let steps = runs[0].steps(t.db()).await.unwrap();
    assert_eq!(steps[0].status, TASK_COMMENTED);
    assert_eq!(steps[0].detail.0["action"], "commented");
    assert_eq!(steps[0].detail.0["ticket_number"], "TKT-2026-0001");

    // Another recipient about the same subject: its own task.
    t.give_up_on(
        BLUE_DUNES,
        TICKET_EMERGENCY,
        TOWER_A_TICKET,
        "TKT-2026-0042",
    )
    .await;
    let opened = t.tasks.opened();
    assert_eq!(opened.len(), 2);
    assert_ne!(opened[0].source_key, opened[1].source_key);
    assert_eq!(t.tasks.comments().len(), 1);

    // After the first task is closed, Sara's next give-up opens a new one.
    t.tasks.close(TicketId(1));
    t.give_up_on_sara(TICKET_EMERGENCY, TOWER_A_TICKET, "TKT-2026-0042")
        .await;
    let opened = t.tasks.opened();
    assert_eq!(opened.len(), 3);
    assert_eq!(opened[2].source_key, opened[0].source_key);
    assert_eq!(t.tasks.comments().len(), 1);
}

/// AC5: with no coordinator in Tower A (only one in Tower B) the task goes
/// to the company admin; a type with no staff role goes there too; with
/// nobody holding either role it is opened unassigned.
#[renox::test]
async fn without_a_holder_the_task_goes_to_the_company_admin() {
    let t = boot_staff().await;
    let _bilal = t.holder("Bilal", COORDINATOR, Some(&tower_b())).await;

    // Nobody yet: unassigned, and the step says so.
    let event = t
        .give_up_on_sara(TICKET_UPDATE, TOWER_A_TICKET, "TKT-2026-0042")
        .await;
    assert_eq!(t.tasks.opened()[0].assignee, None);
    let runs = ReactionRun::of_message(t.db(), event.message.0)
        .await
        .unwrap();
    let steps = runs[0].steps(t.db()).await.unwrap();
    assert_eq!(steps[0].status, TASK_CREATED);
    assert_eq!(steps[0].detail.0["no_holder"], true);

    let dana = t.holder("Dana", wl_kernel::COMPANY_ADMIN_ROLE, None).await;
    t.tasks.close(TicketId(1));
    t.give_up_on_sara(TICKET_UPDATE, TOWER_A_TICKET, "TKT-2026-0042")
        .await;
    let opened = t.tasks.opened();
    assert_eq!(opened[1].assignee, Some(Assignee::user(dana.id)));

    // A type with no staff role, although Tower A has a coordinator now.
    let _aisha = t.holder("Aisha", COORDINATOR, Some(&tower_a())).await;
    t.give_up_on_sara(TICKET_DIGEST, TOWER_A_OTHER_TICKET, "TKT-2026-0044")
        .await;
    let opened = t.tasks.opened();
    assert_eq!(opened.len(), 3);
    assert_eq!(opened[2].assignee, Some(Assignee::user(dana.id)));
}

/// AC6 (module side): with ticketing off (`NoTickets`) the step is
/// recorded as `no_task_creator` and the run still succeeds.
#[renox::test]
async fn without_ticketing_the_step_says_so_and_the_run_succeeds() {
    let t = boot_without_tickets().await;
    t.holder("Aisha", COORDINATOR, Some(&tower_a())).await;
    let event = t
        .give_up_on_sara(TICKET_EMERGENCY, TOWER_A_TICKET, "TKT-2026-0042")
        .await;

    assert!(t.tasks.opened().is_empty());
    let runs = ReactionRun::of_message(t.db(), event.message.0)
        .await
        .unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, RunStatus::Succeeded);
    let steps = runs[0].steps(t.db()).await.unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].status, NO_TASK_CREATOR);
    assert!(t.automation_failed.all().is_empty());
}
