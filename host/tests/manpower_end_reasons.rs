//! BL-MPW-14 (#428), `WRK.TKT.01`, `MPW.DEP.02`, `HRM.ACL.08`: with the real
//! wl-ticketing, wl-attendance and wl-notify in the host of a manpower agency,
//! the account manager ends a deployment for the client's rejection with the
//! conduct box ticked (the replacement request opens and a conduct complaint
//! ticket exists for the worker and the client) and ends a deployment entered
//! in error (its three recorded records are rejected; an accepted one is
//! rejected too, a locked one is left and the account manager is told).

mod manpower_support;
mod support;

use host::roles::SUPERVISOR;
use manpower::{DeploymentStatus, ManpowerRequest, RequestKind, RequestStatus};
use manpower_support::{account, all_valid, boot, deployment, post, reload, worker};
use renox::prelude::*;
use support::{day, messages_to, person};
use wl_attendance::{AttendanceRecord, AttendanceStatus, WorkLocation};
use wl_kernel::Subject;
use wl_ticketing::{Category, Ticket};

/// AC1 and AC2: D (cover on) is ended by A with *client rejected*, the note
/// and the conduct box: D is `ended`, the replacement request for one worker
/// from tomorrow is `open`, and a ticket of the category `conduct` about W is
/// requested by the client and assigned to A.
#[renox::test]
async fn a_rejected_worker_with_conduct_involved_opens_the_replacement_and_a_ticket() {
    let app = boot().await;
    let a = account(&app).await;
    let w = worker(&app, "Wasim Khan").await;
    all_valid(&app, w).await;
    let d = deployment(
        &app,
        &a,
        &a.site,
        w,
        "security_guard",
        DeploymentStatus::Active,
        day(-60),
        Some(day(200)),
    )
    .await;

    post(
        &app,
        &a.amal,
        &format!("/manpower/deployments/{}/end", d.id),
        &[
            ("reason", "client_rejected"),
            ("note", "late three times"),
            ("conduct", "on"),
        ],
    )
    .await
    .assert_status(303);

    let ended = reload(&app, d.id).await;
    assert_eq!(ended.status, DeploymentStatus::Ended);
    assert_eq!(ended.end_note.as_deref(), Some("late three times"));
    let covers = ManpowerRequest::query()
        .where_(ManpowerRequest::KIND.eq(RequestKind::Replacement))
        .get(app.db())
        .await
        .unwrap();
    assert_eq!(covers.len(), 1);
    assert_eq!(covers[0].status, RequestStatus::Open);
    assert_eq!(covers[0].start_date, day(1));
    assert_eq!(covers[0].end_date, Some(day(200)));

    let category = Category::by_key(app.db(), "conduct")
        .await
        .unwrap()
        .expect("the conduct category is seeded");
    let tickets = Ticket::query()
        .where_(Ticket::CATEGORY_ID.eq(category.id))
        .get(app.db())
        .await
        .unwrap();
    assert_eq!(tickets.len(), 1);
    let ticket = &tickets[0];
    assert_eq!(ticket.subject_type, "hr.employee");
    assert_eq!(ticket.subject_id, w.0);
    assert_eq!(ticket.requester_party_id, Some(a.client.0));
    assert_eq!(ticket.assignee_user_id, Some(a.amal.id));
    assert!(ticket.description.contains("late three times"));
    assert_eq!(ticket.source_type.as_deref(), Some("manpower.deployment"));

    // Without the box no ticket is opened.
    let w2 = worker(&app, "Omar Said").await;
    all_valid(&app, w2).await;
    let d2 = deployment(
        &app,
        &a,
        &a.site,
        w2,
        "security_guard",
        DeploymentStatus::Active,
        day(-60),
        Some(day(200)),
    )
    .await;
    post(
        &app,
        &a.amal,
        &format!("/manpower/deployments/{}/end", d2.id),
        &[("reason", "client_rejected"), ("note", "rude")],
    )
    .await
    .assert_status(303);
    assert_eq!(
        Ticket::query()
            .where_(Ticket::CATEGORY_ID.eq(category.id))
            .get(app.db())
            .await
            .unwrap()
            .len(),
        1
    );
}

/// The agency with D (active, five days on site) for W and the three records
/// the supervisor marked at its site: two shifts yesterday, one today.
async fn three_marked_records() -> (
    renox::testing::TestApp,
    manpower_support::Account,
    manpower::Deployment,
    Vec<AttendanceRecord>,
) {
    let app = boot().await;
    let a = account(&app).await;
    let supervisor = person(&app, "Sami", SUPERVISOR).await;
    let w = worker(&app, "Wasim Khan").await;
    all_valid(&app, w).await;
    let d = deployment(
        &app,
        &a,
        &a.site,
        w,
        "security_guard",
        DeploymentStatus::Active,
        day(-5),
        Some(day(200)),
    )
    .await;
    let location = WorkLocation::create(
        app.db(),
        WorkLocation::named("Tower A").in_scope(&Subject::new("manpower.site", a.site.id)),
    )
    .await
    .unwrap();
    let employee = w.0.to_string();
    let place = location.id.to_string();
    app.acting_as(&supervisor);
    // The window is today and yesterday: two shifts yesterday, one today.
    for (check_in, check_out) in [
        (format!("{}T07:00", day(-1)), format!("{}T11:00", day(-1))),
        (format!("{}T13:00", day(-1)), format!("{}T17:00", day(-1))),
        (format!("{}T07:00", day(0)), String::new()),
    ] {
        app.htmx()
            .post(
                "/attendance/mark",
                &[
                    ("employee_id", employee.as_str()),
                    ("location_id", place.as_str()),
                    ("check_in", check_in.as_str()),
                    ("check_out", check_out.as_str()),
                ],
            )
            .await
            .assert_ok();
    }
    let records = records_of(&app, w.0).await;
    assert_eq!(records.len(), 3);
    (app, a, d, records)
}

async fn records_of(app: &renox::testing::TestApp, employee: i64) -> Vec<AttendanceRecord> {
    AttendanceRecord::query()
        .where_(AttendanceRecord::EMPLOYEE_ID.eq(employee))
        .order_by(AttendanceRecord::ID)
        .get(app.db())
        .await
        .unwrap()
}

/// AC5, as the point reads: D has 3 attendance records at the site, none
/// reviewed (recorded or flagged); ending it with *entered in error* rejects
/// all three, and the account manager has nothing to adjust.
#[renox::test]
async fn ending_a_deployment_entered_in_error_rejects_its_three_recorded_records() {
    let (app, a, d, records) = three_marked_records().await;
    assert!(
        records.iter().all(|r| matches!(
            r.status,
            AttendanceStatus::Recorded | AttendanceStatus::Flagged
        )),
        "none is reviewed yet: {:?}",
        records.iter().map(|r| r.status).collect::<Vec<_>>()
    );

    post(
        &app,
        &a.amal,
        &format!("/manpower/deployments/{}/end", d.id),
        &[("reason", "entered_in_error")],
    )
    .await
    .assert_status(303);

    assert_eq!(reload(&app, d.id).await.status, DeploymentStatus::Ended);
    let after = records_of(&app, d.employee_id).await;
    assert_eq!(after.len(), 3);
    assert!(
        after.iter().all(|r| r.status == AttendanceStatus::Rejected),
        "all three are rejected: {:?}",
        after.iter().map(|r| r.status).collect::<Vec<_>>()
    );
    let staff = messages_to(&app, "manpower.notice", &a.amal).await;
    assert!(
        !staff
            .iter()
            .any(|m| m.data.0.to_string().contains("adjustment timesheet")),
        "nothing is left to adjust"
    );
}

/// AC5 and `HRM.ACL.08`: of the three records one was accepted in a review
/// (no timesheet has it beyond open) and one is `locked` in an approved
/// timesheet. The accepted one is rejected too, as the spec says only locked
/// hours need an adjustment; the locked one stays and the account manager is
/// told to make an adjustment timesheet.
#[renox::test]
async fn an_accepted_record_is_rejected_and_a_locked_one_is_left_for_an_adjustment() {
    let (app, a, d, records) = three_marked_records().await;
    for (record, status) in [
        (&records[0], AttendanceStatus::Accepted),
        (&records[1], AttendanceStatus::Locked),
    ] {
        AttendanceRecord::query()
            .where_(AttendanceRecord::ID.eq(record.id))
            .update(app.db(), &[("status", &status)])
            .await
            .unwrap();
    }

    post(
        &app,
        &a.amal,
        &format!("/manpower/deployments/{}/end", d.id),
        &[("reason", "entered_in_error")],
    )
    .await
    .assert_status(303);

    assert_eq!(reload(&app, d.id).await.status, DeploymentStatus::Ended);
    let after = records_of(&app, d.employee_id).await;
    assert_eq!(after[0].status, AttendanceStatus::Rejected);
    assert_eq!(
        after[1].status,
        AttendanceStatus::Locked,
        "left for an adjustment"
    );
    assert_eq!(after[2].status, AttendanceStatus::Rejected);
    let staff = messages_to(&app, "manpower.notice", &a.amal).await;
    assert!(
        staff
            .iter()
            .any(|m| m.data.0.to_string().contains("adjustment timesheet")),
        "the account manager is told"
    );
}
