//! The back-office pages of deadline tasks (wl-ticketing §4.4, §6 `Tasks`;
//! the surfaces of BL-TKT-09 #154, BL-TKT-10 #155 and BL-ACC-13 #337): the
//! list of an agreement's or an owner's tasks, a task's page with the held
//! flag (and no reason) and the *late* badge, and its actions through the
//! real routes, each behind its `tasks.*` permission.

mod support;

use wl_kernel::Subject;
use wl_kernel::deadlines::{DeadlineTaskStatus, NewDeadlineTask, deadline_tasks};
use wl_ticketing::{WlTicketing, deadlines, permissions as perm};

use support::{COOLAIR, boot_with, day};

/// Engagement E-12.
fn e12() -> Subject {
    Subject::new("accounting.engagement", 12)
}

/// Engagement E-13.
fn e13() -> Subject {
    Subject::new("accounting.engagement", 13)
}

/// The list filters on an agreement and on an owner; a held task shows
/// "Held" and no reason, an overdue one "Late", a task completed after it
/// went overdue "Completed late"; the actions run through the real routes
/// with their permissions; without `tasks.view` nothing shows.
#[renox::test]
async fn the_deadline_pages_list_show_and_act_on_the_tasks() {
    let t = boot_with(WlTicketing::new()).await;
    let aisha = t.user("Aisha", &[perm::TASKS_VIEW, perm::TASKS_WORK]).await;
    let omar = t.user("Omar", &[perm::TASKS_VIEW, perm::TASKS_WORK]).await;
    let mona = t
        .user(
            "Mona",
            &[perm::TASKS_VIEW, perm::TASKS_REVIEW, perm::TASKS_MANAGE],
        )
        .await;
    let nobody = t.user("Nadia", &[perm::VIEW]).await;
    let tasks = deadline_tasks(t.state());
    let vat = tasks
        .create(
            t.state(),
            NewDeadlineTask::new(e12(), "vat_return", "VAT Q3", day("2026-11-28"))
                .lead_days(21)
                .party(COOLAIR)
                .owner_user(aisha.id),
        )
        .await
        .unwrap()
        .expect("ticketing is on");
    let ct = tasks
        .create(
            t.state(),
            NewDeadlineTask::new(e12(), "ct_return", "CT return", day("2026-11-28"))
                .owner_user(aisha.id),
        )
        .await
        .unwrap()
        .unwrap();
    let other = tasks
        .create(
            t.state(),
            NewDeadlineTask::new(e13(), "vat_return", "VAT Q3 of E-13", day("2026-12-15"))
                .owner_user(omar.id),
        )
        .await
        .unwrap()
        .unwrap();
    // E-12 is suspended for a restricted reason; the tasks only carry the
    // flag.
    tasks
        .hold(t.state(), &e12(), day("2026-11-01"))
        .await
        .unwrap();
    assert_eq!(
        deadlines::mark_overdue(t.state(), day("2026-11-29"))
            .await
            .unwrap(),
        2
    );

    t.app.acting_as(&aisha);
    t.app
        .get("/deadline-tasks?subject_type=accounting.engagement&subject_id=12")
        .await
        .assert_ok()
        .assert_see("VAT Q3")
        .assert_see("CT return")
        .assert_see("Held")
        .assert_see("Late")
        .assert_see("Overdue")
        .assert_dont_see("VAT Q3 of E-13");
    t.app
        .get("/deadline-tasks?mine=1")
        .await
        .assert_ok()
        .assert_see("CT return")
        .assert_dont_see("VAT Q3 of E-13");
    t.app
        .get(&format!("/deadline-tasks?owner={}", omar.id))
        .await
        .assert_ok()
        .assert_see("VAT Q3 of E-13")
        .assert_dont_see("CT return");
    // The open tasks, without a filter.
    t.app
        .get("/deadline-tasks")
        .await
        .assert_ok()
        .assert_see("CT return")
        .assert_see("VAT Q3 of E-13");
    // The page: the flags, no reason, and the worker's action.
    t.app
        .get(&format!("/deadline-tasks/{}", vat.0))
        .await
        .assert_ok()
        .assert_see("Held")
        .assert_see("Late")
        .assert_see("2026-11-29")
        .assert_see(&format!("deadline-start-{}", vat.0))
        .assert_dont_see("Close as missed")
        .assert_dont_see("sanction");

    // The worker may not close it as missed; the manager is offered it.
    t.task_act(
        &aisha,
        ct.0,
        "close_missed",
        "2026-12-02 09:00",
        &[("reason", "Client filed itself")],
    )
    .await
    .assert_forbidden();
    t.app.acting_as(&mona);
    t.app
        .get(&format!("/deadline-tasks/{}", ct.0))
        .await
        .assert_ok()
        .assert_see("Close as missed");
    t.task_act(
        &mona,
        ct.0,
        "close_missed",
        "2026-12-02 09:00",
        &[("reason", "Client filed itself")],
    )
    .await
    .assert_ok();
    let closed = t.task(ct.0).await;
    assert_eq!(closed.status, DeadlineTaskStatus::Missed);
    assert_eq!(closed.closed_reason.as_deref(), Some("Client filed itself"));

    // Worked and completed late through the routes.
    t.task_act(&aisha, vat.0, "start", "2026-12-02 09:00", &[])
        .await
        .assert_ok();
    t.task_act(&aisha, vat.0, "complete", "2026-12-02 10:00", &[])
        .await
        .assert_ok();
    assert_eq!(t.task(vat.0).await.status, DeadlineTaskStatus::Completed);
    t.app.acting_as(&aisha);
    t.app
        .get(&format!("/deadline-tasks/{}", vat.0))
        .await
        .assert_ok()
        .assert_see("Completed late");

    // The upcoming task of E-13 is waived by the manager with a reason.
    t.task_act(&mona, other.0, "waive", "2026-12-02 11:00", &[])
        .await
        .assert_status(422);
    t.task_act(
        &mona,
        other.0,
        "waive",
        "2026-12-02 11:00",
        &[("reason", "Nil return not needed")],
    )
    .await
    .assert_ok();
    assert_eq!(
        t.task(other.0).await.status,
        DeadlineTaskStatus::NotRequired
    );

    // Without `tasks.view`, nothing.
    t.app.acting_as(&nobody);
    t.app.get("/deadline-tasks").await.assert_forbidden();
    t.app
        .get(&format!("/deadline-tasks/{}", vat.0))
        .await
        .assert_forbidden();
    t.task_act(&nobody, other.0, "start", "2026-12-02 11:00", &[])
        .await
        .assert_forbidden();
}
