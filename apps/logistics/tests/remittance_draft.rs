//! BL-LOG-14 (#312), 03 §4.1 step 11, §7.2 `LOG.RUL.06`, `LOG.COD.01`: the
//! weekly rule builds one draft COD remittance per client from the COD
//! recorded on delivered shipments, whatever happened to the cash after
//! collection, less the client's fee and the differences the client
//! absorbed, leaves out the shipments under review and rebuilds the draft
//! (never duplicates it). The delivery module is the real wl-delivery, the
//! cash custody the real wl-payments.

mod support;

use logistics::notices::REMITTANCE_DRAFTED;
use logistics::adjustment::{self, AdjustmentLine, AdjustmentStatus};
use logistics::remit;
use logistics::{Remittance, RemittanceLine, RemittanceStatus, Shipment, ShipmentStatus};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_delivery::{Run, close};
use wl_kernel::custody::HandoverStatus;
use wl_payments::permissions as pay;

use support::{OTHER_CLIENT, SOUQ, World, boot_with_payments};

const ACCOUNT: &str = "AE070331234567890123456";

/// The lines of `remittance`, with the shipment numbers.
async fn lines_of(w: &World, remittance: &Remittance) -> Vec<(String, RemittanceLine)> {
    let mut out = Vec::new();
    for line in remittance.lines(w.db()).await.unwrap() {
        let shipment = w.shipment(line.shipment_id).await;
        out.push((shipment.number, line));
    }
    out
}

/// The delivery of `shipment` is put under review by the dispatcher.
async fn open_review(w: &World, shipment: &Shipment) {
    let delivery = w.delivery_of(shipment).await;
    w.post(
        &w.dispatcher,
        &format!("/deliveries/{}/open-review", delivery.id),
        &[("reason", "Recipient denies the delivery")],
    )
    .await
    .assert_ok();
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

/// The draft is approved by Bashar and paid by Amal.
async fn pay_draft(w: &World, remittance: &Remittance) {
    w.post(
        &w.bashar,
        &format!("/remittances/{}/approve", remittance.id),
        &[],
    )
    .await
    .assert_status(303);
    w.post(
        &w.amal,
        &format!("/remittances/{}/pay", remittance.id),
        &[("payment_reference", "TRF-0001")],
    )
    .await
    .assert_status(303);
}

/// AC1: client C with 3 `delivered` shipments (COD AED 200, 300, 500), a COD
/// fee of 2% and one driver handover `short` by AED 300: the Monday draft
/// shows COD AED 1,000, fees AED 20 and net AED 980, and the accountant is
/// notified.
#[renox::test]
async fn the_draft_pays_the_cod_recorded_whatever_the_drivers_handover_says() {
    let w = boot_with_payments().await;
    w.cod_terms(SOUQ, 200, Some(ACCOUNT)).await;
    w.delivered(SOUQ, "ORD-1", "200").await;
    w.delivered(SOUQ, "ORD-2", "300").await;
    let big = w.delivered(SOUQ, "ORD-3", "500").await;

    // The driver brings back the run of the 500 and hands over AED 200 only.
    let run = Run::find_or_404(w.db(), w.delivery_of(&big).await.run_id.unwrap())
        .await
        .unwrap();
    w.post(&w.driver, &format!("/driver/runs/{}/return", run.id), &[])
        .await
        .assert_ok();
    let handover = close::handover_of(w.state(), &run).await.unwrap().unwrap();
    let counter = w
        .user_with("Chandra", &[pay::CASH_VIEW, pay::CASH_COUNT])
        .await;
    w.post(
        &counter,
        &format!("/cash/{}/count", handover.id),
        &[("counted", "200.00")],
    )
    .await
    .assert_ok();
    let after = close::handover_of(w.state(), &run).await.unwrap().unwrap();
    assert_eq!(after.status, HandoverStatus::Short);
    assert_eq!(after.difference, 30_000);

    let built = w.rule().await;

    assert_eq!(built.len(), 1);
    let draft = w.the_remittance_of(SOUQ).await;
    assert_eq!(draft.status, RemittanceStatus::Draft);
    assert_eq!(draft.cod_total, 100_000);
    assert_eq!(draft.fee_total, 2_000);
    assert_eq!(draft.net, 98_000);
    assert_eq!(draft.lines(w.db()).await.unwrap().len(), 3);
    // The accountants (holders of `remittances.prepare`) are told.
    let told = w.notices.of_type(REMITTANCE_DRAFTED);
    assert_eq!(told.len(), 1);
    let users = w.notices.users_of_type(REMITTANCE_DRAFTED);
    assert!(users.contains(&w.amal.id) && users.contains(&w.bashar.id));
    assert!(!users.contains(&w.driver.id));
    assert!(told[0].data["summary"].as_str().unwrap().contains("net AED 980.00"));
}

/// AC2: a fourth shipment `under_review` (COD AED 150) is not on the draft;
/// after `close_review` it is on the next draft, and the shipment already on
/// a paid remittance is not on it twice.
#[renox::test]
async fn a_shipment_under_review_waits_and_is_picked_up_after_the_review() {
    let w = boot_with_payments().await;
    w.cod_terms(SOUQ, 200, Some(ACCOUNT)).await;
    let first = w.delivered(SOUQ, "ORD-1", "200").await;
    let second = w.delivered(SOUQ, "ORD-2", "300").await;
    let reviewed = w.delivered(SOUQ, "ORD-4", "150").await;
    open_review(&w, &reviewed).await;

    w.rule().await;
    let monday = w.the_remittance_of(SOUQ).await;
    let numbers: Vec<String> = lines_of(&w, &monday)
        .await
        .into_iter()
        .map(|(number, _)| number)
        .collect();
    assert_eq!(numbers, [first.number.clone(), second.number.clone()]);
    assert_eq!(monday.cod_total, 50_000);
    assert!(!numbers.contains(&reviewed.number));

    // Monday's draft is paid; on Wednesday the review is closed.
    pay_draft(&w, &monday).await;
    close_review(&w, &reviewed).await;
    w.rule().await;

    let all = w.remittances_of(SOUQ).await;
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].status, RemittanceStatus::Paid);
    assert_eq!(all[1].status, RemittanceStatus::Draft);
    let next = lines_of(&w, &all[1]).await;
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].0, reviewed.number);
    assert_eq!(all[1].cod_total, 15_000);
    assert_eq!(all[1].fee_total, 300);
    assert_eq!(all[1].net, 14_700);
}

/// AC3: a shipment with AED 50 absorbed by the client shows AED 250 of
/// AED 300 on its line, with the reason.
#[renox::test]
async fn a_difference_the_client_absorbs_shows_on_the_line_with_its_reason() {
    let w = boot_with_payments().await;
    let shipment = w
        .book_with(&w.dispatcher, SOUQ, "ORD-1", &[("cod", "300")])
        .await;
    let (_out, delivery) = w.out_for_delivery(&shipment).await;
    w.deliver_stop_with(&delivery, "250.00", &[("difference_reason", "recipient_short")])
        .await
        .assert_status(303);
    w.post(
        &w.ops,
        &format!("/deliveries/{}/cod-difference/approve", delivery.id),
        &[("absorbed_by", "client")],
    )
    .await
    .assert_ok();
    let delivered = w.reload(&shipment).await;
    assert_eq!(delivered.status, ShipmentStatus::Delivered);
    assert_eq!(delivered.cod_difference, 5_000);
    assert_eq!(delivered.cod_absorbed_by.as_deref(), Some("client"));

    w.rule().await;

    let draft = w.the_remittance_of(SOUQ).await;
    let line = draft.lines(w.db()).await.unwrap().remove(0);
    assert_eq!(line.cod_amount, 30_000);
    assert_eq!(line.absorbed, 5_000);
    assert_eq!(line.collected_for_client(), 25_000);
    assert_eq!(Some(line.absorbed_reason.clone()), delivered.cod_difference_reason);
    assert!(!line.absorbed_reason.is_empty());
    assert_eq!(draft.cod_total, 25_000);
    assert_eq!(draft.net, 25_000);
    w.get(&w.amal, &format!("/remittances/{}", draft.id))
        .await
        .assert_ok()
        .assert_see(&line.absorbed_reason);
}

/// A difference the driver or the business absorbs never reduces what the
/// client is paid (`Must not`: a driver's shortage is never the client's).
#[renox::test]
async fn a_difference_the_driver_or_the_business_absorbs_does_not_reduce_the_client() {
    let w = boot_with_payments().await;
    for (reference, absorber) in [("ORD-1", "driver"), ("ORD-2", "business")] {
        let shipment = w
            .book_with(&w.dispatcher, SOUQ, reference, &[("cod", "300")])
            .await;
        let (_out, delivery) = w.out_for_delivery(&shipment).await;
        w.deliver_stop_with(&delivery, "250.00", &[("difference_reason", "recipient_short")])
            .await
            .assert_status(303);
        w.post(
            &w.ops,
            &format!("/deliveries/{}/cod-difference/approve", delivery.id),
            &[("absorbed_by", absorber)],
        )
        .await
        .assert_ok();
    }

    w.rule().await;

    let draft = w.the_remittance_of(SOUQ).await;
    assert_eq!(draft.cod_total, 60_000);
    assert_eq!(draft.net, 60_000);
    for (_, line) in lines_of(&w, &draft).await {
        assert_eq!(line.absorbed, 0);
        assert_eq!(line.cod_amount, 30_000);
    }
}

/// AC4: running the rule twice on Monday leaves one draft per client, with
/// one line per shipment, and the accountant is told once.
#[renox::test]
async fn running_the_rule_twice_leaves_one_draft_per_client() {
    let w = boot_with_payments().await;
    w.delivered(SOUQ, "ORD-1", "200").await;
    w.delivered(SOUQ, "ORD-2", "300").await;
    let other = w.delivered(OTHER_CLIENT, "ORD-9", "100").await;
    assert_eq!(other.client_party_id, OTHER_CLIENT.0);

    let first = w.rule().await;
    let second = w.rule().await;

    assert_eq!(first.len(), 2);
    assert!(first.iter().all(|built| built.created));
    assert_eq!(second.len(), 2);
    assert!(second.iter().all(|built| !built.created && !built.changed));
    let souq = w.the_remittance_of(SOUQ).await;
    assert_eq!(souq.lines(w.db()).await.unwrap().len(), 2);
    assert_eq!(w.the_remittance_of(OTHER_CLIENT).await.cod_total, 10_000);
    assert_eq!(
        RemittanceLine::query().count(w.db()).await.unwrap(),
        3,
        "one line per shipment"
    );
    assert_eq!(
        w.notices.of_type(REMITTANCE_DRAFTED).len(),
        2,
        "one notice per draft, none for a rebuild that changed nothing"
    );
}

/// AC5: a draft with an approved +AED 70 adjustment line and a shipment
/// released from review on Tuesday: the next rebuild adds the shipment line,
/// and the +AED 70 line stays, still approved.
#[renox::test]
async fn a_rebuild_adds_the_released_shipment_and_keeps_the_approved_line() {
    let w = boot_with_payments().await;
    w.delivered(SOUQ, "ORD-1", "200").await;
    let reviewed = w.delivered(SOUQ, "ORD-2", "300").await;
    open_review(&w, &reviewed).await;
    w.rule().await;
    let draft = w.the_remittance_of(SOUQ).await;
    assert_eq!(draft.lines(w.db()).await.unwrap().len(), 1);

    // An approved +AED 70 line (prepared by Amal, approved by Bashar).
    let mut line = AdjustmentLine {
        client_party_id: SOUQ.0,
        amount: 7_000,
        reason: "COD under-remitted on SHP-0001".into(),
        source: adjustment::source::MANUAL.into(),
        prepared_by: Some(w.amal.id),
        ..AdjustmentLine::default()
    };
    line.insert(w.db()).await.unwrap();
    let _outcome = line
        .transition(adjustment::APPROVE)
        .by(&w.bashar)
        .run(w.state())
        .await
        .expect("the second user approves the line");
    w.rule().await;
    let attached = w.remittance(&draft).await;
    assert_eq!(attached.adjustment_total, 7_000);
    assert_eq!(attached.net, 27_000);

    // Tuesday: the review is closed. The next rebuild adds the shipment.
    close_review(&w, &reviewed).await;
    w.rule().await;

    let rebuilt = w.the_remittance_of(SOUQ).await;
    assert_eq!(rebuilt.id, draft.id);
    assert_eq!(rebuilt.lines(w.db()).await.unwrap().len(), 2);
    assert_eq!(rebuilt.adjustment_total, 7_000);
    assert_eq!(rebuilt.cod_total, 50_000);
    assert_eq!(rebuilt.net, 57_000);
    let kept = AdjustmentLine::find_or_404(w.db(), line.id).await.unwrap();
    assert_eq!(kept.remittance_id, Some(draft.id));
    assert_eq!(kept.status, AdjustmentStatus::Approved);
}

/// AC6: a client with no COD but an open adjustment line of -AED 40: a draft
/// is created with net AED 0 and the -AED 40 carried forward, and the
/// accountant is told. A second run does not carry it twice.
#[renox::test]
async fn an_open_negative_line_makes_a_draft_of_zero_and_is_carried_forward() {
    let w = boot_with_payments().await;
    let mut line = AdjustmentLine {
        client_party_id: SOUQ.0,
        amount: -4_000,
        reason: "Delivery reversed, COD refunded to recipient".into(),
        source: adjustment::source::DELIVERY_REVERSAL.into(),
        prepared_by: Some(w.amal.id),
        ..AdjustmentLine::default()
    };
    line.insert(w.db()).await.unwrap();
    let _outcome = line
        .transition(adjustment::APPROVE)
        .by(&w.bashar)
        .run(w.state())
        .await
        .expect("the second user approves the line");
    // The line starts no draft before the run.
    assert!(w.remittances_of(SOUQ).await.is_empty());

    w.rule().await;
    w.rule().await;

    let draft = w.the_remittance_of(SOUQ).await;
    assert_eq!(draft.cod_total, 0);
    assert_eq!(draft.adjustment_total, -4_000);
    assert_eq!(draft.net, 0);
    assert_eq!(draft.carried_forward, 4_000);
    let waiting = adjustment::unattached(w.db(), SOUQ).await.unwrap();
    assert_eq!(waiting.len(), 1, "the rest waits for the next draft");
    assert_eq!(waiting[0].amount, -4_000);
    assert_eq!(waiting[0].source, adjustment::source::CARRIED_FORWARD);
    assert_eq!(waiting[0].status, AdjustmentStatus::Approved);
    let told = w.notices.of_type(REMITTANCE_DRAFTED);
    assert_eq!(told.len(), 1);
    assert!(
        told[0].data["summary"]
            .as_str()
            .unwrap()
            .contains("AED 40.00 is carried to the next draft")
    );
}

/// `LOG.RUL.06`, "without held shipments": a shipment that is held has not
/// been delivered, so it is on no draft and starts none.
#[renox::test]
async fn a_held_shipment_is_on_no_draft() {
    let w = boot_with_payments().await;
    let shipment = w
        .book_with(&w.dispatcher, SOUQ, "ORD-1", &[("cod", "300")])
        .await;
    let at_hub = w.at_hub(&w.pick_up(&shipment).await).await;
    w.post(
        &w.hub,
        &format!("/shipments/{}/hold", at_hub.id),
        &[("reason", "client_instruction")],
    )
    .await
    .assert_status(303);
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::OnHold);

    assert!(w.rule().await.is_empty());
    assert!(w.remittances_of(SOUQ).await.is_empty());
}

/// A shipment marked already remitted (BL-LOG-10) is left out of the draft.
#[renox::test]
async fn a_shipment_marked_already_remitted_is_left_out() {
    let w = boot_with_payments().await;
    let remitted = w.delivered(SOUQ, "ORD-1", "200").await;
    w.delivered(SOUQ, "ORD-2", "300").await;
    Shipment::query()
        .where_(Shipment::ID.eq(remitted.id))
        .update(w.db(), &[("cod_already_remitted", &true)])
        .await
        .unwrap();

    w.rule().await;

    let draft = w.the_remittance_of(SOUQ).await;
    assert_eq!(draft.lines(w.db()).await.unwrap().len(), 1);
    assert_eq!(draft.cod_total, 30_000);
}

/// A shipment delivered after the period ended is for the next period.
#[renox::test]
async fn a_shipment_delivered_after_the_period_end_is_for_the_next_draft() {
    let w = boot_with_payments().await;
    w.delivered(SOUQ, "ORD-1", "200").await;

    let before = renox::db::now() - renox::chrono::TimeDelta::hours(1);
    assert!(w.rule_until(before).await.is_empty());
    assert!(w.remittances_of(SOUQ).await.is_empty());

    w.rule().await;
    assert_eq!(w.the_remittance_of(SOUQ).await.cod_total, 20_000);
}

/// The rule is on the scheduler under its task name (weekly, Monday): the
/// scheduled task drafts for the shipments delivered before today.
#[renox::test]
async fn the_rule_is_scheduled_and_the_scheduled_task_builds_the_draft() {
    let w = boot_with_payments().await;
    let shipment = w.delivered(SOUQ, "ORD-1", "200").await;
    // Delivered last week: the period of a run today ends at Dubai midnight.
    let last_week = Some(renox::db::now() - renox::chrono::TimeDelta::days(7));
    Shipment::query()
        .where_(Shipment::ID.eq(shipment.id))
        .update(w.db(), &[("delivered_at", &last_week)])
        .await
        .unwrap();

    w.app
        .kernel()
        .run_scheduled(remit::TASK)
        .await
        .expect("the task is registered and runs");

    assert_eq!(w.the_remittance_of(SOUQ).await.cod_total, 20_000);
}
