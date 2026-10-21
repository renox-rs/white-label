//! BL-DLV-06 (#162) and BL-DLV-11 (#167) with the real `wl-ticketing`: the
//! claim ticket of a lost delivery and the dispatcher's task for an item
//! reported missing are real tickets (category, subject, assignee), not a
//! recording desk's entries.

mod support;

use renox::db::DateTime;
use renox::prelude::*;
use wl_delivery::lost;
use wl_delivery::{Attempt, Delivery, DeliveryStatus, NewRun, Trace, VehicleType, WlDelivery};
use wl_kernel::documents::today;
use wl_ticketing::{Comment, Ticket};

use support::{VAN, World, boot_with_real_tickets};

fn at(text: &str) -> DateTime {
    text.parse().expect("an instant")
}

/// A delivery that went out and whose visit failed, last traced at `when`.
async fn attempted_at(w: &World, when: DateTime) -> Delivery {
    let delivery = w.simple_delivery().await;
    let out = w.out_for_delivery(&delivery).await;
    w.fail(&w.driver, &out, "not_home").await.assert_status(303);
    Attempt::query()
        .where_(Attempt::DELIVERY_ID.eq(delivery.id))
        .update(w.db(), &[("occurred_at", &when)])
        .await
        .unwrap();
    Trace::query()
        .where_(Trace::DELIVERY_ID.eq(delivery.id))
        .update(w.db(), &[("occurred_at", &when)])
        .await
        .unwrap();
    let created = Some(when);
    Delivery::query()
        .where_(Delivery::ID.eq(delivery.id))
        .update(w.db(), &[("created_at", &created)])
        .await
        .unwrap();
    w.reload(&delivery).await
}

/// AC1 of #162 with a real ticket: the rule marks the delivery lost after 7
/// days; one claim ticket exists about the delivery and the delivery links
/// it. Found again, the ticket gets the depot as a comment.
#[renox::test]
async fn the_claim_ticket_of_a_lost_delivery_is_a_real_ticket_and_found_comments_it() {
    let w = boot_with_real_tickets(WlDelivery::new()).await;
    let delivery = attempted_at(&w, at("2026-11-01T06:00:00Z")).await;

    lost::run(w.state(), at("2026-11-08T22:00:00Z"))
        .await
        .unwrap();

    let after = w.reload(&delivery).await;
    assert_eq!(after.status, DeliveryStatus::Lost);
    let claim = after.claim_ticket_id.expect("a claim ticket was opened");
    let tickets = Ticket::query().get(w.db()).await.unwrap();
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0].id, claim);
    assert_eq!(tickets[0].subject_type, "delivery.delivery");
    assert_eq!(tickets[0].subject_id, delivery.id);
    assert!(tickets[0].title.contains(&delivery.number));
    assert!(tickets[0].description.contains("none"), "the liability");

    w.post(
        &w.ops_a,
        &format!("/deliveries/{}/found", delivery.id),
        &[("depot", "Sharjah depot")],
    )
    .await
    .assert_ok();

    let comments = Comment::of_ticket(w.db(), claim).await.unwrap();
    assert!(
        comments.iter().any(|c| c.text.contains("Sharjah depot")),
        "the claim ticket names the depot"
    );
}

/// AC3 of #167 with a real ticket: an undelivered delivery reported missing
/// at the depot gives a real task about the delivery, assigned to the holder
/// of the dispatcher role; a second receipt does not open another.
#[renox::test]
async fn the_missing_item_task_is_a_real_ticket_assigned_to_the_dispatcher() {
    let w = boot_with_real_tickets(WlDelivery::new().dispatcher_role("role_dina")).await;
    let missing = w.simple_delivery().await;
    let other = w.simple_delivery().await;
    w.valid_documents_for(w.driver.id, "2099-12-31");
    let run = wl_delivery::plan::create_run(
        w.state(),
        NewRun::new(today(), w.driver.id).vehicle(VehicleType::Van, Some(VAN)),
    )
    .await
    .unwrap();
    for delivery in [&missing, &other] {
        w.assign(&w.dispatcher, delivery, &run).await.assert_ok();
    }
    w.dispatch(&w.dispatcher, &run).await.assert_ok();
    for delivery in [&missing, &other] {
        w.fail(&w.driver, delivery, "not_home")
            .await
            .assert_status(303);
    }
    w.post(&w.driver, &format!("/driver/runs/{}/return", run.id), &[])
        .await
        .assert_ok();
    let (a, b) = (
        format!("item_{}_0", missing.id),
        format!("item_{}_0", other.id),
    );

    w.post(
        &w.dispatcher,
        &format!("/runs/{}/receipts", run.id),
        &[(a.as_str(), "missing"), (b.as_str(), "received")],
    )
    .await
    .assert_ok();

    let tickets = Ticket::query().get(w.db()).await.unwrap();
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0].subject_type, "delivery.delivery");
    assert_eq!(tickets[0].subject_id, missing.id);
    assert_eq!(tickets[0].assignee_user_id, Some(w.dispatcher.id));
    assert!(tickets[0].title.contains(&missing.number));
    assert_eq!(
        w.reload(&missing).await.status,
        DeliveryStatus::Attempted,
        "a missing delivery stays where it is"
    );
}
