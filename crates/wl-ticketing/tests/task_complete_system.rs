//! A deadline task completed by the system when the work it tracks was done
//! elsewhere (04 return `file`, BL-ACC-19): any open task completes with the
//! evidence it is given, a task that went overdue stays *late*, and the next
//! occurrence follows as for a completion by a user.

mod support;

use wl_kernel::Subject;
use wl_kernel::deadlines::{
    DeadlineTaskId, DeadlineTaskStatus, NewDeadlineTask, TaskRecurrence, deadline_tasks,
};
use wl_ticketing::WlTicketing;

use support::{Booted, CLIENT, COOLAIR, boot_app, day};

/// A quarterly VAT task of E-12 due on `due` that needs evidence.
async fn task(t: &Booted, due: &str) -> DeadlineTaskId {
    deadline_tasks(t.state())
        .create(
            t.state(),
            NewDeadlineTask::new(
                Subject::new("accounting.engagement", 12),
                "vat_return",
                "VAT Q3",
                day(due),
            )
            .party(COOLAIR)
            .evidence_required()
            .reviewer_role("engagement_manager")
            .recurrence(TaskRecurrence::Quarterly),
        )
        .await
        .unwrap()
        .expect("ticketing is on")
}

/// An `upcoming` task completes with the evidence, without a reviewer, and the
/// next quarter follows; completing again does nothing.
#[renox::test]
async fn an_open_task_is_completed_with_the_evidence() {
    let t = boot_app(CLIENT, WlTicketing::new(), |app| app).await;
    let id = task(&t, "2027-01-28").await;
    let tasks = deadline_tasks(t.state());

    assert!(tasks.complete(t.state(), id, "FTA-ACK-1188").await.unwrap());
    let done = t.task(id.0).await;
    assert_eq!(done.status, DeadlineTaskStatus::Completed);
    assert_eq!(done.evidence.as_deref(), Some("FTA-ACK-1188"));
    assert!(done.completed_at.is_some());
    assert!(!done.late);
    assert!(done.next_task_id.is_some(), "the next quarter follows");
    assert!(!tasks.complete(t.state(), id, "again").await.unwrap());
}

/// A task that went overdue stays *late* when it is completed.
#[renox::test]
async fn an_overdue_task_completes_late() {
    let t = boot_app(CLIENT, WlTicketing::new(), |app| app).await;
    let id = task(&t, "2027-01-28").await;
    wl_ticketing::deadlines::mark_overdue(t.state(), day("2027-02-02"))
        .await
        .unwrap();
    assert_eq!(t.task(id.0).await.status, DeadlineTaskStatus::Overdue);

    assert!(
        deadline_tasks(t.state())
            .complete(t.state(), id, "FTA-ACK-9")
            .await
            .unwrap()
    );
    let done = t.task(id.0).await;
    assert_eq!(done.status, DeadlineTaskStatus::Completed);
    assert!(done.late);
}
