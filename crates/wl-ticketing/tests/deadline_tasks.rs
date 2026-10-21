//! The deadline Task lifecycle (wl-ticketing §4.4), the base of #154 and
//! #155: tasks created through the kernel's `DeadlineTasks`, worked, sent
//! for review, completed with evidence, marked overdue by `WRK.RUL.05` and
//! closed as missed, each recurring one followed by its next occurrence.

mod support;

use wl_kernel::deadlines::{
    DeadlineTaskStatus, NewDeadlineTask, TaskCompleted, TaskOverdue, TaskRecurrence, deadline_tasks,
};
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::{PartyId, Subject};
use wl_ticketing::{WlTicketing, deadlines, permissions as perm};

use support::{CLIENT, COOLAIR, Recorded, boot_app, day, record};

/// Engagement E-12 of the accounting vertical.
fn e12() -> Subject {
    Subject::new("accounting.engagement", 12)
}

/// A quarterly VAT return of E-12 due on `due`, owned by `owner`.
fn vat(title: &str, due: &str, owner: i64) -> NewDeadlineTask {
    NewDeadlineTask::new(e12(), "vat_return", title, day(due))
        .lead_days(21)
        .party(COOLAIR)
        .owner_user(owner)
        .evidence_required()
        .recurrence(TaskRecurrence::Quarterly)
        .period_end(day("2026-09-30"))
}

/// A task is worked and completed only with its evidence; completing it
/// emits `TaskCompleted` (not late) and generates the next occurrence, one
/// quarter later and still `upcoming`.
#[renox::test]
async fn a_completed_task_needs_evidence_and_brings_the_next_occurrence() {
    let completed = Recorded::<TaskCompleted>::new();
    let t = boot_app(CLIENT, WlTicketing::new(), |app| record(app, &completed)).await;
    let accountant = t.user("Aisha", &[perm::TASKS_VIEW, perm::TASKS_WORK]).await;
    let tasks = deadline_tasks(t.state());

    let id = tasks
        .create(t.state(), vat("VAT Q3", "2026-11-28", accountant.id))
        .await
        .unwrap()
        .expect("ticketing is on");
    let task = t.task(id.0).await;
    assert_eq!(task.status, DeadlineTaskStatus::Upcoming);
    assert_eq!(task.start_date, day("2026-11-07"));
    assert!(!task.held);

    t.task_act(&accountant, id.0, "start", "2026-11-08 09:00", &[])
        .await
        .assert_ok();
    t.task_act(&accountant, id.0, "complete", "2026-11-20 09:00", &[])
        .await
        .assert_status(422);
    assert_eq!(t.task(id.0).await.status, DeadlineTaskStatus::InProgress);
    t.task_act(
        &accountant,
        id.0,
        "complete",
        "2026-11-20 09:00",
        &[("evidence", "FTA acknowledgement 2311")],
    )
    .await
    .assert_ok();

    let task = t.task(id.0).await;
    assert_eq!(task.status, DeadlineTaskStatus::Completed);
    assert_eq!(task.evidence.as_deref(), Some("FTA acknowledgement 2311"));
    assert!(task.started_at.is_some());
    let events = completed.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].task, id);
    assert!(!events[0].late);
    assert_eq!(events[0].source, e12());

    let next = t.task(task.next_task_id.expect("a next occurrence")).await;
    assert_eq!(next.status, DeadlineTaskStatus::Upcoming);
    assert_eq!(next.due_date, day("2027-02-28"));
    assert_eq!(next.period_end, Some(day("2026-12-30")));
    assert_eq!(next.previous_task_id, Some(task.id));
    assert_eq!(next.owner_user_id, Some(accountant.id));
    let all = tasks.of_subject(t.db(), &e12()).await.unwrap();
    assert_eq!(all.len(), 2, "one next occurrence, once");
}

/// `WRK.RUL.05`: the day after the due date the task is `overdue` (owner
/// and manager told, `TaskOverdue`), not before; it can still be started
/// and completed, then flagged late. Closing one as missed needs a reason.
#[renox::test]
async fn a_past_due_task_goes_overdue_and_can_still_be_completed_late() {
    let overdue = Recorded::<TaskOverdue>::new();
    let completed = Recorded::<TaskCompleted>::new();
    let t = boot_app(
        CLIENT,
        WlTicketing::new().task_manager_role("manager"),
        |app| record(record(app, &overdue), &completed),
    )
    .await;
    let accountant = t.user("Aisha", &[perm::TASKS_VIEW, perm::TASKS_WORK]).await;
    let manager = t
        .user_in_role(
            "Mona",
            "manager",
            &[perm::TASKS_VIEW, perm::TASKS_REVIEW, perm::TASKS_MANAGE],
        )
        .await;
    let tasks = deadline_tasks(t.state());
    let late = tasks
        .create(t.state(), vat("VAT Q3", "2026-11-28", accountant.id))
        .await
        .unwrap()
        .unwrap();
    let missed = tasks
        .create(
            t.state(),
            NewDeadlineTask::new(e12(), "ubo_update", "UBO update", day("2026-11-28"))
                .owner_user(accountant.id),
        )
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        deadlines::mark_overdue(t.state(), day("2026-11-28"))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        deadlines::mark_overdue(t.state(), day("2026-11-29"))
            .await
            .unwrap(),
        2
    );
    let task = t.task(late.0).await;
    assert_eq!(task.status, DeadlineTaskStatus::Overdue);
    assert!(task.late);
    assert_eq!(task.overdue_on, Some(day("2026-11-29")));
    assert_eq!(task.stage.as_deref(), Some("upcoming"));
    assert_eq!(overdue.all().len(), 2);
    let told = t.notices.recipients(deadlines::OVERDUE_NOTICE);
    assert!(told.contains(&NotifyRecipient::User { id: accountant.id }));
    assert!(told.contains(&NotifyRecipient::User { id: manager.id }));

    t.task_act(&accountant, late.0, "start", "2026-11-30 09:00", &[])
        .await
        .assert_ok();
    t.task_act(
        &accountant,
        late.0,
        "complete",
        "2026-11-30 10:00",
        &[("evidence", "FTA acknowledgement 2399")],
    )
    .await
    .assert_ok();
    assert_eq!(t.task(late.0).await.status, DeadlineTaskStatus::Completed);
    assert!(completed.all()[0].late, "completed after the due date");

    t.task_act(&manager, missed.0, "close_missed", "2026-12-01 09:00", &[])
        .await
        .assert_status(422);
    t.task_act(
        &manager,
        missed.0,
        "close_missed",
        "2026-12-01 09:00",
        &[("reason", "Client never sent the register")],
    )
    .await
    .assert_ok();
    let task = t.task(missed.0).await;
    assert_eq!(task.status, DeadlineTaskStatus::Missed);
    assert_eq!(
        task.closed_reason.as_deref(),
        Some("Client never sent the register")
    );
    assert_eq!(task.next_task_id, None, "a one-off task does not recur");
}

/// A task with a reviewer is completed by the reviewer after
/// `submit_review`, not by the worker; `send_back` needs a comment.
#[renox::test]
async fn a_reviewed_task_is_completed_by_its_reviewer() {
    let t = boot_app(CLIENT, WlTicketing::new(), |app| app).await;
    let accountant = t.user("Aisha", &[perm::TASKS_VIEW, perm::TASKS_WORK]).await;
    let reviewer = t
        .user_in_role("Rami", "manager", &[perm::TASKS_VIEW, perm::TASKS_REVIEW])
        .await;
    let id = deadline_tasks(t.state())
        .create(
            t.state(),
            NewDeadlineTask::new(e12(), "ct_return", "CT return", day("2027-09-30"))
                .party(PartyId(5))
                .owner_user(accountant.id)
                .reviewer_role("manager"),
        )
        .await
        .unwrap()
        .unwrap();

    t.task_act(&accountant, id.0, "start", "2026-11-03 09:00", &[])
        .await
        .assert_ok();
    t.task_act(&accountant, id.0, "complete", "2026-11-03 10:00", &[])
        .await
        .assert_status(422);
    t.task_act(&accountant, id.0, "submit_review", "2026-11-03 10:00", &[])
        .await
        .assert_ok();
    assert!(
        t.notices
            .recipients(deadlines::REVIEW_NOTICE)
            .contains(&NotifyRecipient::User { id: reviewer.id })
    );
    t.task_act(&reviewer, id.0, "send_back", "2026-11-04 09:00", &[])
        .await
        .assert_status(422);
    t.task_act(
        &reviewer,
        id.0,
        "send_back",
        "2026-11-04 09:00",
        &[("reason", "Check the loss carried forward")],
    )
    .await
    .assert_ok();
    t.task_act(&accountant, id.0, "submit_review", "2026-11-05 09:00", &[])
        .await
        .assert_ok();
    t.task_act(&reviewer, id.0, "complete", "2026-11-05 10:00", &[])
        .await
        .assert_ok();
    assert_eq!(t.task(id.0).await.status, DeadlineTaskStatus::Completed);
}
