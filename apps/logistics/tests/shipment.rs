//! The base of BL-LOG-01 (#299) and BL-LOG-02 (#300), 03 §7.1 *Shipment*,
//! `LOG.SHP.01`, `LOG.HUB.01`: a shipment is booked with its parcels and its
//! delivery, scanned at pickup and at the hub, and follows the events of
//! its delivery in wl-delivery. Every change goes through the machine and
//! leaves a tracking event.

mod support;

use logistics::{ShipmentStatus, TrackingEvent};
use renox::prelude::*;
use wl_delivery::DeliveryStatus;

use support::{SOUQ, boot};

/// AC: a booking makes a `created` shipment with a number, a label per
/// parcel and a pending delivery whose source is the shipment.
#[renox::test]
async fn a_booking_creates_the_shipment_its_labels_and_its_delivery() {
    let w = boot().await;
    let shipment = w
        .book_with(
            &w.noor,
            SOUQ,
            "ORD-1",
            &[("parcels", "2"), ("cod", "150.50")],
        )
        .await;
    assert_eq!(shipment.status, ShipmentStatus::Created);
    assert!(shipment.number.starts_with("SHP-"));
    assert_eq!(shipment.client_reference.as_deref(), Some("ORD-1"));
    assert_eq!(shipment.booked_by_user_id, Some(w.noor.id));
    assert_eq!(shipment.cod_amount, 15_050);
    assert_eq!(shipment.parcel_count, 2);
    assert_eq!(shipment.booked_weight_grams, 2_400);
    let parcels = shipment.parcels(w.db()).await.unwrap();
    let labels: Vec<_> = parcels.iter().map(|p| p.barcode.clone()).collect();
    assert_eq!(
        labels,
        [format!("{}-1", shipment.number), format!("{}-2", shipment.number)]
    );
    let delivery = w.delivery_of(&shipment).await;
    assert_eq!(delivery.status, DeliveryStatus::Pending);
    assert_eq!(delivery.source(), shipment.subject());
    assert_eq!(delivery.amount_to_collect, 15_050);
    assert_eq!(delivery.lines(w.db()).await.unwrap().len(), 2);
    let events = shipment.events(w.db()).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "booked");
}

/// AC: a client user books only for the company the portal says they act for.
#[renox::test]
async fn a_client_user_books_only_for_their_own_company() {
    let w = boot().await;
    let refused = w
        .try_book(&w.noor, support::OTHER_CLIENT, "ORD-1", &[])
        .await;
    refused.assert_forbidden();
    // Staff book for any client.
    w.book_as(&w.dispatcher, support::OTHER_CLIENT, "ORD-2").await;
}

/// AC: pick up, hub scan, dispatch with the run, deliver: the shipment
/// follows the delivery's events and tracks each step.
#[renox::test]
async fn a_shipment_follows_its_delivery_from_pickup_to_delivered() {
    let w = boot().await;
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-1").await;
    let (out, delivery) = w.out_for_delivery(&shipment).await;
    assert_eq!(out.status, ShipmentStatus::OutForDelivery);
    let line = delivery.lines(w.db()).await.unwrap().remove(0);
    let delivered_field = format!("delivered_{}", line.id);
    let refused_field = format!("refused_{}", line.id);
    w.post(
        &w.driver,
        &format!("/driver/stops/{}/deliver", delivery.id),
        &[
            ("recipient_name", "Mariam Al Hosani"),
            ("photo", "photo-0001"),
            ("signature", "signature-0001"),
            ("latitude", "25.204849"),
            ("longitude", "55.270783"),
            ("cash_collected", "0"),
            (delivered_field.as_str(), "1"),
            (refused_field.as_str(), "0"),
        ],
    )
    .await
    .assert_status(303);
    assert_eq!(
        w.reload(&shipment).await.status,
        ShipmentStatus::Delivered
    );
    let kinds: Vec<String> = TrackingEvent::query()
        .where_(TrackingEvent::SHIPMENT_ID.eq(shipment.id))
        .order_by(TrackingEvent::ID)
        .get(w.db())
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.kind)
        .collect();
    assert_eq!(
        kinds,
        ["booked", "picked_up", "hub_scan", "dispatched", "delivered"]
    );
}

/// AC: a failed attempt of the delivery makes the shipment `attempted`, and
/// the hub scans it in again.
#[renox::test]
async fn a_failed_attempt_makes_the_shipment_attempted() {
    let w = boot().await;
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-1").await;
    let (_out, delivery) = w.out_for_delivery(&shipment).await;
    w.fail_stop(&delivery, "not_home").await.assert_status(303);
    let attempted = w.reload(&shipment).await;
    assert_eq!(attempted.status, ShipmentStatus::Attempted);
    // A reason that counts adds to the attempts of the delivery.
    assert_eq!(w.delivery_of(&shipment).await.attempts, 1);
    let events = attempted.events(w.db()).await.unwrap();
    let last = events.last().unwrap();
    assert_eq!(last.kind, "attempt_failed");
    assert_eq!(last.note, "not_home");
    assert_eq!(w.at_hub(&attempted).await.status, ShipmentStatus::AtHub);
}

/// AC: a pickup with no parcel scanned is refused and the shipment stays
/// `created`.
#[renox::test]
async fn a_pickup_without_a_scanned_parcel_is_refused() {
    let w = boot().await;
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-1").await;
    let path = format!("/shipments/{}/pickup", shipment.id);
    w.post(&w.driver, &path, &[("barcodes", "")])
        .await
        .assert_status(422);
    // A label of another shipment is no parcel of this one.
    let other = w.book_as(&w.noor, SOUQ, "ORD-2").await;
    let foreign = w.label(&other).await;
    w.post(&w.driver, &path, &[("barcodes", foreign.as_str())])
        .await
        .assert_status(422);
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::Created);
}

/// AC: from `created` a hub scan is a drop-off and needs a parcel scanned;
/// the label scan route finds the shipment by its barcode.
#[renox::test]
async fn a_hub_scan_from_created_is_a_drop_off_that_needs_a_parcel() {
    let w = boot().await;
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-1").await;
    let path = format!("/shipments/{}/hub", shipment.id);
    w.post(&w.hub, &path, &[("barcodes", "")])
        .await
        .assert_status(422);
    let label = w.label(&shipment).await;
    w.post(
        &w.hub,
        "/scans",
        &[("barcode", label.as_str()), ("kind", "hub")],
    )
    .await
    .assert_status(303);
    let after = w.reload(&shipment).await;
    assert_eq!(after.status, ShipmentStatus::AtHub);
    assert_eq!(after.parcels_scanned, 1);
}

/// AC: a scan needs `shipments.scan`.
#[renox::test]
async fn a_client_user_cannot_scan() {
    let w = boot().await;
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-1").await;
    let label = w.label(&shipment).await;
    w.post(
        &w.noor,
        &format!("/shipments/{}/pickup", shipment.id),
        &[("barcodes", label.as_str())],
    )
    .await
    .assert_forbidden();
}

/// The models match the migration and the templates compile.
#[renox::test]
async fn the_models_match_the_migration_and_the_views_compile() {
    let w = boot().await;
    w.app.assert_models_match_schema().await;
    w.app.assert_views_compile();
}

/// The staff list and the shipment page answer for a holder of
/// `shipments.view` and not for a client user.
#[renox::test]
async fn the_staff_pages_show_the_shipments() {
    let w = boot().await;
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-1").await;
    w.get(&w.dispatcher, "/shipments")
        .await
        .assert_ok()
        .assert_see(&shipment.number);
    w.get(&w.dispatcher, &format!("/shipments/{}", shipment.id))
        .await
        .assert_ok()
        .assert_see("ORD-1");
    w.get(&w.noor, "/shipments").await.assert_forbidden();
}
