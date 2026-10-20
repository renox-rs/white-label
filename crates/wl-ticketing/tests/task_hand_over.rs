//! A deadline task the firm no longer files is handed over to the client
//! (04 return `cancel`, BL-ACC-17): the system completes it from any open
//! state with the evidence "handed over to client", which is also its closed
//! reason; no completion is reported and the next occurrence follows unless
//! the source is ending.

mod support;

use wl_kernel::Subject;
use wl_kernel::deadlines::{
    DeadlineTaskId, DeadlineTaskStatus, HANDED_OVER_EVIDENCE, HandOverTask, NewDeadlineTask,
    TaskRecurrence, deadline_tasks,
};
use wl_ticketing::WlTicketing;

use support::{Booted, CLIENT, COOLAIR, boot_app, day};

/// Engagement E-12.
fn e12() -> Subject {
    Subject::new("accounting.engagement", 12)
}

/// A quarterly VAT task of E-12 due on `due`.
async fn task(t: &Booted, due: &str) -> DeadlineTaskId {
    deadline_tasks(t.state())
        .create(
            t.state(),
            NewDeadlineTask::new(e12(), "vat_return", "VAT Q4", day(due))
                .party(COOLAIR)
                .recurrence(TaskRecurrence::Quarterly),
        )
        .await
        .unwrap()
        .expect("ticketing is on")
}

/// An `upcoming` task is completed with the evidence and the next
/// occurrence is generated; handing over again does nothing.
#[renox::test]
async fn a_handed_over_task_is_completed_with_the_evidence() {
    let t = boot_app(CLIENT, WlTicketing::new(), |app| app).await;
    let id = task(&t, "2027-01-28").await;
    let tasks = deadline_tasks(t.state());

    assert!(
        tasks
            .hand_over(t.state(), id, HandOverTask::new())
            .await
            .unwrap()
    );
    let done = t.task(id.0).await;
    assert_eq!(done.status, DeadlineTaskStatus::Completed);
    assert_eq!(done.evidence.as_deref(), Some(HANDED_OVER_EVIDENCE));
    assert_eq!(done.closed_reason.as_deref(), Some("handed over to client"));
    assert!(done.completed_at.is_some());
    assert!(done.next_task_id.is_some(), "the next quarter follows");
    assert!(
        !tasks
            .hand_over(t.state(), id, HandOverTask::new())
            .await
            .unwrap(),
        "a closed task is not handed over again"
    );
}

/// With `without_next` (the agreement is ending) no next occurrence follows.
#[renox::test]
async fn a_hand_over_can_leave_no_next_occurrence() {
    let t = boot_app(CLIENT, WlTicketing::new(), |app| app).await;
    let id = task(&t, "2027-01-28").await;
    deadline_tasks(t.state())
        .hand_over(t.state(), id, HandOverTask::new().without_next())
        .await
        .unwrap();
    assert_eq!(t.task(id.0).await.next_task_id, None);
}
