//! BL-HRM-28 (#218), `HRM.RUL.17`: an open onboarding step past its due date
//! is reminded to its assignees on the first run after the due date and then
//! every 2 days; HR and the manager are told once after 5 days; the run
//! pauses for leave and absconding, and the due dates follow the approved
//! leave as recorded now.

mod support;

use renox::prelude::*;
use wl_hr::onboarding::{self, StartRequest, StartTrigger};
use wl_hr::reminders;
use wl_hr::steps::{self, DoneInput};
use wl_hr::template::{StepInput, TemplateInput};
use wl_hr::{
    Employee, EmployeeStatus, LeaveRequest, LeaveStatus, NewEmployee, Onboarding, OnboardingStep,
    StepStatus, employees,
};
use wl_kernel::PartyId;
use wl_kernel::notify::{NotifyRecipient, NotifyRequest};

use support::{Booted, add_template, boot_with, branch, nov};

const REMINDER: &str = "hr.onboarding_step_reminder";
const ESCALATED: &str = "hr.onboarding_step_escalated";

/// The users a request is for.
fn users(request: &NotifyRequest) -> Vec<i64> {
    let mut ids: Vec<i64> = request
        .recipients
        .iter()
        .filter_map(|r| match r {
            NotifyRecipient::User { id } => Some(*id),
            _ => None,
        })
        .collect();
    ids.sort_unstable();
    ids
}

/// A client with no induction, HR user Hana and manager Mia at branch B.
async fn booted() -> (Booted, User, User) {
    let t = boot_with(wl_hr::WlHr::new(), false).await;
    let hana = t.user_in_branch("Hana", "hr").await;
    let mia = t.user_in_branch("Mia", "manager").await;
    (t, hana, mia)
}

/// "Counter" template: a required step due 7 days after the start (9 Nov
/// for a start on 2 Nov), assigned to HR.
fn counter_template() -> TemplateInput {
    TemplateInput::for_role("Counter", "counter").step(StepInput::task(
        "Return the access card",
        "hr",
        7,
    ))
}

/// A counter employee hired on 2 Nov, in onboarding, with the hire onboarding.
async fn hired(t: &Booted, party: i64) -> (Employee, Onboarding) {
    let employee = employees::create(
        t.state(),
        NewEmployee::new(PartyId(party), "counter", nov(2))
            .scope(branch())
            .status(EmployeeStatus::Onboarding),
    )
    .await
    .unwrap();
    let onboarding = t.onboardings(&employee).await.remove(0);
    (employee, onboarding)
}

async fn set_status(t: &Booted, employee: &Employee, status: &str) {
    Employee::query()
        .where_(Employee::ID.eq(employee.id))
        .update(t.db(), &[("status", &status)])
        .await
        .unwrap();
}

async fn step_of(t: &Booted, onboarding: &Onboarding, title: &str) -> OnboardingStep {
    let step = t.step(onboarding, title).await;
    t.reload(&step).await
}

/// AC1 and AC2: a step due 9 Nov still open is reminded to its assignee on
/// 10, 12 and 14 Nov, not on the days between or twice a day; on 14 Nov (5
/// days overdue) HR and the manager are told once.
#[renox::test]
async fn hrm_rul_17_the_assignee_is_reminded_every_second_day_and_hr_is_told_once() {
    let (t, hana, mia) = booted().await;
    add_template(&t, counter_template()).await;
    let (_, onboarding) = hired(&t, 60).await;
    let step = step_of(&t, &onboarding, "Return the access card").await;
    assert_eq!(step.due_date, nov(9));

    for (day, reminders_so_far) in [(9, 0), (10, 1), (11, 1), (12, 2), (13, 2)] {
        reminders::run(t.state(), nov(day)).await.unwrap();
        assert_eq!(
            t.notices.of_type(REMINDER).len(),
            reminders_so_far,
            "9 Nov + {day}"
        );
    }
    // A second run on the same day sends nothing.
    reminders::run(t.state(), nov(12)).await.unwrap();
    assert_eq!(t.notices.of_type(REMINDER).len(), 2);
    assert!(t.notices.of_type(ESCALATED).is_empty());

    let report = reminders::run(t.state(), nov(14)).await.unwrap();
    assert_eq!((report.reminded, report.escalated), (1, 1));
    let sent = t.notices.of_type(REMINDER);
    assert_eq!(sent.len(), 3);
    assert!(sent.iter().all(|r| users(r) == [hana.id]), "{sent:?}");
    let told = t.notices.of_type(ESCALATED);
    assert_eq!(told.len(), 1);
    let mut both = vec![hana.id, mia.id];
    both.sort_unstable();
    assert_eq!(users(&told[0]), both);

    // Once only.
    reminders::run(t.state(), nov(15)).await.unwrap();
    reminders::run(t.state(), nov(16)).await.unwrap();
    assert_eq!(t.notices.of_type(ESCALATED).len(), 1);
    assert_eq!(t.notices.of_type(REMINDER).len(), 4);
}

/// AC3: a step done on 11 Nov is not reminded again, and a waived or
/// cancelled onboarding is left alone.
#[renox::test]
async fn hrm_rul_17_a_done_step_and_a_closed_onboarding_send_nothing_more() {
    let (t, hana, _) = booted().await;
    add_template(&t, counter_template()).await;
    let (_, onboarding) = hired(&t, 60).await;
    let step = step_of(&t, &onboarding, "Return the access card").await;

    reminders::run(t.state(), nov(10)).await.unwrap();
    assert_eq!(t.notices.of_type(REMINDER).len(), 1);
    let done = steps::mark_done(t.state(), step.id, hana.id, DoneInput::default())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.status, StepStatus::Done);
    for day in [12, 14, 16] {
        reminders::run(t.state(), nov(day)).await.unwrap();
    }
    assert_eq!(t.notices.of_type(REMINDER).len(), 1);
    assert!(t.notices.of_type(ESCALATED).is_empty());

    // A cancelled onboarding: no more reminders.
    let (second, _) = hired(&t, 61).await;
    onboarding::cancel_for(t.state(), second.id, "left")
        .await
        .unwrap();
    t.notices.clear();
    reminders::run(t.state(), nov(20)).await.unwrap();
    assert!(t.notices.all().is_empty(), "{:?}", t.notices.all());
}

/// An optional step is reminded like a required one, but HR and the manager
/// are not told about it.
#[renox::test]
async fn hrm_rul_17_hr_is_not_told_about_an_optional_step() {
    let (t, _, _) = booted().await;
    add_template(
        &t,
        TemplateInput::for_role("Counter", "counter")
            .step(StepInput::task("Return the access card", "hr", 7))
            .step(StepInput::task("Tour of the branch", "hr", 7).optional()),
    )
    .await;
    hired(&t, 60).await;

    reminders::run(t.state(), nov(10)).await.unwrap();
    assert_eq!(t.notices.of_type(REMINDER).len(), 2);
    reminders::run(t.state(), nov(14)).await.unwrap();
    let told = t.notices.of_type(ESCALATED);
    assert_eq!(told.len(), 1);
    assert_eq!(
        told[0].subject_label.as_deref(),
        Some("Return the access card")
    );
}

/// AC4 and AC6: an employee with a manual onboarding goes on 10 days' leave
/// from 8 Nov: the step due 9 Nov is due 19 Nov and nothing is sent during
/// the leave. With the leave shortened to 6 days it is due 15 Nov; a
/// cancelled leave puts the date back.
#[renox::test]
async fn hrm_rul_17_leave_moves_the_due_date_and_pauses_the_reminders() {
    let (t, _, _) = booted().await;
    let template = add_template(&t, counter_template()).await;
    let employee = employees::create(
        t.state(),
        NewEmployee::new(PartyId(60), "counter", nov(1))
            .scope(branch())
            .status(EmployeeStatus::Active),
    )
    .await
    .unwrap();
    let onboarding = onboarding::start(
        t.state(),
        StartRequest {
            employee_id: employee.id,
            trigger: StartTrigger::Manual,
            templates: vec![template.id],
            start_on: Some(nov(2)),
        },
    )
    .await
    .unwrap()
    .unwrap();
    let step = step_of(&t, &onboarding, "Return the access card").await;
    assert_eq!(step.due_date, nov(9));

    let mut leave = LeaveRequest::create(
        t.db(),
        LeaveRequest {
            employee_id: employee.id,
            leave_type: "annual".to_owned(),
            from_date: nov(8),
            to_date: nov(17),
            status: LeaveStatus::Approved,
            ..LeaveRequest::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(leave.days(), 10);
    set_status(&t, &employee, "on_leave").await;

    let report = reminders::run(t.state(), nov(10)).await.unwrap();
    assert_eq!(report.moved, 1);
    assert_eq!(
        step_of(&t, &onboarding, "Return the access card")
            .await
            .due_date,
        nov(19)
    );
    for day in [12, 15, 18] {
        reminders::run(t.state(), nov(day)).await.unwrap();
    }
    assert!(t.notices.of_type(REMINDER).is_empty());
    assert!(t.notices.of_type(ESCALATED).is_empty());

    // Shortened to 6 days: 8 to 13 Nov.
    leave.to_date = nov(13);
    leave.save(t.db()).await.unwrap();
    reminders::run(t.state(), nov(14)).await.unwrap();
    assert_eq!(
        step_of(&t, &onboarding, "Return the access card")
            .await
            .due_date,
        nov(15)
    );

    // Cancelled: the date is back.
    leave.status = LeaveStatus::Cancelled;
    leave.save(t.db()).await.unwrap();
    reminders::run(t.state(), nov(14)).await.unwrap();
    assert_eq!(
        step_of(&t, &onboarding, "Return the access card")
            .await
            .due_date,
        nov(9)
    );
}

/// After the leave the employee is back and the step, now due later, is
/// reminded from the day after its moved due date.
#[renox::test]
async fn hrm_rul_17_after_the_leave_the_moved_due_date_counts() {
    let (t, _, _) = booted().await;
    add_template(&t, counter_template()).await;
    let (employee, onboarding) = hired(&t, 60).await;
    LeaveRequest::create(
        t.db(),
        LeaveRequest {
            employee_id: employee.id,
            leave_type: "annual".to_owned(),
            from_date: nov(8),
            to_date: nov(17),
            status: LeaveStatus::Approved,
            ..LeaveRequest::default()
        },
    )
    .await
    .unwrap();
    set_status(&t, &employee, "on_leave").await;
    reminders::run(t.state(), nov(10)).await.unwrap();
    set_status(&t, &employee, "active").await;
    reminders::run(t.state(), nov(18)).await.unwrap();
    reminders::run(t.state(), nov(19)).await.unwrap();
    assert!(t.notices.of_type(REMINDER).is_empty());
    reminders::run(t.state(), nov(20)).await.unwrap();
    assert_eq!(t.notices.of_type(REMINDER).len(), 1);
    assert_eq!(
        step_of(&t, &onboarding, "Return the access card")
            .await
            .due_date,
        nov(19)
    );
}

/// AC5: an absconded employee gets no reminder or escalation.
#[renox::test]
async fn hrm_rul_17_an_absconded_employee_is_not_reminded() {
    let (t, _, _) = booted().await;
    add_template(&t, counter_template()).await;
    let (employee, _) = hired(&t, 60).await;
    set_status(&t, &employee, "absconded").await;
    for day in [10, 12, 14, 16] {
        reminders::run(t.state(), nov(day)).await.unwrap();
    }
    assert!(
        t.notices
            .all()
            .iter()
            .all(|n| n.type_key != REMINDER && n.type_key != ESCALATED)
    );
}

/// AC7: a step due 9 Nov, the employee absconded from 5 Nov and reinstated
/// on 20 Nov at 15:00: the step stays due 9 Nov; on 21 Nov the assignee is
/// reminded and HR and the manager are told once; the next reminder is on 23
/// Nov.
#[renox::test]
async fn hrm_rul_17_after_reinstatement_the_first_run_reminds_and_tells_hr_once() {
    let (t, _, _) = booted().await;
    add_template(&t, counter_template()).await;
    let (employee, onboarding) = hired(&t, 60).await;
    set_status(&t, &employee, "absconded").await;
    for day in [10, 14, 20] {
        reminders::run(t.state(), nov(day)).await.unwrap();
    }
    assert!(t.notices.of_type(REMINDER).is_empty());
    assert_eq!(
        step_of(&t, &onboarding, "Return the access card")
            .await
            .due_date,
        nov(9)
    );

    // Reinstated on 20 Nov at 15:00, after that day's 08:00 run.
    set_status(&t, &employee, "active").await;
    reminders::run(t.state(), nov(21)).await.unwrap();
    assert_eq!(t.notices.of_type(REMINDER).len(), 1);
    assert_eq!(t.notices.of_type(ESCALATED).len(), 1);
    assert_eq!(
        step_of(&t, &onboarding, "Return the access card")
            .await
            .due_date,
        nov(9)
    );
    reminders::run(t.state(), nov(22)).await.unwrap();
    assert_eq!(t.notices.of_type(REMINDER).len(), 1);
    reminders::run(t.state(), nov(23)).await.unwrap();
    assert_eq!(t.notices.of_type(REMINDER).len(), 2);
    assert_eq!(t.notices.of_type(ESCALATED).len(), 1);
}

/// A step nobody is assigned to (an employee without an account records it in
/// person) is reminded to HR.
#[renox::test]
async fn hrm_rul_17_a_step_without_an_assignee_is_reminded_to_hr() {
    let (t, hana, _) = booted().await;
    add_template(
        &t,
        TemplateInput::for_role("Counter", "counter").step(StepInput::task(
            "Read the handbook",
            "employee",
            7,
        )),
    )
    .await;
    let (_, onboarding) = hired(&t, 60).await;
    let step = step_of(&t, &onboarding, "Read the handbook").await;
    assert!(step.assignees(t.db()).await.unwrap().is_empty());
    reminders::run(t.state(), nov(10)).await.unwrap();
    let sent = t.notices.of_type(REMINDER);
    assert_eq!(sent.len(), 1);
    assert_eq!(users(&sent[0]), [hana.id]);
}

/// A waived step is not reminded or escalated (the "must not" of #218); the
/// open step next to it still is.
#[renox::test]
async fn hrm_rul_17_a_waived_step_is_not_reminded() {
    let (t, hana, _) = booted().await;
    add_template(
        &t,
        TemplateInput::for_role("Counter", "counter")
            .step(StepInput::task("Return the access card", "hr", 7))
            .step(StepInput::task("Sign the handbook", "hr", 7)),
    )
    .await;
    let (_, onboarding) = hired(&t, 60).await;
    let waived = step_of(&t, &onboarding, "Return the access card").await;
    let owner = t.user_with("Omar", "owner").await;
    let done = steps::waive(t.state(), waived.id, owner.id, "keeps the card")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.status, StepStatus::Waived);

    for day in [10, 12, 14, 16] {
        reminders::run(t.state(), nov(day)).await.unwrap();
    }
    let sent = t.notices.of_type(REMINDER);
    assert_eq!(sent.len(), 4, "only the open step is reminded");
    assert!(
        sent.iter()
            .all(|r| r.subject_label.as_deref() == Some("Sign the handbook")
                && users(r) == [hana.id]),
        "{sent:?}"
    );
    let told = t.notices.of_type(ESCALATED);
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].subject_label.as_deref(), Some("Sign the handbook"));
}
