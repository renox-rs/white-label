//! BL-PRP-24 (#274, `PRP.ACL.15`) end to end on the Property client's seed:
//! a lifts planned visit is resolved only with the vendor's service report; a
//! user cannot cancel a fire and life safety planned visit, only reschedule it
//! with a reason; and when the lifts contract ends on 2027-06-30 the system
//! cancels the visit due 2027-07-20, the visit due 2027-06-20 stays open and
//! the coordinator's task offers only "reassign" for it, and the building
//! shows `no_contract`.

use host::roles::{ADMIN, COORDINATOR, TECHNICIAN};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::tickets::{Assignee, ComplianceFlagReason, OpenTicket};
use wl_kernel::{PartyId, PartyKind, Subject};
use wl_parties::{CompanyDetails, NewParty, create};
use wl_ticketing::contract_end::{self, CONTRACT_ENDED_REASON, task_key};
use wl_ticketing::flags::ComplianceFlag;
use wl_ticketing::{NewContract, ServiceContract, ServiceContracts, Ticket, TicketStatus, Tickets};

fn date(text: &str) -> NaiveDate {
    text.parse().expect("a date")
}

async fn booted() -> TestApp {
    let app = TestApp::new(host::app()).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    app
}

async fn person(app: &TestApp, name: &str, role: &str) -> User {
    let mut user = User::register(
        app.db(),
        name,
        &format!("{}@example.com", name.to_lowercase()),
        "secret123",
    )
    .await
    .unwrap();
    user.email_verified_at = Some(renox::db::now());
    user.save(app.db()).await.unwrap();
    host::roles::define(app.db()).await.unwrap();
    user.assign_role(app.db(), role).await.unwrap();
    user
}

async fn vendor(app: &TestApp) -> PartyId {
    create(
        app.state(),
        NewParty {
            kind: PartyKind::Company,
            legal_name_en: "LiftCo LLC".into(),
            company: Some(CompanyDetails::default()),
            source: "back_office".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .party_id()
}

fn tower_a_lifts() -> Subject {
    Subject::new("property.building_system", 4)
}

fn tower_a_fire() -> Subject {
    Subject::new("property.building_system", 6)
}

async fn visit(
    app: &TestApp,
    subject: Subject,
    category: &str,
    due: &str,
    assignee: Assignee,
    contract: Option<&ServiceContract>,
) -> Ticket {
    let mut request = OpenTicket::new(subject, category, "Planned visit")
        .planned_due(date(due))
        .assign_to(assignee);
    if let Some(contract) = contract {
        request = request.under_contract(contract.contract_id());
    }
    let opened = Tickets::open_at(app.state(), request, renox::db::now())
        .await
        .unwrap();
    Ticket::find_or_404(app.db(), opened.id.0).await.unwrap()
}

async fn status(app: &TestApp, ticket: &Ticket) -> TicketStatus {
    Ticket::find_or_404(app.db(), ticket.id)
        .await
        .unwrap()
        .status
}

#[renox::test]
async fn bl_prp_24_the_compliance_visit_guard_end_to_end() {
    let app = booted().await;
    let coordinator = person(&app, "Carla", COORDINATOR).await;
    let admin = person(&app, "Alia", ADMIN).await;
    let technician = person(&app, "Tim", TECHNICIAN).await;
    let vendor = vendor(&app).await;
    let by_vendor = Assignee::party(vendor);

    // 1. A lifts planned visit is resolved only with the service report.
    let lifts_visit = visit(
        &app,
        tower_a_lifts(),
        "lifts",
        "2027-03-01",
        Assignee::User { id: technician.id },
        None,
    )
    .await;
    app.acting_as(&technician);
    app.htmx()
        .post(&format!("/tickets/{}/actions/start", lifts_visit.id), &[])
        .await
        .assert_status(303);
    app.htmx()
        .post(&format!("/tickets/{}/actions/resolve", lifts_visit.id), &[])
        .await
        .assert_status(422);
    assert_eq!(status(&app, &lifts_visit).await, TicketStatus::InProgress);
    app.htmx()
        .post(
            &format!("/tickets/{}/service-report", lifts_visit.id),
            &[("reference", "SR-2027-031")],
        )
        .await
        .assert_ok();
    app.htmx()
        .post(&format!("/tickets/{}/actions/resolve", lifts_visit.id), &[])
        .await
        .assert_status(303);
    assert_eq!(status(&app, &lifts_visit).await, TicketStatus::Resolved);

    // 2. A fire and life safety planned visit cannot be cancelled by a user;
    // it is rescheduled to 2027-04-10 with a reason.
    let fire_visit = visit(
        &app,
        tower_a_fire(),
        "fire_life_safety",
        "2027-03-15",
        by_vendor,
        None,
    )
    .await;
    app.acting_as(&coordinator);
    app.htmx()
        .post(
            &format!("/tickets/{}/cancel", fire_visit.id),
            &[("reason", "not needed")],
        )
        .await
        .assert_status(422);
    assert_eq!(status(&app, &fire_visit).await, TicketStatus::Assigned);
    app.htmx()
        .post(
            &format!("/tickets/{}/reschedule", fire_visit.id),
            &[("date", "2027-04-10"), ("reason", "panel shut for repairs")],
        )
        .await
        .assert_ok();
    let moved = Ticket::find_or_404(app.db(), fire_visit.id).await.unwrap();
    assert_eq!(moved.planned_due_date, Some(date("2027-04-10")));
    assert_eq!(moved.status, TicketStatus::Assigned);

    // 3. The lifts contract ends on 2027-06-30.
    let contract = ServiceContracts::create_draft(
        app.state(),
        NewContract {
            vendor,
            title: "Lifts maintenance".into(),
            start_date: date("2026-07-01"),
            end_date: date("2027-06-30"),
            renewal_notice_days: 60,
            sla_policy_id: None,
        },
    )
    .await
    .unwrap();
    ServiceContracts::add_coverage(app.db(), &contract, tower_a_lifts(), "lifts")
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
    let june = visit(
        &app,
        tower_a_lifts(),
        "lifts",
        "2027-06-20",
        by_vendor,
        Some(&contract),
    )
    .await;
    let july = visit(
        &app,
        tower_a_lifts(),
        "lifts",
        "2027-07-20",
        by_vendor,
        Some(&contract),
    )
    .await;

    let report = contract_end::run(app.state(), date("2027-07-01"))
        .await
        .unwrap();
    assert_eq!(report.expired, 1);

    // The 07-20 visit is cancelled by the system, the 06-20 visit stays.
    let july_now = Ticket::find_or_404(app.db(), july.id).await.unwrap();
    assert_eq!(july_now.status, TicketStatus::Cancelled);
    assert_eq!(
        july_now.cancel_reason.as_deref(),
        Some(CONTRACT_ENDED_REASON)
    );
    assert_eq!(status(&app, &june).await, TicketStatus::Assigned);

    // The coordinator's one task lists only the open 06-20 visit and offers
    // only "reassign" for it.
    let tasks = Ticket::query()
        .where_(Ticket::SOURCE_KEY.eq(task_key(contract.id).as_str()))
        .get(app.db())
        .await
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].description.contains(&june.number));
    assert!(!tasks[0].description.contains(&july.number));
    app.acting_as(&coordinator);
    let page = app
        .get(&format!("/service-contracts/{}", contract.id))
        .await;
    page.assert_ok()
        .assert_see(&format!("reassign-visit-{}", june.id))
        .assert_dont_see(&format!("cancel-visit-{}", june.id));
    // A user's cancel of that visit is refused too.
    app.htmx()
        .post(
            &format!("/tickets/{}/cancel", june.id),
            &[("reason", "contract ended")],
        )
        .await
        .assert_status(422);
    assert_eq!(status(&app, &june).await, TicketStatus::Assigned);

    // The building shows no_contract.
    let flags = ComplianceFlag::open(app.db()).await.unwrap();
    assert_eq!(flags.len(), 1);
    assert_eq!(flags[0].reason, ComplianceFlagReason::NoContract);
    assert_eq!(flags[0].subject(), tower_a_lifts());
}
