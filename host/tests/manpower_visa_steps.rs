//! BL-MPW-18 (#432), `MPW.RUL.14` with the real wl-hr and wl-notify in the
//! host of a manpower agency: a visa case step that passes its target tells
//! the PRO the day after the target, HR and the MD seven days later, once
//! each; a step done or a case cancelled gets nothing.

mod manpower_support;
mod support;

use host::roles::{ADMIN, HR, PRO};
use manpower::visa_steps::{self, Report};
use manpower_support::{boot, worker};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use support::{messages_to, person};
use wl_hr::visa::{CANCEL_BY_SYSTEM, TYPE_NEW};
use wl_hr::{VisaCase, VisaCaseStep};
use wl_kernel::workforce::EmployeeId;

fn nov(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
}

struct People {
    pro: User,
    hr: User,
    md: User,
}

/// The MD: the kernel picks the holder of `md`, else the first company admin.
async fn md_of(app: &TestApp) -> User {
    person(app, "Basel", ADMIN).await;
    let id = wl_kernel::role_tasks::assignee(app.db(), manpower::permissions::MD_ROLE)
        .await
        .unwrap()
        .expect("an MD");
    User::find_or_404(app.db(), id).await.unwrap()
}

async fn people(app: &TestApp) -> People {
    People {
        pro: person(app, "Pam", PRO).await,
        hr: person(app, "Hana", HR).await,
        md: md_of(app).await,
    }
}

/// A *new* case of `who` whose entry permit step was done on 1 Nov, so the
/// medical test step (target `target`) started that day. Returns the case and
/// the medical step.
async fn new_case(app: &TestApp, who: EmployeeId, target: Option<i64>) -> (VisaCase, VisaCaseStep) {
    let case = VisaCase::create(
        app.db(),
        VisaCase {
            employee_id: who.0,
            case_type: TYPE_NEW.to_owned(),
            ..VisaCase::default()
        },
    )
    .await
    .unwrap();
    VisaCaseStep::create(
        app.db(),
        VisaCaseStep {
            case_id: case.id,
            step_type: "entry_permit".to_owned(),
            done_on: Some(nov(1)),
            ..VisaCaseStep::default()
        },
    )
    .await
    .unwrap();
    let medical = VisaCaseStep::create(
        app.db(),
        VisaCaseStep {
            case_id: case.id,
            step_type: "medical_fitness".to_owned(),
            target_days: target,
            ..VisaCaseStep::default()
        },
    )
    .await
    .unwrap();
    (case, medical)
}

async fn count(app: &TestApp, user: &User) -> usize {
    messages_to(app, "manpower.notice", user).await.len()
}

/// AC1: the medical test step (target 5 days) started 1 Nov and is not done:
/// nothing on 6 Nov, the PRO on 7 Nov, HR and the MD on 14 Nov (not on 13
/// Nov), each once, however often the run repeats.
#[renox::test]
async fn the_pro_is_told_the_day_after_the_target_and_hr_and_the_md_a_week_later() {
    let app = boot().await;
    let p = people(&app).await;
    let w = worker(&app, "Wasim Khan").await;
    new_case(&app, w, Some(5)).await;

    let report = visa_steps::run_on(app.state(), nov(6)).await.unwrap();
    assert_eq!(report, Report::default());
    assert_eq!(count(&app, &p.pro).await, 0);

    let report = visa_steps::run_on(app.state(), nov(7)).await.unwrap();
    assert_eq!(
        report,
        Report {
            first: 1,
            second: 0
        }
    );
    assert_eq!(count(&app, &p.pro).await, 1);
    assert_eq!(count(&app, &p.hr).await, 0);
    let text = messages_to(&app, "manpower.notice", &p.pro).await[0]
        .data
        .0
        .to_string();
    assert!(text.contains("medical_fitness") && text.contains("not a legal deadline"));

    visa_steps::run_on(app.state(), nov(7)).await.unwrap();
    visa_steps::run_on(app.state(), nov(13)).await.unwrap();
    assert_eq!(count(&app, &p.pro).await, 1, "the PRO is told once");
    assert_eq!(count(&app, &p.hr).await, 0);

    let report = visa_steps::run_on(app.state(), nov(14)).await.unwrap();
    assert_eq!(
        report,
        Report {
            first: 0,
            second: 1
        }
    );
    assert_eq!(count(&app, &p.hr).await, 1);
    assert_eq!(count(&app, &p.md).await, 1);

    visa_steps::run_on(app.state(), nov(14)).await.unwrap();
    visa_steps::run_on(app.state(), nov(20)).await.unwrap();
    assert_eq!(count(&app, &p.pro).await, 1);
    assert_eq!(count(&app, &p.hr).await, 1);
    assert_eq!(count(&app, &p.md).await, 1);
}

/// AC2: the step done on 10 Nov gets no second notice on 14 Nov.
#[renox::test]
async fn a_step_done_before_the_second_notice_gets_none() {
    let app = boot().await;
    let p = people(&app).await;
    let w = worker(&app, "Wasim Khan").await;
    let (_case, medical) = new_case(&app, w, Some(5)).await;

    visa_steps::run_on(app.state(), nov(7)).await.unwrap();
    assert_eq!(count(&app, &p.pro).await, 1);
    VisaCaseStep::query()
        .where_(VisaCaseStep::ID.eq(medical.id))
        .update(app.db(), &[("done_on", &Some(nov(10)))])
        .await
        .unwrap();

    visa_steps::run_on(app.state(), nov(14)).await.unwrap();
    assert_eq!(count(&app, &p.hr).await, 0);
    assert_eq!(count(&app, &p.md).await, 0);
}

/// AC3: a case that was cancelled gets nothing.
#[renox::test]
async fn a_cancelled_case_gets_no_notice() {
    let app = boot().await;
    let p = people(&app).await;
    let w = worker(&app, "Wasim Khan").await;
    let (case, _medical) = new_case(&app, w, Some(5)).await;
    let case = VisaCase::find_or_404(app.db(), case.id).await.unwrap();
    let _outcome = case
        .transition(CANCEL_BY_SYSTEM)
        .by_system()
        .reason("onboarding cancelled")
        .run(app.state())
        .await
        .unwrap();

    visa_steps::run_on(app.state(), nov(7)).await.unwrap();
    visa_steps::run_on(app.state(), nov(14)).await.unwrap();
    assert_eq!(count(&app, &p.pro).await, 0);
    assert_eq!(count(&app, &p.hr).await, 0);
    assert_eq!(count(&app, &p.md).await, 0);
}

/// A step without a target is never late (edge case), and a step whose
/// previous mandatory step is not done has not started.
#[renox::test]
async fn a_step_without_a_target_or_not_started_gets_no_notice() {
    let app = boot().await;
    let p = people(&app).await;
    let w = worker(&app, "Wasim Khan").await;
    new_case(&app, w, None).await;
    let other = worker(&app, "Omar Ali").await;
    let (case, _) = new_case(&app, other, Some(5)).await;
    VisaCaseStep::query()
        .where_(VisaCaseStep::CASE_ID.eq(case.id))
        .where_(VisaCaseStep::STEP_TYPE.eq("entry_permit"))
        .update(app.db(), &[("done_on", &Option::<NaiveDate>::None)])
        .await
        .unwrap();

    visa_steps::run_on(app.state(), nov(20)).await.unwrap();
    assert_eq!(count(&app, &p.pro).await, 0);
}
