//! BL-HRM-29 (#219): a template with *blocks activation* makes `activate`
//! wait for its required steps; the refusal lists the open steps and their
//! assignees. Nothing blocks when the setting is off, for steps of other
//! templates, for optional steps and for a manual onboarding.

mod support;

use renox::prelude::*;
use wl_hr::onboarding::{self, StartRequest, StartTrigger};
use wl_hr::steps::{self, DoneInput};
use wl_hr::template::{StepInput, TemplateInput};
use wl_hr::{Employee, EmployeeStatus, NewEmployee, OnboardingTemplate, employees};
use wl_kernel::PartyId;

use support::{Booted, add_template, boot, boot_with, branch, nov};

/// "Site induction", blocking, with a required safety briefing owned by the
/// manager and an optional site tour.
fn site_induction(blocks: bool) -> TemplateInput {
    TemplateInput {
        blocks_activation: blocks,
        ..TemplateInput::for_role("Site induction", "worker")
            .step(StepInput::task("Safety briefing", "manager", 3))
            .step(StepInput::task("Site tour", "manager", 3).optional())
    }
}

/// A worker hired on 2 Nov at branch B, in onboarding.
async fn worker(t: &Booted, party: i64, role: &str) -> Employee {
    employees::create(
        t.state(),
        NewEmployee::new(PartyId(party), role, nov(2))
            .scope(branch())
            .status(EmployeeStatus::Onboarding),
    )
    .await
    .unwrap()
}

async fn status_of(t: &Booted, employee: &Employee) -> EmployeeStatus {
    Employee::find_or_404(t.db(), employee.id).await.unwrap().status
}

fn activate_path(employee: &Employee) -> String {
    format!("/hr/employees/{}/transitions/activate", employee.id)
}

/// AC1 and AC2: with "Site induction" blocking and its required step open,
/// `activate` is refused and lists the step and who does it; once another HR
/// user waives the step with a reason, `activate` runs.
#[renox::test]
async fn hrm29_a_blocking_template_with_an_open_required_step_refuses_activation() {
    let t = boot_with(wl_hr::WlHr::new(), false).await;
    let hana = t.user_with("Hana", "hr").await;
    let mia = t.user_in_branch("Mia", "manager").await;
    add_template(&t, site_induction(true)).await;
    let sami = worker(&t, 70, "worker").await;
    assert_eq!(sami.status, EmployeeStatus::Onboarding);

    t.app.acting_as(&hana);
    let refused = t
        .app
        .htmx()
        .post(&activate_path(&sami), &[("reason", "")])
        .await;
    refused.assert_status(422);
    let body = refused.text();
    assert!(body.contains("Safety briefing"), "{body}");
    assert!(body.contains("Site induction"), "{body}");
    assert!(body.contains("Mia"), "{body}");
    assert!(!body.contains("Site tour"), "an optional step does not block: {body}");
    assert_eq!(status_of(&t, &sami).await, EmployeeStatus::Onboarding);
    assert!(t.activated.all().is_empty());

    // HR user Hana waives it with a reason (she is not an assignee).
    let onboarding = t.onboardings(&sami).await.remove(0);
    let step = t.step(&onboarding, "Safety briefing").await;
    assert_eq!(step.assignees(t.db()).await.unwrap(), [mia.id]);
    steps::waive(t.state(), step.id, hana.id, "Done at the previous site")
        .await
        .unwrap()
        .unwrap();
    t.app
        .post(&activate_path(&sami), &[("reason", "")])
        .await
        .assert_status(303);
    assert_eq!(status_of(&t, &sami).await, EmployeeStatus::Active);
    assert_eq!(t.activated.all().len(), 1);
}

/// A step done by its assignee lets `activate` run as well.
#[renox::test]
async fn hrm29_a_done_step_does_not_block() {
    let t = boot_with(wl_hr::WlHr::new(), false).await;
    let hana = t.user_with("Hana", "hr").await;
    let mia = t.user_in_branch("Mia", "manager").await;
    add_template(&t, site_induction(true)).await;
    let sami = worker(&t, 70, "worker").await;
    let onboarding = t.onboardings(&sami).await.remove(0);
    let step = t.step(&onboarding, "Safety briefing").await;
    steps::mark_done(t.state(), step.id, mia.id, DoneInput::default())
        .await
        .unwrap()
        .unwrap();
    t.app.acting_as(&hana);
    t.app
        .post(&activate_path(&sami), &[("reason", "")])
        .await
        .assert_status(303);
    assert_eq!(status_of(&t, &sami).await, EmployeeStatus::Active);
}

/// AC3: with the setting off (the default), `activate` ignores open
/// onboarding steps.
#[renox::test]
async fn hrm29_with_the_setting_off_activation_ignores_open_steps() {
    let t = boot_with(wl_hr::WlHr::new(), false).await;
    let hana = t.user_with("Hana", "hr").await;
    t.user_in_branch("Mia", "manager").await;
    let template = add_template(&t, site_induction(false)).await;
    assert!(!template.blocks_activation);
    let sami = worker(&t, 70, "worker").await;
    t.app.acting_as(&hana);
    t.app
        .post(&activate_path(&sami), &[("reason", "")])
        .await
        .assert_status(303);
    assert_eq!(status_of(&t, &sami).await, EmployeeStatus::Active);

    // The setting is read when `activate` runs: switching it on blocks the
    // next employee, switching it off lifts the block again.
    OnboardingTemplate::query()
        .where_(OnboardingTemplate::ID.eq(template.id))
        .update(t.db(), &[("blocks_activation", &true)])
        .await
        .unwrap();
    let tara = worker(&t, 71, "worker").await;
    t.app
        .htmx()
        .post(&activate_path(&tara), &[("reason", "")])
        .await
        .assert_status(422);
    OnboardingTemplate::query()
        .where_(OnboardingTemplate::ID.eq(template.id))
        .update(t.db(), &[("blocks_activation", &false)])
        .await
        .unwrap();
    t.app
        .post(&activate_path(&tara), &[("reason", "")])
        .await
        .assert_status(303);
    assert_eq!(status_of(&t, &tara).await, EmployeeStatus::Active);
}

/// AC4: one onboarding with "All staff" (not blocking, a required step open)
/// and "Site induction" (blocking, every required step done): `activate`
/// runs.
#[renox::test]
async fn hrm29_only_steps_of_the_blocking_template_count() {
    let t = boot().await;
    let hana = t.user_with("Hana", "hr").await;
    let mia = t.user_in_branch("Mia", "manager").await;
    add_template(&t, site_induction(true)).await;
    let sami = worker(&t, 70, "worker").await;
    let onboarding = t.onboardings(&sami).await.remove(0);
    assert_eq!(onboarding.templates(t.db()).await.unwrap().len(), 2);
    let open_all_staff = onboarding
        .steps(t.db())
        .await
        .unwrap()
        .into_iter()
        .filter(|s| s.required && s.status == wl_hr::StepStatus::Open)
        .count();
    assert!(open_all_staff > 1, "the induction has required steps open");

    let briefing = t.step(&onboarding, "Safety briefing").await;
    steps::mark_done(t.state(), briefing.id, mia.id, DoneInput::default())
        .await
        .unwrap()
        .unwrap();
    t.app.acting_as(&hana);
    t.app
        .post(&activate_path(&sami), &[("reason", "")])
        .await
        .assert_status(303);
    assert_eq!(status_of(&t, &sami).await, EmployeeStatus::Active);
}

/// AC5: a minimal record of a 07 operator in `onboarding`, no visa case,
/// labour contract or Emirates ID, and no blocking template: `activate` runs.
/// An onboarding started by hand never blocks, even from a blocking template.
#[renox::test]
async fn hrm29_a_minimal_record_and_a_manual_onboarding_do_not_block() {
    let t = boot().await;
    let hana = t.user_with("Hana", "hr").await;
    t.user_in_branch("Mia", "manager").await;
    let blocking = add_template(&t, site_induction(true)).await;
    let omar = worker(&t, 72, "operator").await;
    t.app.acting_as(&hana);
    t.app
        .post(&activate_path(&omar), &[("reason", "")])
        .await
        .assert_status(303);
    assert_eq!(status_of(&t, &omar).await, EmployeeStatus::Active);

    let lina = worker(&t, 73, "operator").await;
    onboarding::start(
        t.state(),
        StartRequest {
            employee_id: lina.id,
            trigger: StartTrigger::Manual,
            templates: vec![blocking.id],
            start_on: Some(nov(2)),
        },
    )
    .await
    .unwrap()
    .unwrap();
    t.app
        .post(&activate_path(&lina), &[("reason", "")])
        .await
        .assert_status(303);
    assert_eq!(status_of(&t, &lina).await, EmployeeStatus::Active);
}
