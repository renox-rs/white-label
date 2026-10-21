//! BL-LOG-03 (#301), 03 §4.1 *Fewer parcels handed over than booked*, §7.1
//! rows `pick_up` and `receive_at_hub`, `LOG.HUB.01`: the driver scans the
//! parcels received and marks the rest *not received*; the shipment goes
//! ahead with what was received, repriced, and the client is told. With no
//! parcel handed over the pickup fails and the shipment stays `created`.

mod support;

use logistics::rates;
use logistics::{Parcel, Shipment, ShipmentStatus};
use renox::prelude::*;
use wl_kernel::notify::NotifyRecipient;

use support::{SOUQ, World, boot};

/// The client's bands: up to 10 kg AED 25, up to 15 kg AED 30.
async fn bands(w: &World) {
    rates::set_band(w.db(), SOUQ, "", 10_000, 2_500).await.unwrap();
    rates::set_band(w.db(), SOUQ, "", 15_000, 3_000).await.unwrap();
}

/// Three parcels of 5 kg each, AED 150 to collect.
async fn three_parcels(w: &World) -> Shipment {
    bands(w).await;
    w.book_with(
        &w.noor,
        SOUQ,
        "ORD-1",
        &[("parcels", "3"), ("weight_grams", "5000"), ("cod", "150")],
    )
    .await
}

async fn states(w: &World, shipment: &Shipment) -> Vec<String> {
    Parcel::query()
        .where_(Parcel::SHIPMENT_ID.eq(shipment.id))
        .order_by(Parcel::ID)
        .get(w.db())
        .await
        .unwrap()
        .into_iter()
        .map(|p| p.state)
        .collect()
}

/// AC1: given a `created` shipment of 3 parcels (5 kg each, zone price for
/// 15 kg = AED 30, for 10 kg = AED 25), when the driver scans 2 and marks 1
/// *not received*, then the shipment is `picked_up`, its weight is 10 kg, its
/// price is AED 25, and the client is told that 1 parcel was not received.
#[renox::test]
async fn scanning_two_of_three_parcels_reprices_the_shipment_and_tells_the_client() {
    let w = boot().await;
    let shipment = three_parcels(&w).await;
    assert_eq!(shipment.weight_grams, 15_000);
    assert_eq!(shipment.price, 3_000);
    let number = shipment.number.clone();

    w.post(
        &w.driver,
        &format!("/shipments/{}/pickup", shipment.id),
        &[
            ("barcodes", format!("{number}-1 {number}-2").as_str()),
            ("not_received", format!("{number}-3").as_str()),
        ],
    )
    .await
    .assert_status(303);

    let after = w.reload(&shipment).await;
    assert_eq!(after.status, ShipmentStatus::PickedUp);
    assert_eq!(after.weight_grams, 10_000);
    assert_eq!(after.price, 2_500);
    assert_eq!(after.booked_weight_grams, 15_000);
    assert_eq!(after.parcels_scanned, 2);
    assert_eq!(states(&w, &shipment).await, ["scanned", "scanned", "not_received"]);
    // The COD amount is not changed.
    assert_eq!(after.cod_amount, 15_000);

    let notices = w.notices.of_type("logistics.client_notice");
    assert_eq!(notices.len(), 1);
    assert_eq!(
        notices[0].recipients,
        vec![NotifyRecipient::Party { id: SOUQ }]
    );
    assert_eq!(notices[0].subject, shipment.subject());
    let title = notices[0].data["title"].as_str().unwrap();
    let body = notices[0].data["body"].as_str().unwrap();
    assert!(title.contains("1 parcel(s) not received"), "{title}");
    assert!(body.contains(&format!("{number}-3")), "{body}");
    assert!(body.contains("10 kg"), "{body}");
    assert!(body.contains("AED 25.00"), "{body}");
    assert!(body.contains("not changed (AED 150.00)"), "{body}");
    let events = after.events(w.db()).await.unwrap();
    assert!(events.iter().any(|e| e.kind == "picked_up" && e.note.contains("not received")));
}

/// AC1 (a parcel left out of both lists is not received either), and the
/// shipment picked up with every parcel keeps its weight, price and no notice.
#[renox::test]
async fn a_parcel_neither_scanned_nor_listed_is_not_received_and_a_full_pickup_is_not_told() {
    let w = boot().await;
    let shipment = three_parcels(&w).await;
    let number = shipment.number.clone();
    w.post(
        &w.driver,
        &format!("/shipments/{}/pickup", shipment.id),
        &[("barcodes", format!("{number}-1").as_str())],
    )
    .await
    .assert_status(303);
    let after = w.reload(&shipment).await;
    assert_eq!(after.weight_grams, 5_000);
    assert_eq!(after.price, 2_500);
    assert_eq!(states(&w, &shipment).await, ["scanned", "not_received", "not_received"]);

    let whole = w.book_as(&w.noor, SOUQ, "ORD-2").await;
    let picked = w.pick_up(&whole).await;
    assert_eq!(picked.weight_grams, whole.weight_grams);
    assert_eq!(picked.price, whole.price);
    assert_eq!(w.notices.of_type("logistics.client_notice").len(), 1);
}

/// AC1 (who): a client user has no scan; a pickup never refuses for missing
/// parcels, also when only one of three is handed over.
#[renox::test]
async fn only_a_scanning_user_picks_up_and_missing_parcels_never_refuse_it() {
    let w = boot().await;
    let shipment = three_parcels(&w).await;
    let number = shipment.number.clone();
    let path = format!("/shipments/{}/pickup", shipment.id);
    w.post(&w.noor, &path, &[("barcodes", format!("{number}-1").as_str())])
        .await
        .assert_forbidden();
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::Created);
    // A parcel both scanned and marked is a mistake.
    w.post(
        &w.driver,
        &path,
        &[
            ("barcodes", format!("{number}-1").as_str()),
            ("not_received", format!("{number}-1").as_str()),
        ],
    )
    .await
    .assert_status(422);
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::Created);
    w.post(&w.driver, &path, &[("barcodes", format!("{number}-1").as_str())])
        .await
        .assert_status(303);
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::PickedUp);
    // A shipment is picked up once.
    w.post(&w.driver, &path, &[("barcodes", format!("{number}-2").as_str())])
        .await
        .assert_status(422);
}

/// AC2: given no parcel scanned, `pick_up` is refused, a *pickup failed*
/// event is recorded with the reason, and the shipment stays `created`.
#[renox::test]
async fn no_parcel_scanned_fails_the_pickup_with_an_event_and_the_shipment_stays_created() {
    let w = boot().await;
    let shipment = three_parcels(&w).await;
    let number = shipment.number.clone();
    let path = format!("/shipments/{}/pickup", shipment.id);

    let refused = w
        .post(
            &w.driver,
            &path,
            &[
                ("barcodes", ""),
                ("not_received", format!("{number}-1 {number}-2 {number}-3").as_str()),
                ("reason", "client absent"),
            ],
        )
        .await;
    refused.assert_status(422);
    refused.assert_see("Scan at least one parcel");

    let after = w.reload(&shipment).await;
    assert_eq!(after.status, ShipmentStatus::Created);
    assert_eq!(after.parcels_scanned, 0);
    assert_eq!(after.weight_grams, 15_000);
    assert_eq!(states(&w, &shipment).await, ["booked", "booked", "booked"]);
    let events = after.events(w.db()).await.unwrap();
    let failed: Vec<_> = events.iter().filter(|e| e.kind == "pickup_failed").collect();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].note, "client absent");
    assert_eq!(failed[0].actor_user_id, Some(w.driver.id));
    assert!(w.notices.of_type("logistics.client_notice").is_empty());

    // It can be picked up again.
    w.post(&w.driver, &path, &[("barcodes", format!("{number}-1").as_str())])
        .await
        .assert_status(303);
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::PickedUp);
}

/// AC3: the parcel marked *not received* cannot be scanned in at the hub for
/// this shipment.
#[renox::test]
async fn a_parcel_not_received_cannot_be_scanned_in_at_the_hub() {
    let w = boot().await;
    let shipment = three_parcels(&w).await;
    let number = shipment.number.clone();
    w.post(
        &w.driver,
        &format!("/shipments/{}/pickup", shipment.id),
        &[
            ("barcodes", format!("{number}-1 {number}-2").as_str()),
            ("not_received", format!("{number}-3").as_str()),
        ],
    )
    .await
    .assert_status(303);

    let hub = format!("/shipments/{}/hub", shipment.id);
    let refused = w
        .post(
            &w.hub,
            &hub,
            &[("barcodes", format!("{number}-1 {number}-3").as_str())],
        )
        .await;
    refused.assert_status(422);
    refused.assert_see("was not handed over at pickup");
    // The scan page of one label refuses it too.
    w.post(
        &w.hub,
        "/scans",
        &[("barcode", format!("{number}-3").as_str()), ("kind", "hub")],
    )
    .await
    .assert_status(422);
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::PickedUp);
    assert_eq!(states(&w, &shipment).await[2], "not_received");

    w.post(
        &w.hub,
        &hub,
        &[("barcodes", format!("{number}-1 {number}-2").as_str())],
    )
    .await
    .assert_status(303);
    let at_hub = w.reload(&shipment).await;
    assert_eq!(at_hub.status, ShipmentStatus::AtHub);
    assert_eq!(at_hub.weight_grams, 10_000);
    assert_eq!(at_hub.parcels_scanned, 2);
    // Marking more parcels is for the pickup or a drop-off only.
    w.post(
        &w.hub,
        &hub,
        &[("barcodes", ""), ("not_received", format!("{number}-1").as_str())],
    )
    .await
    .assert_status(422);
}

/// AC1 (drop-off): `receive_at_hub` from `created` marks the parcels not
/// handed over the same way, recalculates and tells the client; with nothing
/// scanned it fails and the shipment stays `created`.
#[renox::test]
async fn a_drop_off_at_the_hub_follows_the_same_rule() {
    let w = boot().await;
    let shipment = three_parcels(&w).await;
    let number = shipment.number.clone();
    let hub = format!("/shipments/{}/hub", shipment.id);
    w.post(
        &w.hub,
        &hub,
        &[("barcodes", ""), ("reason", "nothing ready")],
    )
    .await
    .assert_status(422);
    let events = w.reload(&shipment).await.events(w.db()).await.unwrap();
    assert!(events.iter().any(|e| e.kind == "pickup_failed" && e.note == "nothing ready"));
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::Created);

    w.post(
        &w.hub,
        &hub,
        &[
            ("barcodes", format!("{number}-1 {number}-2").as_str()),
            ("not_received", format!("{number}-3").as_str()),
        ],
    )
    .await
    .assert_status(303);
    let after = w.reload(&shipment).await;
    assert_eq!(after.status, ShipmentStatus::AtHub);
    assert_eq!(after.weight_grams, 10_000);
    assert_eq!(after.price, 2_500);
    assert_eq!(states(&w, &shipment).await, ["scanned", "scanned", "not_received"]);
    assert_eq!(w.notices.of_type("logistics.client_notice").len(), 1);
}
