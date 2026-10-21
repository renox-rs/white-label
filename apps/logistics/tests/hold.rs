//! BL-LOG-05 (#303), 03 §4.1 *Wrong or incomplete address* and *Parcel
//! damaged at the hub*, §7.1 rows `hold`, `release` and `return`,
//! `LOG.ACL.14`: a shipment with an exception is `on_hold` with a reason and
//! photos when damaged; it is released to the hub only with the confirmed
//! address or the client's recorded answer; the state it was held from is
//! kept; the client may choose return, and a liable company gets a claim
//! ticket.

mod support;

use logistics::shipment::charge_kind;
use logistics::{AddressChange, Shipment, ShipmentStatus, rates};
use renox::prelude::*;
use wl_delivery::{DeliveryStatus, NewRun, VehicleType};
use wl_kernel::documents::today;
use wl_kernel::notify::NotifyRecipient;

use support::{SOUQ, World, boot};

const TWO_PHOTOS: &str = "damage-1.jpg\ndamage-2.jpg";

async fn held_damaged(w: &World, reference: &str) -> Shipment {
    let shipment = w.book_as(&w.noor, SOUQ, reference).await;
    let at_hub = w.at_hub(&w.pick_up(&shipment).await).await;
    w.post(
        &w.hub,
        &format!("/shipments/{}/hold", at_hub.id),
        &[
            ("reason", "damaged"),
            ("photos", TWO_PHOTOS),
            ("barcode", w.label(&at_hub).await.as_str()),
        ],
    )
    .await
    .assert_status(303);
    w.reload(&at_hub).await
}

/// AC1: given an `at_hub` shipment, hub staff scan it and hold it with reason
/// *damaged* and 2 photos; it is `on_hold` and the client is notified. Without
/// photos the hold is refused.
#[renox::test]
async fn hub_staff_hold_a_damaged_shipment_with_photos_and_the_client_is_told() {
    let w = boot().await;
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-1").await;
    let at_hub = w.at_hub(&w.pick_up(&shipment).await).await;
    let path = format!("/shipments/{}/hold", at_hub.id);

    let refused = w
        .post(&w.hub, &path, &[("reason", "damaged"), ("photos", " ")])
        .await;
    refused.assert_status(422);
    refused.assert_see("photos of the damaged parcel");
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::AtHub);
    // A reason from the list, and a label of this shipment only.
    w.post(&w.hub, &path, &[("reason", "boredom")])
        .await
        .assert_status(422);
    w.post(
        &w.hub,
        &path,
        &[
            ("reason", "damaged"),
            ("photos", TWO_PHOTOS),
            ("barcode", "SHP-9999-1"),
        ],
    )
    .await
    .assert_status(422);
    // A client user has no hold.
    w.post(&w.noor, &path, &[("reason", "client_instruction")])
        .await
        .assert_forbidden();
    assert!(w.notices.of_type("logistics.client_notice").is_empty());

    let label = w.label(&at_hub).await;
    w.post(
        &w.hub,
        &path,
        &[
            ("reason", "damaged"),
            ("photos", TWO_PHOTOS),
            ("barcode", label.as_str()),
        ],
    )
    .await
    .assert_status(303);

    let held = w.reload(&shipment).await;
    assert_eq!(held.status, ShipmentStatus::OnHold);
    assert_eq!(held.hold_reason.as_deref(), Some("damaged"));
    assert_eq!(held.held_from.as_deref(), Some("at_hub"));
    assert_eq!(
        held.hold_photos.as_deref(),
        Some("damage-1.jpg\ndamage-2.jpg")
    );
    let events = held.events(w.db()).await.unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.kind == "hold_scan" && e.note == label)
    );
    assert!(
        events
            .iter()
            .any(|e| e.kind == "held" && e.note == "damaged")
    );
    let notices = w.notices.of_type("logistics.client_notice");
    assert_eq!(notices.len(), 1);
    assert_eq!(
        notices[0].recipients,
        vec![NotifyRecipient::Party { id: SOUQ }]
    );
    assert!(
        notices[0].data["body"]
            .as_str()
            .unwrap()
            .contains("damaged")
    );
}

/// AC1 (must not dispatch): a held shipment is not `at_hub`; its delivery on
/// a dispatched run does not send it out, and a shipment can be held from
/// `created` (nobody picks it up while held).
#[renox::test]
async fn a_held_shipment_is_not_dispatched_and_a_created_one_is_not_picked_up() {
    let w = boot().await;
    let held = held_damaged(&w, "ORD-1").await;
    let delivery = w.delivery_of(&held).await;
    let run = wl_delivery::plan::create_run(
        w.state(),
        NewRun::new(today(), w.driver.id).vehicle(VehicleType::Van, Some(support::VAN)),
    )
    .await
    .unwrap();
    let run_id = run.id.to_string();
    w.post(
        &w.dispatcher,
        &format!("/deliveries/{}/assign", delivery.id),
        &[("run_id", run_id.as_str())],
    )
    .await
    .assert_ok();
    w.post(&w.dispatcher, &format!("/runs/{}/dispatch", run.id), &[])
        .await
        .assert_ok();
    assert_eq!(w.reload(&held).await.status, ShipmentStatus::OnHold);

    let created = w.book_as(&w.noor, SOUQ, "ORD-2").await;
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/hold", created.id),
        &[("reason", "address_issue")],
    )
    .await
    .assert_status(303);
    assert_eq!(
        w.reload(&created).await.held_from.as_deref(),
        Some("created")
    );
    let codes = w.labels(&created).await;
    w.post(
        &w.driver,
        &format!("/shipments/{}/pickup", created.id),
        &[("barcodes", codes.as_str())],
    )
    .await
    .assert_status(422);
    assert_eq!(w.reload(&created).await.status, ShipmentStatus::OnHold);
}

/// AC2: given the client's instruction "deliver as is" recorded, `release`
/// puts it `at_hub`; without an instruction `release` is refused.
#[renox::test]
async fn release_needs_the_clients_recorded_instruction() {
    let w = boot().await;
    let held = held_damaged(&w, "ORD-1").await;
    let release = format!("/shipments/{}/release", held.id);

    let refused = w.post(&w.dispatcher, &release, &[]).await;
    refused.assert_status(422);
    refused.assert_see("instruction first: who asked");
    assert_eq!(w.reload(&held).await.status, ShipmentStatus::OnHold);

    let path = format!("/shipments/{}/hold-instruction", held.id);
    w.post(&w.farah, &path, &[("note", "deliver as is")])
        .await
        .assert_forbidden();
    w.post(&w.noor, &path, &[("note", "  ")])
        .await
        .assert_status(422);
    w.post(
        &w.noor,
        &path,
        &[("note", "deliver as is: portal 2026-11-04 09:10")],
    )
    .await
    .assert_status(303);
    // Hub staff do not release.
    w.post(&w.hub, &release, &[]).await.assert_forbidden();
    w.post(&w.dispatcher, &release, &[])
        .await
        .assert_status(303);

    let after = w.reload(&held).await;
    assert_eq!(after.status, ShipmentStatus::AtHub);
    assert_eq!(after.held_from.as_deref(), Some("at_hub"));
    assert_eq!(after.hold_instruction_by, Some(w.noor.id));
    let events = after.events(w.db()).await.unwrap();
    assert!(events.iter().any(|e| e.kind == "released"));
    // Released, it is not offered a second release.
    w.post(&w.dispatcher, &release, &[])
        .await
        .assert_status(422);
}

/// AC3: given an `attempted` shipment with failure reason *wrong address*, it
/// is `on_hold` with reason *address issue*; after the client confirms a new
/// address in zone B (price AED 25 instead of AED 20), `release` puts it
/// `at_hub` and its price is AED 25, with the change audited.
#[renox::test]
async fn a_wrong_address_holds_the_shipment_until_the_client_confirms_a_new_zone() {
    let w = boot().await;
    rates::set_band(w.db(), SOUQ, "Al Barsha", 5_000, 2_000)
        .await
        .unwrap();
    rates::set_band(w.db(), SOUQ, "Business Bay", 5_000, 2_500)
        .await
        .unwrap();
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-1").await;
    assert_eq!(shipment.price, 2_000);
    let (out, delivery) = w.out_for_delivery(&shipment).await;

    w.fail_stop(&delivery, "wrong_address")
        .await
        .assert_status(303);

    let held = w.reload(&out).await;
    assert_eq!(held.status, ShipmentStatus::OnHold);
    assert_eq!(held.hold_reason.as_deref(), Some("address_issue"));
    assert_eq!(held.held_from.as_deref(), Some("attempted"));
    // Not queued for a reattempt; the client is asked to confirm the address.
    let notices = w.notices.of_type("logistics.client_notice");
    assert_eq!(notices.len(), 1);
    assert!(
        notices[0].data["body"]
            .as_str()
            .unwrap()
            .contains("confirm"),
        "{:?}",
        notices[0].data
    );
    assert_eq!(w.delivery_of(&out).await.status, DeliveryStatus::Attempted);
    let events = held.events(w.db()).await.unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.kind == "held" && e.note == "address_issue")
    );

    let release = format!("/shipments/{}/release", out.id);
    w.post(&w.dispatcher, &release, &[])
        .await
        .assert_status(422);
    // Another client's user cannot confirm; a recorded instruction does not
    // stand for the address.
    let confirm = format!("/shipments/{}/address/confirm", out.id);
    w.post(&w.farah, &confirm, &[("address", "Tower 3, Business Bay")])
        .await
        .assert_forbidden();
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/hold-instruction", out.id),
        &[("note", "client called")],
    )
    .await
    .assert_status(303);
    w.post(&w.dispatcher, &release, &[])
        .await
        .assert_status(422);

    w.post(
        &w.noor,
        &confirm,
        &[
            ("address", "Tower 3, Office 12, Business Bay"),
            ("area", "Business Bay"),
        ],
    )
    .await
    .assert_status(303);
    let confirmed = w.reload(&out).await;
    assert_eq!(confirmed.price, 2_500);
    assert_eq!(confirmed.address, "Tower 3, Office 12, Business Bay");
    assert!(confirmed.address_confirmed_at.is_some());
    assert_eq!(confirmed.address_confirmed_by, Some(w.noor.id));
    let changes = AddressChange::query()
        .where_(AddressChange::SHIPMENT_ID.eq(out.id))
        .get(w.db())
        .await
        .unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].old_address, "Villa 12, Al Barsha 2");
    assert_eq!(changes[0].new_area, "Business Bay");
    assert_eq!((changes[0].old_price, changes[0].new_price), (2_000, 2_500));
    assert_eq!(changes[0].changed_by, Some(w.noor.id));
    // The delivery follows the correction.
    assert_eq!(
        w.delivery_of(&out).await.address,
        "Tower 3, Office 12, Business Bay"
    );

    w.post(&w.dispatcher, &release, &[])
        .await
        .assert_status(303);
    let after = w.reload(&out).await;
    assert_eq!(after.status, ShipmentStatus::AtHub);
    assert_eq!(after.price, 2_500);
}

/// AC3 (unchanged): "address unchanged, confirmed by the client" also
/// stands for the confirmation, with no audit row and no new price.
#[renox::test]
async fn an_address_confirmed_as_it_is_releases_without_a_change() {
    let w = boot().await;
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-1").await;
    let (out, delivery) = w.out_for_delivery(&shipment).await;
    w.fail_stop(&delivery, "wrong_address")
        .await
        .assert_status(303);
    w.post(
        &w.noor,
        &format!("/shipments/{}/address/confirm", out.id),
        &[],
    )
    .await
    .assert_status(303);
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/release", out.id),
        &[],
    )
    .await
    .assert_status(303);
    assert_eq!(w.reload(&out).await.status, ShipmentStatus::AtHub);
    let changes = AddressChange::query()
        .where_(AddressChange::SHIPMENT_ID.eq(out.id))
        .count(w.db())
        .await
        .unwrap();
    assert_eq!(changes, 0);
    let events = w.reload(&out).await.events(w.db()).await.unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.kind == "address_confirmed" && e.note.contains("unchanged"))
    );
}

/// AC3 (before dispatch): the client's user or customer service corrects the
/// address until the shipment is out for delivery, audited, and a zone change
/// prices it at once.
#[renox::test]
async fn the_address_is_corrected_until_the_shipment_is_out_for_delivery() {
    let w = boot().await;
    rates::set_band(w.db(), SOUQ, "Al Barsha", 5_000, 2_000)
        .await
        .unwrap();
    rates::set_band(w.db(), SOUQ, "Jumeirah", 5_000, 3_000)
        .await
        .unwrap();
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-1").await;
    let path = format!("/shipments/{}/address", shipment.id);
    w.post(&w.farah, &path, &[("address", "x"), ("area", "Jumeirah")])
        .await
        .assert_forbidden();
    w.post(&w.noor, &path, &[("address", " "), ("area", "Jumeirah")])
        .await
        .assert_status(422);
    w.post(
        &w.nadia,
        &path,
        &[("address", "Villa 3, Jumeirah 1"), ("area", "Jumeirah")],
    )
    .await
    .assert_status(303);
    let after = w.reload(&shipment).await;
    assert_eq!(after.price, 3_000);
    assert_eq!(after.area, "Jumeirah");
    let changes = AddressChange::query()
        .where_(AddressChange::SHIPMENT_ID.eq(shipment.id))
        .get(w.db())
        .await
        .unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].changed_by, Some(w.nadia.id));
    assert_eq!(w.delivery_of(&shipment).await.area, "Jumeirah");

    let (out, _delivery) = w.out_for_delivery(&shipment).await;
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/address", out.id),
        &[("address", "Villa 9"), ("area", "Al Barsha")],
    )
    .await
    .assert_status(422);
    assert_eq!(w.reload(&out).await.area, "Jumeirah");
}

/// AC4: given the client chooses return, `return` puts it `returning` with
/// one return charge.
#[renox::test]
async fn the_client_chooses_return_and_the_shipment_goes_back_with_one_charge() {
    let w = boot().await;
    rates::set_price(w.db(), SOUQ, charge_kind::RETURN, 1_500)
        .await
        .unwrap();
    let held = held_damaged(&w, "ORD-1").await;
    let path = format!("/shipments/{}/return", held.id);
    w.post(&w.hub, &path, &[]).await.assert_forbidden();
    w.post(
        &w.noor,
        &format!("/shipments/{}/hold-instruction", held.id),
        &[("note", "return it: portal 2026-11-04 09:10")],
    )
    .await
    .assert_status(303);
    w.post(&w.dispatcher, &path, &[]).await.assert_status(303);

    let after = w.reload(&held).await;
    assert_eq!(after.status, ShipmentStatus::Returning);
    let charges = after.charges(w.db()).await.unwrap();
    assert_eq!(charges.len(), 1);
    assert_eq!(charges[0].kind, "return");
    assert_eq!(charges[0].amount, 1_500);
    assert_eq!(w.delivery_of(&held).await.status, DeliveryStatus::Cancelled);
    // The hold told the client; so did the return.
    assert_eq!(w.notices.of_type("logistics.client_notice").len(), 2);
    // Already returning.
    w.post(&w.dispatcher, &path, &[]).await.assert_status(422);
    assert_eq!(after.charges(w.db()).await.unwrap().len(), 1);

    // An attempted shipment is sent back the same way; one at the hub is not.
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-2").await;
    let at_hub = w.at_hub(&w.pick_up(&shipment).await).await;
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/return", at_hub.id),
        &[],
    )
    .await
    .assert_status(422);
    let other = w.book_as(&w.noor, SOUQ, "ORD-3").await;
    let (_out, delivery) = w.out_for_delivery(&other).await;
    w.fail_stop(&delivery, "not_home").await.assert_status(303);
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/return", other.id),
        &[],
    )
    .await
    .assert_status(303);
    assert_eq!(w.reload(&other).await.status, ShipmentStatus::Returning);
}

/// AC5: given "company liable" ticked on a damaged hold, a claim ticket is
/// linked; ticked later, while on hold, the same.
#[renox::test]
async fn company_liable_on_a_damaged_hold_links_a_claim_ticket_once() {
    let w = boot().await;
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-1").await;
    let at_hub = w.at_hub(&w.pick_up(&shipment).await).await;
    w.post(
        &w.hub,
        &format!("/shipments/{}/hold", at_hub.id),
        &[
            ("reason", "damaged"),
            ("photos", TWO_PHOTOS),
            ("company_liable", "1"),
        ],
    )
    .await
    .assert_status(303);
    let held = w.reload(&shipment).await;
    assert!(held.company_liable);
    assert_eq!(held.claim_ticket_id, Some(1));
    assert_eq!(held.claim_ticket_number.as_deref(), Some("TKT-1"));
    let opened = w.tickets.opened();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].category_key, "claim");
    assert_eq!(opened[0].subject, shipment.subject());
    // Ticking it again opens no second ticket.
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/liable", shipment.id),
        &[],
    )
    .await
    .assert_status(303);
    assert_eq!(w.tickets.opened().len(), 1);

    // Later, while on hold; hub staff cannot tick it; a non-damaged hold has none.
    let second = w.book_as(&w.noor, SOUQ, "ORD-2").await;
    let second_hub = w.at_hub(&w.pick_up(&second).await).await;
    w.post(
        &w.hub,
        &format!("/shipments/{}/hold", second_hub.id),
        &[("reason", "damaged"), ("photos", "d.jpg")],
    )
    .await
    .assert_status(303);
    assert!(w.reload(&second).await.claim_ticket_id.is_none());
    let liable = format!("/shipments/{}/liable", second.id);
    w.post(&w.hub, &liable, &[]).await.assert_forbidden();
    w.post(&w.dispatcher, &liable, &[]).await.assert_status(303);
    let later = w.reload(&second).await;
    assert_eq!(later.claim_ticket_number.as_deref(), Some("TKT-2"));
    let third = w.book_as(&w.noor, SOUQ, "ORD-3").await;
    let third_hub = w.at_hub(&w.pick_up(&third).await).await;
    w.post(
        &w.hub,
        &format!("/shipments/{}/hold", third_hub.id),
        &[("reason", "client_instruction")],
    )
    .await
    .assert_status(303);
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/liable", third.id),
        &[],
    )
    .await
    .assert_status(422);
}

/// The client asks to stop delivery while disputing a re-weigh: the shipment
/// is held with reason *re-weigh dispute* (BL-LOG-04 meets BL-LOG-05).
#[renox::test]
async fn a_client_who_disputes_a_reweigh_may_ask_to_stop_the_delivery() {
    let w = boot().await;
    rates::set_band(w.db(), SOUQ, "", 2_000, 2_000)
        .await
        .unwrap();
    rates::set_band(w.db(), SOUQ, "", 4_000, 2_800)
        .await
        .unwrap();
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-1").await;
    let at_hub = w.at_hub(&w.pick_up(&shipment).await).await;
    w.post(
        &w.hub,
        &format!("/shipments/{}/reweigh", at_hub.id),
        &[("weight_kg", "4"), ("photo", "scale.jpg")],
    )
    .await
    .assert_status(303);
    w.post(
        &w.noor,
        &format!("/shipments/{}/reweigh-dispute", at_hub.id),
        &[("note", "it is 1 kg"), ("stop_delivery", "1")],
    )
    .await
    .assert_status(303);
    let held = w.reload(&shipment).await;
    assert_eq!(held.status, ShipmentStatus::OnHold);
    assert_eq!(held.hold_reason.as_deref(), Some("reweigh_dispute"));
    // Released on the client's recorded instruction.
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/hold-instruction", shipment.id),
        &[("note", "client accepts the ticket outcome")],
    )
    .await
    .assert_status(303);
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/release", shipment.id),
        &[],
    )
    .await
    .assert_status(303);
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::AtHub);
}
