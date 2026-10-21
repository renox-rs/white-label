//! BL-LOG-08 (#306), 03 §4.1 steps 10-11 and row *Recipient pays less than
//! the COD amount or refuses to pay*, §6, §9, `WRK.ACL.09`, `LOG.COD.01`,
//! `LOG.ACL.12`: the operations manager chooses who absorbs a COD difference.
//! The *client* only on its instruction (deducted on the remittance line with
//! the reason), the *business* (a COD loss, the client still gets the full
//! amount) or the *driver* (the full amount stays expected in the driver's
//! cash handover); the *customer* is not offered. The delivery module is the
//! real wl-delivery, the custody of the driver's cash the real wl-payments.

mod support;

use logistics::cod;
use logistics::{RemittanceLine, Shipment, ShipmentStatus};
use renox::prelude::*;
use renox::testing::TestResponse;
use wl_delivery::{Delivery, Run};
use wl_kernel::custody::HandoverStatus;
use wl_kernel::delivery::{CodAbsorber, CodDecision};

use support::{SOUQ, World, boot, boot_with_payments};

const INSTRUCTION: &str = "agreed discount, client e-mail 2026-11-04";

/// A shipment with COD AED 300 out for delivery; the driver collected AED 250
/// and asked for the difference with the reason *agreed discount*.
async fn asked(w: &World, reference: &str) -> (Shipment, Delivery) {
    let shipment = w
        .book_with(&w.dispatcher, SOUQ, reference, &[("cod", "300")])
        .await;
    let (out, delivery) = w.out_for_delivery(&shipment).await;
    w.deliver_stop_with(
        &delivery,
        "250.00",
        &[("difference_reason", "agreed_discount")],
    )
    .await
    .assert_status(303);
    assert_eq!(w.reload(&out).await.status, ShipmentStatus::OutForDelivery);
    (out, delivery)
}

/// The operations manager approves with `fields`.
async fn approve(w: &World, delivery: &Delivery, fields: &[(&str, &str)]) -> TestResponse {
    w.post(
        &w.ops,
        &format!("/deliveries/{}/cod-difference/approve", delivery.id),
        fields,
    )
    .await
}

/// The line of the only remittance of Souq Express for `shipment`.
async fn line_of(w: &World, shipment: &Shipment) -> RemittanceLine {
    let draft = w.the_remittance_of(SOUQ).await;
    draft
        .lines(w.db())
        .await
        .unwrap()
        .into_iter()
        .find(|line| line.shipment_id == shipment.id)
        .expect("the shipment is on the draft")
}

/// AC1: COD AED 300 and AED 250 collected, approved with *client* and the
/// instruction "agreed discount, client e-mail 2026-11-04": the shipment
/// stores difference AED 50 absorbed by *client*, and the next draft
/// remittance shows AED 250 for it with the reason on the statement line.
#[renox::test]
async fn a_difference_the_client_absorbs_is_deducted_on_its_remittance_line_with_the_reason() {
    let w = boot().await;
    let (out, delivery) = asked(&w, "ORD-1").await;

    approve(
        &w,
        &delivery,
        &[("absorbed_by", "client"), ("instruction", INSTRUCTION)],
    )
    .await
    .assert_ok();

    let delivered = w.reload(&out).await;
    assert_eq!(delivered.status, ShipmentStatus::Delivered);
    assert_eq!(delivered.cod_amount, 30_000);
    assert_eq!(delivered.cod_collected, 25_000);
    assert_eq!(delivered.cod_difference, 5_000);
    assert_eq!(delivered.cod_absorbed_by.as_deref(), Some("client"));
    assert_eq!(
        delivered.cod_difference_reason.as_deref(),
        Some("agreed_discount")
    );
    assert_eq!(delivered.cod_approved_by, Some(w.ops.id));
    assert_eq!(delivered.cod_instruction.as_deref(), Some(INSTRUCTION));
    // The client's share is not a loss of the business.
    assert!(cod::loss_of(w.db(), out.id).await.unwrap().is_none());

    w.rule().await;

    let line = line_of(&w, &out).await;
    assert_eq!(line.cod_amount, 30_000);
    assert_eq!(line.absorbed, 5_000);
    assert_eq!(line.absorbed_reason, "agreed_discount");
    assert_eq!(line.collected_for_client(), 25_000);
    let draft = w.the_remittance_of(SOUQ).await;
    assert_eq!(draft.cod_total, 25_000);
    w.get(&w.amal, &format!("/remittances/{}", draft.id))
        .await
        .assert_ok()
        .assert_see("agreed_discount");
}

/// AC2: the same with *business*: the remittance shows AED 300 and a COD loss
/// of AED 50 is recorded for the depot (shipment, amount, reason, approver).
#[renox::test]
async fn a_difference_the_business_absorbs_is_a_cod_loss_and_the_client_gets_it_all() {
    let w = boot().await;
    let (out, delivery) = asked(&w, "ORD-1").await;

    approve(&w, &delivery, &[("absorbed_by", "business")])
        .await
        .assert_ok();

    let delivered = w.reload(&out).await;
    assert_eq!(delivered.cod_absorbed_by.as_deref(), Some("business"));
    assert_eq!(delivered.cod_difference, 5_000);
    let loss = cod::loss_of(w.db(), out.id)
        .await
        .unwrap()
        .expect("a COD loss is recorded");
    assert_eq!(loss.amount, 5_000);
    assert_eq!(loss.reason, "agreed_discount");
    assert_eq!(loss.approved_by, Some(w.ops.id));
    assert_eq!(loss.client_party_id, SOUQ.0);
    assert_eq!(cod::losses_total(w.db()).await.unwrap(), 5_000);

    w.rule().await;

    let line = line_of(&w, &out).await;
    assert_eq!(line.absorbed, 0);
    assert_eq!(line.collected_for_client(), 30_000);
    assert_eq!(w.the_remittance_of(SOUQ).await.cod_total, 30_000);
}

/// AC3: the same with *driver*: the remittance shows AED 300, no COD loss, and
/// the driver's handover (real wl-payments) expects AED 300.
#[renox::test]
async fn a_difference_the_driver_absorbs_stays_expected_in_the_drivers_handover() {
    let w = boot_with_payments().await;
    let (out, delivery) = asked(&w, "ORD-1").await;

    approve(&w, &delivery, &[("absorbed_by", "driver")])
        .await
        .assert_ok();

    let delivered = w.reload(&out).await;
    assert_eq!(delivered.cod_absorbed_by.as_deref(), Some("driver"));
    assert_eq!(delivered.cod_difference, 5_000);
    assert!(cod::loss_of(w.db(), out.id).await.unwrap().is_none());
    // The handover of the run expects the whole AED 300.
    let run = Run::find_or_404(w.db(), w.delivery_of(&out).await.run_id.unwrap())
        .await
        .unwrap();
    let handover = wl_delivery::close::handover_of(w.state(), &run)
        .await
        .unwrap()
        .expect("the driver's handover");
    assert_eq!(handover.holder_user, w.driver.id);
    assert_eq!(handover.status, HandoverStatus::Open);
    assert_eq!(handover.expected, 30_000);

    w.rule().await;

    let line = line_of(&w, &out).await;
    assert_eq!(line.absorbed, 0);
    assert_eq!(line.collected_for_client(), 30_000);
}

/// AC4: *client* without an instruction is refused; *customer* is not
/// offered. An instruction recorded on the shipment beforehand does for the
/// approval.
#[renox::test]
async fn the_client_needs_its_instruction_and_the_customer_is_not_offered() {
    let w = boot().await;
    let (out, delivery) = asked(&w, "ORD-1").await;

    approve(
        &w,
        &delivery,
        &[("absorbed_by", "client"), ("instruction", "  ")],
    )
    .await
    .assert_status(422)
    .assert_see("only on its instruction");
    approve(&w, &delivery, &[("absorbed_by", "customer")])
        .await
        .assert_status(422)
        .assert_see("cannot absorb");
    // Nothing is delivered and nothing is chosen.
    let still = w.reload(&out).await;
    assert_eq!(still.status, ShipmentStatus::OutForDelivery);
    assert_eq!(still.cod_absorbed_by, None);
    assert_eq!(still.cod_instruction, None);

    // The instruction is recorded on the shipment by staff who may; then the
    // approver chooses *client* without typing it again.
    w.post(
        &w.hub,
        &format!("/shipments/{}/cod-instruction", out.id),
        &[("note", INSTRUCTION)],
    )
    .await
    .assert_forbidden();
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/cod-instruction", out.id),
        &[("note", " ")],
    )
    .await
    .assert_status(422);
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/cod-instruction", out.id),
        &[("note", INSTRUCTION)],
    )
    .await
    .assert_status(303);
    approve(&w, &delivery, &[("absorbed_by", "client")])
        .await
        .assert_ok();

    let delivered = w.reload(&out).await;
    assert_eq!(delivered.status, ShipmentStatus::Delivered);
    assert_eq!(delivered.cod_absorbed_by.as_deref(), Some("client"));
    assert_eq!(delivered.cod_instruction.as_deref(), Some(INSTRUCTION));
    assert_eq!(delivered.cod_instruction_by, Some(w.dispatcher.id));
}

/// The shipment's own rule refuses the customer whatever the delivery module
/// offers: it is the seam wl-delivery asks before an approval goes through.
#[renox::test]
async fn the_shipments_rule_refuses_the_customer_and_a_client_without_instruction() {
    let w = boot().await;
    let (out, delivery) = asked(&w, "ORD-1").await;
    let rule = wl_kernel::delivery::cod_rule(w.state()).expect("the rule is provided");
    let decision = |absorber| CodDecision {
        delivery: delivery.delivery_id(),
        source: out.subject(),
        absorber,
        instruction: String::new(),
        approver: w.ops.id,
    };

    assert!(
        rule.check(w.state(), decision(CodAbsorber::Customer))
            .await
            .is_err()
    );
    assert!(
        rule.check(w.state(), decision(CodAbsorber::Client))
            .await
            .is_err()
    );
    assert!(
        rule.check(w.state(), decision(CodAbsorber::Business))
            .await
            .is_ok()
    );
    assert!(
        rule.check(w.state(), decision(CodAbsorber::Driver))
            .await
            .is_ok()
    );
    // Another source's delivery is none of its business.
    let other = CodDecision {
        source: wl_kernel::Subject::new("sales.order", 1),
        ..decision(CodAbsorber::Customer)
    };
    assert!(rule.check(w.state(), other).await.is_ok());
}

/// The report *COD differences by who absorbed them* lists the outcomes: the
/// totals per absorber and the shipments, for those who see remittances.
#[renox::test]
async fn the_report_lists_the_differences_by_who_absorbed_them() {
    let w = boot().await;
    let (_a, first) = asked(&w, "ORD-1").await;
    approve(
        &w,
        &first,
        &[("absorbed_by", "client"), ("instruction", INSTRUCTION)],
    )
    .await
    .assert_ok();
    let (_b, second) = asked(&w, "ORD-2").await;
    approve(&w, &second, &[("absorbed_by", "business")])
        .await
        .assert_ok();
    let (c, third) = asked(&w, "ORD-3").await;
    approve(&w, &third, &[("absorbed_by", "business")])
        .await
        .assert_ok();

    let (totals, rows) = cod::report(w.db()).await.unwrap();
    let of = |absorber: &str| {
        totals
            .iter()
            .find(|t| t.absorber == absorber)
            .map(|t| (t.count, t.total))
    };
    assert_eq!(of("client"), Some((1, 5_000)));
    assert_eq!(of("business"), Some((2, 10_000)));
    assert_eq!(of("driver"), None);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[2].number, c.number);

    w.get(&w.amal, "/cod-differences")
        .await
        .assert_ok()
        .assert_see("business")
        .assert_see(&c.number);
    w.get(&w.noor, "/cod-differences").await.assert_forbidden();
}
