//! BL-LOG-09 (#307), 03 §4.1 row *Recipient denies the delivery*, §7.1 rows
//! `open_review` and `close_review`, `WRK.ACL.06`, `LOG.RUL.06`: a disputed
//! delivery puts the shipment `under_review`, its COD is left out of the
//! remittances and its billing is held, the client is told once, and an
//! upheld review releases both. The delivery module is the real wl-delivery,
//! the payout of the paid case the real wl-payments.

mod support;

use logistics::notices::{CLIENT_NOTICE, REMITTANCE_REOPENED};
use logistics::shipment_machine::{CLOSE_REVIEW, OPEN_REVIEW};
use logistics::{RemittanceStatus, Shipment, ShipmentStatus};
use renox::prelude::*;
use renox::state_machine::{self, StateMachine, TransitionError};
use renox::testing::TestResponse;
use wl_kernel::notify::NotifyRecipient;

use support::{SOUQ, World, boot, boot_with_payments};

const ACCOUNT: &str = "AE070331234567890123456";

/// `by` raises the dispute of `shipment` from its page.
async fn dispute(w: &World, by: &User, shipment: &Shipment) -> TestResponse {
    w.post(
        by,
        &format!("/shipments/{}/dispute", shipment.id),
        &[("reason", "The recipient denies the delivery")],
    )
    .await
}

/// The review of the delivery of `shipment` is closed (upheld).
async fn close_review(w: &World, shipment: &Shipment) {
    let delivery = w.delivery_of(shipment).await;
    w.post(
        &w.dispatcher,
        &format!("/deliveries/{}/close-review", delivery.id),
        &[("outcome", "upheld"), ("note", "The recipient did sign")],
    )
    .await
    .assert_ok();
}

/// AC1: given a `delivered` shipment with COD AED 300 not yet remitted, when
/// `DeliveryDisputed` arrives, the shipment is `under_review`, the client is
/// notified once, and the next draft remittance leaves it out; its billing is
/// held.
#[renox::test]
async fn a_disputed_delivery_puts_the_shipment_under_review_and_out_of_the_draft() {
    let w = boot().await;
    let shipment = w.delivered(SOUQ, "ORD-1", "300").await;
    assert_eq!(Shipment::billable(w.db(), SOUQ).await.unwrap().len(), 1);

    // A client user cannot raise it; staff can, and only once.
    w.post(
        &w.noor,
        &format!("/shipments/{}/dispute", shipment.id),
        &[("reason", "Not mine")],
    )
    .await
    .assert_forbidden();
    dispute(&w, &w.dispatcher, &shipment).await.assert_status(303);

    let reviewed = w.reload(&shipment).await;
    assert_eq!(reviewed.status, ShipmentStatus::UnderReview);
    let ticket = reviewed.review_ticket_id.expect("the dispute ticket is linked");
    assert_eq!(w.tickets.opened().len(), 1);
    let notices = w.notices.of_type(CLIENT_NOTICE);
    assert_eq!(notices.len(), 1);
    assert_eq!(
        notices[0].recipients,
        vec![NotifyRecipient::Party { id: SOUQ }]
    );
    let body = notices[0].data["body"].as_str().unwrap();
    assert!(body.contains(&shipment.number) && body.contains(&format!("ticket {ticket}")));
    let events = reviewed.events(w.db()).await.unwrap();
    assert!(events.iter().any(|e| e.kind == "review_opened"));

    // Once per review: a second dispute finds it no longer delivered.
    dispute(&w, &w.dispatcher, &reviewed)
        .await
        .assert_status(422)
        .assert_see("Only a delivered shipment");
    assert_eq!(w.notices.of_type(CLIENT_NOTICE).len(), 1);

    // The next draft leaves it out, and so does the monthly billing.
    assert!(w.rule().await.is_empty());
    assert!(w.remittances_of(SOUQ).await.is_empty());
    assert!(Shipment::billable(w.db(), SOUQ).await.unwrap().is_empty());
}

/// AC2: given that shipment already in an `approved` remittance, the
/// remittance goes back to `draft` without the line and the accountant is
/// told. The other shipment stays on it.
#[renox::test]
async fn an_approved_remittance_holding_it_goes_back_to_a_draft_without_the_line() {
    let w = boot().await;
    let disputed = w.delivered(SOUQ, "ORD-1", "300").await;
    let other = w.delivered(SOUQ, "ORD-2", "200").await;
    w.rule().await;
    let draft = w.the_remittance_of(SOUQ).await;
    assert_eq!(draft.cod_total, 50_000);
    w.post(&w.bashar, &format!("/remittances/{}/approve", draft.id), &[])
        .await
        .assert_status(303);
    assert_eq!(w.remittance_status(&draft).await, RemittanceStatus::Approved);

    dispute(&w, &w.ops, &disputed).await.assert_status(303);

    let back = w.remittance(&draft).await;
    assert_eq!(back.status, RemittanceStatus::Draft);
    assert_eq!(back.approved_by, None);
    let lines = back.lines(w.db()).await.unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].shipment_id, other.id);
    assert_eq!(back.cod_total, 20_000);
    assert_eq!(back.net, 20_000);
    // The accountants are told which shipment took it back to a draft.
    let told = w.notices.of_type(REMITTANCE_REOPENED);
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].data["shipment"], disputed.number.as_str());
    let users = w.notices.users_of_type(REMITTANCE_REOPENED);
    assert!(users.contains(&w.amal.id) && users.contains(&w.bashar.id));
    // The history keeps the system's reopen.
    let history = state_machine::history(w.db(), logistics::Remittance::TABLE, draft.id)
        .await
        .unwrap();
    assert!(history.iter().any(|row| row.transition == "reopen_system"));
}

/// A line on a plain `draft` is removed and the figures are worked out again;
/// a `paid` remittance is never touched.
#[renox::test]
async fn a_draft_loses_the_line_and_a_paid_remittance_keeps_it() {
    let w = boot_with_payments().await;
    w.cod_terms(SOUQ, 0, Some(ACCOUNT)).await;
    let first = w.delivered(SOUQ, "ORD-1", "300").await;
    let second = w.delivered(SOUQ, "ORD-2", "200").await;
    w.rule().await;
    let draft = w.the_remittance_of(SOUQ).await;

    dispute(&w, &w.dispatcher, &first).await.assert_status(303);

    let after = w.remittance(&draft).await;
    assert_eq!(after.status, RemittanceStatus::Draft);
    assert_eq!(after.lines(w.db()).await.unwrap().len(), 1);
    assert_eq!(after.cod_total, 20_000);
    assert!(w.notices.of_type(REMITTANCE_REOPENED).is_empty());

    // The second one is paid; its review afterwards takes nothing back.
    w.post(&w.bashar, &format!("/remittances/{}/approve", draft.id), &[])
        .await
        .assert_status(303);
    w.post(
        &w.amal,
        &format!("/remittances/{}/pay", draft.id),
        &[("payment_reference", "TRF-0001")],
    )
    .await
    .assert_status(303);
    dispute(&w, &w.dispatcher, &second).await.assert_status(303);

    let paid = w.remittance(&draft).await;
    assert_eq!(paid.status, RemittanceStatus::Paid);
    assert_eq!(paid.lines(w.db()).await.unwrap().len(), 1);
    assert_eq!(paid.cod_total, 20_000);
    assert_eq!(w.reload(&second).await.status, ShipmentStatus::UnderReview);
}

/// AC3: when `DeliveryDisputeClosed` with *upheld* arrives, the shipment is
/// `delivered` and the next draft remittance includes AED 300 (and billing
/// takes it again).
#[renox::test]
async fn an_upheld_review_releases_the_cod_and_the_billing() {
    let w = boot().await;
    let shipment = w.delivered(SOUQ, "ORD-1", "300").await;
    dispute(&w, &w.dispatcher, &shipment).await.assert_status(303);
    assert!(w.rule().await.is_empty());

    close_review(&w, &shipment).await;

    let released = w.reload(&shipment).await;
    assert_eq!(released.status, ShipmentStatus::Delivered);
    assert!(released.delivered_at.is_some());
    let events = released.events(w.db()).await.unwrap();
    assert!(events.iter().any(|e| e.kind == "review_closed"));
    assert_eq!(Shipment::billable(w.db(), SOUQ).await.unwrap().len(), 1);
    let built = w.rule().await;
    assert_eq!(built.len(), 1);
    let draft = w.the_remittance_of(SOUQ).await;
    assert_eq!(draft.cod_total, 30_000);
    let lines = draft.lines(w.db()).await.unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].shipment_id, shipment.id);
}

/// AC4: no user can run `open_review` or `close_review` on the shipment: they
/// are not offered, and running one is refused for every role, the operations
/// manager included.
#[renox::test]
async fn no_user_can_open_or_close_a_review_of_a_shipment() {
    let w = boot().await;
    let shipment = w.delivered(SOUQ, "ORD-1", "300").await;
    for user in [&w.ops, &w.dispatcher, &w.amal, &w.driver, &w.noor] {
        let offered = w.offered(user, &shipment).await;
        assert!(!offered.iter().any(|n| n == OPEN_REVIEW || n == CLOSE_REVIEW));
        let result = shipment
            .transition(OPEN_REVIEW)
            .by(user)
            .run(w.state())
            .await;
        assert!(matches!(result, Err(TransitionError::Forbidden)));
    }
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::Delivered);

    dispute(&w, &w.dispatcher, &shipment).await.assert_status(303);
    let reviewed = w.reload(&shipment).await;
    assert_eq!(reviewed.status, ShipmentStatus::UnderReview);
    for user in [&w.ops, &w.dispatcher, &w.amal] {
        let offered = w.offered(user, &reviewed).await;
        assert!(!offered.iter().any(|n| n == OPEN_REVIEW || n == CLOSE_REVIEW));
        let result = reviewed
            .transition(CLOSE_REVIEW)
            .by(user)
            .run(w.state())
            .await;
        assert!(matches!(result, Err(TransitionError::Forbidden)));
    }
    assert_eq!(
        w.reload(&shipment).await.status,
        ShipmentStatus::UnderReview
    );
    // The machine declares both as the system's.
    let machine = Shipment::machine();
    assert!(machine.find(OPEN_REVIEW).unwrap().is_system());
    assert!(machine.find(CLOSE_REVIEW).unwrap().is_system());
}

/// With ticketing off the review still opens (there is no ticket to link).
#[renox::test]
async fn a_review_opens_without_a_ticket_when_ticketing_is_off() {
    let w = boot().await;
    let shipment = w.delivered(SOUQ, "ORD-1", "300").await;
    w.tickets.off(true);

    dispute(&w, &w.dispatcher, &shipment).await.assert_status(303);

    let reviewed = w.reload(&shipment).await;
    assert_eq!(reviewed.status, ShipmentStatus::UnderReview);
    assert_eq!(reviewed.review_ticket_id, None);
    let notices = w.notices.of_type(CLIENT_NOTICE);
    assert_eq!(notices.len(), 1);
    assert!(!notices[0].data["body"].as_str().unwrap().contains("ticket"));
}
