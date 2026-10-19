//! BL-HRM-26 (#216): the minimal employee record and its `activate`; the
//! default induction template; the install command is idempotent.

mod support;

use renox::prelude::*;
use wl_hr::{Employee, EmployeeStatus, OnboardingTemplate, StepKind, seed};

use support::{boot, boot_with};

/// `activate` takes a record from `onboarding` to `active` for
/// `employees.manage`, and emits `EmployeeActivated`; a minimal record has no
/// statutory item to check.
#[renox::test]
async fn hrm26_activate_takes_an_onboarding_record_to_active() {
    let t = boot().await;
    let (ravi, _) = t.ravi().await;
    assert_eq!(ravi.status, EmployeeStatus::Onboarding);
    let hana = t.user_with("Hana", "hr").await;
    let viewer = t.user_with("Vera", "manager").await;

    t.app.acting_as(&viewer);
    t.app
        .post(&format!("/hr/employees/{}/transitions/activate", ravi.id), &[("reason", "")])
        .await
        .assert_forbidden();
    assert_eq!(
        Employee::find_or_404(t.db(), ravi.id).await.unwrap().status,
        EmployeeStatus::Onboarding
    );

    t.app.acting_as(&hana);
    t.app
        .post(&format!("/hr/employees/{}/transitions/activate", ravi.id), &[("reason", "")])
        .await
        .assert_status(303);
    let after = Employee::find_or_404(t.db(), ravi.id).await.unwrap();
    assert_eq!(after.status, EmployeeStatus::Active);
    let activated = t.activated.all();
    assert_eq!(activated.len(), 1);
    assert_eq!(activated[0].employee.0, ravi.id);
    assert_eq!(activated[0].hire_date, support::nov(2));
    assert_eq!(activated[0].scope, Some(support::branch()));
    // Only a record in onboarding is activated.
    t.app
        .htmx()
        .post(&format!("/hr/employees/{}/transitions/activate", ravi.id), &[("reason", "")])
        .await
        .assert_status(422);
}

/// The seeded template is the *all staff* induction of §7, and the installer
/// stores it once.
#[renox::test]
async fn hrm26_the_default_induction_template_is_seeded_once() {
    let t = boot().await;
    let templates = OnboardingTemplate::query().get(t.db()).await.unwrap();
    assert_eq!(templates.len(), 1);
    let induction = &templates[0];
    assert!(induction.all_staff && induction.active && !induction.blocks_activation);
    assert_eq!(induction.version, 1);
    let steps = induction.steps(t.db()).await.unwrap();
    let shape: Vec<(&str, i64, StepKind)> = steps
        .iter()
        .map(|s| (s.owner.as_str(), s.due_days, s.kind))
        .collect();
    assert_eq!(
        shape,
        [
            ("manager", 1, StepKind::Task),
            ("hr", 1, StepKind::Task),
            ("manager", 3, StepKind::Checklist),
            ("hr", 3, StepKind::Checklist),
            ("employee", 7, StepKind::SopAcknowledgement),
        ]
    );
    assert!(steps.iter().all(|s| s.required && !s.evidence_required));
    // A second run adds nothing.
    assert_eq!(seed::install(t.db()).await.unwrap().total(), 0);
    let bare = boot_with(wl_hr::WlHr::new(), false).await;
    assert_eq!(seed::install(bare.db()).await.unwrap().total(), 1);
    assert_eq!(seed::install(bare.db()).await.unwrap().total(), 0);
}
