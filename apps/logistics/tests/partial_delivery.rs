//! BL-LOG-07 (#305), 03 §4.1 row *Recipient refuses some parcels of a
//! multi-parcel shipment*, §6, §7.1 rows `deliver`, `create_return` and
//! `return_delivered`, `WRK.POD.01`, `LOG.RUL.12`: a recipient may accept some
//! parcels and refuse the others; the shipment is `delivered` for the accepted
//! parcels and the refused ones go back on a linked return shipment created
//! `returning` with the return charge. The delivery module is the real
//! wl-delivery (partial acceptance, proof, cash difference approval and run
//! close).

mod support;

use logistics::shipment::charge_kind;
use logistics::shipment_machine::{DELIVER, RefusedParcel, ReturnPlan, ShipmentInput};
use logistics::{Shipment, ShipmentStatus, rates};
use renox::prelude::*;
use renox::state_machine::{self, StateMachine, TransitionError};

use support::{SOUQ, World, boot};

/// A shipment of 3 parcels with COD `cod` dirhams, `out_for_delivery`.
async fn three_parcels(w: &World, cod: &str) -> (Shipment, wl_delivery::Delivery) {
    let shipment = w
        .book_with(
            &w.dispatcher,
            SOUQ,
            "ORD-1",
            &[("parcels", "3"), ("cod", cod)],
        )
        .await;
    w.out_for_delivery(&shipment).await
}

/// The only return shipment of `shipment`.
async fn the_return_of(w: &World, shipment: &Shipment) -> Shipment {
    let mut returns = shipment.returns(w.db()).await.unwrap();
    assert_eq!(returns.len(), 1, "one return shipment");
    returns.remove(0)
}

/// AC1: given a shipment of 3 parcels, when the recipient accepts 2 and
/// refuses 1 with reason *damaged* and a photo, then the shipment is
/// `delivered` with 2 parcels, a linked return shipment created by
/// `create_return` exists in `returning` (with no `created` history row) with
/// the refused parcel and one return charge, and `ShipmentDelivered` is
/// emitted.
#[renox::test]
async fn two_parcels_accepted_and_one_refused_makes_a_linked_return_shipment() {
    let w = boot().await;
    rates::set_price(w.db(), SOUQ, charge_kind::RETURN, 1_500)
        .await
        .unwrap();
    logistics::partial::set_pickup_address(w.db(), SOUQ, "Warehouse 4, Al Quoz", "Al Quoz")
        .await
        .unwrap();
    let (out, delivery) = three_parcels(&w, "300").await;
    let parcels = out.parcels(w.db()).await.unwrap();
    assert_eq!(parcels.len(), 3);

    w.deliver_parcels(&delivery, "300.00", &[(2, "damaged", "damage-photo-1")], &[])
        .await
        .assert_status(303);

    let delivered = w.reload(&out).await;
    assert_eq!(delivered.status, ShipmentStatus::Delivered);
    assert_eq!(delivered.parcel_count, 2);
    assert_eq!(delivered.parcels(w.db()).await.unwrap().len(), 2);
    assert_eq!(delivered.cod_amount, 30_000);
    // The return charge is on the return shipment only.
    assert!(delivered.charges(w.db()).await.unwrap().is_empty());

    let ret = the_return_of(&w, &delivered).await;
    assert_eq!(ret.status, ShipmentStatus::Returning);
    assert_eq!(ret.return_of_id, Some(delivered.id));
    assert_eq!(ret.client_party_id, SOUQ.0);
    assert_eq!(ret.cod_amount, 0);
    assert_eq!(ret.address, "Warehouse 4, Al Quoz");
    assert_eq!(ret.area, "Al Quoz");
    assert_eq!(ret.parcel_count, 1);
    let moved = ret.parcels(w.db()).await.unwrap();
    assert_eq!(moved.len(), 1);
    assert_eq!(moved[0].id, parcels[1].id);
    assert_eq!(moved[0].barcode, parcels[1].barcode);
    assert_eq!(moved[0].refusal_reason.as_deref(), Some("damaged"));
    let charges = ret.charges(w.db()).await.unwrap();
    assert_eq!(charges.len(), 1);
    assert_eq!(charges[0].kind, charge_kind::RETURN);
    assert_eq!(charges[0].amount, 1_500);
    // Born `returning`: the history starts with the insert, never `created`.
    let history = state_machine::history(w.db(), Shipment::TABLE, ret.id)
        .await
        .unwrap();
    assert_eq!(history[0].to_state, "returning");
    assert_eq!(history[0].reason.as_deref(), Some("create_return"));
    assert!(history.iter().all(|row| row.to_state != "created"));

    // `ShipmentDelivered` carries what was kept and what went back; the
    // expected COD stays the full amount.
    let events = w.shipment_delivered.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].shipment, delivered.id);
    assert_eq!(events[0].delivered_parcels.len(), 2);
    assert_eq!(events[0].refused_parcels, vec![parcels[1].barcode.clone()]);
    assert_eq!(events[0].return_shipment, Some(ret.id));
    assert_eq!(events[0].cod_expected, 30_000);
    let tracked = ret.events(w.db()).await.unwrap();
    assert!(tracked.iter().any(|e| e.kind == "return_created"));
}

/// AC2: given the refused parcel without a reason, `deliver` is refused: at
/// the stop (wl-delivery) and by the shipment's own guard.
#[renox::test]
async fn a_refused_parcel_without_a_reason_is_not_delivered() {
    let w = boot().await;
    let (out, delivery) = three_parcels(&w, "300").await;

    w.deliver_parcels(&delivery, "300.00", &[(2, "", "")], &[])
        .await
        .assert_status(422)
        .assert_see("refusal reason");
    let after = w.reload(&out).await;
    assert_eq!(after.status, ShipmentStatus::OutForDelivery);
    assert!(after.returns(w.db()).await.unwrap().is_empty());
    assert!(w.shipment_delivered.all().is_empty());

    // The shipment's guard says the same, whatever sends the event.
    let parcel = out.parcels(w.db()).await.unwrap().remove(1);
    let result = after
        .transition(DELIVER)
        .by_system()
        .input(&ShipmentInput {
            refused: vec![RefusedParcel {
                parcel_id: parcel.id,
                reason: " ".into(),
            }],
            return_plan: Some(ReturnPlan {
                number: "SHP-9999".into(),
                ..ReturnPlan::default()
            }),
            ..ShipmentInput::default()
        })
        .run(w.state())
        .await;
    assert!(matches!(result, Err(TransitionError::Guard(_))));
    assert_eq!(w.reload(&out).await.status, ShipmentStatus::OutForDelivery);
}

/// AC3: given a single-parcel shipment, no refused quantity can be entered:
/// the stop refuses it and the driver can only fail the attempt.
#[renox::test]
async fn a_single_parcel_shipment_cannot_be_partly_refused() {
    let w = boot().await;
    let shipment = w.book_as(&w.dispatcher, SOUQ, "ORD-1").await;
    let (out, delivery) = w.out_for_delivery(&shipment).await;
    assert!(!delivery.partial_acceptance);

    w.deliver_parcels(&delivery, "0.00", &[(1, "damaged", "photo-9")], &[])
        .await
        .assert_status(422)
        .assert_see("Partial acceptance is off");
    assert_eq!(w.reload(&out).await.status, ShipmentStatus::OutForDelivery);
    assert!(out.returns(w.db()).await.unwrap().is_empty());

    // Refusing the only parcel is a failed attempt.
    w.fail_stop(&delivery, "refused").await.assert_status(303);
    assert_eq!(w.reload(&out).await.status, ShipmentStatus::Attempted);
    assert!(out.returns(w.db()).await.unwrap().is_empty());
}

/// AC4: given a 3-parcel shipment with COD AED 300, 1 parcel refused and
/// AED 200 paid, the cash expected is AED 300, the AED 100 difference needs
/// the operations manager's approval with an absorber, and the return
/// shipment's COD is 0.
#[renox::test]
async fn the_cod_stays_in_full_when_a_parcel_is_refused_and_a_shortfall_needs_approval() {
    let w = boot().await;
    let (out, delivery) = three_parcels(&w, "300").await;

    w.deliver_parcels(
        &delivery,
        "200.00",
        &[(3, "wrong_item", "")],
        &[("difference_reason", "recipient_short")],
    )
    .await
    .assert_status(303);
    // Waiting for the operations manager: nothing is delivered yet.
    assert_eq!(w.reload(&out).await.status, ShipmentStatus::OutForDelivery);
    assert!(out.returns(w.db()).await.unwrap().is_empty());

    // The driver cannot approve the shortfall; the operations manager can.
    let path = format!("/deliveries/{}/cod-difference/approve", delivery.id);
    w.post(&w.driver, &path, &[("absorbed_by", "business")])
        .await
        .assert_forbidden();
    w.post(&w.ops, &path, &[("absorbed_by", "business")])
        .await
        .assert_ok();

    let delivered = w.reload(&out).await;
    assert_eq!(delivered.status, ShipmentStatus::Delivered);
    assert_eq!(delivered.cod_amount, 30_000);
    assert_eq!(delivered.cod_difference, 10_000);
    assert_eq!(delivered.cod_absorbed_by.as_deref(), Some("business"));
    let ret = the_return_of(&w, &delivered).await;
    assert_eq!(ret.cod_amount, 0);
    assert_eq!(ret.cod_difference, 0);
    let events = w.shipment_delivered.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].cod_expected, 30_000);
    assert_eq!(events[0].return_shipment, Some(ret.id));
}

/// AC5: when the client signs the e-POD for the return, the return shipment
/// is `returned`; the driver offers it, without the e-POD it is refused.
#[renox::test]
async fn the_clients_pod_makes_the_return_shipment_returned() {
    let w = boot().await;
    let (out, delivery) = three_parcels(&w, "0").await;
    w.deliver_parcels(&delivery, "0.00", &[(1, "damaged", "photo-1")], &[])
        .await
        .assert_status(303);
    let ret = the_return_of(&w, &w.reload(&out).await).await;
    let path = format!("/shipments/{}/returned", ret.id);
    assert!(
        w.offered(&w.driver, &ret)
            .await
            .iter()
            .any(|n| n == "return_delivered")
    );

    w.post(&w.driver, &path, &[("pod_reference", " ")])
        .await
        .assert_status(422);
    assert_eq!(w.reload(&ret).await.status, ShipmentStatus::Returning);
    w.post(&w.driver, &path, &[("pod_reference", "pod-client-0001")])
        .await
        .assert_status(303);

    let returned = w.reload(&ret).await;
    assert_eq!(returned.status, ShipmentStatus::Returned);
    assert_eq!(returned.return_pod_ref.as_deref(), Some("pod-client-0001"));
    // The original is untouched by it.
    assert_eq!(w.reload(&out).await.status, ShipmentStatus::Delivered);
}

/// The refused parcel comes back with the driver: at run close the depot's
/// receipt is written on the return shipment, and a parcel reported missing
/// stays on it.
#[renox::test]
async fn the_depot_receipt_of_a_refused_parcel_is_written_on_the_return_shipment() {
    let w = boot().await;
    let (out, delivery) = three_parcels(&w, "0").await;
    w.deliver_parcels(&delivery, "0.00", &[(1, "damaged", "photo-1")], &[])
        .await
        .assert_status(303);
    let run_id = w.delivery_of(&out).await.run_id.expect("on a run");
    w.post(&w.driver, &format!("/driver/runs/{run_id}/return"), &[])
        .await
        .assert_ok();
    let line = delivery.lines(w.db()).await.unwrap().remove(0);
    let received = format!("received_{}_{}", delivery.id, line.id);

    w.post(
        &w.dispatcher,
        &format!("/runs/{run_id}/receipts"),
        &[(received.as_str(), "1")],
    )
    .await
    .assert_ok();

    let ret = the_return_of(&w, &w.reload(&out).await).await;
    let barcode = ret.parcels(w.db()).await.unwrap().remove(0).barcode;
    let tracked = ret.events(w.db()).await.unwrap();
    assert!(
        tracked
            .iter()
            .any(|e| e.kind == "refused_received" && e.note == barcode)
    );
    assert!(!tracked.iter().any(|e| e.kind == "refused_missing"));
}
