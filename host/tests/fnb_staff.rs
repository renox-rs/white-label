//! BL-FNB-17 (#381), 06 §7.2, `HRM.ONB.01`, `HRM.RUL.17`: the onboarding of a
//! line cook runs with the 06 checklist: a step past its due date is reminded
//! to the kitchen manager, the head chef or the cook who owns it on the first
//! run after the due date and then every second day, and HR and the kitchen
//! manager are told once after five days. The real wl-hr job on the real
//! host, the messages in the real wl-notify.

mod support;

use host::roles::{HEAD_CHEF, HR, KITCHEN_MANAGER, LINE_COOK};
use renox::chrono::Days;
use renox::prelude::*;
use support::*;
use wl_hr::{Onboarding, OnboardingStep, StepStatus, reminders};
use wl_kernel::documents::today;

const REMINDER: &str = "hr.onboarding_step_reminder";
const ESCALATED: &str = "hr.onboarding_step_escalated";

#[renox::test]
async fn a_late_step_of_the_line_cook_checklist_is_reminded_every_second_day_and_escalated_once() {
    let app = fnb_booted().await;
    let manager = person(&app, "Mounir", KITCHEN_MANAGER).await;
    let chef = person(&app, "Samir", HEAD_CHEF).await;
    let hr = person(&app, "Hana", HR).await;
    let cook = person(&app, "Rizwan", LINE_COOK).await;
    let party = tenant(&app, "Rizwan Ahmed").await;
    let employee = wl_hr::employees::create(
        app.state(),
        wl_hr::NewEmployee::new(party, "line_cook", today()).user(cook.id),
    )
    .await
    .unwrap();
    let onboarding = Onboarding::query()
        .where_(Onboarding::EMPLOYEE_ID.eq(employee.id))
        .first(app.db())
        .await
        .unwrap()
        .expect("the onboarding started");
    let steps = OnboardingStep::query()
        .where_(OnboardingStep::ONBOARDING_ID.eq(onboarding.id))
        .where_(OnboardingStep::STATUS.eq(StepStatus::Open))
        .get(app.db())
        .await
        .unwrap();
    let due = steps.iter().map(|s| s.due_date).min().expect("an open step");
    let step = steps.iter().find(|s| s.due_date == due).unwrap();

    // On the due date itself nothing is late.
    let early = reminders::run(app.state(), due).await.unwrap();
    app.run_all_jobs().await;
    assert_eq!(early.reminded, 0);
    for user in [&manager, &chef, &cook, &hr] {
        assert!(messages_to(&app, REMINDER, user).await.is_empty());
    }

    // The first run after it reminds the owner of the step.
    let first = reminders::run(app.state(), due + Days::new(1)).await.unwrap();
    app.run_all_jobs().await;
    assert!(first.reminded >= 1, "{first:?}");
    let owner = match step.owner.as_str() {
        "role:kitchen_manager" | "manager" => &manager,
        "role:head_chef" => &chef,
        _ => &cook,
    };
    let reminded = messages_to(&app, REMINDER, owner).await.len();
    assert!(
        reminded >= 1,
        "the owner ({}) of the step is reminded",
        step.owner
    );
    // The same day again sends nothing.
    reminders::run(app.state(), due + Days::new(1)).await.unwrap();
    app.run_all_jobs().await;
    assert_eq!(messages_to(&app, REMINDER, owner).await.len(), reminded);
    // Two days later the owner is reminded again.
    reminders::run(app.state(), due + Days::new(3)).await.unwrap();
    app.run_all_jobs().await;
    assert!(messages_to(&app, REMINDER, owner).await.len() > reminded);
    assert!(messages_to(&app, ESCALATED, &hr).await.is_empty());

    // Five days after the due date HR and the manager are told, once.
    let late = reminders::run(app.state(), due + Days::new(5)).await.unwrap();
    app.run_all_jobs().await;
    assert!(late.escalated >= 1, "{late:?}");
    let told = messages_to(&app, ESCALATED, &hr).await.len();
    assert!(told >= 1);
    assert!(!messages_to(&app, ESCALATED, &manager).await.is_empty());
    reminders::run(app.state(), due + Days::new(5)).await.unwrap();
    app.run_all_jobs().await;
    assert_eq!(
        messages_to(&app, ESCALATED, &hr).await.len(),
        told,
        "told once"
    );
}
