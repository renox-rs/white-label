//! A deadline task whose period stays but whose due date moves keeps its id
//! (04 `ACC.RUL.10`, BL-ACC-15): the system gives an `upcoming` task the new
//! due date, the start date follows the lead time, and the move is audited.

mod support;

use renox::prelude::Model;
use wl_kernel::Subject;
use wl_kernel::deadlines::{
    DeadlineTaskId, DeadlineTaskStatus, NewDeadlineTask, TaskRecurrence, deadline_tasks,
};
use wl_ticketing::{DeadlineTask, WlTicketing};

use support::{Booted, CLIENT, COOLAIR, boot_app, day};

/// A quarterly VAT task of E-12 due on `due`, 21 days of lead time.
async fn task(t: &Booted, due: &str) -> DeadlineTaskId {
    deadline_tasks(t.state())
        .create(
            t.state(),
            NewDeadlineTask::new(
                Subject::new("accounting.engagement", 12),
                "vat_return",
                "VAT Q2",
                day(due),
            )
            .party(COOLAIR)
            .lead_days(21)
            .recurrence(TaskRecurrence::Quarterly),
        )
        .await
        .unwrap()
        .expect("ticketing is on")
}

/// The due date and the start date move, the id and the state stay, and the
/// audit trail names both dates and the reason; an unchanged date, a blank
/// reason and a started task move nothing.
#[renox::test]
async fn an_upcoming_task_gets_the_new_due_date_and_keeps_its_id() {
    let t = boot_app(CLIENT, WlTicketing::new(), |app| app).await;
    let id = task(&t, "2027-05-28").await;
    let tasks = deadline_tasks(t.state());

    assert!(
        tasks
            .reschedule(t.state(), id, day("2027-06-28"), "tax profile changed")
            .await
            .unwrap()
    );
    let moved = t.task(id.0).await;
    assert_eq!(moved.status, DeadlineTaskStatus::Upcoming);
    assert_eq!(moved.due_date, day("2027-06-28"));
    assert_eq!(moved.start_date, day("2027-06-07"));
    let audit = renox::audit::for_subject(t.db(), DeadlineTask::TABLE, id.0, 10)
        .await
        .unwrap();
    let entry = audit
        .iter()
        .find(|entry| entry.action == "task.rescheduled")
        .expect("audited");
    let data = entry.data.to_string();
    assert!(data.contains("2027-05-28") && data.contains("2027-06-28"));
    assert!(data.contains("tax profile changed"));

    assert!(
        !tasks
            .reschedule(t.state(), id, day("2027-06-28"), "again")
            .await
            .unwrap(),
        "already due that day"
    );
    assert!(
        tasks
            .reschedule(t.state(), id, day("2027-07-28"), "  ")
            .await
            .is_err(),
        "a reason is required"
    );
    renox::db::sql("UPDATE wl_ticketing_deadline_tasks SET status = 'in_progress' WHERE id = ?")
        .bind(id.0)
        .execute(t.db())
        .await
        .unwrap();
    assert!(
        !tasks
            .reschedule(t.state(), id, day("2027-07-28"), "late")
            .await
            .unwrap(),
        "a started task keeps its due date"
    );
    assert_eq!(t.task(id.0).await.due_date, day("2027-06-28"));
}
