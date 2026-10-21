//! BL-LOG-02 (#300), 03 §7.1 row `cancel`, `LOG.ACL.09`: before pickup a
//! client user of the company, or staff, cancels a `created` shipment; a
//! cancellation fee line is recorded only when the rate card has one, and the
//! delivery is cancelled with it.

mod support;

use logistics::rates;
use logistics::shipment::charge_kind;
use logistics::{Charge, ShipmentStatus};
use wl_delivery::DeliveryStatus;

use support::{OTHER_CLIENT, SOUQ, boot};

async fn charges(w: &support::World, shipment: &logistics::Shipment) -> Vec<Charge> {
    w.reload(shipment).await.charges(w.db()).await.unwrap()
}

/// AC1: given a `created` shipment and a rate card with a cancellation fee of
/// AED 10, the client user can `cancel` it and one cancellation fee line of
/// AED 10 is recorded for the next monthly invoice.
#[renox::test]
async fn a_client_user_cancels_a_created_shipment_with_a_fee_line() {
    let w = boot().await;
    rates::set_price(w.db(), SOUQ, charge_kind::CANCELLATION, 1_000)
        .await
        .unwrap();
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-1").await;
    assert!(
        w.offered(&w.noor, &shipment)
            .await
            .contains(&"cancel".to_owned())
    );
    w.post(&w.noor, &format!("/shipments/{}/cancel", shipment.id), &[])
        .await
        .assert_status(303);
    assert_eq!(
        w.reload(&shipment).await.status,
        ShipmentStatus::Cancelled
    );
    let lines = charges(&w, &shipment).await;
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].kind, "cancellation");
    assert_eq!(lines[0].amount, 1_000);
    assert_eq!(lines[0].client_party_id, SOUQ.0);
    // The open delivery goes with it.
    assert_eq!(
        w.delivery_of(&shipment).await.status,
        DeliveryStatus::Cancelled
    );
}

/// AC1: given a rate card without a cancellation fee, `cancel` records no fee
/// line.
#[renox::test]
async fn a_cancel_without_a_fee_on_the_rate_card_records_no_line() {
    let w = boot().await;
    let shipment = w.book_as(&w.farah, OTHER_CLIENT, "ORD-1").await;
    w.post(&w.farah, &format!("/shipments/{}/cancel", shipment.id), &[])
        .await
        .assert_status(303);
    assert_eq!(
        w.reload(&shipment).await.status,
        ShipmentStatus::Cancelled
    );
    assert!(charges(&w, &shipment).await.is_empty());
}

/// AC1 (who): a client user of the company cancels what a colleague booked,
/// a client user of another company does not.
#[renox::test]
async fn only_the_users_of_the_clients_company_cancel_its_shipment() {
    let w = boot().await;
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-1").await;
    let path = format!("/shipments/{}/cancel", shipment.id);
    w.post(&w.farah, &path, &[]).await.assert_status(422);
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::Created);
    w.post(&w.nadia, &path, &[]).await.assert_status(303);
    assert_eq!(
        w.reload(&shipment).await.status,
        ShipmentStatus::Cancelled
    );
}
