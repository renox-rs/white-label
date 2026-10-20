//! The 04 Accounting client's deadline task settings and roles in the host
//! (BL-TKT-09 #154 Must-not, BL-ACC-13 #337, BL-ACC-12 #336): a started task
//! is not waived (*waive a started task* off), `close_missed` takes only a
//! reason of the 04 list, the engagement manager waives and closes as
//! missed, and the MLRO sees the tasks and the staff tasks assigned to them.

mod support;

use host::roles::{ACCOUNTANT, COMPLIANCE, ENGAGEMENT_MANAGER, PARTNER};
use renox::chrono::NaiveDateTime;
use renox::prelude::*;
use renox::testing::TestApp;
use support::{noon_clock, person};
use wl_kernel::deadlines::{DeadlineTaskStatus, NewDeadlineTask, deadline_tasks};
use wl_kernel::{BusinessCalendar, ClientConfig, Subject};
use wl_ticketing::DeadlineTask;

/// The demo accounting firm.
const ACCOUNTING_CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-accounting/client.toml"
);

/// The accounting host, with `wl:install` and the seeds run as the demo does.
async fn booted() -> TestApp {
    let config = ClientConfig::load(ACCOUNTING_CLIENT).expect("the accounting client");
    let app = TestApp::new(
        host::app_for(config)
            .expect("the app")
            .provide(noon_clock()),
    )
    .await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    app
}

/// Engagement 12.
fn e12() -> Subject {
    Subject::new("accounting.engagement", 12)
}

/// `by` runs `action` on task `id` through its route at `at` (Asia/Dubai,
/// `2026-11-20 09:00`, ahead of the real clock), with `pairs`.
async fn act(
    app: &TestApp,
    by: &User,
    id: i64,
    action: &str,
    at: &str,
    pairs: &[(&str, &str)],
) -> renox::testing::TestResponse {
    let local = NaiveDateTime::parse_from_str(at, "%Y-%m-%d %H:%M").unwrap();
    let ahead = (BusinessCalendar::default().from_local(local) - renox::db::now()).num_seconds();
    assert!(ahead > 0, "{at} is ahead of the clock");
    app.travel_back();
    app.travel(std::time::Duration::from_secs(ahead.unsigned_abs()));
    app.acting_as(by);
    let response = app
        .htmx()
        .post(&format!("/deadline-tasks/{id}/actions/{action}"), pairs)
        .await;
    app.travel_back();
    response
}

async fn status(app: &TestApp, id: i64) -> DeadlineTaskStatus {
    DeadlineTask::find_or_404(app.db(), id)
        .await
        .unwrap()
        .status
}

/// A1, A2: in 04 a started task is not waived; the engagement manager
/// waives an upcoming one; `close_missed` takes a reason of the 04 list
/// only; the MLRO and the partner see the tasks.
#[renox::test]
async fn the_04_deadline_task_settings_and_roles_are_in_force() {
    let app = booted().await;
    let aisha = person(&app, "Aisha", ACCOUNTANT).await;
    let mona = person(&app, "Mona", ENGAGEMENT_MANAGER).await;
    let mariam = person(&app, "Mariam", COMPLIANCE).await;
    let pia = person(&app, "Pia", PARTNER).await;
    let tasks = deadline_tasks(app.state());
    let new = |title: &str, due: &str| {
        NewDeadlineTask::new(e12(), "vat_return", title, due.parse().unwrap()).owner_user(aisha.id)
    };
    let started = tasks
        .create(app.state(), new("CT return", "2026-12-31"))
        .await
        .unwrap()
        .expect("ticketing is on");
    let upcoming = tasks
        .create(app.state(), new("VAT Q4", "2027-01-28"))
        .await
        .unwrap()
        .unwrap();
    let late = tasks
        .create(app.state(), new("VAT Q3", "2026-11-28"))
        .await
        .unwrap()
        .unwrap();

    act(&app, &aisha, started.0, "start", "2026-11-20 09:00", &[])
        .await
        .assert_ok();
    // A1: *waive a started task* is off for 04.
    act(
        &app,
        &mona,
        started.0,
        "waive",
        "2026-11-20 10:00",
        &[("reason", "Not needed")],
    )
    .await
    .assert_status(422);
    assert_eq!(
        status(&app, started.0).await,
        DeadlineTaskStatus::InProgress
    );
    // A2: the engagement manager waives an upcoming task; the accountant may
    // not.
    act(
        &app,
        &aisha,
        upcoming.0,
        "waive",
        "2026-11-20 10:00",
        &[("reason", "Nil return not needed")],
    )
    .await
    .assert_forbidden();
    act(
        &app,
        &mona,
        upcoming.0,
        "waive",
        "2026-11-20 10:00",
        &[("reason", "Nil return not needed")],
    )
    .await
    .assert_ok();
    assert_eq!(
        status(&app, upcoming.0).await,
        DeadlineTaskStatus::NotRequired
    );

    // ACC.RUL.04 and close_missed with the 04 reasons only.
    app.at_travelled_time(wl_ticketing::deadlines::mark_overdue(
        app.state(),
        "2026-11-29".parse().unwrap(),
    ))
    .await
    .unwrap();
    assert_eq!(status(&app, late.0).await, DeadlineTaskStatus::Overdue);
    act(
        &app,
        &mona,
        late.0,
        "close_missed",
        "2026-12-02 09:00",
        &[("reason", "Forgot")],
    )
    .await
    .assert_status(422);
    act(
        &app,
        &mona,
        late.0,
        "close_missed",
        "2026-12-02 09:00",
        &[("reason_code", "client_filed_itself")],
    )
    .await
    .assert_ok();
    let closed = DeadlineTask::find_or_404(app.db(), late.0).await.unwrap();
    assert_eq!(closed.status, DeadlineTaskStatus::Missed);
    assert_eq!(closed.closed_reason.as_deref(), Some("Client filed itself"));

    // The MLRO and the partner see the tasks; the MLRO the staff tasks too.
    for viewer in [&mariam, &pia, &mona] {
        app.acting_as(viewer);
        app.get("/deadline-tasks?subject_type=accounting.engagement&subject_id=12")
            .await
            .assert_ok()
            .assert_see("CT return");
    }
    app.acting_as(&mariam);
    app.get("/tickets").await.assert_ok();
}

/// `ACC.ACL.05`: the MLRO's tasks about a client are tickets of the
/// confidential category, which only the compliance role (the MLRO and the
/// deputy) can see: not the engagement manager, the partner or the
/// accountant. The category is seeded confidential in the accounting host.
#[renox::test]
async fn the_mlros_tasks_are_confidential_tickets_only_the_compliance_role_sees() {
    let holds = |role: &str| {
        host::roles::ROLES
            .iter()
            .find(|(name, _)| *name == role)
            .is_some_and(|(_, grants)| {
                grants.contains(&wl_ticketing::permissions::CONFIDENTIAL_VIEW)
            })
    };
    assert!(holds(COMPLIANCE));
    for role in [ENGAGEMENT_MANAGER, PARTNER, ACCOUNTANT] {
        assert!(!holds(role), "{role} sees confidential tickets");
    }
    let app = booted().await;
    let category =
        wl_ticketing::Category::by_key(app.db(), wl_kernel::tickets::CONFIDENTIAL_TASK_CATEGORY)
            .await
            .unwrap()
            .expect("the confidential task category is seeded");
    assert!(category.confidential && category.active);
}
