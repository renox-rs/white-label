//! BL-PRP-27 (#277), the ticketing settings of the Property client with the
//! real modules of the host: a lifts planned visit missed by 3 days notifies
//! the property manager and flags the building (`WRK.RUL.14`); a vendor that
//! has not started 2 hours after the visit time is recorded as a no-show and
//! the coordinator may reassign it (`WRK.RUL.19`); when a contract ends the
//! property manager and the company admin are told, the coordinator has a task
//! due in one working day and the building shows `no_contract`
//! (`WRK.RUL.18`).

mod support;

use host::roles::{ADMIN, COORDINATOR, PROPERTY_MANAGER};
use renox::chrono::Duration;
use renox::prelude::*;
use support::*;
use wl_kernel::BusinessCalendar;
use wl_kernel::tickets::{Assignee, ComplianceFlagReason, OpenTicket};
use wl_ticketing::contract_end::{self, NO_CONTRACT_NOTICE, task_key};
use wl_ticketing::flags::ComplianceFlag;
use wl_ticketing::no_show::{self, NOTIFICATION_TYPE as NO_SHOW};
use wl_ticketing::visit_check::{self, NOTIFICATION_TYPE as VISIT_MISSED};
use wl_ticketing::{NewContract, NoShow, ServiceContracts, Ticket};

/// A lifts visit of the vendor planned 3 days ago: missed today, not 2 days
/// ago; the property manager and the coordinator are told and the building
/// is flagged `visit_missed`. A cleaning visit planned as long ago tells the
/// coordinator only and flags nothing.
#[renox::test]
async fn a_lifts_visit_missed_by_three_days_notifies_the_property_manager_and_flags_the_building() {
    let app = booted().await;
    let coordinator = person(&app, "Carla", COORDINATOR).await;
    let manager = person(&app, "Pam", PROPERTY_MANAGER).await;
    let vendor = company(&app, "LiftCo LLC").await;
    let (_tower, subject) = building(&app, "TWR-A").await;
    let lifts = vendor_ticket(&app, &subject, "lifts", vendor, Some(day(-3))).await;
    let cleaning = vendor_ticket(&app, &subject, "general", vendor, Some(day(-3))).await;

    // Two days after the due date: not missed yet.
    let early = visit_check::run(app.state(), day(-1)).await.unwrap();
    assert_eq!(early.missed, 0);
    assert!(ComplianceFlag::open(app.db()).await.unwrap().is_empty());

    // Three days after: reported.
    let report = visit_check::run(app.state(), day(0)).await.unwrap();
    assert_eq!(report.missed, 2);
    assert_eq!(messages_to(&app, VISIT_MISSED, &coordinator).await.len(), 2);
    let to_manager = messages_to(&app, VISIT_MISSED, &manager).await;
    assert_eq!(
        to_manager.len(),
        1,
        "only the lifts visit is compliance-critical"
    );
    assert!(
        to_manager[0].data.0["number"] == lifts.number.as_str(),
        "{}",
        to_manager[0].data.0
    );
    assert!(
        to_manager
            .iter()
            .all(|m| m.data.0["number"] != cleaning.number.as_str())
    );
    let flags = ComplianceFlag::open(app.db()).await.unwrap();
    assert_eq!(flags.len(), 1);
    assert_eq!(flags[0].reason, ComplianceFlagReason::VisitMissed);
    assert_eq!(flags[0].subject(), subject);

    // Once: the next night adds nothing.
    visit_check::run(app.state(), day(1)).await.unwrap();
    assert_eq!(messages_to(&app, VISIT_MISSED, &manager).await.len(), 1);
}

/// A vendor not started 2 hours after the visit time is a no-show, not before;
/// the coordinator is told (the property manager too for lifts) and the page
/// offers to reassign it or to set a new time.
#[renox::test]
async fn a_vendor_not_started_two_hours_after_the_visit_is_a_no_show() {
    let app = booted().await;
    let coordinator = person(&app, "Carla", COORDINATOR).await;
    let manager = person(&app, "Pam", PROPERTY_MANAGER).await;
    let vendor = company(&app, "LiftCo LLC").await;
    let (_tower, subject) = building(&app, "TWR-A").await;
    let lifts = vendor_ticket(&app, &subject, "lifts", vendor, None).await;
    let at = renox::db::now();
    Ticket::where_eq("id", lifts.id)
        .update(app.db(), &[("visit_at", &Some(at))])
        .await
        .unwrap();

    let early = at + Duration::hours(2) - Duration::minutes(1);
    assert_eq!(no_show::run(app.state(), early).await.unwrap().recorded, 0);
    assert_eq!(
        no_show::run(app.state(), at + Duration::hours(2))
            .await
            .unwrap()
            .recorded,
        1
    );
    let rows = NoShow::query().get(app.db()).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].ticket_id, lifts.id);
    assert_eq!(messages_to(&app, NO_SHOW, &coordinator).await.len(), 1);
    assert_eq!(
        messages_to(&app, NO_SHOW, &manager).await.len(),
        1,
        "lifts are compliance-critical: the property manager is told too"
    );

    app.acting_as(&coordinator);
    app.get(&format!("/tickets/{}", lifts.id))
        .await
        .assert_ok()
        .assert_see(&format!("assign-ticket-{}", lifts.id))
        .assert_see(&format!("new-time-{}", lifts.id))
        .assert_see("did not turn up");
}

/// When the lifts contract ends, the property manager and the company admin
/// are told, the coordinator's task is due in one working day and the
/// building shows `no_contract`.
#[renox::test]
async fn an_ended_lifts_contract_tells_the_manager_and_the_admin_and_flags_the_building() {
    let app = booted().await;
    person(&app, "Carla", COORDINATOR).await;
    let manager = person(&app, "Pam", PROPERTY_MANAGER).await;
    let admin = person(&app, "Alia", ADMIN).await;
    let vendor = company(&app, "LiftCo LLC").await;
    let (_tower, subject) = building(&app, "TWR-A").await;
    let contract = ServiceContracts::create_draft(
        app.state(),
        NewContract {
            vendor,
            title: "Lifts maintenance".into(),
            start_date: day(0),
            end_date: day(30),
            renewal_notice_days: 60,
            sla_policy_id: None,
        },
    )
    .await
    .unwrap();
    ServiceContracts::add_coverage(app.db(), &contract, subject.clone(), "lifts")
        .await
        .unwrap();
    app.acting_as(&admin);
    app.htmx()
        .post(
            &format!("/service-contracts/{}/activate", contract.id),
            &[("x", "y")],
        )
        .await
        .assert_ok();
    // A visit planned before the end stays open: the coordinator gets a task.
    wl_ticketing::Tickets::open_at(
        app.state(),
        OpenTicket::new(subject.clone(), "lifts", "Quarterly visit")
            .planned_due(day(10))
            .assign_to(Assignee::party(vendor))
            .under_contract(contract.contract_id()),
        renox::db::now(),
    )
    .await
    .unwrap();

    let report = contract_end::run(app.state(), day(31)).await.unwrap();
    assert_eq!(report.expired, 1);
    assert_eq!(report.ended[0].left.len(), 1);
    assert_eq!(
        messages_to(&app, NO_CONTRACT_NOTICE, &manager).await.len(),
        1
    );
    assert_eq!(messages_to(&app, NO_CONTRACT_NOTICE, &admin).await.len(), 1);
    let flags = ComplianceFlag::open(app.db()).await.unwrap();
    assert_eq!(flags.len(), 1);
    assert_eq!(flags[0].reason, ComplianceFlagReason::NoContract);
    assert_eq!(flags[0].subject(), subject);

    // The coordinator's task: due one working day after it was opened.
    let tasks = Ticket::query()
        .where_(Ticket::SOURCE_KEY.eq(task_key(contract.id).as_str()))
        .get(app.db())
        .await
        .unwrap();
    assert_eq!(tasks.len(), 1);
    let due = tasks[0].due_at.expect("a due time");
    let expected = BusinessCalendar::default().add_working_days(renox::db::now(), 1);
    assert!(
        (due - expected).num_seconds().abs() < 60,
        "due {due} against {expected}"
    );
}
