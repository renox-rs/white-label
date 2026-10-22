//! BL-HRM-10 (#200), `POR.ACL.01`, in the real host of a manpower agency with
//! wl-hr and wl-portal: after `exit` of an employee, the portal of their user
//! is read-only (a page reads, an action is refused with 403) until the day
//! it closes, 60 days later (exit 2026-11-10: read-only until 2027-01-08,
//! closed from 2027-01-09), when the page is refused too.

mod support;

use host::roles::ADMIN;
use renox::chrono::{NaiveDate, NaiveDateTime};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use support::{noon_clock, person, tenant};
use wl_hr::{EmployeeStatus, FinalSettlement, SettlementStatus, VisaCase, VisaCaseStatus};
use wl_kernel::documents::today;
use wl_kernel::{BusinessCalendar, ClientConfig, PartyId};
use wl_portal::{NewGrant, PortalAccess, PortalAction, PortalGuard};

/// A manpower agency with HR and the portal on.
const CLIENT: &str = r#"
[client]
name = "Gulf Manpower Services LLC"
app = "manpower"

[modules]
enabled = ["parties", "documents", "notify", "invoicing", "ticketing", "payments", "hr", "attendance", "portal"]

[[legal_entities]]
code = "GMS"
name_en = "Gulf Manpower Services LLC"
default = true
"#;

/// A portal page of another module: runs the portal guard for a message.
struct Probe;

async fn message(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(party): Path<i64>,
) -> Result<String> {
    PortalGuard::require(&state, &lang, &user, PartyId(party), PortalAction::Message).await?;
    Ok("done".to_owned())
}

impl Module for Probe {
    fn name(&self) -> &'static str {
        "probe"
    }

    fn routes(&self) -> Routes {
        Routes::new()
            .post("/probe/portal/{party}/message", message)
            .require_auth()
    }
}

async fn boot() -> TestApp {
    let config = ClientConfig::parse(CLIENT).unwrap();
    let app = TestApp::new(
        host::app_for(config)
            .unwrap()
            .provide(noon_clock())
            .module(Probe),
    )
    .await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    app
}

/// Moves the clock to noon of `day` (Asia/Dubai, ahead of the real clock).
fn travel_to(app: &TestApp, day: &str) {
    let local = NaiveDateTime::parse_from_str(&format!("{day} 12:00"), "%Y-%m-%d %H:%M").unwrap();
    let ahead = (BusinessCalendar::default().from_local(local) - renox::db::now()).num_seconds();
    assert!(ahead > 0, "{day} is ahead of the clock");
    app.travel_back();
    app.travel(std::time::Duration::from_secs(ahead.unsigned_abs()));
}

/// An absconded worker with a portal account that acts for their own party,
/// the cancellation case completed and the settlement approved: `exit` can run.
async fn absconded_worker(app: &TestApp) -> (wl_hr::Employee, User, PartyId) {
    let party = tenant(app, "Wasim Khan").await;
    let mut employee = wl_hr::Employee {
        party_id: party.0,
        job_role: "security_guard".into(),
        job_title: "Guard".into(),
        hire_date: today(),
        status: EmployeeStatus::Absconded,
        ..wl_hr::Employee::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    employee
        .insert_in_state(&mut tx, EmployeeStatus::Absconded, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let account = person(app, "Wasim", host::roles::MEMBER).await;
    renox::db::sql("UPDATE wl_hr_employees SET user_id = ? WHERE id = ?")
        .bind(account.id)
        .bind(employee.id)
        .execute(app.db())
        .await
        .unwrap();
    PortalAccess::register(
        app.state(),
        account.id,
        NewGrant::new(party, party, "tenant", &["primary_contact"]),
    )
    .await
    .unwrap();
    let mut case = VisaCase {
        employee_id: employee.id,
        case_type: "cancellation".into(),
        variant: "standard".into(),
        status: VisaCaseStatus::Completed,
        ..VisaCase::default()
    };
    let mut settlement = FinalSettlement {
        employee_id: employee.id,
        reason: "absconding".into(),
        payee: "employee".into(),
        statement_version: 1,
        status: SettlementStatus::Approved,
        ..FinalSettlement::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    case.insert_in_state(&mut tx, VisaCaseStatus::Completed, "fixture")
        .await
        .unwrap();
    settlement
        .insert_in_state(&mut tx, SettlementStatus::Approved, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    (employee, account, party)
}

/// AC4: HR runs `exit` on 2026-11-10; the worker's portal then answers a page
/// (GET, 404 for a section nobody registered: the grant let them through) and
/// refuses an action (POST, 403) up to 2027-01-08, and refuses both from
/// 2027-01-09.
#[renox::test]
async fn hrm10_after_the_exit_the_portal_reads_but_does_not_act_until_it_closes() {
    let app = boot().await;
    let (employee, account, party) = absconded_worker(&app).await;
    let hr = person(&app, "Hana", ADMIN).await;
    let page = format!("/portal/{}/nothing", party.0);
    let action = format!("/probe/portal/{}/message", party.0);

    travel_to(&app, "2026-11-10");
    app.acting_as(&hr);
    app.htmx()
        .post(
            &format!("/hr/employees/{}/transitions/exit", employee.id),
            &[("reason", "")],
        )
        .await
        .assert_status(303);
    let employee = wl_hr::Employee::find_or_404(app.db(), employee.id)
        .await
        .unwrap();
    assert_eq!(employee.status, EmployeeStatus::Exited);

    app.acting_as(&account);
    app.get(&page).await.assert_status(404);
    app.post(&action, &[]).await.assert_forbidden();

    let day = |d: &str| NaiveDate::parse_from_str(d, "%Y-%m-%d").unwrap();
    assert_eq!(
        day("2027-01-08"),
        day("2026-11-10") + renox::chrono::Days::new(59)
    );
    travel_to(&app, "2027-01-08");
    app.acting_as(&account);
    app.get(&page).await.assert_status(404);
    app.post(&action, &[]).await.assert_forbidden();

    travel_to(&app, "2027-01-09");
    app.acting_as(&account);
    app.get(&page).await.assert_forbidden();
    app.post(&action, &[]).await.assert_forbidden();
    app.travel_back();
}

/// Control: a worker who has not exited acts in the portal as before.
#[renox::test]
async fn hrm10_a_worker_who_has_not_exited_acts_in_the_portal() {
    let app = boot().await;
    let party = tenant(&app, "Wajid Ali").await;
    let account = person(&app, "Wajid", host::roles::MEMBER).await;
    PortalAccess::register(
        app.state(),
        account.id,
        NewGrant::new(party, party, "tenant", &["primary_contact"]),
    )
    .await
    .unwrap();
    app.acting_as(&account);
    app.post(&format!("/probe/portal/{}/message", party.0), &[])
        .await
        .assert_ok();
}
