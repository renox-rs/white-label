//! BL-LOG-04 (#302), 03 §4.1 *Weight or size differs at the hub*, §7.2
//! `LOG.RUL.07`, `LOG.HUB.01`, `FIN.RAT.01` (the client's bands and
//! tolerance stand in for the rate card): hub staff record the measured
//! weight; above the client's tolerance the shipment is priced on it, the
//! surcharge waits for the next draft (held while a ticket is open) and the
//! client is told. The rate cards of wl-invoicing are not built: the bands
//! are `logistics::rates`, the draft is read through `logistics::billing`.

mod support;

use logistics::billing;
use logistics::reweigh::{self, Measurement};
use logistics::shipment::charge_kind;
use logistics::{Charge, Shipment, rates};
use renox::prelude::*;
use renox::testing::TestResponse;

use support::{SOUQ, World, boot};

/// Weight bands: 2 kg AED 20, 3 kg AED 24, 4 kg AED 28, 5 kg AED 32 and
/// 12 kg AED 45; the tolerance is 0.5 kg.
async fn card(w: &World, tolerance: i64, divisor: Option<i64>) {
    for (up_to, amount) in [
        (2_000, 2_000),
        (3_000, 2_400),
        (4_000, 2_800),
        (5_000, 3_200),
        (12_000, 4_500),
    ] {
        rates::set_band(w.db(), SOUQ, "", up_to, amount).await.unwrap();
    }
    reweigh::set_terms(w.db(), SOUQ, tolerance, divisor).await.unwrap();
}

/// A shipment of one parcel booked at `grams`, scanned in at the hub.
async fn at_the_hub(w: &World, reference: &str, grams: &str) -> Shipment {
    let shipment = w
        .book_with(&w.noor, SOUQ, reference, &[("weight_grams", grams)])
        .await;
    w.at_hub(&w.pick_up(&shipment).await).await
}

async fn measure(
    w: &World,
    shipment: &Shipment,
    kg: &str,
    photo: &str,
    size: [&str; 3],
) -> TestResponse {
    w.post(
        &w.hub,
        &format!("/shipments/{}/reweigh", shipment.id),
        &[
            ("weight_kg", kg),
            ("length_cm", size[0]),
            ("width_cm", size[1]),
            ("height_cm", size[2]),
            ("photo", photo),
        ],
    )
    .await
}

async fn surcharge(w: &World, shipment: &Shipment) -> Option<Charge> {
    reweigh::surcharge_of(w.db(), shipment).await.unwrap()
}

/// AC1: given a shipment booked at 2 kg (AED 20) with tolerance 0.5 kg, when
/// hub staff record 4 kg with a photo, then the shipment is priced at the 4 kg
/// band (AED 28), a re-weigh surcharge of AED 8 is recorded, and the client is
/// told.
#[renox::test]
async fn four_kilos_with_a_photo_prices_the_band_and_records_the_surcharge() {
    let w = boot().await;
    card(&w, 500, None).await;
    let shipment = at_the_hub(&w, "ORD-1", "2000").await;
    assert_eq!(shipment.price, 2_000);

    measure(&w, &shipment, "4", "scale-0001.jpg", ["", "", ""])
        .await
        .assert_status(303);

    let after = w.reload(&shipment).await;
    assert_eq!(after.price, 2_000);
    assert_eq!(after.reweigh_price, Some(2_800));
    assert_eq!(after.charged_price(), 2_800);
    assert_eq!(after.measured_weight_grams, Some(4_000));
    let line = surcharge(&w, &shipment).await.expect("a surcharge line");
    assert_eq!(line.kind, charge_kind::REWEIGH);
    assert_eq!(line.amount, 800);
    assert!(!line.held);
    // The cash to collect is not touched.
    assert_eq!(after.cod_amount, shipment.cod_amount);
    let measurements = reweigh::measurements(w.db(), &after).await.unwrap();
    assert_eq!(measurements.len(), 1);
    assert_eq!(measurements[0].photo_ref.as_deref(), Some("scale-0001.jpg"));
    assert_eq!(measurements[0].measured_by, Some(w.hub.id));

    let notices = w.notices.of_type("logistics.client_notice");
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].subject, shipment.subject());
    let body = notices[0].data["body"].as_str().unwrap();
    assert!(body.contains("AED 28.00"), "{body}");
    assert!(body.contains("AED 8.00"), "{body}");
    let events = after.events(w.db()).await.unwrap();
    assert!(events.iter().any(|e| e.kind == "reweighed"));
}

/// AC2: given a measured 2.4 kg, no surcharge and no notice.
#[renox::test]
async fn a_measurement_within_the_tolerance_does_not_reprice_or_tell_the_client() {
    let w = boot().await;
    card(&w, 500, None).await;
    let shipment = at_the_hub(&w, "ORD-1", "2000").await;
    // No photo is needed within the tolerance.
    measure(&w, &shipment, "2.4", "", ["", "", ""])
        .await
        .assert_status(303);
    let after = w.reload(&shipment).await;
    assert_eq!(after.charged_price(), 2_000);
    assert_eq!(after.reweigh_price, None);
    assert_eq!(after.measured_weight_grams, Some(2_400));
    assert!(surcharge(&w, &shipment).await.is_none());
    assert!(w.notices.of_type("logistics.client_notice").is_empty());
    assert_eq!(reweigh::measurements(w.db(), &after).await.unwrap().len(), 1);
}

/// AC3: given the same shipment re-weighed at 4 kg and then at 3 kg (3 kg band
/// AED 24), the latest counts: price AED 24, surcharge AED 4; re-weighed again
/// at 1.8 kg, price AED 20 and no surcharge.
#[renox::test]
async fn the_latest_measurement_counts_and_a_lighter_one_never_reprices() {
    let w = boot().await;
    card(&w, 500, None).await;
    let shipment = at_the_hub(&w, "ORD-1", "2000").await;
    measure(&w, &shipment, "4", "scale-1.jpg", ["", "", ""])
        .await
        .assert_status(303);
    assert_eq!(surcharge(&w, &shipment).await.unwrap().amount, 800);

    measure(&w, &shipment, "3", "scale-2.jpg", ["", "", ""])
        .await
        .assert_status(303);
    let after = w.reload(&shipment).await;
    assert_eq!(after.charged_price(), 2_400);
    assert_eq!(surcharge(&w, &shipment).await.unwrap().amount, 400);

    measure(&w, &shipment, "1.8", "", ["", "", ""])
        .await
        .assert_status(303);
    let after = w.reload(&shipment).await;
    assert_eq!(after.charged_price(), 2_000);
    assert_eq!(after.reweigh_price, None);
    assert!(surcharge(&w, &shipment).await.is_none());
    // Every measurement stays in the history.
    let all: Vec<Measurement> = reweigh::measurements(w.db(), &after).await.unwrap();
    let weights: Vec<i64> = all.iter().map(|m| m.weight_grams).collect();
    assert_eq!(weights, [4_000, 3_000, 1_800]);
    // The client was told of the two surcharges, not of the removal.
    assert_eq!(w.notices.of_type("logistics.client_notice").len(), 2);
}

/// AC4: given a rate card pricing by volume with divisor 5,000, a shipment
/// booked at 5 kg (5 kg band) and measured 4.8 kg with 50 x 40 x 30 cm
/// (volumetric 12 kg), it is priced at the 12 kg band.
#[renox::test]
async fn a_card_that_prices_by_volume_compares_the_volumetric_weight() {
    let w = boot().await;
    card(&w, 0, Some(5_000)).await;
    let shipment = at_the_hub(&w, "ORD-1", "5000").await;
    assert_eq!(shipment.price, 3_200);
    measure(&w, &shipment, "4.8", "scale-1.jpg", ["50", "40", "30"])
        .await
        .assert_status(303);
    let after = w.reload(&shipment).await;
    assert_eq!(after.measured_weight_grams, Some(12_000));
    assert_eq!(after.charged_price(), 4_500);
    assert_eq!(surcharge(&w, &shipment).await.unwrap().amount, 1_300);
    // Without the dimensions the actual 4.8 kg is below the booked weight.
    let other = at_the_hub(&w, "ORD-2", "5000").await;
    measure(&w, &other, "4.8", "", ["", "", ""])
        .await
        .assert_status(303);
    assert_eq!(w.reload(&other).await.charged_price(), 3_200);
}

/// AC5: given a shipment on the issued November invoice and a measurement of
/// 4 kg recorded afterwards, the November invoice is unchanged and the AED 8
/// surcharge is on the client's next draft.
#[renox::test]
async fn a_measurement_after_the_invoice_waits_for_the_next_draft() {
    let w = boot().await;
    card(&w, 500, None).await;
    let shipment = at_the_hub(&w, "ORD-1", "2000").await;
    let november = renox::db::now();
    billing::mark_shipment_invoiced(w.db(), shipment.id, november)
        .await
        .unwrap();
    assert!(billing::unbilled_surcharges(w.db(), SOUQ).await.unwrap().is_empty());

    measure(&w, &shipment, "4", "scale-1.jpg", ["", "", ""])
        .await
        .assert_status(303);

    let after = w.reload(&shipment).await;
    assert!(after.invoiced_on.is_some(), "the invoice marker stays");
    assert_eq!(after.price, 2_000, "the invoiced price is not changed");
    let draft = billing::unbilled_surcharges(w.db(), SOUQ).await.unwrap();
    assert_eq!(draft.len(), 1);
    assert_eq!(draft[0].shipment_id, shipment.id);
    assert_eq!(draft[0].amount, 800);
    // The January draft takes it; a later measurement does not touch a line
    // that is on an invoice.
    let ids: Vec<i64> = draft.iter().map(|c| c.id).collect();
    billing::mark_billed(w.db(), &ids, renox::db::now()).await.unwrap();
    assert!(billing::unbilled_surcharges(w.db(), SOUQ).await.unwrap().is_empty());
    measure(&w, &shipment, "3", "scale-2.jpg", ["", "", ""])
        .await
        .assert_status(303);
    assert_eq!(surcharge(&w, &shipment).await.unwrap().amount, 800);
}

/// AC6: given a measured 4 kg without a photo, saving is refused.
#[renox::test]
async fn above_the_tolerance_without_a_photo_saving_is_refused() {
    let w = boot().await;
    card(&w, 500, None).await;
    let shipment = at_the_hub(&w, "ORD-1", "2000").await;
    let refused = measure(&w, &shipment, "4", "  ", ["", "", ""]).await;
    refused.assert_status(422);
    refused.assert_see("photo of the scale");
    let after = w.reload(&shipment).await;
    assert_eq!(after.charged_price(), 2_000);
    assert_eq!(after.measured_weight_grams, None);
    assert!(reweigh::measurements(w.db(), &after).await.unwrap().is_empty());
    assert!(surcharge(&w, &shipment).await.is_none());
    assert!(w.notices.of_type("logistics.client_notice").is_empty());
    // Nobody reprices without a recorded measurement, and a client user has
    // no scale; a shipment not yet collected is not measured.
    w.post(
        &w.noor,
        &format!("/shipments/{}/reweigh", shipment.id),
        &[("weight_kg", "4"), ("photo", "x.jpg")],
    )
    .await
    .assert_forbidden();
    let created = w.book_as(&w.noor, SOUQ, "ORD-2").await;
    measure(&w, &created, "4", "x.jpg", ["", "", ""])
        .await
        .assert_status(422);
}

/// AC7: given an open re-weigh ticket on the shipment, the AED 8 surcharge is
/// not on the draft invoice; when the ticket confirms it, the next draft
/// includes it.
#[renox::test]
async fn an_open_ticket_holds_the_surcharge_out_of_the_draft_until_it_is_confirmed() {
    let w = boot().await;
    card(&w, 500, None).await;
    let shipment = at_the_hub(&w, "ORD-1", "2000").await;
    measure(&w, &shipment, "4", "scale-1.jpg", ["", "", ""])
        .await
        .assert_status(303);
    assert_eq!(billing::unbilled_surcharges(w.db(), SOUQ).await.unwrap().len(), 1);

    let path = format!("/shipments/{}/reweigh-dispute", shipment.id);
    // Another client's user cannot dispute it.
    w.post(&w.farah, &path, &[("note", "too heavy")])
        .await
        .assert_forbidden();
    w.post(&w.noor, &path, &[("note", "the parcel is 2 kg")])
        .await
        .assert_status(303);
    let opened = w.tickets.opened();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].category_key, "billing");
    assert_eq!(opened[0].subject, shipment.subject());
    assert_eq!(opened[0].description, "the parcel is 2 kg");
    let held = surcharge(&w, &shipment).await.unwrap();
    assert!(held.held);
    assert_eq!(held.ticket_id, Some(1));
    assert!(billing::unbilled_surcharges(w.db(), SOUQ).await.unwrap().is_empty());
    // The shipment itself is not held for it.
    assert_eq!(
        w.reload(&shipment).await.status,
        logistics::ShipmentStatus::AtHub
    );
    w.post(&w.noor, &path, &[("note", "again")])
        .await
        .assert_status(422);

    let resolve = format!("/shipments/{}/reweigh-dispute/resolve", shipment.id);
    w.post(&w.noor, &resolve, &[("outcome", "confirm")])
        .await
        .assert_forbidden();
    w.post(&w.hub, &resolve, &[("outcome", "confirm")])
        .await
        .assert_forbidden();
    w.post(&w.dispatcher, &resolve, &[("outcome", "confirm")])
        .await
        .assert_status(303);
    let draft = billing::unbilled_surcharges(w.db(), SOUQ).await.unwrap();
    assert_eq!(draft.len(), 1);
    assert_eq!(draft[0].amount, 800);
    assert_eq!(w.tickets.resolved().len(), 1);
}

/// AC7 (the other outcome): when the ticket removes the surcharge, it is
/// gone and the shipment is priced on the booked weight; with no surcharge
/// there is nothing to dispute.
#[renox::test]
async fn a_ticket_that_removes_the_surcharge_restores_the_booked_price() {
    let w = boot().await;
    card(&w, 500, None).await;
    let shipment = at_the_hub(&w, "ORD-1", "2000").await;
    let path = format!("/shipments/{}/reweigh-dispute", shipment.id);
    w.post(&w.dispatcher, &path, &[("note", "nothing to dispute")])
        .await
        .assert_status(422);
    measure(&w, &shipment, "4", "scale-1.jpg", ["", "", ""])
        .await
        .assert_status(303);
    w.post(&w.dispatcher, &path, &[("note", "weighed twice")])
        .await
        .assert_status(303);
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/reweigh-dispute/resolve", shipment.id),
        &[("outcome", "remove")],
    )
    .await
    .assert_status(303);
    let after = w.reload(&shipment).await;
    assert_eq!(after.charged_price(), 2_000);
    assert_eq!(after.reweigh_price, None);
    assert!(surcharge(&w, &shipment).await.is_none());
    assert!(billing::unbilled_surcharges(w.db(), SOUQ).await.unwrap().is_empty());
    assert_eq!(w.tickets.resolved().len(), 1);
}
