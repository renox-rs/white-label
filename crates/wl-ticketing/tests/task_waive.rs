//! BL-TKT-09 (#154): a task is waived when its source agreement ends or no
//! longer applies. The system waives the `upcoming` tasks of an ended
//! agreement with "source agreement ended" and no next occurrence; a started
//! task is waived too only when *waive a started task* is on (04: off, it is
//! listed for the manager); a `tasks.manage` waive needs a reason.

mod support;

use wl_kernel::Subject;
use wl_kernel::deadlines::{
    DeadlineTaskId, DeadlineTaskStatus, NewDeadlineTask, SOURCE_ENDED_REASON, TaskRecurrence,
    deadline_tasks,
};
use wl_kernel::notify::NotifyRecipient;
use wl_ticketing::{WlTicketing, deadlines, permissions as perm};

use support::{Booted, CLIENT, COOLAIR, boot_app, day};

/// Engagement E-12.
fn e12() -> Subject {
    Subject::new("accounting.engagement", 12)
}

/// The people of the scenario and the two tasks of E-12: "VAT Q4"
/// `upcoming` and "CT return" started (`in_progress`).
struct Scenario {
    t: Booted,
    accountant: renox::auth::User,
    manager: renox::auth::User,
    vat: DeadlineTaskId,
    ct: DeadlineTaskId,
}

async fn scenario(module: WlTicketing) -> Scenario {
    let t = boot_app(CLIENT, module.task_manager_role("manager"), |app| app).await;
    let accountant = t.user("Aisha", &[perm::TASKS_VIEW, perm::TASKS_WORK]).await;
    let manager = t
        .user_in_role(
            "Mona",
            "manager",
            &[perm::TASKS_VIEW, perm::TASKS_REVIEW, perm::TASKS_MANAGE],
        )
        .await;
    let tasks = deadline_tasks(t.state());
    let vat = tasks
        .create(
            t.state(),
            NewDeadlineTask::new(e12(), "vat_return", "VAT Q4", day("2027-01-28"))
                .lead_days(21)
                .party(COOLAIR)
                .owner_user(accountant.id)
                .recurrence(TaskRecurrence::Quarterly),
        )
        .await
        .unwrap()
        .expect("ticketing is on");
    let ct = tasks
        .create(
            t.state(),
            NewDeadlineTask::new(e12(), "ct_return", "CT return", day("2027-09-30"))
                .party(COOLAIR)
                .owner_user(accountant.id)
                .recurrence(TaskRecurrence::Yearly),
        )
        .await
        .unwrap()
        .expect("ticketing is on");
    t.task_act(&accountant, ct.0, "start", "2026-12-01 09:00", &[])
        .await
        .assert_ok();
    assert_eq!(t.task(ct.0).await.status, DeadlineTaskStatus::InProgress);
    Scenario {
        t,
        accountant,
        manager,
        vat,
        ct,
    }
}

/// Done when 1: E-12 ends; "VAT Q4" is `not_required` with reason "source
/// agreement ended" and no next occurrence; with the 04 setting off, "CT
/// return" stays `in_progress` and is listed for the manager on one task. A
/// `tasks.manage` waive after the end generates no next occurrence either.
#[renox::test]
async fn an_ended_agreement_waives_its_upcoming_tasks_and_lists_the_started_ones() {
    let s = scenario(WlTicketing::new().waive_started_tasks(false)).await;
    let t = &s.t;
    let tasks = deadline_tasks(t.state());

    let ended = tasks
        .end_source(t.state(), &e12(), day("2026-12-20"))
        .await
        .unwrap();
    assert_eq!(ended.waived, [s.vat]);
    assert_eq!(ended.listed, [s.ct]);

    let vat = t.task(s.vat.0).await;
    assert_eq!(vat.status, DeadlineTaskStatus::NotRequired);
    assert_eq!(vat.closed_reason.as_deref(), Some(SOURCE_ENDED_REASON));
    assert_eq!(vat.closed_reason.as_deref(), Some("source agreement ended"));
    assert_eq!(vat.next_task_id, None, "no next occurrence");

    let ct = t.task(s.ct.0).await;
    assert_eq!(ct.status, DeadlineTaskStatus::InProgress);
    let listing = ended.manager_task.expect("the manager's task");
    let ticket = t.ticket(listing.id.0).await;
    assert_eq!(ticket.assignee_user_id, Some(s.manager.id));
    assert!(
        ticket.description.contains("CT return (in_progress)"),
        "{}",
        ticket.description
    );
    assert!(
        t.notices.of_type(deadlines::WAIVED_NOTICE).is_empty(),
        "an upcoming task waived tells nobody"
    );

    // A task added after the end and waived by the manager: no next one.
    let late = tasks
        .create(
            t.state(),
            NewDeadlineTask::new(e12(), "vat_return", "VAT Q1", day("2027-04-28"))
                .recurrence(TaskRecurrence::Quarterly),
        )
        .await
        .unwrap()
        .unwrap();
    t.task_act(
        &s.manager,
        late.0,
        "waive",
        "2026-12-21 09:00",
        &[("reason", "Engagement ended")],
    )
    .await
    .assert_ok();
    assert_eq!(t.task(late.0).await.next_task_id, None);
    assert_eq!(
        tasks.of_subject(t.db(), &e12()).await.unwrap().len(),
        3,
        "no task was generated"
    );
    // Ending twice does nothing more.
    let again = tasks
        .end_source(t.state(), &e12(), day("2026-12-21"))
        .await
        .unwrap();
    assert!(again.waived.is_empty() && again.manager_task.is_none());
}

/// Done when 2: with the setting on (the module default), the same end also
/// waives "CT return", keeps its recorded work, and tells the assignee and
/// the counterpart; nothing is listed for the manager.
#[renox::test]
async fn with_the_setting_on_a_started_task_is_waived_too() {
    let s = scenario(WlTicketing::new()).await;
    let t = &s.t;
    let started_at = t.task(s.ct.0).await.started_at;
    assert!(started_at.is_some());

    let ended = deadline_tasks(t.state())
        .end_source(t.state(), &e12(), day("2026-12-20"))
        .await
        .unwrap();
    assert_eq!(ended.waived, [s.vat, s.ct]);
    assert!(ended.listed.is_empty());
    assert!(ended.manager_task.is_none());

    let ct = t.task(s.ct.0).await;
    assert_eq!(ct.status, DeadlineTaskStatus::NotRequired);
    assert_eq!(ct.closed_reason.as_deref(), Some("source agreement ended"));
    assert_eq!(ct.started_at, started_at, "the work recorded is kept");
    assert_eq!(ct.next_task_id, None);

    let told = t.notices.of_type(deadlines::WAIVED_NOTICE);
    assert_eq!(told.len(), 1, "only the started task is told");
    assert_eq!(told[0].subject, ct.subject());
    assert!(told[0].recipients.contains(&NotifyRecipient::User {
        id: s.accountant.id
    }));
    assert!(
        told[0]
            .recipients
            .contains(&NotifyRecipient::Party { id: COOLAIR })
    );
}

/// Done when 3: `waive` by a `tasks.manage` holder without a reason is
/// refused; with one the task is waived and, its agreement running, the
/// next occurrence is generated.
#[renox::test]
async fn a_waive_without_a_reason_is_refused() {
    let s = scenario(WlTicketing::new().waive_started_tasks(false)).await;
    let t = &s.t;

    t.task_act(&s.manager, s.vat.0, "waive", "2026-12-02 09:00", &[])
        .await
        .assert_status(422);
    t.task_act(
        &s.manager,
        s.vat.0,
        "waive",
        "2026-12-02 09:00",
        &[("reason", "  ")],
    )
    .await
    .assert_status(422);
    assert_eq!(t.task(s.vat.0).await.status, DeadlineTaskStatus::Upcoming);

    // A started task is not waived while the setting is off, even with a
    // reason; the worker may not waive at all.
    t.task_act(
        &s.manager,
        s.ct.0,
        "waive",
        "2026-12-02 09:00",
        &[("reason", "Nil return not needed")],
    )
    .await
    .assert_status(422);
    t.task_act(
        &s.accountant,
        s.vat.0,
        "waive",
        "2026-12-02 09:00",
        &[("reason", "Nil return not needed")],
    )
    .await
    .assert_forbidden();

    t.task_act(
        &s.manager,
        s.vat.0,
        "waive",
        "2026-12-02 09:00",
        &[("reason", "Nil return not needed")],
    )
    .await
    .assert_ok();
    let vat = t.task(s.vat.0).await;
    assert_eq!(vat.status, DeadlineTaskStatus::NotRequired);
    assert_eq!(vat.closed_reason.as_deref(), Some("Nil return not needed"));
    let next = t.task(vat.next_task_id.expect("the next quarter")).await;
    assert_eq!(next.due_date, day("2027-04-28"));
    assert_eq!(next.status, DeadlineTaskStatus::Upcoming);
}
