//! Acceptance checks of #337 (BL-ACC-13, 04 `ACC.RUL.04`) on the deadline
//! Task lifecycle, with wl-ticketing set as the 04 host sets it: the nightly
//! overdue run is recorded under `ACC.RUL.04`, a VAT return waiting on the
//! client goes `overdue` the day after its due date and the assignee, the
//! manager and the partner are told; it is then started and completed with
//! evidence, flagged *late*; `close_missed` needs a reason and brings the
//! next occurrence. (The owner alert of a failed `ACC.RUL.04` run is checked
//! in `apps/accounting/tests/overdue_rule.rs`.)

mod support;

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::automation::RuleRun;
use wl_kernel::compliance::{occurrence_key, schedule_subject};
use wl_kernel::deadlines::{
    DeadlineTaskStatus, NewDeadlineTask, TaskCompleted, TaskOverdue, TaskRecurrence, deadline_tasks,
};
use wl_kernel::notify::NotifyRecipient;
use wl_ticketing::{WlTicketing, deadlines, permissions as perm};

use support::{CLIENT, COOLAIR, Recorded, boot_app, day, record};

/// The 04 rule id of the overdue run.
const ACC_RUL_04: &str = "ACC.RUL.04";
/// The 04 manager role.
const MANAGER: &str = "engagement_manager";
/// The 04 partner role.
const PARTNER: &str = "partner";

/// Engagement E-12 of the accounting vertical.
fn e12() -> Subject {
    Subject::new("accounting.engagement", 12)
}

/// Whether a run of `rule` exists for the night of 2026-11-29.
async fn ran(db: &Db, rule: &str) -> bool {
    RuleRun::find_occurrence(
        db,
        rule,
        &schedule_subject(),
        &occurrence_key(rule, day("2026-11-29")),
    )
    .await
    .unwrap()
    .is_some()
}

/// wl-ticketing as the 04 host sets it, with the 04 reasons for
/// `close_missed` (§7.1 *Task*).
fn as_04() -> WlTicketing {
    WlTicketing::new()
        .overdue_rule_id(ACC_RUL_04)
        .task_manager_role(MANAGER)
        .task_overdue_role(PARTNER)
        .waive_started_tasks(false)
        .missed_reason("client_no_documents", "Client did not provide documents")
        .missed_reason("client_filed_itself", "Client filed itself")
        .missed_reason("engagement_ended", "Engagement ended")
        .missed_reason_with_text("other", "Other")
}

/// A quarterly VAT return of E-12 due on `due`, owned by `owner`, needing
/// the client's input and evidence.
fn vat(title: &str, due: &str, owner: i64) -> NewDeadlineTask {
    NewDeadlineTask::new(e12(), "vat_return", title, day(due))
        .lead_days(21)
        .party(COOLAIR)
        .owner_user(owner)
        .evidence_required()
        .needs_client_input()
        .recurrence(TaskRecurrence::Quarterly)
        .period_end(day("2026-09-30"))
}

/// DW1-DW3: a VAT return due 2026-11-28 waiting on the client is `overdue`
/// on 2026-11-29 (assignee, manager and partner told, not the client; the
/// run is `ACC.RUL.04`'s); started and completed on 2026-12-02 with
/// evidence it is `completed` and *late*; `close_missed` without a reason
/// is refused, with "Client filed itself" it is `missed` and the next
/// occurrence exists.
#[renox::test]
async fn bl_acc_13_an_overdue_task_is_completed_late_or_closed_missed_with_a_reason() {
    let overdue = Recorded::<TaskOverdue>::new();
    let completed = Recorded::<TaskCompleted>::new();
    let t = boot_app(CLIENT, as_04(), |app| {
        record(record(app, &overdue), &completed)
    })
    .await;
    let accountant = t.user("Aisha", &[perm::TASKS_VIEW, perm::TASKS_WORK]).await;
    let manager = t
        .user_in_role(
            "Mona",
            MANAGER,
            &[perm::TASKS_VIEW, perm::TASKS_REVIEW, perm::TASKS_MANAGE],
        )
        .await;
    let partner = t.user_in_role("Pia", PARTNER, &[perm::TASKS_VIEW]).await;
    let tasks = deadline_tasks(t.state());
    let late = tasks
        .create(t.state(), vat("VAT Q3", "2026-11-28", accountant.id))
        .await
        .unwrap()
        .expect("ticketing is on");
    let missed = tasks
        .create(
            t.state(),
            vat("VAT Q3 (branch)", "2026-11-28", accountant.id),
        )
        .await
        .unwrap()
        .unwrap();
    t.task_act(&accountant, late.0, "start", "2026-11-10 09:00", &[])
        .await
        .assert_ok();
    t.task_act(
        &accountant,
        late.0,
        "wait",
        "2026-11-10 10:00",
        &[("reason", "Waiting on the client's sales ledger")],
    )
    .await
    .assert_ok();
    assert_eq!(t.task(late.0).await.status, DeadlineTaskStatus::Waiting);

    // DW1: not on the due date, the day after.
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
    assert_eq!(task.overdue_on, Some(day("2026-11-29")));
    assert_eq!(task.stage.as_deref(), Some("waiting"));
    assert_eq!(overdue.all().len(), 2);
    let told = t.notices.recipients(deadlines::OVERDUE_NOTICE);
    for (who, id) in [
        ("the assignee", accountant.id),
        ("the manager", manager.id),
        ("the partner", partner.id),
    ] {
        assert!(
            told.contains(&NotifyRecipient::User { id }),
            "{who} is told: {told:?}"
        );
    }
    assert!(
        !told
            .iter()
            .any(|to| matches!(to, NotifyRecipient::Party { .. })),
        "the client is not told"
    );
    assert!(ran(t.db(), ACC_RUL_04).await, "the run is ACC.RUL.04's");
    assert!(!ran(t.db(), deadlines::OVERDUE_RULE).await);

    // DW2: started and completed late, with evidence.
    t.task_act(&accountant, late.0, "start", "2026-12-02 09:00", &[])
        .await
        .assert_ok();
    assert_eq!(t.task(late.0).await.status, DeadlineTaskStatus::InProgress);
    t.task_act(
        &accountant,
        late.0,
        "complete",
        "2026-12-02 10:00",
        &[("evidence", "FTA acknowledgement 2399")],
    )
    .await
    .assert_ok();
    let task = t.task(late.0).await;
    assert_eq!(task.status, DeadlineTaskStatus::Completed);
    assert!(task.late, "completed late");
    let summary = tasks.find(t.db(), late).await.unwrap().unwrap();
    assert!(summary.late);
    assert!(completed.all()[0].late);

    // DW3: closed as missed only with a reason from the 04 list (a free
    // text, an unknown code or "other" without its text is refused); the
    // next occurrence follows.
    for refused in [
        vec![],
        vec![("reason", "Client filed itself")],
        vec![("reason_code", "forgot")],
        vec![("reason_code", "other")],
    ] {
        t.task_act(
            &manager,
            missed.0,
            "close_missed",
            "2026-12-02 11:00",
            &refused,
        )
        .await
        .assert_status(422);
    }
    assert_eq!(t.task(missed.0).await.status, DeadlineTaskStatus::Overdue);
    // The form offers the list.
    t.app.acting_as(&manager);
    t.app
        .get(&format!("/deadline-tasks/{}", missed.0))
        .await
        .assert_ok()
        .assert_see("Close as missed")
        .assert_see("Client did not provide documents")
        .assert_see("Late");
    t.task_act(
        &manager,
        missed.0,
        "close_missed",
        "2026-12-02 11:00",
        &[("reason_code", "client_filed_itself")],
    )
    .await
    .assert_ok();
    let task = t.task(missed.0).await;
    assert_eq!(task.status, DeadlineTaskStatus::Missed);
    assert_eq!(task.closed_reason.as_deref(), Some("Client filed itself"));
    let next = t
        .task(task.next_task_id.expect("the next occurrence"))
        .await;
    assert_eq!(next.status, DeadlineTaskStatus::Upcoming);
    assert_eq!(next.due_date, day("2027-02-28"));
}
