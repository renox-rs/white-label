//! BL-HRM-26 (#216): `HRM.RUL.16` starts the onboarding from the templates
//! that apply when an employee is created in `onboarding`; HR starts a
//! template by hand; the steps are assigned to HR, the manager or the
//! employee, due from the hire date; assignees and due dates follow the
//! people; exit, death and a cancelled onboarding cancel it;
//! `Workforce::employees_in`.

mod support;

use renox::prelude::*;
use wl_hr::onboarding::{self, StartRefused, StartRequest, StartTrigger};
use wl_hr::{
    Employee, EmployeeStatus, NewEmployee, Onboarding, OnboardingStatus, OnboardingTemplate,
    StepStatus, employees,
};
use wl_kernel::PartyId;
use wl_kernel::workforce::{
    EmployeeAssignmentChanged, EmployeeDeceased, EmployeeExited, EmployeeId,
    EmployeeOnboardingCancelled, workforce,
};

use support::{
    add_template, boot, boot_with, branch, driver_template, nov, other_branch, supervisor_template,
};

/// AC1: "All staff" and "Driver": one onboarding with both templates' steps;
/// the "SOPs acknowledged" step is due 2026-11-09 and assigned to Ravi.
#[renox::test]
async fn hrm26_one_onboarding_with_the_steps_of_every_template_that_applies() {
    let t = boot().await;
    add_template(&t, driver_template()).await;
    // The SOPs of the role are pending, so the step stays open.
    t.sops.set_pending(&[1]);
    // The steps due first (day 1) are HR's and the manager's: HR is told.
    let hana = t.user_in_branch("Hana", "hr").await;
    let (ravi, account) = t.ravi().await;
    assert_eq!(ravi.status, EmployeeStatus::Onboarding);

    let all = t.onboardings(&ravi).await;
    assert_eq!(all.len(), 1, "one onboarding for both templates");
    let onboarding = &all[0];
    assert_eq!(onboarding.start_trigger, "hire");
    assert_eq!(onboarding.started_on, nov(2));
    assert_eq!(onboarding.templates(t.db()).await.unwrap().len(), 2);
    let steps = onboarding.steps(t.db()).await.unwrap();
    assert_eq!(steps.len(), 5 + 4);
    let sops = t.step(onboarding, "SOPs acknowledged").await;
    assert_eq!(sops.due_date, nov(9));
    assert_eq!(sops.assignees(t.db()).await.unwrap(), [account.id]);
    assert_eq!(sops.status, StepStatus::Open);
    // Each step keeps its template.
    let handbook = t.step(onboarding, "Read the handbook").await;
    assert_ne!(handbook.template_id, sops.template_id);
    assert_eq!(handbook.due_date, nov(7));

    let started = t.started.all();
    assert_eq!(started.len(), 1);
    assert_eq!(started[0].employee, EmployeeId(ravi.id));
    assert_eq!(started[0].job_role, "driver");
    assert_eq!(started[0].scope, Some(branch()));
    assert_eq!(started[0].trigger, "hire");
    // The assignees of the first steps are told.
    let told = t.notices.of_type("hr.onboarding_step_assigned");
    assert!(
        told.iter()
            .any(|n| n.recipients == [wl_kernel::notify::NotifyRecipient::User { id: hana.id }]),
        "{told:?}"
    );
}

/// AC2: no manager in the branch, HR users H1 and H2 hold
/// `onboarding.manage` there: the "welcome" step goes to both; when H1 marks
/// it done, it is done for both. The owner is not an assignee while someone
/// else holds the permission.
#[renox::test]
async fn hrm26_a_step_without_a_manager_goes_to_every_hr_user_and_any_one_marks_it() {
    let t = boot().await;
    let h1 = t.user_in_branch("Hana", "hr").await;
    let h2 = t.user_in_branch("Hiba", "hr").await;
    let owner = t.user_with("Omar", "owner").await;
    let (ravi, _) = t.ravi().await;
    let onboarding = &t.onboardings(&ravi).await[0];
    let welcome = t.step(onboarding, "Welcome and introductions").await;
    let mut who = welcome.assignees(t.db()).await.unwrap();
    who.sort_unstable();
    assert_eq!(who, [h1.id, h2.id]);
    assert!(!who.contains(&owner.id));

    let done = wl_hr::steps::mark_done(t.state(), welcome.id, h1.id, Default::default())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.status, StepStatus::Done);
    assert_eq!(done.done_by, Some(h1.id));
    // Done for both: H2 finds it no longer open.
    assert!(
        wl_hr::steps::mark_done(t.state(), welcome.id, h2.id, Default::default())
            .await
            .unwrap()
            .is_err()
    );
}

/// With nobody but the owner holding the permission, the owner gets the `hr`
/// steps.
#[renox::test]
async fn hrm26_the_owner_gets_hr_steps_only_when_nobody_else_holds_the_permission() {
    let t = boot().await;
    let owner = t.user_with("Omar", "owner").await;
    let (ravi, _) = t.ravi().await;
    let onboarding = &t.onboardings(&ravi).await[0];
    let account = t.step(onboarding, "System account and access").await;
    assert_eq!(account.assignees(t.db()).await.unwrap(), [owner.id]);
}

/// AC3: the event `EmployeeOnboardingCancelled` cancels the onboarding with
/// the event's reason, and its open steps.
#[renox::test]
async fn hrm26_a_cancelled_onboarding_cancels_the_onboarding_and_its_open_steps() {
    let t = boot().await;
    t.sops.set_pending(&[1]);
    let (ravi, _) = t.ravi().await;
    t.state()
        .emit(EmployeeOnboardingCancelled {
            employee: EmployeeId(ravi.id),
            reason: "candidate withdrew".into(),
        })
        .await
        .unwrap();
    let onboarding = Onboarding::find_or_404(t.db(), t.onboardings(&ravi).await[0].id)
        .await
        .unwrap();
    assert_eq!(onboarding.status, OnboardingStatus::Cancelled);
    assert_eq!(
        onboarding.cancel_reason.as_deref(),
        Some("candidate withdrew")
    );
    for step in onboarding.steps(t.db()).await.unwrap() {
        assert_eq!(step.status, StepStatus::Cancelled, "{}", step.title_en);
    }
}

/// AC10 and the exit: death and exit cancel a manual onboarding in progress.
#[renox::test]
async fn hrm26_death_and_exit_cancel_a_running_onboarding() {
    let t = boot().await;
    add_template(&t, supervisor_template()).await;
    // The SOPs of the role are pending, so every step is open.
    t.sops.set_pending(&[1]);
    for (n, deceased) in [(60, true), (61, false)] {
        let employee = employees::create(
            t.state(),
            NewEmployee::new(PartyId(n), "supervisor", nov(1))
                .scope(branch())
                .status(EmployeeStatus::Active),
        )
        .await
        .unwrap();
        let started = onboarding::start(
            t.state(),
            StartRequest {
                employee_id: employee.id,
                trigger: StartTrigger::Manual,
                templates: Vec::new(),
                start_on: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(started.status, OnboardingStatus::InProgress);
        if deceased {
            t.state()
                .emit(EmployeeDeceased {
                    employee: EmployeeId(employee.id),
                    date: nov(3),
                })
                .await
                .unwrap();
        } else {
            t.state()
                .emit(EmployeeExited {
                    employee: EmployeeId(employee.id),
                    exit_date: nov(3),
                    route: "standard".into(),
                })
                .await
                .unwrap();
        }
        let after = Onboarding::find_or_404(t.db(), started.id).await.unwrap();
        assert_eq!(after.status, OnboardingStatus::Cancelled);
        assert!(
            after
                .steps(t.db())
                .await
                .unwrap()
                .iter()
                .all(|s| s.status == StepStatus::Cancelled)
        );
    }
}

/// AC4 and AC5: HR cancels the manual "Supervisor" onboarding of an active
/// employee only with a reason; its steps are due from today.
#[renox::test]
async fn hrm26_a_manual_onboarding_is_due_from_today_and_is_cancelled_with_a_reason() {
    let t = boot().await;
    let supervisor = add_template(&t, supervisor_template()).await;
    let hana = t.user_with("Hana", "hr").await;
    let employee = employees::create(
        t.state(),
        NewEmployee::new(PartyId(62), "supervisor", nov(1))
            .scope(branch())
            .status(EmployeeStatus::Active),
    )
    .await
    .unwrap();
    // An employee created active starts no onboarding by itself.
    assert!(t.onboardings(&employee).await.is_empty());

    let request = || StartRequest {
        employee_id: employee.id,
        trigger: StartTrigger::Manual,
        templates: vec![supervisor.id],
        start_on: None,
    };
    let started = onboarding::start(t.state(), request())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(started.start_trigger, "manual");
    let today = wl_kernel::documents::today();
    assert_eq!(started.started_on, today);
    let step = t.step(&started, "Supervisor briefing").await;
    assert_eq!(step.due_date, onboarding::due_date(today, 2));
    // Not twice from the same template while one is in progress.
    assert_eq!(
        onboarding::start(t.state(), request())
            .await
            .unwrap()
            .unwrap_err(),
        StartRefused::AlreadyRunning
    );

    t.app.acting_as(&hana);
    let path = format!("/hr/onboardings/{}/cancel", started.id);
    t.app
        .htmx()
        .post(&path, &[("reason", "  ")])
        .await
        .assert_status(422);
    assert_eq!(
        Onboarding::find_or_404(t.db(), started.id)
            .await
            .unwrap()
            .status,
        OnboardingStatus::InProgress
    );
    t.app
        .post(&path, &[("reason", "no longer needed")])
        .await
        .assert_ok();
    let after = Onboarding::find_or_404(t.db(), started.id).await.unwrap();
    assert_eq!(after.status, OnboardingStatus::Cancelled);
    assert_eq!(after.cancel_reason.as_deref(), Some("no longer needed"));
    // Once cancelled, the template can start again.
    assert!(
        onboarding::start(t.state(), request())
            .await
            .unwrap()
            .is_ok()
    );
}

/// AC6: a minimal record created in `onboarding` starts the onboarding too;
/// with no template that applies it is created `active` and starts none.
#[renox::test]
async fn hrm26_the_state_of_a_new_record_follows_the_templates() {
    let with = boot().await;
    let auto = employees::create(
        with.state(),
        NewEmployee::new(PartyId(70), "technician", nov(2)).scope(branch()),
    )
    .await
    .unwrap();
    assert_eq!(
        auto.status,
        EmployeeStatus::Onboarding,
        "the all staff template applies"
    );
    assert_eq!(with.onboardings(&auto).await.len(), 1);

    let without = boot_with(wl_hr::WlHr::new(), false).await;
    let plain = employees::create(
        without.state(),
        NewEmployee::new(PartyId(71), "technician", nov(2)).scope(branch()),
    )
    .await
    .unwrap();
    assert_eq!(plain.status, EmployeeStatus::Active);
    assert!(without.onboardings(&plain).await.is_empty());
    assert!(
        employees::create(without.state(), NewEmployee::new(PartyId(72), "  ", nov(2)),)
            .await
            .is_err()
    );
}

/// AC7: `employees_in(driver, branch B)` returns Ravi while he is onboarding
/// or active, and not after he is absconded or has exited.
#[renox::test]
async fn hrm26_employees_in_lists_the_working_staff_of_a_role_in_a_scope() {
    let t = boot().await;
    let (ravi, _) = t.ravi().await;
    let in_branch = |role: &'static str, scope| {
        let state = t.state().clone();
        async move {
            workforce(&state)
                .employees_in(&state.db, role, &scope)
                .await
                .unwrap()
        }
    };
    assert_eq!(in_branch("driver", branch()).await, [EmployeeId(ravi.id)]);
    assert!(in_branch("driver", other_branch()).await.is_empty());
    assert!(in_branch("cleaner", branch()).await.is_empty());
    for (status, listed) in [
        ("active", true),
        ("on_leave", true),
        ("notice", true),
        ("absconded", false),
        ("deceased", false),
        ("exited", false),
        ("onboarding_cancelled", false),
    ] {
        Employee::query()
            .where_(Employee::ID.eq(ravi.id))
            .update(t.db(), &[("status", &status)])
            .await
            .unwrap();
        assert_eq!(
            !in_branch("driver", branch()).await.is_empty(),
            listed,
            "{status}"
        );
    }
}

/// AC8: Ravi's branch manager M1 is replaced by M2 while "welcome" is open:
/// the step is assigned to M2. A departed assignee with nobody left sends the
/// step to HR.
#[renox::test]
async fn hrm26_open_steps_follow_the_manager() {
    let t = boot().await;
    let hana = t.user_in_branch("Hana", "hr").await;
    let m1 = t.user_in_branch("Mona", "manager").await;
    let (ravi, _) = t.ravi().await;
    let onboarding = &t.onboardings(&ravi).await[0];
    let welcome = t.step(onboarding, "Welcome and introductions").await;
    assert_eq!(welcome.assignees(t.db()).await.unwrap(), [m1.id]);

    let m2 = t.user_in_branch("Maha", "manager").await;
    m1.remove_role_in(t.db(), "manager", &support::branch_scope())
        .await
        .unwrap();
    t.state()
        .emit(EmployeeAssignmentChanged {
            employee: EmployeeId(ravi.id),
            job_role: "driver".into(),
            scope: Some(branch()),
            from: Some("manager".into()),
            to: Some("manager".into()),
        })
        .await
        .unwrap();
    assert_eq!(
        t.reload(&welcome).await.assignees(t.db()).await.unwrap(),
        [m2.id]
    );

    m2.remove_role_in(t.db(), "manager", &support::branch_scope())
        .await
        .unwrap();
    onboarding::re_resolve(t.state(), ravi.id).await.unwrap();
    assert_eq!(
        t.reload(&welcome).await.assignees(t.db()).await.unwrap(),
        [hana.id]
    );
    // A done step keeps who did it.
    let done = wl_hr::steps::mark_done(t.state(), welcome.id, hana.id, Default::default())
        .await
        .unwrap()
        .unwrap();
    onboarding::re_resolve(t.state(), ravi.id).await.unwrap();
    assert_eq!(t.reload(&done).await.done_by, Some(hana.id));
}

/// AC9: Ravi's hire date corrected from 2 to 5 Nov: the open "SOPs
/// acknowledged" step is due 2026-11-12. A role change emits the assignment
/// event and removes no step.
#[renox::test]
async fn hrm26_a_corrected_hire_date_moves_the_due_dates_and_a_role_change_keeps_the_steps() {
    let t = boot().await;
    t.sops.set_pending(&[1]);
    let (ravi, _) = t.ravi().await;
    let onboarding = t.onboardings(&ravi).await.remove(0);
    let sops = t.step(&onboarding, "SOPs acknowledged").await;
    assert_eq!(sops.due_date, nov(9));
    employees::correct_hire_date(t.state(), ravi.id, nov(5))
        .await
        .unwrap();
    assert_eq!(t.reload(&sops).await.due_date, nov(12));

    let before = onboarding.steps(t.db()).await.unwrap().len();
    let changed = employees::change_job_role(t.state(), ravi.id, "Supervisor")
        .await
        .unwrap();
    assert_eq!(changed.job_role, "supervisor");
    assert_eq!(onboarding.steps(t.db()).await.unwrap().len(), before);
    let events = t.changed.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].from.as_deref(), Some("driver"));
    assert_eq!(events[0].to.as_deref(), Some("supervisor"));
    // The role change started no onboarding.
    assert_eq!(t.onboardings(&changed).await.len(), 1);
}

/// The running onboarding keeps the frozen version of its template.
#[renox::test]
async fn hrm26_a_template_edited_after_the_start_does_not_change_the_running_onboarding() {
    let t = boot().await;
    let template = add_template(&t, driver_template()).await;
    let (ravi, _) = t.ravi().await;
    let onboarding = t.onboardings(&ravi).await.remove(0);
    let applied = onboarding.templates(t.db()).await.unwrap();
    let frozen = applied
        .iter()
        .find(|a| a.template_id == template.id)
        .unwrap();
    assert_eq!(frozen.template_version, 1);

    OnboardingTemplate::query()
        .where_(OnboardingTemplate::ID.eq(template.id))
        .update(t.db(), &[("version", &2_i64), ("name_en", &"Driver v2")])
        .await
        .unwrap();
    wl_hr::template::TemplateStep::query()
        .where_(wl_hr::template::TemplateStep::TEMPLATE_ID.eq(template.id))
        .update(t.db(), &[("title_en", &"Renamed")])
        .await
        .unwrap();
    assert_eq!(
        t.step(&onboarding, "Read the handbook").await.title_en,
        "Read the handbook"
    );
    let reloaded = Onboarding::find_or_404(t.db(), onboarding.id)
        .await
        .unwrap();
    let applied = reloaded.templates(t.db()).await.unwrap();
    assert_eq!(
        applied
            .iter()
            .find(|a| a.template_id == template.id)
            .unwrap()
            .template_version,
        1
    );
}

/// The employee page lists the onboarding with its steps and assignees; the
/// templates page lists the templates.
#[renox::test]
async fn hrm26_the_pages_show_the_onboarding_and_the_templates() {
    let t = boot().await;
    add_template(&t, driver_template()).await;
    let hana = t.user_with("Hana", "hr").await;
    let (ravi, _) = t.ravi().await;
    t.app.acting_as(&hana);
    t.app.get("/hr/employees").await.assert_ok();
    t.app
        .get(&format!("/hr/employees/{}", ravi.id))
        .await
        .assert_ok()
        .assert_see("Welcome and introductions")
        .assert_see("2026-11-09");
    t.app
        .get("/hr/onboarding-templates")
        .await
        .assert_ok()
        .assert_see("All staff induction")
        .assert_see("Driver");
    let nobody = t.user("Nobody").await;
    t.app.acting_as(&nobody);
    t.app.get("/hr/employees").await.assert_forbidden();
}

/// A manual start through the page by HR; an unknown employee is a 404.
#[renox::test]
async fn hrm26_hr_starts_a_template_by_hand_on_the_page() {
    let t = boot().await;
    let supervisor = add_template(&t, supervisor_template()).await;
    let hana = t.user_with("Hana", "hr").await;
    let employee = employees::create(
        t.state(),
        NewEmployee::new(PartyId(80), "supervisor", nov(1))
            .scope(branch())
            .status(EmployeeStatus::Active),
    )
    .await
    .unwrap();
    t.app.acting_as(&hana);
    let path = format!("/hr/employees/{}/onboardings", employee.id);
    t.app
        .post(
            &path,
            &[
                ("template", &supervisor.id.to_string()),
                ("start_date", "2026-12-01"),
            ],
        )
        .await
        .assert_ok();
    let onboardings = t.onboardings(&employee).await;
    assert_eq!(onboardings.len(), 1);
    assert_eq!(
        onboardings[0].started_on,
        renox::chrono::NaiveDate::from_ymd_opt(2026, 12, 1).unwrap()
    );
    // A user with no right to manage onboarding is refused.
    let nobody = t.user("Nobody").await;
    t.app.acting_as(&nobody);
    t.app
        .post(&path, &[("template", &supervisor.id.to_string())])
        .await
        .assert_forbidden();
}
