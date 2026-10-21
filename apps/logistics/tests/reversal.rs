//! BL-LOG-10 (#308), 03 §4.1 row *Recipient denies the delivery, or it was
//! marked by mistake*, §4.4 step 3, §7.1 row `reverse_delivery`, `WRK.ACL.10`,
//! `LOG.ACL.12`: a delivered shipment follows the reversal of its delivery
//! back to `attempted` or `out_for_delivery`, leaves billing, keeps or
//! refunds its COD (a negative adjustment line when already remitted), and
//! records the liability. The delivery module is the real wl-delivery, the
//! payout and the later shortage the real wl-payments.

mod support;

use logistics::adjustment::{AdjustmentLine, AdjustmentStatus};
use logistics::notices::CLIENT_NOTICE;
use logistics::reversal::{Reversal, TASK_HR_CASE, TASK_NONE, TASK_OPEN, TASK_SHORTAGE};
use logistics::shipment_machine::{REVERSE_DELIVERY, REVERSE_DELIVERY_ON_ROAD};
use logistics::{RemittanceStatus, Shipment, ShipmentStatus};
use renox::auth::permissions;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_delivery::permissions as dperm;
use wl_kernel::custody::{HandoverKind, HandoverStatus};
use wl_kernel::tickets::Assignee;
use wl_payments::CashHandover;
use wl_payments::permissions as pay;

use support::{SOUQ, World, boot_with_payments};

use logistics::permissions as perm;

const ACCOUNT: &str = "AE070331234567890123456";

/// An operations manager: reverses deliveries, decides the tasks and counts
/// cash.
async fn manager(w: &World, name: &str) -> User {
    let user = w
        .user_with(
            name,
            &[
                dperm::VIEW,
                dperm::MANAGE,
                dperm::REVERSE,
                perm::SHIPMENTS_VIEW,
                perm::SHIPMENTS_MANAGE,
                pay::CASH_VIEW,
                pay::CASH_COUNT,
            ],
        )
        .await;
    user.assign_role(w.db(), perm::OPERATIONS_MANAGER_ROLE)
        .await
        .unwrap();
    user
}

/// The operations manager role exists before anybody holds it.
async fn roles(w: &World) {
    permissions::define_role(w.db(), perm::OPERATIONS_MANAGER_ROLE, &[perm::SHIPMENTS_VIEW])
        .await
        .unwrap();
    permissions::define_role(w.db(), perm::INVOICE_OWNER_ROLE, &[perm::REMITTANCES_VIEW])
        .await
        .unwrap();
    permissions::define_role(w.db(), perm::HR_ROLE, &[perm::SHIPMENTS_VIEW])
        .await
        .unwrap();
}

/// `by` asks to reverse the delivery of `shipment`.
async fn ask(w: &World, by: &User, shipment: &Shipment, liability: &str) {
    let delivery = w.delivery_of(shipment).await;
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
    .assert_ok();
}

/// `by` approves it with the cash treatment `cash` (`refund`, `keep` or none).
async fn approve(w: &World, by: &User, shipment: &Shipment, cash: &str) {
    let delivery = w.delivery_of(shipment).await;
    w.post(
        by,
        &format!("/deliveries/{}/reverse/approve", delivery.id),
        &[("cash_treatment", cash)],
    )
    .await
    .assert_ok();
}

/// The driver brings the run of `shipment` back and the depot counts the
/// cash: the run is closed and the cash handed over.
async fn hand_over(w: &World, shipment: &Shipment, counted: &str) {
    let delivery = w.delivery_of(shipment).await;
    let run = wl_delivery::Run::find_or_404(w.db(), delivery.run_id.unwrap())
        .await
        .unwrap();
    w.post(&w.driver, &format!("/driver/runs/{}/return", run.id), &[])
        .await
        .assert_ok();
    let handover = wl_delivery::close::handover_of(w.state(), &run)
        .await
        .unwrap()
        .expect("the driver's handover of the run");
    let counter = w
        .user_with(
            &format!("Counter{}", shipment.id),
            &[pay::CASH_VIEW, pay::CASH_COUNT],
        )
        .await;
    w.post(
        &counter,
        &format!("/cash/{}/count", handover.id.0),
        &[("counted", counted)],
    )
    .await
    .assert_ok();
}

/// Approves the draft of the client and pays it: the COD is remitted.
async fn remit(w: &World) {
    w.cod_terms(SOUQ, 0, Some(ACCOUNT)).await;
    w.rule().await;
    let draft = w.the_remittance_of(SOUQ).await;
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
    assert_eq!(w.remittance_status(&draft).await, RemittanceStatus::Paid);
}

async fn adjustments(w: &World) -> Vec<AdjustmentLine> {
    AdjustmentLine::query()
        .order_by(AdjustmentLine::ID)
        .get(w.db())
        .await
        .unwrap()
}

async fn reversals(w: &World, shipment: &Shipment) -> Vec<Reversal> {
    logistics::reversal::of_shipment(w.db(), shipment.id)
        .await
        .unwrap()
}

/// AC1: given a `delivered` shipment with COD AED 120 already remitted, when
/// the delivery is reversed with *refund to payer* and liability *driver*,
/// the shipment is `attempted`, a draft adjustment line of -AED 120 with the
/// reason waits for approval on the client's next remittance, a claim ticket
/// is linked, the operations manager has a task for AED 120 and the client is
/// notified. With the driver's handover counted, *raise shortage* on the task
/// creates a later shortage handover `short` of AED 120 with the shipment as
/// source.
#[renox::test]
async fn a_refunded_reversal_of_remitted_cod_drafts_a_negative_line_and_a_task_for_the_driver() {
    let w = boot_with_payments().await;
    roles(&w).await;
    let boss = manager(&w, "Rana").await;
    let second = manager(&w, "Samir").await;
    let shipment = w.delivered(SOUQ, "ORD-1", "120").await;
    remit(&w).await;
    hand_over(&w, &shipment, "120.00").await;

    ask(&w, &boss, &shipment, "driver").await;
    // Nothing changes before the second manager approves.
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::Delivered);
    approve(&w, &second, &shipment, "refund").await;

    let reversed = w.reload(&shipment).await;
    assert_eq!(reversed.status, ShipmentStatus::Attempted);
    assert!(reversed.reversed);
    assert_eq!(reversed.cod_amount, 0);
    assert_eq!(reversed.cod_released, 12_000);
    assert_eq!(reversed.liability.as_deref(), Some("driver"));
    assert_eq!(reversed.cash_treatment.as_deref(), Some("refunded"));
    // The e-POD is wl-delivery's and is marked reversed there.
    let proofs = wl_delivery::Proof::query()
        .where_(wl_delivery::Proof::DELIVERY_ID.eq(reversed.delivery_id.unwrap()))
        .get(w.db())
        .await
        .unwrap();
    assert!(proofs.iter().all(|proof| proof.reversed));

    // The negative line, awaiting approval, with its reason.
    let lines = adjustments(&w).await;
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].amount, -12_000);
    assert_eq!(lines[0].status, AdjustmentStatus::Pending);
    assert_eq!(lines[0].source, "delivery_reversal");
    assert_eq!(lines[0].shipment_id, Some(shipment.id));
    assert_eq!(lines[0].reason, "Delivery reversed, COD refunded to recipient");
    let paid = w.the_remittance_of(SOUQ).await;
    assert_eq!(lines[0].corrects_remittance_id, Some(paid.id));
    // The next weekly run puts it on the client's draft; it counts once
    // approved by another user.
    w.rule().await;
    let drafts = w.remittances_of(SOUQ).await;
    assert_eq!(drafts.len(), 2);
    let draft = &drafts[1];
    assert_eq!(draft.status, RemittanceStatus::Draft);
    assert_eq!(adjustments(&w).await[0].remittance_id, Some(draft.id));
    assert_eq!(w.remittance(draft).await.adjustment_total, 0);
    w.post(
        &w.bashar,
        &format!("/remittances/adjustments/{}/approve", lines[0].id),
        &[],
    )
    .await
    .assert_status(303);
    assert_eq!(w.remittance(draft).await.adjustment_total, -12_000);

    // The claim ticket is linked, the task is the operations manager's.
    let claim = reversed.claim_ticket_id.expect("a claim ticket is linked");
    let opened = w.tickets.opened();
    let claims: Vec<_> = opened.iter().filter(|t| t.category_key == "claim").collect();
    assert_eq!(claims.len(), 1);
    assert_eq!(claim, i64::try_from(
        opened.iter().position(|t| t.category_key == "claim").unwrap() + 1
    ).unwrap());
    let task = opened
        .iter()
        .find(|t| t.title.contains("Driver liable for AED 120.00"))
        .expect("the operations manager has a task for AED 120");
    assert_eq!(task.assignee, Some(Assignee::user(boss.id)));
    assert!(task.title.contains(&shipment.number));
    let all = reversals(&w, &shipment).await;
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].task_status, TASK_OPEN);
    assert_eq!(all[0].cash_amount, 12_000);
    assert_eq!(all[0].driver_user_id, Some(w.driver.id));
    assert_eq!(all[0].approver_id, Some(second.id));
    assert!(all[0].remitted && all[0].settled);
    // The client is told.
    let told: Vec<_> = w
        .notices
        .of_type(CLIENT_NOTICE)
        .into_iter()
        .filter(|n| n.data["title"].as_str().unwrap().contains("reversed"))
        .collect();
    assert_eq!(told.len(), 1);
    assert!(told[0].data["body"].as_str().unwrap().contains(&shipment.number));
    // The task is on the operations manager's page.
    w.get(&boss, "/reversals")
        .await
        .assert_ok()
        .assert_see(&shipment.number);

    // Raise shortage: the driver's handover was counted, so a later shortage
    // handover is made with the shipment as its source.
    w.post(
        &w.dispatcher,
        &format!("/reversals/{}/raise-shortage", all[0].id),
        &[],
    )
    .await
    .assert_forbidden();
    w.post(
        &second,
        &format!("/reversals/{}/raise-shortage", all[0].id),
        &[],
    )
    .await
    .assert_status(303);
    let decided = reversals(&w, &shipment).await.remove(0);
    assert_eq!(decided.task_status, TASK_SHORTAGE);
    let later = CashHandover::query()
        .where_(CashHandover::KIND.eq(HandoverKind::LaterShortage))
        .get(w.db())
        .await
        .unwrap();
    assert_eq!(later.len(), 1);
    assert_eq!(later[0].status, HandoverStatus::Short);
    assert_eq!(later[0].expected, 12_000);
    assert_eq!(later[0].holder_user_id, w.driver.id);
    assert_eq!(later[0].source_type.as_deref(), Some("logistics.shipment"));
    assert_eq!(later[0].source_id, Some(shipment.id));
    assert_eq!(decided.shortage_handover_id, Some(later[0].id));
    // Decided once.
    w.post(
        &second,
        &format!("/reversals/{}/raise-shortage", all[0].id),
        &[],
    )
    .await
    .assert_status(422);
}

/// The other action of the task: *open HR case* asks the HR role to follow
/// up, and closes the task.
#[renox::test]
async fn the_task_can_open_an_hr_case_instead() {
    let w = boot_with_payments().await;
    roles(&w).await;
    let hr = w.user_with("Hala", &[perm::SHIPMENTS_VIEW]).await;
    hr.assign_role(w.db(), perm::HR_ROLE).await.unwrap();
    let boss = manager(&w, "Rana").await;
    let second = manager(&w, "Samir").await;
    let shipment = w.delivered(SOUQ, "ORD-1", "120").await;

    ask(&w, &boss, &shipment, "driver").await;
    approve(&w, &second, &shipment, "").await;
    let reversal = reversals(&w, &shipment).await.remove(0);
    assert_eq!(reversal.task_status, TASK_OPEN);

    w.post(&second, &format!("/reversals/{}/hr-case", reversal.id), &[])
        .await
        .assert_status(303);

    let decided = reversals(&w, &shipment).await.remove(0);
    assert_eq!(decided.task_status, TASK_HR_CASE);
    assert!(decided.hr_ticket_id.is_some());
    let case = w
        .tickets
        .opened()
        .into_iter()
        .find(|t| t.title.starts_with("HR case"))
        .expect("an HR case");
    assert_eq!(case.assignee, Some(Assignee::user(hr.id)));
    // No shortage after an HR case.
    w.post(
        &second,
        &format!("/reversals/{}/raise-shortage", reversal.id),
        &[],
    )
    .await
    .assert_status(422);
}

/// AC2: given a `delivered` shipment not yet invoiced, after the reversal it
/// is not on the next monthly draft; once delivered again it is.
#[renox::test]
async fn a_reversed_shipment_leaves_billing_until_it_is_delivered_again() {
    let w = boot_with_payments().await;
    roles(&w).await;
    let boss = manager(&w, "Rana").await;
    let second = manager(&w, "Samir").await;
    let shipment = w.delivered(SOUQ, "ORD-1", "120").await;
    assert_eq!(Shipment::billable(w.db(), SOUQ).await.unwrap().len(), 1);

    ask(&w, &boss, &shipment, "none").await;
    approve(&w, &second, &shipment, "").await;

    // The run is still out: back on the road.
    let reversed = w.reload(&shipment).await;
    assert_eq!(reversed.status, ShipmentStatus::OutForDelivery);
    assert!(reversed.delivered_at.is_none());
    assert!(Shipment::billable(w.db(), SOUQ).await.unwrap().is_empty());
    let events = reversed.events(w.db()).await.unwrap();
    assert!(events.iter().any(|e| e.kind == "reversed"));

    let delivery = w.delivery_of(&reversed).await;
    w.deliver_stop(&delivery, "120.00").await.assert_status(303);

    let again = w.reload(&shipment).await;
    assert_eq!(again.status, ShipmentStatus::Delivered);
    assert!(!again.reversed);
    assert!(again.delivered_at.is_some());
    let billable = Shipment::billable(w.db(), SOUQ).await.unwrap();
    assert_eq!(billable.len(), 1);
    assert_eq!(billable[0].id, shipment.id);
    // Each reversal is a row of its own in the history.
    let history = renox::state_machine::history(w.db(), Shipment::TABLE, shipment.id)
        .await
        .unwrap();
    assert!(
        history
            .iter()
            .any(|row| row.transition == REVERSE_DELIVERY_ON_ROAD)
    );
}

/// AC3: given an invoiced shipment (AED 25 charge), after the reversal the
/// invoice owner's task shows a credit due of AED 25.
#[renox::test]
async fn an_invoiced_shipment_gets_a_credit_due_for_the_invoice_owner() {
    let w = boot_with_payments().await;
    roles(&w).await;
    let owner = w.user_with("Ibrahim", &[perm::REMITTANCES_VIEW]).await;
    owner
        .assign_role(w.db(), perm::INVOICE_OWNER_ROLE)
        .await
        .unwrap();
    let boss = manager(&w, "Rana").await;
    let second = manager(&w, "Samir").await;
    let shipment = w.delivered(SOUQ, "ORD-1", "120").await;
    Shipment::query()
        .where_(Shipment::ID.eq(shipment.id))
        .update(w.db(), &[("price", &2_500_i64)])
        .await
        .unwrap();
    logistics::billing::mark_shipment_invoiced(w.db(), shipment.id, renox::db::now())
        .await
        .unwrap();

    ask(&w, &boss, &shipment, "none").await;
    approve(&w, &second, &shipment, "").await;

    let reversed = w.reload(&shipment).await;
    assert_eq!(reversed.credit_due, 2_500);
    assert!(reversed.invoiced_on.is_none());
    let task = w
        .tickets
        .opened()
        .into_iter()
        .find(|t| t.title.contains("Credit AED 25.00"))
        .expect("the invoice owner has a task to credit AED 25");
    assert_eq!(task.assignee, Some(Assignee::user(owner.id)));
    assert!(task.title.contains(&shipment.number));
    assert_eq!(reversals(&w, &shipment).await[0].credit_due, 2_500);
}

/// AC4: given *keep for next attempt* and COD already remitted, the later
/// delivery does not add the COD to a remittance again.
#[renox::test]
async fn kept_cod_that_was_remitted_is_never_remitted_twice() {
    let w = boot_with_payments().await;
    roles(&w).await;
    let boss = manager(&w, "Rana").await;
    let second = manager(&w, "Samir").await;
    let shipment = w.delivered(SOUQ, "ORD-1", "120").await;
    remit(&w).await;

    ask(&w, &boss, &shipment, "none").await;
    approve(&w, &second, &shipment, "").await;

    let reversed = w.reload(&shipment).await;
    assert_eq!(reversed.status, ShipmentStatus::OutForDelivery);
    assert!(reversed.cod_already_remitted);
    assert_eq!(reversed.cod_amount, 12_000);
    // Nothing is taken back from the paid remittance.
    assert!(adjustments(&w).await.is_empty());
    assert_eq!(w.the_remittance_of(SOUQ).await.cod_total, 12_000);

    let delivery = w.delivery_of(&reversed).await;
    w.deliver_stop(&delivery, "120.00").await.assert_status(303);
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::Delivered);

    assert!(w.rule().await.is_empty());
    assert_eq!(w.remittances_of(SOUQ).await.len(), 1);
}

/// AC5: given liability *client*, no claim ticket and no task are created.
#[renox::test]
async fn liability_client_creates_no_claim_and_no_task() {
    let w = boot_with_payments().await;
    roles(&w).await;
    let boss = manager(&w, "Rana").await;
    let second = manager(&w, "Samir").await;
    let shipment = w.delivered(SOUQ, "ORD-1", "120").await;
    hand_over(&w, &shipment, "120.00").await;

    ask(&w, &boss, &shipment, "client").await;
    approve(&w, &second, &shipment, "keep").await;

    let reversed = w.reload(&shipment).await;
    assert_eq!(reversed.status, ShipmentStatus::Attempted);
    assert_eq!(reversed.claim_ticket_id, None);
    let opened = w.tickets.opened();
    assert!(opened.iter().all(|t| t.category_key != "claim"));
    assert!(opened.iter().all(|t| !t.title.contains("liable")));
    let all = reversals(&w, &shipment).await;
    assert_eq!(all[0].task_status, TASK_NONE);
    assert_eq!(all[0].task_ticket_id, None);
    assert_eq!(all[0].loss_amount, 0);
    // The client is still told.
    assert_eq!(
        w.notices
            .of_type(CLIENT_NOTICE)
            .into_iter()
            .filter(|n| n.data["title"].as_str().unwrap().contains("reversed"))
            .count(),
        1
    );
}

/// *Business* liability opens a claim ticket and records the loss, with no
/// task for the operations manager.
#[renox::test]
async fn liability_business_opens_a_claim_and_records_the_loss() {
    let w = boot_with_payments().await;
    roles(&w).await;
    let boss = manager(&w, "Rana").await;
    let second = manager(&w, "Samir").await;
    let shipment = w.delivered(SOUQ, "ORD-1", "120").await;

    ask(&w, &boss, &shipment, "business").await;
    approve(&w, &second, &shipment, "").await;

    let reversed = w.reload(&shipment).await;
    assert!(reversed.claim_ticket_id.is_some());
    let all = reversals(&w, &shipment).await;
    assert_eq!(all[0].loss_amount, 12_000);
    assert_eq!(all[0].task_status, TASK_NONE);
    assert_eq!(
        w.tickets
            .opened()
            .iter()
            .filter(|t| t.category_key == "claim")
            .count(),
        1
    );
}

/// A shipment whose delivery is under review is reversed from the review: its
/// COD was already off the remittances, and the review's hold ends.
#[renox::test]
async fn a_shipment_under_review_is_reversed_from_the_review() {
    let w = boot_with_payments().await;
    roles(&w).await;
    let boss = manager(&w, "Rana").await;
    let second = manager(&w, "Samir").await;
    let shipment = w.delivered(SOUQ, "ORD-1", "120").await;
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/dispute", shipment.id),
        &[("reason", "The recipient denies the delivery")],
    )
    .await
    .assert_status(303);
    assert_eq!(
        w.reload(&shipment).await.status,
        ShipmentStatus::UnderReview
    );

    ask(&w, &boss, &shipment, "none").await;
    approve(&w, &second, &shipment, "").await;

    let reversed = w.reload(&shipment).await;
    assert_eq!(reversed.status, ShipmentStatus::OutForDelivery);
    assert!(Shipment::billable(w.db(), SOUQ).await.unwrap().is_empty());
    assert!(w.rule().await.is_empty());
}

/// No user runs `reverse_delivery`: it is not offered and every role is
/// refused; only the event of the delivery module moves the shipment.
#[renox::test]
async fn no_user_can_run_reverse_delivery_on_a_shipment() {
    let w = boot_with_payments().await;
    roles(&w).await;
    let boss = manager(&w, "Rana").await;
    let shipment = w.delivered(SOUQ, "ORD-1", "120").await;
    for user in [&w.ops, &w.dispatcher, &w.amal, &w.driver, &w.noor, &boss] {
        let offered = w.offered(user, &shipment).await;
        assert!(
            !offered
                .iter()
                .any(|n| n == REVERSE_DELIVERY || n == REVERSE_DELIVERY_ON_ROAD)
        );
        for name in [REVERSE_DELIVERY, REVERSE_DELIVERY_ON_ROAD] {
            let result = shipment.transition(name).by(user).run(w.state()).await;
            assert!(matches!(result, Err(TransitionError::Forbidden)));
        }
    }
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::Delivered);
    let machine = Shipment::machine();
    assert!(machine.find(REVERSE_DELIVERY).unwrap().is_system());
    assert!(machine.find(REVERSE_DELIVERY_ON_ROAD).unwrap().is_system());
}
