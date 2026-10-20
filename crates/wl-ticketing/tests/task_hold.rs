//! BL-TKT-10 (#155): the tasks of a paused agreement are held. They keep
//! their state and due date, the client is not reminded, `mark_overdue`
//! still runs, tasks generated meanwhile are created held, and on release
//! the manager gets the list of what went overdue while held.

mod support;

use wl_kernel::deadlines::{
    DeadlineTaskId, DeadlineTaskStatus, NewDeadlineTask, TaskOverdue, TaskRecurrence,
    deadline_tasks,
};
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::{PartyId, Subject};
use wl_ticketing::{WlTicketing, deadlines, permissions as perm};

use support::{CLIENT, COOLAIR, Recorded, SAFEFIRE, boot_app, day, record};

/// Engagement E-12 (suspended in the scenario).
fn e12() -> Subject {
    Subject::new("accounting.engagement", 12)
}

/// Engagement E-13 (running), for comparison.
fn e13() -> Subject {
    Subject::new("accounting.engagement", 13)
}

/// "VAT Q3" of `source` due 2026-11-28, starting on 2026-11-05, with an open
/// document request (it needs the client's input).
fn vat_q3(source: Subject, party: PartyId, owner: i64) -> NewDeadlineTask {
    NewDeadlineTask::new(source, "vat_return", "VAT Q3", day("2026-11-28"))
        .lead_days(23)
        .party(party)
        .owner_user(owner)
        .needs_client_input()
}

/// Done when 1–3: E-12 suspended on 2026-11-01 with "VAT Q3" due
/// 2026-11-28 and an open document request: the task is held, its due date
/// stays, and no client reminder is sent on 2026-11-05 (the owner still is
/// reminded; E-13's client is). On 2026-11-29 it becomes `overdue` and stays
/// held. When E-12 resumes on 2026-12-03 the flag clears and the manager
/// gets "VAT Q3 (overdue since 2026-11-29)", once.
#[renox::test]
async fn a_held_task_keeps_its_date_goes_overdue_and_is_listed_on_release() {
    let overdue = Recorded::<TaskOverdue>::new();
    let t = boot_app(
        CLIENT,
        WlTicketing::new().task_manager_role("manager"),
        |app| record(app, &overdue),
    )
    .await;
    let accountant = t
        .user("Aisha", &[perm::TASKS_VIEW, perm::TASKS_WORK])
        .await;
    let manager = t
        .user_in_role("Mona", "manager", &[perm::TASKS_VIEW, perm::TASKS_MANAGE])
        .await;
    let tasks = deadline_tasks(t.state());
    let held_id = tasks
        .create(t.state(), vat_q3(e12(), COOLAIR, accountant.id))
        .await
        .unwrap()
        .expect("ticketing is on");
    let running_id = tasks
        .create(t.state(), vat_q3(e13(), SAFEFIRE, accountant.id))
        .await
        .unwrap()
        .unwrap();

    // 2026-11-01: E-12 is suspended.
    let held = tasks.hold(t.state(), &e12(), day("2026-11-01")).await.unwrap();
    assert_eq!(held.len(), 1);
    assert!(held[0].held);
    let task = t.task(held_id.0).await;
    assert!(task.held);
    assert_eq!(task.status, DeadlineTaskStatus::Upcoming);
    assert_eq!(task.due_date, day("2026-11-28"));
    assert!(!t.task(running_id.0).await.held);

    // 2026-11-05: the start date of both; only E-13's client is reminded.
    let report = deadlines::remind(t.state(), day("2026-11-05"))
        .await
        .unwrap();
    assert_eq!(report.client, 1);
    assert_eq!(report.held, 1);
    assert_eq!(report.staff, 2, "the owner is reminded of both");
    let to_clients = t.notices.recipients(deadlines::CLIENT_REMINDER_NOTICE);
    assert_eq!(to_clients, [NotifyRecipient::Party { id: SAFEFIRE }]);
    assert!(
        !t.notices
            .all()
            .iter()
            .flat_map(|r| r.recipients.clone())
            .any(|r| r == NotifyRecipient::Party { id: COOLAIR }),
        "nothing goes to the client of the held task"
    );
    let task = t.task(held_id.0).await;
    assert_eq!(task.due_date, day("2026-11-28"));
    assert_eq!(task.status, DeadlineTaskStatus::Upcoming);

    // 2026-11-29: `mark_overdue` still runs; the task stays held.
    deadlines::mark_overdue(t.state(), day("2026-11-29"))
        .await
        .unwrap();
    let task = t.task(held_id.0).await;
    assert_eq!(task.status, DeadlineTaskStatus::Overdue);
    assert!(task.held);
    assert_eq!(task.overdue_on, Some(day("2026-11-29")));
    assert!(
        overdue
            .all()
            .iter()
            .any(|e| e.task == held_id && e.held),
        "TaskOverdue for the held task"
    );
    assert!(
        t.notices
            .recipients(deadlines::OVERDUE_NOTICE)
            .contains(&NotifyRecipient::User { id: manager.id }),
        "staff are still told"
    );

    // 2026-12-03: E-12 resumes.
    let released = tasks
        .release(t.state(), &e12(), day("2026-12-03"))
        .await
        .unwrap();
    assert_eq!(released.released, [held_id]);
    assert_eq!(released.overdue.len(), 1);
    assert!(!t.task(held_id.0).await.held);
    let listing = released.manager_task.expect("the manager's list");
    let ticket = t.ticket(listing.id.0).await;
    assert_eq!(ticket.assignee_user_id, Some(manager.id));
    assert!(
        ticket
            .description
            .contains("VAT Q3 (overdue since 2026-11-29)"),
        "{}",
        ticket.description
    );
    let again = tasks
        .release(t.state(), &e12(), day("2026-12-04"))
        .await
        .unwrap();
    assert!(again.manager_task.is_none(), "the list is sent once");
    assert!(again.released.is_empty());
}

/// Done when 4: a task generated on 2026-11-15 while E-12 is held is
/// created held, whether the vertical creates it or it is the next
/// occurrence of a completed one. Holding never changes state or due date.
#[renox::test]
async fn a_task_generated_while_held_is_created_held() {
    let t = boot_app(CLIENT, WlTicketing::new(), |app| app).await;
    let accountant = t
        .user("Aisha", &[perm::TASKS_VIEW, perm::TASKS_WORK])
        .await;
    let tasks = deadline_tasks(t.state());
    let monthly: DeadlineTaskId = tasks
        .create(
            t.state(),
            NewDeadlineTask::new(e12(), "payroll", "Payroll October", day("2026-11-20"))
                .owner_user(accountant.id)
                .recurrence(TaskRecurrence::Monthly),
        )
        .await
        .unwrap()
        .unwrap();
    t.task_act(&accountant, monthly.0, "start", "2026-11-02 09:00", &[])
        .await
        .assert_ok();
    tasks.hold(t.state(), &e12(), day("2026-11-01")).await.unwrap();
    let task = t.task(monthly.0).await;
    assert!(task.held);
    assert_eq!(task.status, DeadlineTaskStatus::InProgress, "state kept");
    assert_eq!(task.due_date, day("2026-11-20"), "due date kept");

    // 2026-11-15: a new task of E-12, and the next occurrence of the one
    // completed that day.
    let generated = tasks
        .create(
            t.state(),
            NewDeadlineTask::new(e12(), "ct_registration", "CT registration", day("2026-12-31")),
        )
        .await
        .unwrap()
        .unwrap();
    assert!(t.task(generated.0).await.held);
    t.task_act(&accountant, monthly.0, "complete", "2026-11-15 09:00", &[])
        .await
        .assert_ok();
    let next = t
        .task(t.task(monthly.0).await.next_task_id.expect("the next month"))
        .await;
    assert!(next.held);
    assert_eq!(next.due_date, day("2026-12-20"));

    // A running engagement's tasks are not held.
    let other = tasks
        .create(
            t.state(),
            NewDeadlineTask::new(e13(), "ct_registration", "CT registration", day("2026-12-31")),
        )
        .await
        .unwrap()
        .unwrap();
    assert!(!t.task(other.0).await.held);
}
