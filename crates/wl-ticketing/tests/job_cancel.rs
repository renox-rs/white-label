//! BL-PRP-24 (#274, `PRP.ACL.15`): cancelling a job after work started, and
//! the guard of compliance-critical planned visits: resolved only with the
//! vendor's service report, never skipped by a user, rescheduled with a reason.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::tickets::OpenTicket;
use wl_ticketing::charge;
use wl_ticketing::job_cancel;
use wl_ticketing::{Category, Comment, Priority, Ticket, TicketStatus, Tickets};

use support::{Booted, COOLAIR, boot, dubai};

async fn categories(t: &Booted) {
    for (key, critical) in [("lifts", true), ("repair", false)] {
        Category::create(
            t.db(),
            Category {
                key: key.into(),
                name_en: key.into(),
                default_priority: Priority::Normal,
                compliance_critical: critical,
                active: true,
                ..Category::default()
            },
        )
        .await
        .unwrap();
    }
}

/// A planned lifts visit due 2027-03-01, assigned to the technician.
async fn visit(t: &Booted, coordinator: &User, technician: &User) -> Ticket {
    let due = NaiveDate::from_ymd_opt(2027, 3, 1).unwrap();
    let request = OpenTicket::new(Subject::new("property.building_system", 9), "lifts", "Lift service")
        .planned_due(due);
    let opened = Tickets::open_at(t.state(), request, dubai("2027-02-20", 8, 0)).await.unwrap();
    let ticket = t.ticket(opened.id.0).await;
    let tech = technician.id.to_string();
    t.assign(coordinator, &ticket, "2027-02-20 09:00", &[("user", tech.as_str())])
        .await
        .assert_ok();
    t.reload(&ticket).await
}

async fn post(t: &Booted, user: &User, path: &str, pairs: &[(&str, &str)]) -> renox::testing::TestResponse {
    t.app.acting_as(user);
    t.app.htmx().post(path, pairs).await
}

/// A lifts planned visit without a service report cannot be resolved; with
/// it, it can.
#[renox::test]
async fn bl_prp_24_a_compliance_visit_is_resolved_only_with_the_service_report() {
    let t = boot().await;
    categories(&t).await;
    let coordinator = t.coordinator().await;
    let technician = t.technician("Tim").await;
    let visit = visit(&t, &coordinator, &technician).await;
    t.run(&technician, &visit, "start", "2027-03-01 10:00", &[]).await.assert_ok();

    t.run(&technician, &visit, "resolve", "2027-03-01 12:00", &[]).await.assert_status(422);
    assert_eq!(t.reload(&visit).await.status, TicketStatus::InProgress);

    post(&t, &technician, &format!("/tickets/{}/service-report", visit.id), &[("reference", "  ")])
        .await
        .assert_status(422);
    post(&t, &technician, &format!("/tickets/{}/service-report", visit.id), &[("reference", "SR-2027-031")])
        .await
        .assert_ok();
    assert_eq!(t.reload(&visit).await.service_report.as_deref(), Some("SR-2027-031"));
    t.run(&technician, &visit, "resolve", "2027-03-01 12:00", &[]).await.assert_ok();
    assert_eq!(t.reload(&visit).await.status, TicketStatus::Resolved);
}

/// A user cancel of a compliance-critical planned visit is refused;
/// rescheduling it with a reason is allowed and kept in the comments.
#[renox::test]
async fn bl_prp_24_a_compliance_visit_is_rescheduled_not_skipped() {
    let t = boot().await;
    categories(&t).await;
    let coordinator = t.coordinator().await;
    let technician = t.technician("Tim").await;
    let visit = visit(&t, &coordinator, &technician).await;

    post(&t, &coordinator, &format!("/tickets/{}/cancel", visit.id), &[("reason", "not needed")])
        .await
        .assert_status(422);
    assert_eq!(t.reload(&visit).await.status, TicketStatus::Assigned);

    let path = format!("/tickets/{}/reschedule", visit.id);
    post(&t, &coordinator, &path, &[("date", "2027-04-10"), ("reason", "  ")])
        .await
        .assert_status(422);
    post(&t, &coordinator, &path, &[("date", "2027-04-10"), ("reason", "lift shut for repairs")])
        .await
        .assert_ok();
    let moved = t.reload(&visit).await;
    assert_eq!(moved.planned_due_date, NaiveDate::from_ymd_opt(2027, 4, 10));
    let comments = Comment::of_ticket(t.db(), visit.id).await.unwrap();
    assert!(comments.iter().any(|c| c.text.contains("lift shut for repairs")));

    // The system's cancel (the contract ended) is still allowed.
    wl_ticketing::work::auto_cancel(t.state(), &moved, "contract_ended").await.unwrap();
    assert_eq!(t.reload(&visit).await.status, TicketStatus::Cancelled);
}

async fn held_job(t: &Booted, coordinator: &User, technician: &User) -> Ticket {
    let tenant = t.user("Tara", &[wl_ticketing::permissions::VIEW]).await;
    let request = OpenTicket::new(Subject::new("property.unit", 12), "repair", "AC repair")
        .requester_user(tenant.id)
        .bill_to(COOLAIR);
    let opened = Tickets::open_at(t.state(), request, dubai("2027-02-20", 8, 0)).await.unwrap();
    let ticket = t.ticket(opened.id.0).await;
    let tech = technician.id.to_string();
    t.assign(coordinator, &ticket, "2027-02-20 09:00", &[("user", tech.as_str())])
        .await
        .assert_ok();
    t.run(technician, &ticket, "start", "2027-02-20 10:00", &[]).await.assert_ok();
    t.run(technician, &ticket, "hold", "2027-02-20 11:00", &[("hold_reason", "parts")])
        .await
        .assert_ok();
    let held = t.reload(&ticket).await;
    assert_eq!(held.status, TicketStatus::OnHold);
    held
}

/// A job `on_hold` cancelled with a reason creates no invoice.
#[renox::test]
async fn bl_prp_24_cancel_after_work_started_charges_nothing_by_default() {
    let t = boot().await;
    categories(&t).await;
    let coordinator = t.coordinator().await;
    let technician = t.technician("Tim").await;
    let held = held_job(&t, &coordinator, &technician).await;
    post(&t, &coordinator, &format!("/tickets/{}/cancel", held.id), &[("reason", "tenant fixed it")])
        .await
        .assert_ok();
    let cancelled = t.reload(&held).await;
    assert_eq!(cancelled.status, TicketStatus::Cancelled);
    assert_eq!(cancelled.cancel_reason.as_deref(), Some("tenant fixed it"));
    assert!(t.invoices.all().is_empty());
    assert!(!cancelled.chargeable);
}

/// With "charge" ticked, a reason and one charge line of AED 200, a draft
/// invoice of AED 200 is made; without the reason or the line the cancel is
/// refused and nothing changes.
#[renox::test]
async fn bl_prp_24_cancel_with_a_charge_invoices_the_work_done() {
    let t = boot().await;
    categories(&t).await;
    let coordinator = t.coordinator().await;
    let technician = t.technician("Tim").await;
    let held = held_job(&t, &coordinator, &technician).await;
    let path = format!("/tickets/{}/cancel", held.id);

    post(
        &t,
        &coordinator,
        &path,
        &[("reason", "tenant damage"), ("charge", "1"), ("charge_reason", ""), ("description", "Call-out"), ("unit_price", "200")],
    )
    .await
    .assert_status(422);
    assert_eq!(t.reload(&held).await.status, TicketStatus::OnHold);

    post(
        &t,
        &coordinator,
        &path,
        &[
            ("reason", "tenant damage"),
            ("charge", "1"),
            ("charge_reason", "tenant damage"),
            ("description", "Call-out"),
            ("unit_price", "200"),
        ],
    )
    .await
    .assert_ok();
    let cancelled = t.reload(&held).await;
    assert_eq!(cancelled.status, TicketStatus::Cancelled);
    assert!(cancelled.chargeable);
    let lines = charge::lines_of(t.db(), held.id).await.unwrap();
    assert_eq!(lines.len(), 1);
    let drafts = t.invoices.all();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].draft.party, COOLAIR);
    assert_eq!(drafts[0].draft.lines[0].unit_price, 20_000);
    assert_eq!(cancelled.invoice_id, Some(drafts[0].id));
    let _ = job_cancel::is_compliance_visit;
}
