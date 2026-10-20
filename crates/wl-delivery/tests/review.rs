//! BL-DLV-04 (#160), `WRK.ACL.06`, `X-11`: a delivered delivery that the
//! recipient or the client denies goes under review; its cash on delivery
//! and its billing are frozen until the dispatcher closes the review as
//! upheld. The ticket is the conversation; the events drive the invoice
//! flag (`wl-invoicing`).

mod support;

use renox::prelude::*;
use renox::state_machine;
use wl_delivery::{Delivery, DeliveryStatus, Trace};
use wl_kernel::delivery::{ReviewOutcome, requester};
use wl_kernel::tickets::TicketId;
use wl_kernel::DeliveryId;

use support::boot;

/// The history reasons and states of `delivery`, oldest first.
async fn history(w: &support::World, delivery: &Delivery) -> Vec<(String, String, Option<String>)> {
    state_machine::history(w.db(), Delivery::TABLE, delivery.id)
        .await
        .unwrap()
        .into_iter()
        .map(|h| (h.transition.clone(), h.to_state.clone(), h.reason.clone()))
        .collect()
}

/// AC1: a `delivered` COD delivery of AED 300; the dispatcher runs
/// `open_review` with reason "recipient denies": the state is `under_review`,
/// the frozen flag is set, a ticket is linked and `DeliveryDisputed` is
/// emitted.
#[renox::test]
async fn opening_a_review_freezes_the_delivery_links_a_ticket_and_emits_the_dispute() {
    let w = boot().await;
    let delivery = w.delivered_cod(30_000).await;
    assert!(!delivery.frozen);

    w.open_review(&w.manager, &delivery, "recipient denies")
        .await
        .assert_ok();

    let after = w.reload(&delivery).await;
    assert_eq!(after.status, DeliveryStatus::UnderReview);
    assert!(after.frozen);
    assert_eq!(after.review_reason.as_deref(), Some("recipient denies"));
    assert_eq!(after.review_ticket_id, Some(1));
    let opened = w.tickets.opened();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].category_key, "delivery_claim");
    assert_eq!(opened[0].subject, delivery.delivery_id().subject());
    assert_eq!(opened[0].description, "recipient denies");

    let events = w.disputed.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].delivery, delivery.delivery_id());
    assert_eq!(events[0].source, delivery.source());
    assert_eq!(events[0].reason, "recipient denies");
    assert_eq!(events[0].ticket, Some(TicketId(1)));
    let trail = history(&w, &after).await;
    assert_eq!(
        trail.last().cloned(),
        Some((
            "open_review".to_owned(),
            "under_review".to_owned(),
            Some("recipient denies".to_owned())
        ))
    );
    // The review is a trace: the untraced clock of the lost rule restarts.
    let traces = Trace::query()
        .where_(Trace::DELIVERY_ID.eq(delivery.id))
        .where_(Trace::KIND.eq("review"))
        .count(w.db())
        .await
        .unwrap();
    assert_eq!(traces, 1);
}

/// AC1 (denied): only a holder of `deliveries.manage` opens a review, and a
/// reason is required; nothing changes and no ticket is opened.
#[renox::test]
async fn opening_a_review_needs_the_permission_a_reason_and_a_delivered_delivery() {
    let w = boot().await;
    let delivery = w.delivered_cod(30_000).await;

    w.open_review(&w.dispatcher, &delivery, "recipient denies")
        .await
        .assert_forbidden();
    w.open_review(&w.manager, &delivery, "   ").await.assert_status(422);
    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::Delivered);
    assert!(w.tickets.opened().is_empty());
    assert!(w.disputed.all().is_empty());

    let pending = w.simple_delivery().await;
    w.open_review(&w.manager, &pending, "recipient denies")
        .await
        .assert_status(422);
    assert!(w.tickets.opened().is_empty());
}

/// AC2: the COD of AED 300 is excluded from a remittance prepared afterwards:
/// the cash a remittance may take (`remittable_cod` of the kernel contract)
/// is 300 before the review, 0 during it, and 300 again once it is closed.
#[renox::test]
async fn the_cash_of_a_delivery_under_review_is_held_out_of_remittances() {
    let w = boot().await;
    let delivery = w.delivered_cod(30_000).await;
    let id = DeliveryId(delivery.id);
    let desk = requester(w.state());
    assert_eq!(desk.remittable_cod(w.state(), id).await.unwrap(), 30_000);
    assert!(!desk.is_frozen(w.state(), id).await.unwrap());

    w.open_review(&w.manager, &delivery, "recipient denies")
        .await
        .assert_ok();
    assert_eq!(desk.remittable_cod(w.state(), id).await.unwrap(), 0);
    assert!(desk.is_frozen(w.state(), id).await.unwrap());

    w.close_review(&w.manager, &w.reload(&delivery).await, "Proof checked")
        .await
        .assert_ok();
    assert_eq!(desk.remittable_cod(w.state(), id).await.unwrap(), 30_000);
    assert!(!desk.is_frozen(w.state(), id).await.unwrap());
}

/// AC3: a dispute ticket raised by the recipient for a `delivered`
/// delivery; the system runs `open_review` with that ticket as the review's
/// ticket (no second ticket is opened).
#[renox::test]
async fn a_dispute_ticket_from_the_recipient_puts_the_delivery_under_review_as_the_system() {
    let w = boot().await;
    let delivery = w.delivered_cod(30_000).await;

    requester(w.state())
        .open_review(
            w.state(),
            delivery.delivery_id(),
            "Recipient says nothing arrived",
            Some(TicketId(900)),
        )
        .await
        .unwrap();

    let after = w.reload(&delivery).await;
    assert_eq!(after.status, DeliveryStatus::UnderReview);
    assert!(after.frozen);
    assert_eq!(after.review_ticket_id, Some(900));
    assert!(w.tickets.opened().is_empty());
    let events = w.disputed.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].ticket, Some(TicketId(900)));
    let trail = history(&w, &after).await;
    assert_eq!(trail.last().unwrap().0, "open_review_system");
}

/// AC3 (no ticket yet): the system opens the ticket itself when the dispute
/// came without one.
#[renox::test]
async fn the_system_opens_the_ticket_when_the_dispute_came_without_one() {
    let w = boot().await;
    let delivery = w.delivered_cod(30_000).await;

    requester(w.state())
        .open_review(w.state(), delivery.delivery_id(), "Client denies", None)
        .await
        .unwrap();

    assert_eq!(w.reload(&delivery).await.review_ticket_id, Some(1));
    assert_eq!(w.tickets.opened().len(), 1);
}

/// Edge: a second dispute while under review does not open another review;
/// the new complaint is linked to the existing ticket with a comment.
#[renox::test]
async fn a_second_dispute_is_linked_to_the_existing_ticket() {
    let w = boot().await;
    let delivery = w.delivered_cod(30_000).await;
    w.open_review(&w.manager, &delivery, "recipient denies")
        .await
        .assert_ok();
    let under_review = w.reload(&delivery).await;

    // A user is refused, naming the ticket.
    w.open_review(&w.manager, &under_review, "again")
        .await
        .assert_status(422)
        .assert_see("ticket 1");
    // The system links the new ticket as a comment on the first.
    requester(w.state())
        .open_review(w.state(), delivery.delivery_id(), "client also denies", Some(TicketId(55)))
        .await
        .unwrap();

    assert_eq!(w.disputed.all().len(), 1);
    let comments = w.tickets.comments();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].0, TicketId(1));
    assert!(comments[0].1.text.contains("55"));
    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::UnderReview);
}

/// Without `wl-ticketing` the review still opens, with no ticket and a
/// `DeliveryDisputed` whose ticket is empty.
#[renox::test]
async fn without_ticketing_the_review_opens_with_no_ticket() {
    let w = boot().await;
    w.tickets.off(true);
    let delivery = w.delivered_cod(30_000).await;

    w.open_review(&w.manager, &delivery, "recipient denies")
        .await
        .assert_ok();

    let after = w.reload(&delivery).await;
    assert_eq!(after.status, DeliveryStatus::UnderReview);
    assert_eq!(after.review_ticket_id, None);
    assert_eq!(w.disputed.all()[0].ticket, None);
}

/// AC4: the dispatcher runs `close_review` with outcome *upheld* and a note:
/// the delivery is `delivered`, the flag is cleared, the ticket is resolved
/// and `DeliveryDisputeClosed` with *upheld* is emitted.
#[renox::test]
async fn closing_a_review_as_upheld_releases_the_delivery_and_resolves_the_ticket() {
    let w = boot().await;
    let delivery = w.delivered_cod(30_000).await;
    w.open_review(&w.manager, &delivery, "recipient denies")
        .await
        .assert_ok();
    let under_review = w.reload(&delivery).await;

    w.close_review(&w.manager, &under_review, "Photo and signature match")
        .await
        .assert_ok();

    let after = w.reload(&delivery).await;
    assert_eq!(after.status, DeliveryStatus::Delivered);
    assert!(!after.frozen);
    assert_eq!(after.review_note.as_deref(), Some("Photo and signature match"));
    assert_eq!(
        w.tickets.resolved(),
        vec![(TicketId(1), "Photo and signature match".to_owned())]
    );
    let events = w.dispute_closed.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].delivery, delivery.delivery_id());
    assert_eq!(events[0].outcome, ReviewOutcome::Upheld);
    assert_eq!(events[0].ticket, Some(TicketId(1)));
    // The proof was not edited by the review.
    let proof = after.latest_proof(w.db()).await.unwrap().unwrap();
    assert_eq!(proof.cash_collected, 30_000);
    assert!(!proof.reversed);
}

/// AC5: `close_review` without a note is refused; the delivery stays under
/// review, frozen, and nothing is resolved or emitted.
#[renox::test]
async fn closing_a_review_without_a_note_is_refused() {
    let w = boot().await;
    let delivery = w.delivered_cod(30_000).await;
    w.open_review(&w.manager, &delivery, "recipient denies")
        .await
        .assert_ok();
    let under_review = w.reload(&delivery).await;

    w.close_review(&w.manager, &under_review, "  ")
        .await
        .assert_status(422);

    let after = w.reload(&delivery).await;
    assert_eq!(after.status, DeliveryStatus::UnderReview);
    assert!(after.frozen);
    assert!(w.tickets.resolved().is_empty());
    assert!(w.dispute_closed.all().is_empty());
}

/// Only *upheld* is offered: any other outcome is refused, and a closed
/// review cannot be closed twice.
#[renox::test]
async fn only_the_upheld_outcome_closes_a_review_and_only_once() {
    let w = boot().await;
    let delivery = w.delivered_cod(30_000).await;
    w.open_review(&w.manager, &delivery, "recipient denies")
        .await
        .assert_ok();
    let under_review = w.reload(&delivery).await;

    w.post(
        &w.manager,
        &format!("/deliveries/{}/close-review", delivery.id),
        &[("outcome", "reversed"), ("note", "x")],
    )
    .await
    .assert_status(422);
    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::UnderReview);

    w.close_review(&w.manager, &under_review, "Checked").await.assert_ok();
    w.close_review(&w.manager, &under_review, "Checked again")
        .await
        .assert_status(422);
    assert_eq!(w.dispute_closed.all().len(), 1);
}

/// Only a holder of `deliveries.manage` closes a review.
#[renox::test]
async fn closing_a_review_needs_the_manage_permission() {
    let w = boot().await;
    let delivery = w.delivered_cod(30_000).await;
    w.open_review(&w.manager, &delivery, "recipient denies")
        .await
        .assert_ok();
    let under_review = w.reload(&delivery).await;

    w.close_review(&w.dispatcher, &under_review, "Checked")
        .await
        .assert_forbidden();
    w.close_review(&w.driver, &under_review, "Checked")
        .await
        .assert_forbidden();

    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::UnderReview);
}

/// The delivery page offers `open_review` on a delivered delivery and
/// `close_review` on one under review, to a manager only.
#[renox::test]
async fn the_delivery_page_offers_the_review_actions_to_a_manager() {
    let w = boot().await;
    let delivery = w.delivered_cod(30_000).await;

    w.app.acting_as(&w.manager);
    w.app
        .get(&format!("/deliveries/{}", delivery.id))
        .await
        .assert_ok()
        .assert_see("Put under review");
    w.app.acting_as(&w.dispatcher);
    w.app
        .get(&format!("/deliveries/{}", delivery.id))
        .await
        .assert_ok()
        .assert_dont_see("Put under review");

    w.open_review(&w.manager, &delivery, "recipient denies")
        .await
        .assert_ok();
    w.app.acting_as(&w.manager);
    w.app
        .get(&format!("/deliveries/{}", delivery.id))
        .await
        .assert_ok()
        .assert_see("Close the review")
        .assert_see("recipient denies");
}
