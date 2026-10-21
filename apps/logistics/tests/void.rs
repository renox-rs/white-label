//! BL-LOG-01 (#299), 03 §4.1 *Duplicate booking*, §7.1 row `void`,
//! `PLT.ACL.09`: a duplicate booking is voided while `created`, keeps its
//! number, its label is invalid and it is never billed.

mod support;

use logistics::{Shipment, ShipmentStatus};
use renox::prelude::*;
use wl_delivery::DeliveryStatus;
use wl_kernel::voids::{self, VoidRecord};

use support::{SOUQ, boot};

/// AC2 (all states): a voided shipment keeps its reference, so booking it
/// again needs a new reference.
#[renox::test]
async fn a_voided_shipment_keeps_its_reference() {
    let w = boot().await;
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-77").await;
    w.post(
        &w.noor,
        &format!("/shipments/{}/void", shipment.id),
        &[("reason", "duplicate")],
    )
    .await
    .assert_status(303);
    w.try_book(&w.noor, SOUQ, "ORD-77", &[])
        .await
        .assert_status(422);
    w.book_as(&w.noor, SOUQ, "ORD-77-B").await;
}

/// AC3: a `created` shipment with no scan is voided by the client user who
/// booked it with reason *duplicate*: it is `voided`, its label no longer
/// validates at scan, its number is kept and `RecordVoided` is emitted.
#[renox::test]
async fn the_client_user_who_booked_a_created_shipment_voids_it() {
    let w = boot().await;
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-77").await;
    let label = w.label(&shipment).await;
    w.post(
        &w.noor,
        &format!("/shipments/{}/void", shipment.id),
        &[("reason", "duplicate")],
    )
    .await
    .assert_status(303);

    let after = w.reload(&shipment).await;
    assert_eq!(after.status, ShipmentStatus::Voided);
    assert_eq!(after.number, shipment.number);
    let record = VoidRecord::query()
        .where_(VoidRecord::SUBJECT_ID.eq(shipment.id))
        .first(w.db())
        .await
        .unwrap()
        .expect("the void is recorded");
    assert_eq!(record.number.as_deref(), Some(shipment.number.as_str()));
    assert_eq!(record.reason, "duplicate");
    assert_eq!(record.voided_by, w.noor.id);
    assert_eq!(record.state_before, "created");
    let numbers = voids::void_numbers(w.db(), "shipment").await.unwrap();
    assert_eq!(numbers, vec![shipment.number.clone()]);

    let events = w.voided.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].subject, shipment.subject());
    assert_eq!(events[0].reason, "duplicate");
    assert_eq!(events[0].user_id, w.noor.id);

    // The label does not validate at a pickup or a hub scan.
    for kind in ["pickup", "hub"] {
        let refused = w
            .post(
                &w.driver,
                "/scans",
                &[("barcode", label.as_str()), ("kind", kind)],
            )
            .await;
        refused.assert_status(422);
        refused.assert_see("label invalid");
    }
    assert_eq!(
        w.reload(&shipment).await.status,
        ShipmentStatus::Voided
    );
}

/// AC3 (system): the void cancels the shipment's pending delivery and leaves
/// a tracking event; a void needs a reason.
#[renox::test]
async fn a_void_cancels_the_delivery_and_needs_a_reason() {
    let w = boot().await;
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-77").await;
    let path = format!("/shipments/{}/void", shipment.id);
    w.post(&w.noor, &path, &[("reason", " ")])
        .await
        .assert_status(422);
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::Created);
    w.post(&w.noor, &path, &[("reason", "entered in error")])
        .await
        .assert_status(303);
    assert_eq!(
        w.delivery_of(&shipment).await.status,
        DeliveryStatus::Cancelled
    );
    let events = w.reload(&shipment).await.events(w.db()).await.unwrap();
    assert_eq!(events.last().unwrap().kind, "voided");
    assert_eq!(events.last().unwrap().note, "entered in error");
}

/// AC4: given a shipment with one parcel scanned at pickup (now
/// `picked_up`), `void` is not offered and a call to `void` is refused;
/// `cancel_return` is offered.
#[renox::test]
async fn a_picked_up_shipment_cannot_be_voided_and_is_sent_back_instead() {
    let w = boot().await;
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-77").await;
    let created = w.offered(&w.dispatcher, &shipment).await;
    assert!(created.contains(&"void".to_owned()));
    let picked = w.pick_up(&shipment).await;
    let path = format!("/shipments/{}/void", picked.id);
    w.post(&w.noor, &path, &[("reason", "duplicate")])
        .await
        .assert_status(422);
    w.post(&w.dispatcher, &path, &[("reason", "duplicate")])
        .await
        .assert_status(422);
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::PickedUp);

    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/instruction", picked.id),
        &[("note", "client e-mail 2026-11-03 10:15")],
    )
    .await
    .assert_status(303);
    let offered = w.offered(&w.dispatcher, &picked).await;
    assert!(!offered.contains(&"void".to_owned()), "{offered:?}");
    assert!(offered.contains(&"cancel_return".to_owned()), "{offered:?}");
    assert!(w.voided.all().is_empty());
}

/// AC4 (safety check): a scan recorded without a state change still stops
/// the void.
#[renox::test]
async fn a_scanned_parcel_stops_the_void_even_without_a_state_change() {
    let w = boot().await;
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-77").await;
    // The scan was saved but the pickup failed: the row says a parcel was
    // scanned while the shipment is still `created`.
    Shipment::query()
        .where_(Shipment::ID.eq(shipment.id))
        .update(w.db(), &[("parcels_scanned", &1_i64)])
        .await
        .unwrap();
    assert!(
        !w.offered(&w.dispatcher, &shipment)
            .await
            .contains(&"void".to_owned())
    );
    let refused = w
        .post(
            &w.dispatcher,
            &format!("/shipments/{}/void", shipment.id),
            &[("reason", "duplicate")],
        )
        .await;
    refused.assert_status(422);
    refused.assert_see("A parcel was scanned");
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::Created);
}

/// AC5: given a shipment voided and a pickup scan of its label committed
/// after it, the scan is refused with "label invalid".
#[renox::test]
async fn a_scan_after_the_void_is_refused_as_label_invalid() {
    let w = boot().await;
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-77").await;
    let label = w.label(&shipment).await;
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/void", shipment.id),
        &[("reason", "duplicate")],
    )
    .await
    .assert_status(303);
    let refused = w
        .post(
            &w.driver,
            &format!("/shipments/{}/pickup", shipment.id),
            &[("barcodes", label.as_str())],
        )
        .await;
    refused.assert_status(422);
    refused.assert_see("label invalid");
    let after = w.reload(&shipment).await;
    assert_eq!(after.status, ShipmentStatus::Voided);
    assert_eq!(after.parcels_scanned, 0);
}

/// AC5: given the scan committed first, `void` is refused.
#[renox::test]
async fn a_void_after_the_scan_is_refused() {
    let w = boot().await;
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-77").await;
    w.pick_up(&shipment).await;
    w.post(
        &w.noor,
        &format!("/shipments/{}/void", shipment.id),
        &[("reason", "duplicate")],
    )
    .await
    .assert_status(422);
    let after = w.reload(&shipment).await;
    assert_eq!(after.status, ShipmentStatus::PickedUp);
    assert!(w.voided.all().is_empty());
}

/// AC6: given a client user who did not book the shipment, `void` is not
/// offered and a call is refused; staff with `shipments.manage` void any
/// `created` shipment.
#[renox::test]
async fn a_client_user_who_did_not_book_the_shipment_cannot_void_it() {
    let w = boot().await;
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-77").await;
    assert!(
        w.offered(&w.noor, &shipment)
            .await
            .contains(&"void".to_owned())
    );
    let path = format!("/shipments/{}/void", shipment.id);
    for other in [&w.nadia, &w.farah] {
        assert!(
            !w.offered(other, &shipment)
                .await
                .contains(&"void".to_owned())
        );
        w.post(other, &path, &[("reason", "duplicate")])
            .await
            .assert_status(422);
    }
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::Created);
    // Hub staff do not hold the void permission at all (the kernel's guard
    // refuses it, so the answer is a 422 like any refused transition).
    w.post(&w.hub, &path, &[("reason", "duplicate")])
        .await
        .assert_status(422);
    // A dispatcher voids a shipment a client user booked.
    w.post(&w.dispatcher, &path, &[("reason", "duplicate")])
        .await
        .assert_status(303);
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::Voided);
}

/// AC7: a voided shipment never appears on a monthly invoice draft: the
/// shipments a client's invoice may bill leave it out.
#[renox::test]
async fn a_voided_shipment_never_appears_on_a_monthly_invoice_draft() {
    let w = boot().await;
    let kept = w.book_as(&w.noor, SOUQ, "ORD-1").await;
    let voided = w.book_as(&w.noor, SOUQ, "ORD-2").await;
    w.post(
        &w.noor,
        &format!("/shipments/{}/void", voided.id),
        &[("reason", "duplicate")],
    )
    .await
    .assert_status(303);
    let billable = Shipment::billable(w.db(), SOUQ).await.unwrap();
    let ids: Vec<i64> = billable.iter().map(|s| s.id).collect();
    assert_eq!(ids, vec![kept.id]);
}
