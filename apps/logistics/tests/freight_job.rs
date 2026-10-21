//! The freight job lifecycle (03 Logistics §4.2, §7.1 *Freight job*,
//! `LOG.JOB.01`; base for BL-LOG-21, BL-LOG-22): booked, documents pending,
//! in transit, customs clearance, ready for delivery, delivered, closed and
//! cancelled, each step by the user who holds its permission and with the
//! guard the specification gives it.

mod support;

use logistics::{FreightJob, FreightStatus, JobLine};
use renox::prelude::*;

use support::{boot, boot_with_parties};

/// The job is opened `booked` with its mode and route, a `booked` milestone
/// and a number from the series; only a holder of `freight.manage` opens one.
#[renox::test]
async fn a_job_is_opened_booked_with_its_route_and_a_milestone() {
    let w = boot().await;
    let job = w.open_job().await;
    assert_eq!(job.number, "FRJ-0001");
    assert_eq!(job.mode, "sea");
    assert_eq!(job.origin, "Shanghai");
    assert_eq!(job.destination, "Jebel Ali");
    assert_eq!(job.eta.map(|d| d.to_string()).as_deref(), Some("2026-11-20"));
    let events = job.events(w.db()).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "booked");

    w.post(
        &w.hub,
        "/freight",
        &[("client_party_id", "501"), ("description", "x")],
    )
    .await
    .assert_forbidden();
    w.post(
        &w.docs,
        "/freight",
        &[
            ("client_party_id", "501"),
            ("description", "x"),
            ("mode", "rail"),
        ],
    )
    .await
    .assert_status(422);
}

/// The job goes through every milestone, each guard refuses what is missing,
/// and the client is told of the missing documents and of the departure.
#[renox::test]
async fn a_job_goes_from_booked_to_delivered_with_the_guards_of_each_step() {
    let w = boot().await;
    let job = w.open_job().await;

    w.job_action(&w.docs, &job, "request-documents", &[("missing", "packing list")])
        .await
        .assert_status(303);
    let pending = w.job(job.id).await;
    assert_eq!(pending.status, FreightStatus::DocumentsPending);
    assert_eq!(pending.documents_note.as_deref(), Some("packing list"));
    let notices = w.notices.to_clients();
    assert_eq!(notices.len(), 1);
    assert!(notices[0].data["body"].as_str().unwrap().contains("packing list"));

    // No BL or AWB, no departure.
    w.job_action(&w.docs, &job, "depart", &[("bl_awb", " ")])
        .await
        .assert_status(422);
    assert_eq!(w.job(job.id).await.status, FreightStatus::DocumentsPending);
    w.job_action(&w.docs, &job, "depart", &[("bl_awb", "MAEU123456789")])
        .await
        .assert_status(303);
    let moving = w.job(job.id).await;
    assert_eq!(moving.status, FreightStatus::InTransit);
    assert_eq!(moving.bl_awb.as_deref(), Some("MAEU123456789"));
    assert_eq!(w.notices.to_clients().len(), 2);

    w.job_action(&w.docs, &job, "arrive", &[])
        .await
        .assert_status(303);
    assert_eq!(w.job(job.id).await.status, FreightStatus::CustomsClearance);

    // The declaration, the duties and the delivery order are all needed.
    w.job_action(
        &w.docs,
        &job,
        "clear",
        &[("declaration_no", "DEC-1"), ("duties", "250.00"), ("delivery_order", "")],
    )
    .await
    .assert_status(422);
    w.job_action(
        &w.docs,
        &job,
        "clear",
        &[
            ("declaration_no", "DEC-1"),
            ("duties", "250.00"),
            ("delivery_order", "DO-1"),
        ],
    )
    .await
    .assert_status(303);
    let cleared = w.job(job.id).await;
    assert_eq!(cleared.status, FreightStatus::ReadyForDelivery);
    assert_eq!(cleared.duties, Some(25_000));
    assert_eq!(cleared.delivery_order_ref.as_deref(), Some("DO-1"));

    // `deliver` is the e-POD, by a holder of `jobs.deliver`.
    w.job_action(&w.docs, &job, "deliver", &[("pod_reference", " ")])
        .await
        .assert_status(422);
    w.job_action(&w.docs_limited, &job, "deliver", &[("pod_reference", "epod-1")])
        .await
        .assert_forbidden();
    w.job_action(&w.docs, &job, "deliver", &[("pod_reference", "epod-1")])
        .await
        .assert_status(303);
    let delivered = w.job(job.id).await;
    assert_eq!(delivered.status, FreightStatus::Delivered);
    assert_eq!(delivered.pod_ref.as_deref(), Some("epod-1"));
    let kinds: Vec<String> = delivered
        .events(w.db())
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.kind)
        .collect();
    assert_eq!(
        kinds,
        [
            "booked",
            "documents_requested",
            "departed",
            "arrived",
            "cleared",
            "delivered"
        ]
    );
}

/// A step out of order is refused and nobody without `jobs.manage` moves a
/// job.
#[renox::test]
async fn a_step_out_of_order_or_without_the_permission_is_refused() {
    let w = boot().await;
    let job = w.open_job().await;
    for action in ["arrive", "deliver"] {
        w.job_action(&w.docs, &job, action, &[("pod_reference", "epod-1")])
            .await
            .assert_status(422);
    }
    w.job_action(&w.dispatcher, &job, "request-documents", &[("missing", "x")])
        .await
        .assert_forbidden();
    w.job_action(&w.docs, &job, "no-such-action", &[])
        .await
        .assert_status(404);
    assert_eq!(w.job(job.id).await.status, FreightStatus::Booked);
}

/// `close` needs every cost allocated and the invoice issued, and keeps the
/// margin; the job is `closed`.
#[renox::test]
async fn close_needs_the_costs_allocated_and_the_invoice_issued_and_keeps_the_margin() {
    let w = boot().await;
    let job = w.job_delivered().await;

    // No invoice yet.
    w.job_action(&w.docs, &job, "close", &[])
        .await
        .assert_status(422)
        .assert_see("Issue the invoice");
    w.job_revenue(&job, "10000.00").await;
    // A cost without a liability waits.
    w.post(
        &w.docs,
        &format!("/freight/{}/costs", job.id),
        &[("description", "Haulage"), ("amount", "7000.00")],
    )
    .await
    .assert_status(303);
    w.job_action(&w.docs, &job, "close", &[])
        .await
        .assert_status(422)
        .assert_see("Allocate every supplier bill");
    let line = JobLine::query()
        .where_(JobLine::KIND.eq("cost"))
        .first(w.db())
        .await
        .unwrap()
        .expect("the cost");
    w.post(
        &w.docs,
        &format!("/freight/lines/{}/allocate", line.id),
        &[("liability", "someone")],
    )
    .await
    .assert_status(422);
    w.post(
        &w.docs,
        &format!("/freight/lines/{}/allocate", line.id),
        &[("liability", "company")],
    )
    .await
    .assert_status(303);

    // Closing is the right of `jobs.close`.
    w.job_action(&w.docs_limited, &job, "close", &[])
        .await
        .assert_forbidden();
    w.job_action(&w.docs, &job, "close", &[])
        .await
        .assert_status(303);
    let closed = w.job(job.id).await;
    assert_eq!(closed.status, FreightStatus::Closed);
    assert!(closed.closed_at.is_some());
    let margins = logistics::freight_costing::margins_of(w.db(), job.id)
        .await
        .unwrap();
    assert_eq!(margins.len(), 1);
    assert_eq!(margins[0].close_no, 1);
    assert_eq!(margins[0].revenue, 1_000_000);
    assert_eq!(margins[0].cost, 700_000);
    assert_eq!(margins[0].margin, 300_000);
    // A closed job takes no new cost: it is reopened first.
    w.post(
        &w.docs,
        &format!("/freight/{}/costs", job.id),
        &[("description", "Late bill"), ("amount", "800.00")],
    )
    .await
    .assert_status(422);
}

/// `cancel` before departure takes a reason; a departed job is not cancelled.
#[renox::test]
async fn a_job_is_cancelled_before_departure_with_a_reason() {
    let w = boot().await;
    let job = w.open_job().await;
    w.job_action(&w.docs, &job, "cancel", &[("reason", " ")])
        .await
        .assert_status(422);
    w.job_action(&w.docs, &job, "cancel", &[("reason", "Client changed plans")])
        .await
        .assert_status(303);
    let cancelled = w.job(job.id).await;
    assert_eq!(cancelled.status, FreightStatus::Cancelled);
    assert_eq!(cancelled.cancel_reason.as_deref(), Some("Client changed plans"));
    w.job_action(&w.docs, &job, "depart", &[("bl_awb", "BL-1")])
        .await
        .assert_status(422);

    let departed = w.in_transit().await;
    w.job_action(&w.docs, &departed, "cancel", &[("reason", "Too late")])
        .await
        .assert_status(422);
    assert_eq!(w.job(departed.id).await.status, FreightStatus::InTransit);
}

/// A container of a job keeps its free-time end dates; the same number is
/// not added twice.
#[renox::test]
async fn a_container_keeps_its_free_time_end_dates() {
    let w = boot().await;
    let job = w.open_job().await;
    let path = format!("/freight/{}/containers", job.id);
    w.post(
        &w.docs,
        &path,
        &[
            ("number", "msku1234567"),
            ("size", "40HC"),
            ("port_storage_end", "2026-11-25"),
            ("demurrage_end", "2026-11-27"),
            ("detention_end", "2026-12-05"),
        ],
    )
    .await
    .assert_status(303);
    let containers = job.containers(w.db()).await.unwrap();
    assert_eq!(containers.len(), 1);
    assert_eq!(containers[0].number, "MSKU1234567");
    assert_eq!(
        containers[0].demurrage_end.map(|d| d.to_string()).as_deref(),
        Some("2026-11-27")
    );
    w.post(&w.docs, &path, &[("number", "MSKU1234567")])
        .await
        .assert_status(422);
    w.post(&w.hub, &path, &[("number", "TGHU7654321")])
        .await
        .assert_forbidden();
}

/// The page shows staff the milestones and the margin, and a client user of
/// the company the status and milestones without the costs; another
/// client's user sees nothing.
#[renox::test]
async fn the_page_shows_staff_the_costs_and_the_client_only_the_move() {
    let w = boot().await;
    let job = w.job_closed().await;
    let path = format!("/freight/{}", job.id);
    w.get(&w.docs, &path)
        .await
        .assert_ok()
        .assert_see("FRJ-0001")
        .assert_see("Shipping line bill");
    let client = w.get(&w.noor, &path).await;
    client.assert_ok();
    client.assert_see("FRJ-0001");
    client.assert_dont_see("Shipping line bill");
    w.get(&w.farah, &path).await.assert_forbidden();
}

/// Open jobs continue when the client is blocked (`PTY.ACL.03`): the real
/// wl-parties blocks C after the job was opened and the job still departs.
#[renox::test]
async fn an_open_job_continues_when_the_client_is_blocked() {
    let w = boot_with_parties().await;
    let client = w.client_party("Najm Client LLC").await;
    w.post(
        &w.docs,
        "/freight",
        &[
            ("client_party_id", client.0.to_string().as_str()),
            ("description", "Pallets to Muscat"),
        ],
    )
    .await
    .assert_status(303);
    let job = FreightJob::query().first(w.db()).await.unwrap().unwrap();
    w.block_party(client, "non_payment").await;
    w.job_action(&w.docs, &job, "depart", &[("bl_awb", "BL-77")])
        .await
        .assert_status(303);
    assert_eq!(w.job(job.id).await.status, FreightStatus::InTransit);
}
