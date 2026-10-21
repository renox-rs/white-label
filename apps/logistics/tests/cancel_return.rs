//! BL-LOG-02 (#300), 03 §4.1 *Client cancels*, §7.1 rows `cancel`,
//! `cancel_return`, `fail_attempt` and `return_delivered`, `LOG.ACL.09`,
//! BL-LOG-26 (the failure reason *cancelled by client*): a client cancels
//! before pickup; after pickup only a dispatcher sends the shipment back,
//! on the client's recorded instruction; out for delivery the driver first
//! fails the stop.

mod support;

use logistics::rates;
use logistics::shipment::charge_kind;
use logistics::{Charge, ShipmentStatus};
use renox::prelude::*;
use wl_delivery::{Attempt, DeliveryStatus};
use wl_kernel::notify::NotifyRecipient;

use support::{OTHER_CLIENT, SOUQ, boot};

const INSTRUCTION: &str = "client e-mail 2026-11-03 10:15";

async fn charges(w: &support::World, shipment: &logistics::Shipment) -> Vec<Charge> {
    w.reload(shipment).await.charges(w.db()).await.unwrap()
}

/// AC2: given a `picked_up` shipment, the client user has no cancel action;
/// when dispatcher A records the instruction and runs `cancel_return`, the
/// shipment is `returning`, its open delivery is cancelled, one return charge
/// is recorded, and the client is notified.
#[renox::test]
async fn a_dispatcher_sends_a_picked_up_shipment_back_on_the_clients_instruction() {
    let w = boot().await;
    rates::set_price(w.db(), SOUQ, charge_kind::RETURN, 1_500)
        .await
        .unwrap();
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-1").await;
    let picked = w.pick_up(&shipment).await;

    let offered = w.offered(&w.noor, &picked).await;
    assert!(!offered.contains(&"cancel".to_owned()), "{offered:?}");
    assert!(
        !offered.contains(&"cancel_return".to_owned()),
        "{offered:?}"
    );
    w.post(&w.noor, &format!("/shipments/{}/cancel", picked.id), &[])
        .await
        .assert_status(422);
    w.post(
        &w.noor,
        &format!("/shipments/{}/cancel-return", picked.id),
        &[("instruction", INSTRUCTION)],
    )
    .await
    .assert_forbidden();

    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/instruction", picked.id),
        &[("note", INSTRUCTION)],
    )
    .await
    .assert_status(303);
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/cancel-return", picked.id),
        &[("instruction", "")],
    )
    .await
    .assert_status(303);

    let after = w.reload(&shipment).await;
    assert_eq!(after.status, ShipmentStatus::Returning);
    assert_eq!(after.return_instruction.as_deref(), Some(INSTRUCTION));
    assert_eq!(after.return_instruction_by, Some(w.dispatcher.id));
    assert_eq!(
        w.delivery_of(&shipment).await.status,
        DeliveryStatus::Cancelled
    );
    let lines = charges(&w, &shipment).await;
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].kind, "return");
    assert_eq!(lines[0].amount, 1_500);

    let notices = w.notices.of_type("logistics.client_notice");
    assert_eq!(notices.len(), 1);
    assert_eq!(
        notices[0].recipients,
        vec![NotifyRecipient::Party { id: SOUQ }]
    );
    assert_eq!(notices[0].subject, shipment.subject());
    // Already returning: not offered again and no second charge.
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/cancel-return", picked.id),
        &[("instruction", INSTRUCTION)],
    )
    .await
    .assert_status(422);
    assert_eq!(charges(&w, &shipment).await.len(), 1);
}

/// AC2 (at the hub, on one call): the instruction can come with the call.
#[renox::test]
async fn the_instruction_can_come_with_the_call_and_a_card_without_a_return_line_costs_nothing() {
    let w = boot().await;
    let shipment = w.book_as(&w.farah, OTHER_CLIENT, "ORD-1").await;
    let at_hub = w.at_hub(&w.pick_up(&shipment).await).await;
    assert_eq!(at_hub.status, ShipmentStatus::AtHub);
    w.post(
        &w.ops,
        &format!("/shipments/{}/cancel-return", shipment.id),
        &[("instruction", "call 2026-11-03 09:00 by the shop owner")],
    )
    .await
    .assert_status(303);
    let after = w.reload(&shipment).await;
    assert_eq!(after.status, ShipmentStatus::Returning);
    assert_eq!(
        after.return_instruction.as_deref(),
        Some("call 2026-11-03 09:00 by the shop owner")
    );
    let lines = charges(&w, &shipment).await;
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].amount, 0);
    assert!(
        lines[0].note.contains("no return price"),
        "{}",
        lines[0].note
    );
}

/// AC3: given `cancel_return` without an instruction, it is refused.
#[renox::test]
async fn cancel_return_without_an_instruction_is_refused() {
    let w = boot().await;
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-1").await;
    let picked = w.pick_up(&shipment).await;
    let refused = w
        .post(
            &w.dispatcher,
            &format!("/shipments/{}/cancel-return", picked.id),
            &[("instruction", "  ")],
        )
        .await;
    refused.assert_status(422);
    refused.assert_see("instruction first: who asked");
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::PickedUp);
    assert!(charges(&w, &shipment).await.is_empty());
    assert!(w.notices.of_type("logistics.client_notice").is_empty());
}

/// AC4: given an `out_for_delivery` shipment, `cancel_return` is not offered;
/// after the driver's `fail_attempt` with *cancelled by client*, the attempt
/// count is unchanged and `cancel_return` is offered.
#[renox::test]
async fn out_for_delivery_the_driver_fails_the_stop_first_and_it_does_not_count() {
    let w = boot().await;
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-1").await;
    let (out, delivery) = w.out_for_delivery(&shipment).await;
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/instruction", out.id),
        &[("note", INSTRUCTION)],
    )
    .await
    .assert_status(303);
    let offered = w.offered(&w.dispatcher, &out).await;
    assert!(
        !offered.contains(&"cancel_return".to_owned()),
        "{offered:?}"
    );
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/cancel-return", out.id),
        &[("instruction", INSTRUCTION)],
    )
    .await
    .assert_status(422);
    assert_eq!(
        w.reload(&shipment).await.status,
        ShipmentStatus::OutForDelivery
    );

    w.fail_stop(&delivery, "cancelled_by_client")
        .await
        .assert_status(303);
    let attempted = w.reload(&shipment).await;
    assert_eq!(attempted.status, ShipmentStatus::Attempted);
    let after = w.delivery_of(&shipment).await;
    assert_eq!(after.attempts, 0);
    let visits = Attempt::query()
        .where_(Attempt::DELIVERY_ID.eq(after.id))
        .get(w.db())
        .await
        .unwrap();
    assert_eq!(visits.len(), 1);
    assert_eq!(
        visits[0].failure_reason.as_deref(),
        Some("cancelled_by_client")
    );
    assert!(!visits[0].counts);

    let offered = w.offered(&w.dispatcher, &attempted).await;
    assert!(offered.contains(&"cancel_return".to_owned()), "{offered:?}");
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/cancel-return", attempted.id),
        &[("instruction", "")],
    )
    .await
    .assert_status(303);
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::Returning);
}

/// AC5: when the client signs the e-POD for the returned goods,
/// `return_delivered` makes the shipment `returned`.
#[renox::test]
async fn the_clients_epod_makes_the_returned_shipment_returned() {
    let w = boot().await;
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-1").await;
    let picked = w.pick_up(&shipment).await;
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/cancel-return", picked.id),
        &[("instruction", INSTRUCTION)],
    )
    .await
    .assert_status(303);
    let path = format!("/shipments/{}/returned", shipment.id);
    // No e-POD, no return; hub staff cannot record it.
    w.post(&w.driver, &path, &[("pod_reference", " ")])
        .await
        .assert_status(422);
    w.post(&w.hub, &path, &[("pod_reference", "epod-0001")])
        .await
        .assert_forbidden();
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::Returning);

    w.post(&w.driver, &path, &[("pod_reference", "epod-0001")])
        .await
        .assert_status(303);
    let after = w.reload(&shipment).await;
    assert_eq!(after.status, ShipmentStatus::Returned);
    assert_eq!(after.return_pod_ref.as_deref(), Some("epod-0001"));
    let events = after.events(w.db()).await.unwrap();
    assert_eq!(events.last().unwrap().kind, "returned");
}
