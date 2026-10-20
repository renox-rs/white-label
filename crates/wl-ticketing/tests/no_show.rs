//! BL-TKT-05 (#150): `WRK.RUL.19`, a vendor that has not started a ticket two
//! clock hours after its visit time (or at its response due time without an
//! appointment) is recorded as a no-show against the vendor and the contract;
//! the coordinator may reassign it or set a new time, and the contract page
//! counts the no-shows.

mod support;

use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::PartyId;
use wl_kernel::tickets::{OpenTicket, ServiceContractId};
use wl_ticketing::no_show::{self, NOTIFICATION_TYPE};
use wl_ticketing::{Category, NoShow, ServiceContract, Ticket, TicketStatus, Tickets};

use support::contracts::{World, tower_a_cleaning, tower_a_fire_alarm, world};
use support::{COOLAIR, dubai, local};

/// A ticket on Tower A cleaning in `category`, assigned to CoolAir at 08:30
/// on 3 November with the visit at `visit` (`2026-11-03 10:00`), when there is
/// one.
async fn vendor_ticket(
    w: &World,
    category: &str,
    subject: wl_kernel::Subject,
    requester: Option<&renox::prelude::User>,
    visit: Option<&str>,
) -> Ticket {
    let mut request = OpenTicket::new(subject, category, "Chiller down");
    if let Some(requester) = requester {
        request = request.requester_user(requester.id);
    }
    let opened = Tickets::open_at(w.t.state(), request, dubai("2026-11-03", 8, 0))
        .await
        .expect("the ticket opens");
    let ticket = w.t.ticket(opened.id.0).await;
    let party = COOLAIR.0.to_string();
    let visit = visit.map(local);
    let mut pairs = vec![("party", party.as_str()), ("priority", "high")];
    if let Some(visit) = &visit {
        pairs.push(("visit", visit.as_str()));
    }
    w.t.assign(&w.coordinator, &ticket, "2026-11-03 08:30", &pairs)
        .await
        .assert_ok();
    w.t.reload(&ticket).await
}

async fn plumbing_contract(w: &World) -> ServiceContract {
    w.contract(
        COOLAIR,
        "2026-01-01",
        "2026-12-31",
        &[(tower_a_cleaning(), "plumbing")],
    )
    .await
}

/// AC1: TKT assigned to CoolAir under the contract with the visit at 10:00
/// and still `assigned` at 12:00: a no-show is recorded, `VendorNoShow` is
/// emitted and the coordinator is told, with the actions "Reassign" and "New
/// time" on the ticket.
#[renox::test]
async fn tkt05_a_vendor_that_has_not_started_two_hours_after_the_visit_is_a_no_show() {
    let w = world().await;
    let contract = plumbing_contract(&w).await;
    let ticket = vendor_ticket(
        &w,
        "plumbing",
        tower_a_cleaning(),
        None,
        Some("2026-11-03 10:00"),
    )
    .await;
    assert_eq!(
        ticket.contract_id,
        Some(contract.id),
        "handled under the contract"
    );

    let early = no_show::run(w.t.state(), dubai("2026-11-03", 11, 59))
        .await
        .unwrap();
    assert_eq!(early.recorded, 0);
    assert!(w.no_shows.all().is_empty());

    let report = no_show::run(w.t.state(), dubai("2026-11-03", 12, 0))
        .await
        .unwrap();
    assert_eq!(report.recorded, 1);
    let rows = NoShow::of_contract(w.t.db(), contract.id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].ticket_id, ticket.id);
    assert_eq!(rows[0].vendor(), COOLAIR);
    assert_eq!(rows[0].scheduled_at, dubai("2026-11-03", 10, 0));

    let events = w.no_shows.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].ticket.0, ticket.id);
    assert_eq!(events[0].vendor, COOLAIR);
    assert_eq!(events[0].contract, Some(ServiceContractId(contract.id)));
    assert_eq!(events[0].scheduled_at, dubai("2026-11-03", 10, 0));

    assert_eq!(w.told(NOTIFICATION_TYPE, &w.coordinator), 1);
    assert_eq!(
        w.told(NOTIFICATION_TYPE, &w.manager),
        0,
        "plumbing is not compliance-critical"
    );
    let notice = w.t.notices.of_type(NOTIFICATION_TYPE).remove(0);
    assert_eq!(notice.data["number"], ticket.number.as_str());
    assert_eq!(notice.data["link"], format!("/tickets/{}", ticket.id));
    assert_eq!(notice.data["scheduled"], "03 Nov 2026 10:00");

    // Nothing is reassigned by itself; the page offers both actions.
    assert_eq!(w.t.reload(&ticket).await.assignee_party_id, Some(COOLAIR.0));
    w.t.app.acting_as(&w.coordinator);
    w.t.app
        .get(&format!("/tickets/{}", ticket.id))
        .await
        .assert_ok()
        .assert_see(&format!("assign-ticket-{}", ticket.id))
        .assert_see(&format!("new-time-{}", ticket.id))
        .assert_see("did not turn up");
}

/// Running the rule again for the same occurrence records nothing twice.
#[renox::test]
async fn tkt05_one_occurrence_is_recorded_once() {
    let w = world().await;
    vendor_ticket(
        &w,
        "plumbing",
        tower_a_cleaning(),
        None,
        Some("2026-11-03 10:00"),
    )
    .await;
    let at = dubai("2026-11-03", 12, 0);
    assert_eq!(no_show::run(w.t.state(), at).await.unwrap().recorded, 1);
    assert_eq!(no_show::run(w.t.state(), at).await.unwrap().recorded, 0);
    assert_eq!(
        no_show::run(w.t.state(), dubai("2026-11-03", 18, 0))
            .await
            .unwrap()
            .recorded,
        0
    );
    assert_eq!(w.no_shows.all().len(), 1);
    assert_eq!(w.t.notices.of_type(NOTIFICATION_TYPE).len(), 1);
}

/// AC2: reassigning to "Imran" records `vendor_no_show`.
#[renox::test]
async fn tkt05_reassigning_after_a_no_show_records_the_reason() {
    let w = world().await;
    let ticket = vendor_ticket(
        &w,
        "plumbing",
        tower_a_cleaning(),
        None,
        Some("2026-11-03 10:00"),
    )
    .await;
    no_show::run(w.t.state(), dubai("2026-11-03", 12, 0))
        .await
        .unwrap();

    let imran = w.imran.id.to_string();
    w.t.assign(
        &w.coordinator,
        &ticket,
        "2026-11-03 12:05",
        &[("user", imran.as_str()), ("reason_code", "vendor_no_show")],
    )
    .await
    .assert_ok();
    let ticket = w.t.reload(&ticket).await;
    assert_eq!(ticket.assignee_user_id, Some(w.imran.id));
    let history = w.t.history(&ticket).await;
    assert_eq!(
        history.last().unwrap().reason_code.as_deref(),
        Some("vendor_no_show")
    );

    // He is not a vendor: nothing more is recorded for him.
    no_show::run(w.t.state(), dubai("2026-11-04", 12, 0))
        .await
        .unwrap();
    assert_eq!(w.no_shows.all().len(), 1);
}

/// AC2: setting 15:00 with CoolAir tells the tenant 15:00; the vendor can be
/// a no-show again two hours after the new time.
#[renox::test]
async fn tkt05_a_new_time_with_the_same_vendor_tells_the_requester() {
    let w = world().await;
    let tenant = w.t.user("Tina", &[wl_ticketing::permissions::CREATE]).await;
    let ticket = vendor_ticket(
        &w,
        "plumbing",
        tower_a_cleaning(),
        Some(&tenant),
        Some("2026-11-03 10:00"),
    )
    .await;
    no_show::run(w.t.state(), dubai("2026-11-03", 12, 0))
        .await
        .unwrap();

    w.t.app.acting_as(&w.coordinator);
    w.t.app
        .htmx()
        .post(
            &format!("/tickets/{}/new-time", ticket.id),
            &[("visit_at", "2026-11-03T15:00:00")],
        )
        .await
        .assert_ok();
    let moved = w.t.reload(&ticket).await;
    assert_eq!(moved.visit_at, Some(dubai("2026-11-03", 15, 0)));
    assert_eq!(moved.assignee_party_id, Some(COOLAIR.0), "the same vendor");
    let told = w.t.notices.of_type("ticketing.visit_rescheduled");
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].data["visit"], "03 Nov 2026 15:00");

    // A new occurrence two hours after the new time.
    assert_eq!(
        no_show::run(w.t.state(), dubai("2026-11-03", 16, 59))
            .await
            .unwrap()
            .recorded,
        0
    );
    assert_eq!(
        no_show::run(w.t.state(), dubai("2026-11-03", 17, 0))
            .await
            .unwrap()
            .recorded,
        1
    );
    assert_eq!(w.no_shows.all().len(), 2);
    assert_eq!(w.no_shows.all()[1].scheduled_at, dubai("2026-11-03", 15, 0));
}

/// A visit time moved before the trigger skips the old occurrence.
#[renox::test]
async fn tkt05_a_visit_moved_before_the_trigger_is_judged_by_its_new_time() {
    let w = world().await;
    let ticket = vendor_ticket(
        &w,
        "plumbing",
        tower_a_cleaning(),
        None,
        Some("2026-11-03 10:00"),
    )
    .await;
    let party = COOLAIR.0.to_string();
    w.t.assign(
        &w.coordinator,
        &ticket,
        "2026-11-03 09:00",
        &[
            ("party", party.as_str()),
            ("visit", local("2026-11-03 14:00").as_str()),
        ],
    )
    .await
    .assert_ok();
    assert_eq!(
        no_show::run(w.t.state(), dubai("2026-11-03", 12, 0))
            .await
            .unwrap()
            .recorded,
        0
    );
    assert_eq!(
        no_show::run(w.t.state(), dubai("2026-11-03", 16, 0))
            .await
            .unwrap()
            .recorded,
        1
    );
    assert_eq!(w.no_shows.all()[0].scheduled_at, dubai("2026-11-03", 14, 0));
}

/// AC3: for a fire-safety ticket the property manager is told too.
#[renox::test]
async fn tkt05_a_compliance_critical_no_show_tells_the_manager_too() {
    let w = world().await;
    vendor_ticket(
        &w,
        "fire_safety",
        tower_a_fire_alarm(),
        None,
        Some("2026-11-03 10:00"),
    )
    .await;
    no_show::run(w.t.state(), dubai("2026-11-03", 12, 0))
        .await
        .unwrap();
    assert_eq!(w.told(NOTIFICATION_TYPE, &w.coordinator), 1);
    assert_eq!(w.told(NOTIFICATION_TYPE, &w.manager), 1);
}

/// AC4: the contract page counts one no-show for CoolAir.
#[renox::test]
async fn tkt05_the_contract_page_counts_the_no_shows_per_vendor() {
    let w = world().await;
    let contract = plumbing_contract(&w).await;
    vendor_ticket(
        &w,
        "plumbing",
        tower_a_cleaning(),
        None,
        Some("2026-11-03 10:00"),
    )
    .await;
    no_show::run(w.t.state(), dubai("2026-11-03", 12, 0))
        .await
        .unwrap();

    w.t.app.acting_as(&w.coordinator);
    w.t.app
        .get(&format!("/service-contracts/{}", contract.id))
        .await
        .assert_ok()
        .assert_see("Vendor no-shows")
        .assert_see("CoolAir");
    let row = no_show_row(&w, &contract).await;
    assert!(
        row.contains(r#"<td>CoolAir</td><td class="rx-num">1</td>"#),
        "one no-show for CoolAir: {row}"
    );
    assert!(row.contains("03 Nov 2026 10:00"), "{row}");

    // A second ticket of the same vendor raises the count to 2.
    vendor_ticket(
        &w,
        "plumbing",
        tower_a_cleaning(),
        None,
        Some("2026-11-03 13:00"),
    )
    .await;
    no_show::run(w.t.state(), dubai("2026-11-03", 15, 0))
        .await
        .unwrap();
    let row = no_show_row(&w, &contract).await;
    assert!(
        row.contains(r#"<td>CoolAir</td><td class="rx-num">2</td>"#),
        "two no-shows for CoolAir: {row}"
    );
    assert!(row.contains("03 Nov 2026 13:00"), "{row}");
}

/// AC5: a ticket started at 11:30 records nothing.
#[renox::test]
async fn tkt05_a_ticket_started_in_time_records_nothing() {
    let w = world().await;
    let ticket = vendor_ticket(
        &w,
        "plumbing",
        tower_a_cleaning(),
        None,
        Some("2026-11-03 10:00"),
    )
    .await;
    w.t.run(&w.coordinator, &ticket, "start", "2026-11-03 11:30", &[])
        .await
        .assert_ok();
    assert_eq!(w.t.reload(&ticket).await.status, TicketStatus::InProgress);
    let report = no_show::run(w.t.state(), dubai("2026-11-03", 12, 30))
        .await
        .unwrap();
    assert_eq!(report.recorded, 0);
    assert!(w.no_shows.all().is_empty());
}

/// Without an appointment the response due time is the trigger, with no
/// extra two hours.
#[renox::test]
async fn tkt05_without_an_appointment_the_response_due_time_is_the_trigger() {
    let w = world().await;
    // The `general` category carries the standard SLA policy.
    let ticket = vendor_ticket(&w, "general", tower_a_cleaning(), None, None).await;
    let due: DateTime = ticket
        .response_due
        .expect("the SLA started with the assignment");
    assert!(ticket.visit_at.is_none());

    let before = no_show::run(w.t.state(), due - renox::chrono::TimeDelta::minutes(1))
        .await
        .unwrap();
    assert_eq!(before.recorded, 0);
    let at = no_show::run(w.t.state(), due).await.unwrap();
    assert_eq!(at.recorded, 1);
    assert_eq!(w.no_shows.all()[0].scheduled_at, due);
}

/// A vendor without a contract is recorded against the vendor only; a staff
/// assignee is never a no-show.
#[renox::test]
async fn tkt05_an_ad_hoc_vendor_has_no_contract_and_staff_are_never_recorded() {
    let w = world().await;
    let ticket = vendor_ticket(
        &w,
        "plumbing",
        tower_a_cleaning(),
        None,
        Some("2026-11-03 10:00"),
    )
    .await;
    assert_eq!(ticket.contract_id, None);
    no_show::run(w.t.state(), dubai("2026-11-03", 12, 0))
        .await
        .unwrap();
    let events = w.no_shows.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].contract, None);
    assert_eq!(events[0].vendor, PartyId(COOLAIR.0));

    // A staff assignee with a visit long past.
    let staff = Tickets::open_at(
        w.t.state(),
        OpenTicket::new(tower_a_cleaning(), "plumbing", "Leak")
            .assign_to(wl_kernel::tickets::Assignee::user(w.imran.id)),
        dubai("2026-11-03", 8, 0),
    )
    .await
    .unwrap();
    let staff = w.t.ticket(staff.id.0).await;
    let imran = w.imran.id.to_string();
    w.t.assign(
        &w.coordinator,
        &staff,
        "2026-11-03 08:30",
        &[
            ("user", imran.as_str()),
            ("visit", local("2026-11-03 09:00").as_str()),
        ],
    )
    .await
    .assert_ok();
    no_show::run(w.t.state(), dubai("2026-11-05", 12, 0))
        .await
        .unwrap();
    assert_eq!(w.no_shows.all().len(), 1, "only the vendor's");
}

/// The contract page's HTML.
async fn no_show_row(w: &World, contract: &ServiceContract) -> String {
    w.t.app.acting_as(&w.coordinator);
    w.t.app
        .get(&format!("/service-contracts/{}", contract.id))
        .await
        .assert_ok()
        .text()
}

/// AC2: the new-time notice goes to the tenant who asked, not to the
/// coordinator who set the time.
#[renox::test]
async fn tkt05_the_new_time_notice_is_addressed_to_the_requester() {
    let w = world().await;
    let tenant = w.t.user("Tina", &[wl_ticketing::permissions::CREATE]).await;
    let ticket = vendor_ticket(
        &w,
        "plumbing",
        tower_a_cleaning(),
        Some(&tenant),
        Some("2026-11-03 10:00"),
    )
    .await;
    no_show::run(w.t.state(), dubai("2026-11-03", 12, 0))
        .await
        .unwrap();
    w.t.app.acting_as(&w.coordinator);
    w.t.app
        .htmx()
        .post(
            &format!("/tickets/{}/new-time", ticket.id),
            &[("visit_at", "2026-11-03T15:00:00")],
        )
        .await
        .assert_ok();
    let told = w.t.notices.of_type("ticketing.visit_rescheduled");
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].data["visit"], "03 Nov 2026 15:00");
    let to: Vec<_> = told[0].recipients.clone();
    assert_eq!(
        to,
        [wl_kernel::notify::NotifyRecipient::User { id: tenant.id }]
    );
}

/// A ticket `on_hold` is not `assigned` any more: the rule records nothing
/// for it, however late it is.
#[renox::test]
async fn tkt05_a_ticket_on_hold_is_not_a_no_show() {
    let w = world().await;
    let mut ticket = Ticket {
        number: "TKT-2026-9600".into(),
        subject_type: tower_a_cleaning().subject_type,
        subject_id: tower_a_cleaning().subject_id,
        category_id: Category::by_key(w.t.db(), "plumbing")
            .await
            .unwrap()
            .unwrap()
            .id,
        title: "Held visit".into(),
        assignee_party_id: Some(COOLAIR.0),
        visit_at: Some(dubai("2026-11-03", 10, 0)),
        status: TicketStatus::OnHold,
        ..Ticket::default()
    };
    let mut tx = w.t.db().begin().await.unwrap();
    ticket
        .insert_in_state(&mut tx, TicketStatus::OnHold, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(w.t.reload(&ticket).await.status, TicketStatus::OnHold);
    let report = no_show::run(w.t.state(), dubai("2026-11-03", 13, 0))
        .await
        .unwrap();
    assert_eq!(report.recorded, 0);
    assert!(w.no_shows.all().is_empty());
    assert!(w.t.notices.of_type(NOTIFICATION_TYPE).is_empty());
}

/// BL-KRN-04: a start entered late, with a business time before the trigger
/// (11:30), is judged by that time: no no-show is recorded when the rule runs
/// afterwards. A start whose business time is after the trigger, entered once
/// the no-show was recorded, does not take the record back.
#[renox::test]
async fn tkt05_a_late_entered_start_is_judged_by_its_business_time() {
    let w = world().await;
    let early = vendor_ticket(
        &w,
        "plumbing",
        tower_a_cleaning(),
        None,
        Some("2026-11-03 10:00"),
    )
    .await;
    // Entered long after the visit day, business time 11:30.
    w.t.run(&w.coordinator, &early, "start", "2026-11-03 11:30", &[])
        .await
        .assert_ok();
    let started = w.t.reload(&early).await;
    assert_eq!(started.status, TicketStatus::InProgress);
    assert_eq!(
        no_show::run(w.t.state(), dubai("2026-11-04", 9, 0))
            .await
            .unwrap()
            .recorded,
        0
    );

    let late = vendor_ticket(
        &w,
        "plumbing",
        tower_a_cleaning(),
        None,
        Some("2026-11-03 10:00"),
    )
    .await;
    assert_eq!(
        no_show::run(w.t.state(), dubai("2026-11-03", 12, 0))
            .await
            .unwrap()
            .recorded,
        1,
        "the second ticket was still assigned at 12:00"
    );
    w.t.run(&w.coordinator, &late, "start", "2026-11-03 12:30", &[])
        .await
        .assert_ok();
    assert_eq!(
        NoShow::query()
            .where_(NoShow::TICKET_ID.eq(late.id))
            .get(w.t.db())
            .await
            .unwrap()
            .len(),
        1,
        "the record stays"
    );
    assert_eq!(w.no_shows.all().len(), 1);
}
