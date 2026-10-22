//! BL-MPW-19 (#433), `HRM.RUL.15` with the real wl-attendance, wl-hr, wl-portal
//! and wl-notify in the host of a manpower agency: a timesheet of a deployment
//! that the client does not answer is escalated at E = 6 days to the client's
//! escalation contact (from the contract) and the account manager's manager,
//! with a task to collect a signed sheet, and is deemed approved at the
//! contract's N days, telling the client approver; without the clause nothing
//! is deemed and without an escalation contact only the manager is told; the
//! days in force when the timesheet was submitted apply.

mod manpower_support;
mod support;

use host::roles::{HR, MEMBER};
use manpower_support::{account, account_named, boot, deployment, portal_grant, worker};
use renox::chrono::Duration;
use renox::db::now;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use support::{day, messages_to, messages_to_party, person, tenant};
use wl_attendance::escalation;
use wl_attendance::timesheet::{ApprovalBasis, ApproverKind};
use wl_attendance::{Timesheet, TimesheetStatus};
use wl_hr::{EmployeeStatus, NewEmployee, employees};
use wl_kernel::PartyId;
use wl_kernel::tickets::task_creator;
use wl_portal::{AccessGrant, GrantStatus};

const ESCALATED: &str = "attendance.timesheet_escalated";
const DEEMED: &str = "attendance.timesheet_deemed";

/// The people and records of a contract with a worker on site.
struct Cast {
    amal: User,
    mona: User,
    carl: User,
    contact: PartyId,
    contract: i64,
}

/// Amal is the account manager (an employee of HR), Mona the HR user who looks
/// after her, Carl the client's approver in the portal, Fiona a possible
/// escalation contact, and the worker is on site under the contract.
async fn cast(app: &TestApp) -> (Cast, wl_kernel::workforce::EmployeeId, i64) {
    let a = account(app).await;
    let mona = person(app, "Mona", HR).await;
    let party = tenant(app, "Amal Account").await;
    employees::create(
        app.state(),
        NewEmployee::new(party, "account_manager", day(-300))
            .status(EmployeeStatus::Active)
            .user(a.amal.id),
    )
    .await
    .unwrap();
    let carl = person(app, "Carl", MEMBER).await;
    let carl_party = tenant(app, "Carl Client").await;
    let mut grant = AccessGrant {
        user_id: Some(carl.id),
        person_party_id: carl_party.0,
        party_id: a.client.0,
        audience: "tenant".into(),
        roles: "approver".into(),
        source: "invitation".into(),
        valid_from: day(-30),
        status: GrantStatus::Active,
        ..AccessGrant::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    grant
        .insert_in_state(&mut tx, GrantStatus::Active, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let contact = tenant(app, "Fiona Finance").await;
    let w = worker(app, "Wasim Khan").await;
    let d = deployment(
        app,
        &a,
        &a.site,
        w,
        "security_guard",
        manpower::DeploymentStatus::Active,
        day(-50),
        Some(day(200)),
    )
    .await;
    (
        Cast {
            amal: a.amal,
            mona,
            carl,
            contact,
            contract: a.contract.id,
        },
        w,
        d.id,
    )
}

/// The account manager fills the contract's fields on its page.
async fn fill(app: &TestApp, cast: &Cast, days: &str, contact: Option<PartyId>) {
    app.acting_as(&cast.amal);
    let contact = contact.map(|party| party.0.to_string()).unwrap_or_default();
    app.htmx()
        .post(
            &format!("/manpower/contracts/{}/timesheet-terms", cast.contract),
            &[
                ("deemed_approval_days", days),
                ("escalation_contact_party_id", contact.as_str()),
            ],
        )
        .await
        .assert_status(303);
}

/// A timesheet of the worker's deployment, submitted `ago` days before the
/// real day, waiting for the client approver.
async fn waiting(
    app: &TestApp,
    worker: wl_kernel::workforce::EmployeeId,
    deployment: i64,
    ago: i64,
) -> Timesheet {
    let context = wl_kernel::Subject::new(manpower::deployment::SUBJECT_TYPE, deployment);
    let mut sheet = Timesheet::regular(worker, day(-6), day(0)).in_context(&context);
    sheet.approver_kind = ApproverKind::External;
    sheet.approver_role = Some("client_approver".into());
    sheet.status = TimesheetStatus::Submitted;
    sheet.submitted_at = Some(now() - Duration::days(ago));
    let mut tx = app.db().begin().await.unwrap();
    sheet
        .insert_in_state(&mut tx, TimesheetStatus::Submitted, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    sheet
}

async fn reload(app: &TestApp, sheet: &Timesheet) -> Timesheet {
    Timesheet::find_or_404(app.db(), sheet.id).await.unwrap()
}

async fn run_on(app: &TestApp, after: i64) -> escalation::Report {
    escalation::run(app.state(), day(after)).await.unwrap()
}

/// AC1 with the real modules: the contract has deemed days 10 and the escalation
/// contact Fiona. A timesheet submitted today is untouched at day 5, escalated
/// at day 6 (Fiona and the account manager's manager Mona are told, the account
/// manager gets the task to collect a signed sheet, the state does not change),
/// not escalated twice, and deemed approved at day 10 (basis `deemed`, Carl the
/// client approver and Amal told).
#[renox::test]
async fn an_unanswered_timesheet_is_escalated_at_six_days_and_deemed_approved_at_the_contract_days()
{
    let app = boot().await;
    let (cast, worker, deployment) = cast(&app).await;
    fill(&app, &cast, "10", Some(cast.contact)).await;
    let sheet = waiting(&app, worker, deployment, 0).await;

    let report = run_on(&app, 5).await;
    assert_eq!((report.escalated, report.deemed), (0, 0));

    let report = run_on(&app, 6).await;
    assert_eq!((report.escalated, report.deemed), (1, 0));
    assert_eq!(
        reload(&app, &sheet).await.status,
        TimesheetStatus::Submitted
    );
    assert_eq!(
        messages_to_party(&app, ESCALATED, cast.contact).await.len(),
        1,
        "the escalation contact of the contract"
    );
    assert_eq!(
        messages_to(&app, ESCALATED, &cast.mona).await.len(),
        1,
        "the account manager's manager"
    );
    let key = escalation::task_key(sheet.id);
    assert!(
        task_creator(app.state())
            .find_open(app.state(), &key)
            .await
            .unwrap()
            .is_some(),
        "the task to collect a signed sheet"
    );
    assert_eq!(run_on(&app, 7).await.escalated, 0, "once");
    assert_eq!(
        messages_to_party(&app, ESCALATED, cast.contact).await.len(),
        1
    );

    assert_eq!(run_on(&app, 9).await.deemed, 0);
    let report = run_on(&app, 10).await;
    assert_eq!(report.deemed, 1);
    let deemed = reload(&app, &sheet).await;
    assert_eq!(deemed.status, TimesheetStatus::Approved);
    assert_eq!(deemed.approval_basis, Some(ApprovalBasis::Deemed));
    assert_eq!(messages_to(&app, DEEMED, &cast.carl).await.len(), 1);
    assert_eq!(messages_to(&app, DEEMED, &cast.amal).await.len(), 1);
}

/// AC2 and AC3 with the real modules: a contract without deemed days and
/// without an escalation contact. The escalation at day 6 goes to the account
/// manager's manager only, and the timesheet is never deemed approved however
/// long it waits.
#[renox::test]
async fn without_the_fields_only_the_manager_is_told_and_nothing_is_ever_deemed() {
    let app = boot().await;
    let (cast, worker, deployment) = cast(&app).await;
    let sheet = waiting(&app, worker, deployment, 0).await;

    let report = run_on(&app, 6).await;
    assert_eq!(report.escalated, 1);
    assert_eq!(messages_to(&app, ESCALATED, &cast.mona).await.len(), 1);
    assert!(
        messages_to_party(&app, ESCALATED, cast.contact)
            .await
            .is_empty()
    );

    assert_eq!(run_on(&app, 40).await.deemed, 0);
    assert_eq!(run_on(&app, 400).await.deemed, 0);
    assert_eq!(
        reload(&app, &sheet).await.status,
        TimesheetStatus::Submitted
    );
    assert!(messages_to(&app, DEEMED, &cast.carl).await.is_empty());
}

/// AC4 with the real modules: the timesheet was submitted three days ago under
/// deemed days 10; the account manager changes them to 5 afterwards. The
/// timesheet already submitted is not deemed at day 5 but at day 10 counted
/// from its submission; one submitted after the change is deemed at day 5.
#[renox::test]
async fn a_change_of_the_days_leaves_a_timesheet_already_submitted_as_it_was() {
    let app = boot().await;
    let (cast, worker, deployment) = cast(&app).await;
    // The contract was made with 10 days, three days before the real day.
    manpower::ClientContract::query()
        .where_(manpower::ClientContract::ID.eq(cast.contract))
        .update(app.db(), &[("deemed_approval_days", &Some(10_i64))])
        .await
        .unwrap();
    let old = waiting(&app, worker, deployment, 3).await;
    fill(&app, &cast, "5", None).await;
    let new = waiting(&app, worker, deployment, 0).await;

    // Day 5 from today: the new sheet is five days old, the old one eight.
    let report = run_on(&app, 5).await;
    assert_eq!(report.deemed, 1);
    assert_eq!(reload(&app, &new).await.status, TimesheetStatus::Approved);
    assert_eq!(reload(&app, &old).await.status, TimesheetStatus::Submitted);
    // Day 7: the old one is ten days old.
    assert_eq!(run_on(&app, 7).await.deemed, 1);
    assert_eq!(reload(&app, &old).await.status, TimesheetStatus::Approved);
}

/// BL-MPW-19 (#433), who may approve: a timesheet of Gulf Tower's deployment is
/// approved only by an approver of Gulf Tower. The approver of Marina Bay,
/// who holds the same global role `client_approver`, and a member of staff
/// without the permission are refused and the sheet stays `submitted`; Gulf
/// Tower's own approver approves it. The other client's approver meets 404 as of
/// BL-MPW-28 (`MPW.ACL.01`: a sheet of another client is not found, never a 403
/// with data); the member of staff stays 403.
#[renox::test]
async fn a_client_approver_approves_only_the_timesheets_of_their_own_client() {
    let app = boot().await;
    let (cast, worker, deployment) = cast(&app).await;
    let gulf = manpower::ClientContract::find_or_404(app.db(), cast.contract)
        .await
        .unwrap()
        .client();
    renox::auth::permissions::define_role(app.db(), "client_approver", &[])
        .await
        .unwrap();
    let marina = account_named(&app, "Marina Bay LLC", "CC-0002", "Mira").await;
    let own = person(&app, "Gus", "client_approver").await;
    portal_grant(&app, &own, gulf, "approver").await;
    let other = person(&app, "Marco", "client_approver").await;
    portal_grant(&app, &other, marina.client, "approver").await;
    let staff = person(&app, "Sami", MEMBER).await;
    let sheet = waiting(&app, worker, deployment, 0).await;
    let path = format!("/timesheets/{}/approve", sheet.id);

    for (refused, status) in [(&other, 404), (&staff, 403)] {
        app.acting_as(refused);
        app.htmx().post(&path, &[]).await.assert_status(status);
        assert_eq!(
            reload(&app, &sheet).await.status,
            TimesheetStatus::Submitted
        );
    }
    app.acting_as(&own);
    app.htmx().post(&path, &[]).await.assert_ok();
    let approved = reload(&app, &sheet).await;
    assert_eq!(approved.status, TimesheetStatus::Approved);
    assert_eq!(approved.approved_by, Some(own.id));
}
