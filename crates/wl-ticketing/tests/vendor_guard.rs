//! BL-TKT-06 (#151, `WRK.ACL.11`): a contract covering a guarded category
//! (01: fire and life safety) is not activated unless the vendor's document
//! (the Civil Defence approval) is valid on the start date, and a ticket or
//! planned visit in that category is not resolved while the assigned
//! vendor's document is expired or missing. In 01 the approval is a legal
//! block that is never overridden.

mod support;

use wl_kernel::documents;
use wl_kernel::tickets::{Assignee, OpenTicket};
use wl_ticketing::{ContractStatus, Ticket, TicketStatus, Tickets};

use support::contracts::{
    World, date, tower_a_cleaning, tower_a_fire_alarm, world_with_documents,
};
use support::documents::{CIVIL_DEFENCE, Documents};
use support::{SAFEFIRE, dubai};

/// SafeFire LLC's contract for the fire alarm, valid on its start date, with
/// one planned visit started and assigned to the vendor.
async fn fire_alarm_visit(w: &World, docs: &Documents, expires: &str) -> Ticket {
    docs.set(&SAFEFIRE.subject(), CIVIL_DEFENCE, Some(date(expires)));
    let contract = w
        .contract(
            SAFEFIRE,
            "2026-01-01",
            "2027-12-31",
            &[(tower_a_fire_alarm(), "fire_safety")],
        )
        .await;
    let visit = w
        .visit(
            &contract,
            "fire_safety",
            tower_a_fire_alarm(),
            "2026-12-01",
            Some(Assignee::party(SAFEFIRE)),
        )
        .await;
    w.start(&visit).await
}

/// AC1: SafeFire's Civil Defence approval expired on 2026-11-30: on
/// 2026-12-02 the resolve of a fire-alarm visit is refused, naming the
/// document and the date.
#[renox::test]
async fn tkt06_resolving_a_fire_alarm_visit_is_refused_while_the_approval_is_expired() {
    let (w, docs) = world_with_documents().await;
    let visit = fire_alarm_visit(&w, &docs, "2026-11-30").await;

    w.t.run(&w.coordinator, &visit, "resolve", "2026-12-02 10:00", &[])
        .await
        .assert_status(422)
        .assert_see("Civil Defence approval expired on 2026-11-30")
        .assert_dont_see("overrides/new");
    let after = w.t.reload(&visit).await;
    assert_eq!(after.status, TicketStatus::InProgress);
    assert!(after.resolved_at.is_none());

    // The approval is valid through its expiry date; a planned visit of a
    // compliance-critical category is also resolved with the vendor's service
    // report attached (BL-PRP-24, `PRP.ACL.15`).
    w.t.app.acting_as(&w.coordinator);
    w.t.app
        .htmx()
        .post(
            &format!("/tickets/{}/service-report", visit.id),
            &[("reference", "SR-2026-114")],
        )
        .await
        .assert_ok();
    w.t.run(&w.coordinator, &visit, "resolve", "2026-11-30 10:00", &[])
        .await
        .assert_ok();
    assert_eq!(w.t.reload(&visit).await.status, TicketStatus::Resolved);
}

/// A missing approval refuses the resolve too.
#[renox::test]
async fn tkt06_a_missing_approval_refuses_the_resolve() {
    let (w, docs) = world_with_documents().await;
    let visit = fire_alarm_visit(&w, &docs, "2027-12-31").await;
    docs.remove(&SAFEFIRE.subject(), CIVIL_DEFENCE);
    w.t.run(&w.coordinator, &visit, "resolve", "2026-12-02 10:00", &[])
        .await
        .assert_status(422)
        .assert_see("Civil Defence approval missing");
    assert_eq!(w.t.reload(&visit).await.status, TicketStatus::InProgress);
}

/// AC2: a plumbing ticket by the same vendor resolves normally.
#[renox::test]
async fn tkt06_a_plumbing_ticket_of_the_same_vendor_resolves_normally() {
    let (w, docs) = world_with_documents().await;
    docs.set(&SAFEFIRE.subject(), CIVIL_DEFENCE, Some(date("2026-11-30")));
    let opened = Tickets::open_at(
        w.t.state(),
        OpenTicket::new(tower_a_cleaning(), "plumbing", "Leaking tap")
            .assign_to(Assignee::party(SAFEFIRE)),
        dubai("2026-12-01", 8, 0),
    )
    .await
    .unwrap();
    let ticket = w.t.ticket(opened.id.0).await;
    let ticket = w.start(&ticket).await;
    w.t.run(&w.coordinator, &ticket, "resolve", "2026-12-02 10:00", &[])
        .await
        .assert_ok();
    assert_eq!(w.t.reload(&ticket).await.status, TicketStatus::Resolved);
}

/// A staff assignee is not a vendor: the document guard does not apply.
#[renox::test]
async fn tkt06_a_staff_assignee_is_not_guarded() {
    let (w, _docs) = world_with_documents().await;
    let opened = Tickets::open_at(
        w.t.state(),
        OpenTicket::new(tower_a_fire_alarm(), "fire_safety", "Check the panel")
            .assign_to(Assignee::user(w.imran.id)),
        dubai("2026-12-01", 8, 0),
    )
    .await
    .unwrap();
    let ticket = w.t.ticket(opened.id.0).await;
    w.t.run(&w.imran, &ticket, "start", "2026-12-02 09:00", &[])
        .await
        .assert_ok();
    w.t.run(&w.imran, &ticket, "resolve", "2026-12-02 10:00", &[])
        .await
        .assert_ok();
    assert_eq!(w.t.reload(&ticket).await.status, TicketStatus::Resolved);
}

/// AC3: activating a fire-safety contract with start 2027-01-01 is refused
/// while the approval expires 2026-12-31, and allowed after a renewal valid to
/// 2027-12-31 is accepted.
#[renox::test]
async fn tkt06_activating_a_fire_safety_contract_needs_the_approval_on_the_start_date() {
    let (w, docs) = world_with_documents().await;
    docs.set(&SAFEFIRE.subject(), CIVIL_DEFENCE, Some(date("2026-12-31")));
    let draft = w
        .draft(
            SAFEFIRE,
            "2027-01-01",
            "2027-12-31",
            &[(tower_a_fire_alarm(), "fire_safety")],
        )
        .await;

    w.try_activate(&draft)
        .await
        .assert_status(422)
        .assert_see("Civil Defence approval expired on 2026-12-31")
        .assert_dont_see("overrides/new");
    assert_eq!(w.reload(&draft).await.status, ContractStatus::Draft);
    assert!(w.activated.all().is_empty());

    // The renewal is accepted.
    docs.set(&SAFEFIRE.subject(), CIVIL_DEFENCE, Some(date("2027-12-31")));
    let active = w.activate(&draft).await;
    assert_eq!(active.status, ContractStatus::Active);
    assert_eq!(w.activated.all().len(), 1);
}

/// A contract that covers no guarded category activates whatever the vendor's
/// documents are.
#[renox::test]
async fn tkt06_a_contract_for_an_unguarded_category_activates_without_the_approval() {
    let (w, _docs) = world_with_documents().await;
    let contract = w
        .contract(
            SAFEFIRE,
            "2027-01-01",
            "2027-12-31",
            &[(tower_a_cleaning(), "cleaning")],
        )
        .await;
    assert_eq!(contract.status, ContractStatus::Active);
}

/// AC4: no override action is offered in 01: the Civil Defence approval is a
/// legal block with the override mode `never`.
#[renox::test]
async fn tkt06_no_override_is_offered_for_the_legal_block() {
    let (w, docs) = world_with_documents().await;
    let visit = fire_alarm_visit(&w, &docs, "2026-11-30").await;
    let refusal = documents::guard(w.t.state())
        .check(w.t.state(), "ticket.resolve", visit.id, date("2026-12-02"))
        .await
        .unwrap()
        .expect_err("the approval is expired");
    assert_eq!(refusal.failures.len(), 1);
    assert_eq!(refusal.failures[0].subject, SAFEFIRE.subject());
    assert!(!refusal.failures[0].override_allowed());
    assert!(refusal.override_links().is_empty());
    assert!(refusal.message().contains("override: not allowed"));
}
