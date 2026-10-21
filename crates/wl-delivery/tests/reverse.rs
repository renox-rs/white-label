//! BL-DLV-05 (#161), `WRK.ACL.06`, `WRK.ACL.10`, `X-11`: a delivery marked
//! delivered by mistake is reversed by `reverse_delivery`, never by editing
//! the proof. A manager asks with a reason and a liability, a second manager
//! approves (never the requester, never the run's driver); the original
//! proof is kept and marked reversed, a non-counting *reversed* attempt is
//! added, the cash is taken out of the driver's open custody or, once
//! handed over, refunded to the payer or kept, and `DeliveryReversed` is
//! emitted.

mod support;

use renox::prelude::*;
use wl_delivery::{Attempt, Delivery, DeliveryStatus, Proof, Run, RunStatus};
use wl_kernel::PartyId;
use wl_kernel::approvals::{ApprovalRequest, ApprovalStatus};
use wl_kernel::payments::RefundSource;
use wl_kernel::tickets::{TASK_CATEGORY, TicketId};

use support::{
    World, boot_with_custody, boot_with_real_payments, chief_driver, stop_fields, user_with,
};

async fn ask(
    w: &World,
    by: &User,
    delivery: &Delivery,
    liability: &str,
) -> renox::testing::TestResponse {
    w.post(
        by,
        &format!("/deliveries/{}/reverse", delivery.id),
        &[
            ("reason", "marked_by_mistake"),
            ("liability", liability),
            ("note", "Wrong flat"),
        ],
    )
    .await
}

async fn approve(
    w: &World,
    by: &User,
    delivery: &Delivery,
    cash: &str,
) -> renox::testing::TestResponse {
    w.post(
        by,
        &format!("/deliveries/{}/reverse/approve", delivery.id),
        &[("cash_treatment", cash)],
    )
    .await
}

async fn proofs(w: &World, delivery: &Delivery) -> Vec<Proof> {
    Proof::query()
        .where_(Proof::DELIVERY_ID.eq(delivery.id))
        .order_by(Proof::ID)
        .get(w.db())
        .await
        .unwrap()
}

async fn attempts(w: &World, delivery: &Delivery) -> Vec<Attempt> {
    delivery.attempts_made(w.db()).await.unwrap()
}

async fn request_of(w: &World, delivery: &Delivery) -> ApprovalRequest {
    ApprovalRequest::pending_about(w.db(), &delivery.delivery_id().subject())
        .await
        .unwrap()
        .pop()
        .expect("a request waits for approval")
}

async fn run_of(w: &World, delivery: &Delivery) -> Run {
    Run::find_or_404(w.db(), delivery.run_id.unwrap())
        .await
        .unwrap()
}

/// A COD delivery of `collect` fils delivered in full for the party 5, on a
/// run that is still out.
async fn delivered_for_party(w: &World, collect: i64) -> Delivery {
    let delivery = w
        .request_for_party(5, &[("Tiles", 10_000, None)], collect)
        .await;
    let out = w.out_for_delivery(&delivery).await;
    let line = w.lines(&out).await.remove(0);
    let cash = format!("{}.{:02}", collect / 100, collect % 100);
    w.deliver(
        &w.driver,
        &out,
        &stop_fields(&[(line.id, "10", "0", "", "")], &cash),
    )
    .await
    .assert_status(303);
    w.reload(&out).await
}

/// The driver brings the run back and the depot counts the cash: the run is
/// closed and the cash handed over.
async fn close_run(w: &World, delivery: &Delivery, counted: i64) {
    let run = run_of(w, delivery).await;
    w.post(&w.driver, &format!("/driver/runs/{}/return", run.id), &[])
        .await
        .assert_ok();
    w.custody
        .count(
            w.state(),
            w.driver.id,
            &run.subject(),
            counted,
            w.dispatcher.id,
        )
        .await;
    assert_eq!(w.reload_run(&run).await.status, RunStatus::Closed);
}

/// AC1: a `delivered` COD delivery (AED 120 collected, not yet handed over)
/// on a `dispatched` run; manager A asks (*marked by mistake*, liability
/// *driver*) and manager B approves: the delivery is `out_for_delivery`, the
/// original proof is marked reversed, a *reversed* attempt exists, AED 120
/// leaves the driver's custody and `DeliveryReversed` is emitted.
#[renox::test]
async fn a_second_manager_reverses_a_delivery_and_the_cash_leaves_the_custody() {
    let w = boot_with_custody().await;
    let delivery = w.delivered_cod(12_000).await;
    assert_eq!(w.custody.collections().len(), 1);

    ask(&w, &w.ops_a, &delivery, "driver").await.assert_ok();
    // Nothing changes before the approval.
    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::Delivered);
    let request = request_of(&w, &delivery).await;
    assert_eq!(request.action, "delivery.reverse_delivery");
    assert_eq!(request.requester_id, w.ops_a.id);
    assert!(w.lost.all().is_empty());

    approve(&w, &w.ops_b, &delivery, "").await.assert_ok();

    let after = w.reload(&delivery).await;
    assert_eq!(after.status, DeliveryStatus::OutForDelivery);
    assert!(!after.frozen);
    let saved = proofs(&w, &delivery).await;
    assert_eq!(saved.len(), 1, "the original proof is kept");
    assert!(saved[0].reversed);
    assert_eq!(saved[0].cash_collected, 12_000);
    assert_eq!(saved[0].recipient_name, "Mariam Al Hosani");
    assert_eq!(saved[0].reversal_liability.as_deref(), Some("driver"));
    assert_eq!(saved[0].reversed_by, Some(w.ops_b.id));
    assert!(saved[0].reversed_at.is_some());
    assert!(
        saved[0]
            .reversal_reason
            .as_deref()
            .unwrap()
            .starts_with("Marked delivered by mistake")
    );
    let visits = attempts(&w, &delivery).await;
    assert_eq!(visits.len(), 2);
    assert_eq!(visits[1].failure_reason.as_deref(), Some("reversed"));
    assert!(!visits[1].counts);
    assert!(w.custody.collections().is_empty());
    assert_eq!(w.custody.released(), vec![delivery.delivery_id().subject()]);

    let events = w.reversed.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].delivery, delivery.delivery_id());
    assert_eq!(events[0].source, delivery.source());
    assert_eq!(events[0].liability, "driver");
    assert_eq!(events[0].proof, Some(saved[0].id));
    assert_eq!(events[0].cash_treatment.as_deref(), Some("removed"));
    assert_eq!(events[0].approver, Some(w.ops_b.id));
    assert!(events[0].reason.starts_with("Marked delivered by mistake"));
    // The driver's liability is followed up.
    let opened = w.tickets.opened();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].category_key, TASK_CATEGORY);
    // The reversal is a trace: the untraced clock restarts.
    let traced = wl_delivery::Trace::query()
        .where_(wl_delivery::Trace::DELIVERY_ID.eq(delivery.id))
        .where_(wl_delivery::Trace::KIND.eq("reversal"))
        .count(w.db())
        .await
        .unwrap();
    assert_eq!(traced, 1);
}

/// AC2: the same with the run `closed` and the cash handed over: the state
/// becomes `attempted` and B must choose *refund to payer* or *keep for next
/// attempt*; a refund is asked of payments for the payer.
#[renox::test]
async fn once_the_cash_was_handed_over_the_approver_chooses_a_refund_or_to_keep_it() {
    let w = boot_with_custody().await;
    let delivery = delivered_for_party(&w, 12_000).await;
    close_run(&w, &delivery, 12_000).await;

    ask(&w, &w.ops_a, &delivery, "business").await.assert_ok();
    // B must choose.
    approve(&w, &w.ops_b, &delivery, "")
        .await
        .assert_status(422)
        .assert_see("choose refund to payer or keep for next attempt");
    approve(&w, &w.ops_b, &delivery, "burn")
        .await
        .assert_status(422);
    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::Delivered);

    approve(&w, &w.ops_b, &delivery, "refund").await.assert_ok();

    let after = w.reload(&delivery).await;
    assert_eq!(after.status, DeliveryStatus::Attempted);
    assert!(proofs(&w, &delivery).await[0].reversed);
    let refunds = w.payments.refunds();
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].0.amount, 12_000);
    assert_eq!(refunds[0].0.party, PartyId(5));
    assert_eq!(
        refunds[0].0.source,
        RefundSource::Order(delivery.delivery_id().subject())
    );
    assert_eq!(refunds[0].1, Some(w.ops_b.id));
    let events = w.reversed.all();
    assert_eq!(events[0].cash_treatment.as_deref(), Some("refunded"));
    assert_eq!(events[0].liability, "business");
    // The cash stays in the counted handover: nothing is released.
    assert!(w.custody.released().is_empty());
    // Liability is not the driver's: no follow-up task.
    assert!(w.tickets.opened().is_empty());
}

/// AC2, the other choice: *keep for next attempt* asks no refund.
#[renox::test]
async fn keeping_the_handed_over_cash_asks_no_refund() {
    let w = boot_with_custody().await;
    let delivery = delivered_for_party(&w, 12_000).await;
    close_run(&w, &delivery, 12_000).await;

    ask(&w, &w.ops_a, &delivery, "none").await.assert_ok();
    approve(&w, &w.ops_b, &delivery, "keep").await.assert_ok();

    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::Attempted);
    assert!(w.payments.refunds().is_empty());
    assert_eq!(w.reversed.all()[0].cash_treatment.as_deref(), Some("kept"));
}

/// A refund needs a payer: a delivery whose recipient is no party cannot be
/// refunded, and the approval is refused with nothing changed.
#[renox::test]
async fn a_refund_needs_a_recipient_party() {
    let w = boot_with_custody().await;
    let delivery = w.delivered_cod(12_000).await;
    close_run(&w, &delivery, 12_000).await;

    ask(&w, &w.ops_a, &delivery, "none").await.assert_ok();
    approve(&w, &w.ops_b, &delivery, "refund")
        .await
        .assert_status(422)
        .assert_see("recipient party");
    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::Delivered);
    assert!(w.payments.refunds().is_empty());
}

/// The target state is worked out at the final approval: asked while the run
/// was out, approved after it closed, the delivery is `attempted`.
#[renox::test]
async fn the_target_state_is_worked_out_when_the_reversal_is_approved() {
    let w = boot_with_custody().await;
    let delivery = w.delivered_cod(12_000).await;
    ask(&w, &w.ops_a, &delivery, "none").await.assert_ok();

    close_run(&w, &delivery, 12_000).await;
    approve(&w, &w.ops_b, &delivery, "keep").await.assert_ok();

    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::Attempted);
}

/// AC3: manager A cannot approve their own request.
#[renox::test]
async fn the_requester_cannot_approve_their_own_reversal() {
    let w = boot_with_custody().await;
    let delivery = w.delivered_cod(12_000).await;
    ask(&w, &w.ops_a, &delivery, "none").await.assert_ok();

    approve(&w, &w.ops_a, &delivery, "")
        .await
        .assert_forbidden();

    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::Delivered);
    assert!(w.reversed.all().is_empty());
}

/// AC4: the driver of the run can neither request nor approve, even holding
/// `deliveries.reverse`; a reason and a liability are required.
#[renox::test]
async fn the_driver_of_the_run_can_neither_request_nor_approve() {
    let w = boot_with_custody().await;
    let chief = chief_driver(&w).await;
    let delivery = w.simple_delivery().await;
    let (out, _run) = w.out_for_delivery_by(&chief, &delivery).await;
    let line = w.lines(&out).await.remove(0);
    w.deliver(
        &chief,
        &out,
        &stop_fields(&[(line.id, "10", "0", "", "")], "0"),
    )
    .await
    .assert_status(303);

    ask(&w, &chief, &out, "none")
        .await
        .assert_status(422)
        .assert_see("driver of the run cannot request");
    assert!(
        ApprovalRequest::pending_about(w.db(), &out.delivery_id().subject())
            .await
            .unwrap()
            .is_empty()
    );

    ask(&w, &w.ops_a, &out, "none").await.assert_ok();
    approve(&w, &chief, &out, "").await.assert_forbidden();
    assert_eq!(w.reload(&out).await.status, DeliveryStatus::Delivered);
    // The page hides the button from the driver of the run.
    w.app.acting_as(&chief);
    w.app
        .get(&format!("/deliveries/{}", out.id))
        .await
        .assert_ok()
        .assert_dont_see("Reverse delivery")
        .assert_dont_see("Approve reversal");
}

/// A reversal needs a known reason and a liability, and the delivery must be
/// `delivered` or under review.
#[renox::test]
async fn a_reversal_needs_a_reason_a_liability_and_a_delivered_delivery() {
    let w = boot_with_custody().await;
    let delivery = w.delivered_cod(12_000).await;

    w.post(
        &w.ops_a,
        &format!("/deliveries/{}/reverse", delivery.id),
        &[("reason", "because"), ("liability", "none")],
    )
    .await
    .assert_status(422);
    w.post(
        &w.ops_a,
        &format!("/deliveries/{}/reverse", delivery.id),
        &[("reason", "wrong_recipient"), ("liability", "")],
    )
    .await
    .assert_status(422);
    // A user without the permission cannot ask.
    ask(&w, &w.dispatcher, &delivery, "none")
        .await
        .assert_forbidden();
    assert!(
        ApprovalRequest::pending_about(w.db(), &delivery.delivery_id().subject())
            .await
            .unwrap()
            .is_empty()
    );

    let pending = w.simple_delivery().await;
    ask(&w, &w.ops_a, &pending, "none").await.assert_status(422);
}

/// AC5: when B rejects, the delivery stays `delivered` and the proof is
/// unchanged; nothing of the effects runs.
#[renox::test]
async fn a_rejected_reversal_leaves_the_delivery_and_its_proof_alone() {
    let w = boot_with_custody().await;
    let delivery = w.delivered_cod(12_000).await;
    let before = proofs(&w, &delivery).await;
    ask(&w, &w.ops_a, &delivery, "driver").await.assert_ok();
    let request = request_of(&w, &delivery).await;

    let reject = format!("/deliveries/{}/reverse/reject", delivery.id);
    w.post(&w.ops_b, &reject, &[("comment", "")])
        .await
        .assert_status(422);
    w.post(&w.ops_b, &reject, &[("comment", "The proof is fine")])
        .await
        .assert_ok();

    assert_eq!(
        ApprovalRequest::find(w.db(), request.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ApprovalStatus::Rejected
    );
    let after = w.reload(&delivery).await;
    assert_eq!(after.status, DeliveryStatus::Delivered);
    let kept = proofs(&w, &delivery).await;
    assert_eq!(kept.len(), 1);
    assert!(!kept[0].reversed);
    assert_eq!(kept[0].id, before[0].id);
    assert_eq!(attempts(&w, &delivery).await.len(), 1);
    assert_eq!(w.custody.collections().len(), 1);
    assert!(w.reversed.all().is_empty());
}

/// AC6: attempts 2 of max 3 (a first delivery reversed, then delivered
/// again): after the reversal the count is still 2; every proof stays and the
/// delivery can be delivered and reversed again.
#[renox::test]
async fn the_reversed_attempt_does_not_count_and_every_proof_stays() {
    let w = boot_with_custody().await;
    let delivery = w.delivered_cod(12_000).await;
    assert_eq!(w.reload(&delivery).await.attempts, 1);

    ask(&w, &w.ops_a, &delivery, "none").await.assert_ok();
    approve(&w, &w.ops_b, &delivery, "").await.assert_ok();
    let again = w.reload(&delivery).await;
    assert_eq!(again.status, DeliveryStatus::OutForDelivery);
    assert_eq!(again.attempts, 1, "the reversed attempt does not count");

    let line = w.lines(&again).await.remove(0);
    w.deliver(
        &w.driver,
        &again,
        &stop_fields(&[(line.id, "10", "0", "", "")], "120.00"),
    )
    .await
    .assert_status(303);
    let second = w.reload(&delivery).await;
    assert_eq!(second.status, DeliveryStatus::Delivered);
    assert_eq!(second.attempts, 2);
    assert_eq!(second.max_attempts, 3);
    assert_eq!(w.custody.collections().len(), 1);

    ask(&w, &w.ops_a, &second, "none").await.assert_ok();
    approve(&w, &w.ops_b, &second, "").await.assert_ok();

    let last = w.reload(&delivery).await;
    assert_eq!(last.status, DeliveryStatus::OutForDelivery);
    assert_eq!(last.attempts, 2, "still 2 of 3 after the reversal");
    let saved = proofs(&w, &delivery).await;
    assert_eq!(saved.len(), 2);
    assert!(saved.iter().all(|p| p.reversed));
    assert_eq!(w.reversed.all().len(), 2);
}

/// AC7: an `under_review` delivery: after the reversal the frozen flag is
/// cleared and the review's ticket is resolved with the outcome *reversed*.
#[renox::test]
async fn reversing_a_delivery_under_review_clears_the_flag_and_resolves_the_ticket() {
    let w = boot_with_custody().await;
    let delivery = w.delivered_cod(12_000).await;
    w.open_review(&w.manager, &delivery, "recipient denies")
        .await
        .assert_ok();
    let reviewed = w.reload(&delivery).await;
    assert!(reviewed.frozen);
    assert_eq!(reviewed.status, DeliveryStatus::UnderReview);

    ask(&w, &w.ops_a, &reviewed, "none").await.assert_ok();
    approve(&w, &w.ops_b, &reviewed, "").await.assert_ok();

    let after = w.reload(&delivery).await;
    assert!(!after.frozen);
    assert_eq!(after.status, DeliveryStatus::OutForDelivery);
    let resolved = w.tickets.resolved();
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].0, TicketId(1));
    assert!(resolved[0].1.contains("Marked delivered by mistake"));
    // Only the review's ticket: no follow-up for a liability that is not the
    // driver's.
    assert_eq!(w.tickets.opened().len(), 1);
}

/// AC7, driver liable: the review's ticket stays open for the HR follow-up
/// and gets a comment.
#[renox::test]
async fn when_the_driver_is_liable_the_review_ticket_stays_open_for_hr() {
    let w = boot_with_custody().await;
    let delivery = w.delivered_cod(12_000).await;
    w.open_review(&w.manager, &delivery, "recipient denies")
        .await
        .assert_ok();
    let reviewed = w.reload(&delivery).await;

    ask(&w, &w.ops_a, &reviewed, "driver").await.assert_ok();
    approve(&w, &w.ops_b, &reviewed, "").await.assert_ok();

    assert!(!w.reload(&delivery).await.frozen);
    assert!(w.tickets.resolved().is_empty());
    assert_eq!(w.tickets.comments().len(), 1);
    assert_eq!(w.tickets.comments()[0].0, TicketId(1));
    assert_eq!(w.tickets.opened().len(), 1, "no second ticket");
    assert_eq!(w.reversed.all()[0].liability, "driver");
}

/// The screens: the second manager sees the request with both buttons, the
/// requester sees the request but cannot decide, and a plain dispatcher sees
/// neither.
#[renox::test]
async fn the_delivery_page_shows_the_request_to_the_second_manager() {
    let w = boot_with_custody().await;
    let delivery = w.delivered_cod(12_000).await;

    w.app.acting_as(&w.ops_a);
    w.app
        .get(&format!("/deliveries/{}", delivery.id))
        .await
        .assert_ok()
        .assert_see("Reverse delivery");
    ask(&w, &w.ops_a, &delivery, "driver").await.assert_ok();

    w.app.acting_as(&w.ops_b);
    w.app
        .get(&format!("/deliveries/{}", delivery.id))
        .await
        .assert_ok()
        .assert_see("Reversal waiting for approval")
        .assert_see("Approve reversal")
        .assert_see("Reject reversal");
    w.app.acting_as(&w.ops_a);
    w.app
        .get(&format!("/deliveries/{}", delivery.id))
        .await
        .assert_ok()
        .assert_see("Reversal waiting for approval")
        .assert_dont_see("Approve reversal");
    w.app.acting_as(&w.dispatcher);
    w.app
        .get(&format!("/deliveries/{}", delivery.id))
        .await
        .assert_ok()
        .assert_dont_see("Reverse delivery");
}

/// The driver's real handover of the delivery's run.
async fn real_handover(w: &World, delivery: &Delivery) -> wl_kernel::custody::HandoverSummary {
    let run = run_of(w, delivery).await;
    wl_delivery::close::handover_of(w.state(), &run)
        .await
        .unwrap()
        .expect("the driver's handover of the run")
}

/// AC1 of #161 on the real `wl-payments`: reversing a delivered COD delivery
/// while its run is out takes the AED 120 out of the driver's real handover.
#[renox::test]
async fn reversing_while_the_run_is_out_takes_the_cash_out_of_the_real_custody() {
    let w = boot_with_real_payments().await;
    let delivery = w.delivered_cod(12_000).await;
    assert_eq!(real_handover(&w, &delivery).await.expected, 12_000);

    ask(&w, &w.ops_a, &delivery, "driver").await.assert_ok();
    approve(&w, &w.ops_b, &delivery, "").await.assert_ok();

    assert_eq!(
        w.reload(&delivery).await.status,
        DeliveryStatus::OutForDelivery
    );
    let handover = real_handover(&w, &delivery).await;
    assert_eq!(handover.expected, 0);
    assert_eq!(handover.holder_user, w.driver.id);
    assert_eq!(
        w.reversed.all()[0].cash_treatment.as_deref(),
        Some("removed")
    );
}

/// AC2 of #161 on the real `wl-payments`: with the run closed and the cash
/// counted, *refund to payer* asks a real refund for the party, AED 120, from
/// the delivery as an order, waiting to be approved.
#[renox::test]
async fn a_refund_after_the_hand_over_is_a_real_requested_refund_of_the_order() {
    use wl_payments::permissions as pay;
    use wl_payments::refund::{Refund, RefundStatus};
    let w = boot_with_real_payments().await;
    let delivery = delivered_for_party(&w, 12_000).await;
    let run = run_of(&w, &delivery).await;
    w.post(&w.driver, &format!("/driver/runs/{}/return", run.id), &[])
        .await
        .assert_ok();
    let counter = user_with(&w, "Chandra", &[pay::CASH_VIEW, pay::CASH_COUNT]).await;
    let handover = real_handover(&w, &delivery).await;
    w.post(
        &counter,
        &format!("/cash/{}/count", handover.id.0),
        &[("counted", "120.00")],
    )
    .await
    .assert_ok();
    assert_eq!(w.reload_run(&run).await.status, RunStatus::Closed);

    ask(&w, &w.ops_a, &delivery, "business").await.assert_ok();
    approve(&w, &w.ops_b, &delivery, "refund").await.assert_ok();

    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::Attempted);
    let refunds = Refund::query().get(w.db()).await.unwrap();
    assert_eq!(refunds.len(), 1);
    let refund = &refunds[0];
    assert_eq!(refund.status, RefundStatus::Requested);
    assert_eq!(refund.amount, 12_000);
    assert_eq!(refund.currency, "AED");
    assert_eq!(refund.party_id, 5);
    assert_eq!(refund.source_kind, "order");
    assert_eq!(refund.source_subject(), delivery.delivery_id().subject());
    assert_eq!(refund.requested_by, Some(w.ops_b.id));
    let entity = wl_kernel::LegalEntity::default_entity(w.db())
        .await
        .unwrap();
    assert_eq!(refund.legal_entity_id, entity.id);
}
