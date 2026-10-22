//! BL-MPW-20 (#434), the onboarding and SOP values of the seeded manpower
//! agency with the real wl-hr and wl-documents: hiring a worker creates the
//! worker checklist with tasks for HR, the account manager and the employee
//! (`HRM.RUL.16`); an overdue step is reminded on the first run and again 2
//! days later (`HRM.RUL.17`); HR cannot mark the employee's own step without
//! the signature and the witness (`HRM.ACL.10`); a new version of an HR-owned
//! SOP asks its readers to acknowledge within 7 days (`DOC.RUL.04`).

mod manpower_support;
mod support;

use host::roles::{ACCOUNT_MANAGER, ADMIN, HR};
use manpower_support::boot_seeded_probe;
use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use renox::testing::TestApp;
use support::{messages_to, person, tenant};
use wl_documents::acknowledgements::{AckStatus, Acknowledgements};
use wl_documents::{Document, Sop};
use wl_hr::{
    EmployeeStatus, NewEmployee, Onboarding, OnboardingStep, StepStatus, employees, reminders,
};
use wl_kernel::documents::today;

fn at(day: NaiveDate, plus: u64) -> NaiveDate {
    day.checked_add_days(Days::new(plus)).unwrap()
}

/// A worker hired in `onboarding`: the employee, their login and onboarding.
async fn hired_worker(app: &TestApp, name: &str) -> (wl_hr::Employee, User, Onboarding) {
    let user = User::register(
        app.db(),
        name,
        &format!("{}@example.com", name.to_lowercase()),
        "secret123",
    )
    .await
    .unwrap();
    let party = tenant(app, name).await;
    let employee = employees::create(
        app.state(),
        NewEmployee::new(party, "worker", today())
            .user(user.id)
            .status(EmployeeStatus::Onboarding),
    )
    .await
    .unwrap();
    let onboarding = Onboarding::query()
        .where_(Onboarding::EMPLOYEE_ID.eq(employee.id))
        .first(app.db())
        .await
        .unwrap()
        .expect("hiring starts the onboarding");
    (employee, user, onboarding)
}

async fn step(app: &TestApp, onboarding: &Onboarding, title: &str) -> OnboardingStep {
    onboarding
        .steps(app.db())
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.title_en == title)
        .unwrap_or_else(|| panic!("the step {title}"))
}

/// AC11, `HRM.RUL.16`: hiring a worker creates the worker checklist; the
/// contract and documents step is HR's, the client-site rules step the account
/// manager's, the employee's own steps the employee's.
#[renox::test]
async fn hiring_a_worker_creates_the_checklist_with_tasks_for_hr_the_account_manager_and_the_employee()
 {
    let app = boot_seeded_probe().await;
    let hr = person(&app, "Hana", HR).await;
    let am = person(&app, "Amal", ACCOUNT_MANAGER).await;
    let (_employee, worker, onboarding) = hired_worker(&app, "Wasim").await;

    assert_eq!(onboarding.start_trigger, "hire");
    let template = wl_hr::OnboardingTemplate::query()
        .where_(wl_hr::OnboardingTemplate::NAME_EN.eq("Worker onboarding"))
        .first(app.db())
        .await
        .unwrap()
        .unwrap();
    assert!(
        onboarding
            .templates(app.db())
            .await
            .unwrap()
            .iter()
            .any(|t| t.template_id == template.id)
    );
    let hr_step = step(&app, &onboarding, "Contract, visa and ID documents").await;
    assert_eq!(hr_step.assignees(app.db()).await.unwrap(), [hr.id]);
    let site_rules = step(&app, &onboarding, "Client-site rules and site pass").await;
    assert_eq!(site_rules.assignees(app.db()).await.unwrap(), [am.id]);
    let own = step(&app, &onboarding, "Upload your personal documents").await;
    assert_eq!(own.assignees(app.db()).await.unwrap(), [worker.id]);
    assert_eq!(own.due_date, at(today(), 3));
}

/// AC11, `HRM.RUL.17`: the worker's own step is due 3 days after hiring. The
/// first run after the due date reminds the worker, the next day does not,
/// and 2 days after the first reminder the worker is reminded again.
#[renox::test]
async fn an_overdue_step_is_reminded_on_the_first_run_and_again_two_days_later() {
    let app = boot_seeded_probe().await;
    let (_employee, worker, onboarding) = hired_worker(&app, "Wasim").await;
    let own = step(&app, &onboarding, "Upload your personal documents").await;
    let due = own.due_date;
    let reminders_to = || async {
        messages_to(&app, "hr.onboarding_step_reminder", &worker)
            .await
            .len()
    };
    let last = || async {
        OnboardingStep::find_or_404(app.db(), own.id)
            .await
            .unwrap()
            .last_reminded_on
    };

    reminders::run(app.state(), due).await.unwrap();
    assert_eq!(reminders_to().await, 0, "not overdue on the due date");
    reminders::run(app.state(), at(due, 1)).await.unwrap();
    assert_eq!(last().await, Some(at(due, 1)), "the first run after");
    assert_eq!(reminders_to().await, 1);
    reminders::run(app.state(), at(due, 2)).await.unwrap();
    assert_eq!(reminders_to().await, 1, "the next day nothing");
    reminders::run(app.state(), at(due, 3)).await.unwrap();
    assert_eq!(last().await, Some(at(due, 3)), "2 days later again");
    assert_eq!(reminders_to().await, 2);
}

/// AC11, `HRM.ACL.10`: HR cannot mark the worker's own step; in person it
/// needs the worker's signature, and then HR is the witness.
#[renox::test]
async fn hr_cannot_mark_the_workers_own_step_without_the_signature_and_a_witness() {
    let app = boot_seeded_probe().await;
    let hr = person(&app, "Hana", HR).await;
    let (_employee, worker, onboarding) = hired_worker(&app, "Wasim").await;
    let own = step(&app, &onboarding, "Upload your personal documents").await;
    let open = || async { OnboardingStep::find_or_404(app.db(), own.id).await.unwrap() };

    app.acting_as(&hr);
    let by_hr = app
        .htmx()
        .post(&format!("/hr/steps/{}/done", own.id), &[("note", "done")])
        .await;
    assert!(
        [403, 422].contains(&by_hr.status.as_u16()),
        "refused: {}",
        by_hr.status
    );
    app.htmx()
        .post(
            &format!("/hr/steps/{}/in-person", own.id),
            &[("note", "done"), ("signature", "  ")],
        )
        .await
        .assert_status(422);
    assert_eq!(open().await.status, StepStatus::Open);

    app.htmx()
        .post(
            &format!("/hr/steps/{}/in-person", own.id),
            &[
                ("note", "done"),
                ("signature", "data:image/png;base64,AAAA"),
            ],
        )
        .await
        .assert_ok();
    let done = open().await;
    assert_eq!(done.status, StepStatus::Done);
    assert_eq!(done.witness_id, Some(hr.id));
    assert_eq!(done.done_by, Some(worker.id), "recorded for the worker");
}

/// Uploads a new version of the SOP as `by` and accepts it.
async fn publish_version(app: &TestApp, by: &User, sop: &Sop) -> Document {
    app.acting_as(by);
    let uploaded = app
        .post(
            &format!("/probe/sops/{}/versions", sop.id),
            &[("file_name", "hr-policies.pdf")],
        )
        .await;
    uploaded.assert_ok();
    let version = Document::find_or_404(app.db(), uploaded.text().parse().unwrap())
        .await
        .unwrap();
    app.htmx()
        .post(
            &format!("/documents/sop-versions/{}/accept", version.id),
            &[],
        )
        .await
        .assert_ok();
    Document::find_or_404(app.db(), version.id).await.unwrap()
}

/// AC11, `DOC.RUL.04`: HR owns the worker conduct SOP; each new version asks
/// the workers, its required readers, to acknowledge within 7 days, again for
/// the second version.
#[renox::test]
async fn a_new_version_of_the_hr_sop_asks_the_workers_to_acknowledge_within_seven_days() {
    let app = boot_seeded_probe().await;
    // The owner role is HR; managing SOPs is the admin's permission.
    let owner = person(&app, "Hana", HR).await;
    owner.assign_role(app.db(), ADMIN).await.unwrap();
    let (employee, worker, _) = hired_worker(&app, "Wasim").await;
    let sop = Sop::query()
        .where_(Sop::CODE.eq("SOP-MPW-HR-POLICIES"))
        .first(app.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sop.owner_role, "hr");

    let first = publish_version(&app, &owner, &sop).await;
    let asked = Acknowledgements::of_version(app.db(), sop.id, first.id)
        .await
        .unwrap();
    let mine = asked
        .iter()
        .find(|a| a.employee_id == employee.id)
        .expect("the worker is asked");
    assert_eq!(mine.status, AckStatus::Pending);
    assert_eq!(mine.due_date, at(today(), 7));

    // The worker acknowledges in their own session; a second version asks again.
    Acknowledgements::open(app.state(), mine.id, worker.id, first.id)
        .await
        .unwrap()
        .unwrap();
    app.acting_as(&worker);
    app.htmx()
        .post(
            &format!("/documents/acknowledgements/{}/acknowledge", mine.id),
            &[],
        )
        .await
        .assert_ok();
    let second = publish_version(&app, &owner, &sop).await;
    let again = Acknowledgements::of_version(app.db(), sop.id, second.id)
        .await
        .unwrap();
    let asked = again
        .iter()
        .find(|a| a.employee_id == employee.id)
        .expect("asked again");
    assert_eq!(asked.status, AckStatus::Pending);
    assert_eq!(asked.due_date, at(today(), 7));
}
