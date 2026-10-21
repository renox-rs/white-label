//! BL-LOG-11 (#309), 03 §4.1 row *Shipment cannot be found*, §4.4 steps 3 and
//! 4, §7.1 rows `mark_lost`, `found` and `hold`, `WRK.RUL.17`: a shipment whose
//! delivery is lost is lost with the delivery's claim ticket, off billing and
//! without COD; goods found return it to the hub, and a settled claim holds it
//! until the client chooses. The delivery module is the real wl-delivery.

mod support;

use logistics::notices::CLIENT_NOTICE;
use logistics::shipment::charge_kind;
use logistics::shipment_machine::{FOUND, FOUND_SYSTEM, MARK_LOST_APPROVED, MARK_LOST_SYSTEM};
use logistics::{Shipment, ShipmentStatus};
use renox::auth::permissions;
use renox::chrono::TimeDelta;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::tickets::Assignee;

use logistics::permissions as perm;
use support::{SOUQ, World, boot};

/// A shipment with COD AED 100 that went out and whose visit failed.
async fn attempted(w: &World) -> Shipment {
    let shipment = w
        .book_with(&w.dispatcher, SOUQ, "ORD-1", &[("cod", "100")])
        .await;
    let (_out, delivery) = w.out_for_delivery(&shipment).await;
    w.fail_stop(&delivery, "not_home").await.assert_status(303);
    let after = w.reload(&shipment).await;
    assert_eq!(after.status, ShipmentStatus::Attempted);
    after
}

/// The delivery of `shipment` is untraced for 7 days: wl-delivery marks it
/// lost (`WRK.RUL.17`) and the shipment follows.
async fn untraced(w: &World) {
    let marked = wl_delivery::lost::run(w.state(), renox::db::now() + TimeDelta::days(8))
        .await
        .unwrap();
    assert_eq!(marked, 1);
}

/// An attempted shipment whose delivery was lost.
async fn lost(w: &World) -> Shipment {
    let shipment = attempted(w).await;
    untraced(w).await;
    let after = w.reload(&shipment).await;
    assert_eq!(after.status, ShipmentStatus::Lost);
    after
}

async fn transitions(w: &World, shipment: &Shipment) -> Vec<String> {
    renox::state_machine::history(w.db(), Shipment::TABLE, shipment.id)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.transition)
        .collect()
}

fn lost_notices(w: &World) -> Vec<renox::serde_json::Value> {
    w.notices
        .of_type(CLIENT_NOTICE)
        .into_iter()
        .filter(|n| n.data["title"].as_str().unwrap().contains("is lost"))
        .map(|n| n.data)
        .collect()
}

/// AC1: given an `attempted` shipment whose delivery becomes `lost`, the
/// shipment is `lost`, linked to the delivery's claim ticket, the client gets
/// one notice, `ShipmentLost` is emitted, and the billing leaves it out.
#[renox::test]
async fn an_attempted_shipment_follows_its_lost_delivery_with_the_claim_ticket() {
    let w = boot().await;
    let shipment = attempted(&w).await;
    assert_eq!(Shipment::billable(w.db(), SOUQ).await.unwrap().len(), 1);

    untraced(&w).await;

    let after = w.reload(&shipment).await;
    assert_eq!(after.status, ShipmentStatus::Lost);
    let delivery = w.delivery_of(&after).await;
    assert!(delivery.claim_ticket_id.is_some());
    assert_eq!(after.claim_ticket_id, delivery.claim_ticket_id);
    // The one claim is the delivery's: the shipment opens none of its own.
    assert!(w.tickets.opened().iter().all(|t| t.category_key != "claim"));
    // No COD is expected, what was expected is kept for when it is found.
    assert_eq!(after.cod_amount, 0);
    assert_eq!(after.lost_cod, 10_000);
    assert_eq!(after.lost_from.as_deref(), Some("attempted"));
    let notices = lost_notices(&w);
    assert_eq!(notices.len(), 1);
    assert!(
        notices[0]["body"]
            .as_str()
            .unwrap()
            .contains(&shipment.number)
    );
    let events = w.shipment_lost.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].shipment, shipment.id);
    assert_eq!(events[0].from, "attempted");
    assert_eq!(events[0].claim_ticket, after.claim_ticket_id);
    assert!(Shipment::billable(w.db(), SOUQ).await.unwrap().is_empty());
    assert!(
        transitions(&w, &after)
            .await
            .contains(&MARK_LOST_SYSTEM.to_owned())
    );
    let tracked = after.events(w.db()).await.unwrap();
    assert!(tracked.iter().any(|e| e.kind == "lost"));
}

/// AC2: given the shipment was on an issued invoice (charge AED 25), the
/// invoice owner gets a task to credit AED 25.
#[renox::test]
async fn an_invoiced_lost_shipment_gets_a_task_to_credit_its_charge() {
    let w = boot().await;
    permissions::define_role(w.db(), perm::INVOICE_OWNER_ROLE, &[perm::REMITTANCES_VIEW])
        .await
        .unwrap();
    let owner = w.user_with("Ibrahim", &[perm::REMITTANCES_VIEW]).await;
    owner
        .assign_role(w.db(), perm::INVOICE_OWNER_ROLE)
        .await
        .unwrap();
    let shipment = attempted(&w).await;
    Shipment::query()
        .where_(Shipment::ID.eq(shipment.id))
        .update(w.db(), &[("price", &2_500_i64)])
        .await
        .unwrap();
    logistics::billing::mark_shipment_invoiced(w.db(), shipment.id, renox::db::now())
        .await
        .unwrap();

    untraced(&w).await;

    let after = w.reload(&shipment).await;
    assert_eq!(after.credit_due, 2_500);
    let task = w
        .tickets
        .opened()
        .into_iter()
        .find(|t| t.title.contains("Credit AED 25.00"))
        .expect("a task to credit AED 25");
    assert_eq!(task.assignee, Some(Assignee::user(owner.id)));
    assert!(task.description.contains("is lost"));
}

/// AC3: given a `lost` shipment scanned at Depot B, it is `at_hub`, the client
/// is notified and the claim ticket gets a comment.
#[renox::test]
async fn a_lost_shipment_scanned_at_a_depot_is_back_at_the_hub() {
    let w = boot().await;
    let shipment = lost(&w).await;
    let claim = shipment.claim_ticket_id.expect("a claim ticket");

    // Only a scanner, and only with a depot.
    w.post(
        &w.noor,
        &format!("/shipments/{}/found", shipment.id),
        &[("depot", "Depot B")],
    )
    .await
    .assert_forbidden();
    w.post(
        &w.hub,
        &format!("/shipments/{}/found", shipment.id),
        &[("depot", " ")],
    )
    .await
    .assert_status(422);
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::Lost);

    w.post(
        &w.hub,
        &format!("/shipments/{}/found", shipment.id),
        &[("depot", "Depot B")],
    )
    .await
    .assert_status(303);

    let found = w.reload(&shipment).await;
    assert_eq!(found.status, ShipmentStatus::AtHub);
    assert_eq!(found.found_depot.as_deref(), Some("Depot B"));
    // The COD is expected again.
    assert_eq!(found.cod_amount, 10_000);
    assert_eq!(found.lost_cod, 0);
    let told: Vec<_> = w
        .notices
        .of_type(CLIENT_NOTICE)
        .into_iter()
        .filter(|n| n.data["title"].as_str().unwrap().contains("was found"))
        .collect();
    assert_eq!(told.len(), 1);
    assert!(told[0].data["body"].as_str().unwrap().contains("Depot B"));
    let comments = w.tickets.comments();
    assert!(
        comments
            .iter()
            .any(|(ticket, comment)| ticket.0 == claim && comment.text.contains("Depot B"))
    );
    // Not settled: no hold, and billing starts again with its delivery.
    assert!(transitions(&w, &found).await.contains(&FOUND.to_owned()));
    assert!(
        !transitions(&w, &found)
            .await
            .contains(&"hold_system".to_owned())
    );
    assert_eq!(Shipment::billable(w.db(), SOUQ).await.unwrap().len(), 1);
}

/// AC4: a shipment marked lost by the system with liability *none* has a claim
/// ticket linked.
#[renox::test]
async fn a_system_loss_has_liability_none_and_a_claim_ticket() {
    let w = boot().await;
    let shipment = lost(&w).await;
    assert_eq!(shipment.liability.as_deref(), Some("none"));
    assert!(shipment.claim_ticket_id.is_some());
}

/// Without ticketing there is no claim ticket and the loss still stands.
#[renox::test]
async fn a_loss_stands_without_ticketing() {
    let w = boot().await;
    w.tickets.off(true);
    let shipment = lost(&w).await;
    assert_eq!(shipment.claim_ticket_id, None);
    let tracked = shipment.events(w.db()).await.unwrap();
    assert!(tracked.iter().any(|e| e.kind == "claim_ticket_skipped"));
    assert_eq!(lost_notices(&w).len(), 1);
}

/// AC5: given the claim was settled with a credit of AED 400, scanning it
/// reopens the ticket and the history shows `found` then a system `hold`
/// (*client instruction*), leaving the shipment `on_hold`; when the client
/// chooses delivery, a draft invoice of AED 400 is created and the shipment
/// is released.
#[renox::test]
async fn a_found_shipment_after_a_settled_claim_is_held_until_the_client_chooses() {
    let w = boot().await;
    let shipment = lost(&w).await;
    let claim = shipment.claim_ticket_id.unwrap();

    // The claim is settled by staff, on a lost shipment, with an amount.
    w.post(
        &w.noor,
        &format!("/shipments/{}/claim-settled", shipment.id),
        &[("amount", "400")],
    )
    .await
    .assert_forbidden();
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/claim-settled", shipment.id),
        &[("amount", "0")],
    )
    .await
    .assert_status(422);
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/claim-settled", shipment.id),
        &[("amount", "400")],
    )
    .await
    .assert_status(303);
    assert_eq!(w.reload(&shipment).await.claim_settled_amount, 40_000);

    w.post(
        &w.hub,
        &format!("/shipments/{}/found", shipment.id),
        &[("depot", "Depot B")],
    )
    .await
    .assert_status(303);

    let held = w.reload(&shipment).await;
    assert_eq!(held.status, ShipmentStatus::OnHold);
    assert_eq!(held.hold_reason.as_deref(), Some("client_instruction"));
    let history = transitions(&w, &held).await;
    let found_at = history.iter().position(|t| t == FOUND).unwrap();
    assert_eq!(history[found_at + 1], "hold_system");
    // The ticket is reopened: the comment says so.
    assert!(w.tickets.comments().iter().any(|(ticket, comment)| {
        ticket.0 == claim
            && comment.text.contains("reopened")
            && comment.text.contains("AED 400.00")
    }));

    // It cannot be released before the client chooses; the choice is checked.
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/release", shipment.id),
        &[],
    )
    .await
    .assert_status(422);
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/claim-choice", shipment.id),
        &[("choice", "maybe")],
    )
    .await
    .assert_status(422);
    w.post(
        &w.noor,
        &format!("/shipments/{}/claim-choice", shipment.id),
        &[("choice", "delivery")],
    )
    .await
    .assert_forbidden();
    assert!(w.invoices.drafts().is_empty());

    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/claim-choice", shipment.id),
        &[("choice", "delivery")],
    )
    .await
    .assert_status(303);

    let released = w.reload(&shipment).await;
    assert_eq!(released.status, ShipmentStatus::AtHub);
    assert_eq!(released.claim_choice.as_deref(), Some("delivery"));
    let drafts = w.invoices.drafts();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].party, SOUQ);
    assert_eq!(drafts[0].lines.len(), 1);
    assert_eq!(drafts[0].lines[0].net(), 40_000);
    assert_eq!(
        drafts[0].lines[0].description,
        "Claim reversed, goods found"
    );
    assert_eq!(drafts[0].lines[0].origin, Some(released.subject()));
    assert_eq!(released.claim_invoice_id, Some(1));
    // Chosen once.
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/claim-choice", shipment.id),
        &[("choice", "keep")],
    )
    .await
    .assert_status(422);
    assert_eq!(w.invoices.drafts().len(), 1);
}

/// The other choice: keeping the settlement sends the goods back to the client
/// with the return charge, and no invoice is drafted.
#[renox::test]
async fn keeping_the_settlement_sends_the_goods_back() {
    let w = boot().await;
    let shipment = lost(&w).await;
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/claim-settled", shipment.id),
        &[("amount", "400")],
    )
    .await
    .assert_status(303);
    w.post(
        &w.hub,
        &format!("/shipments/{}/found", shipment.id),
        &[("depot", "Depot B")],
    )
    .await
    .assert_status(303);

    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/claim-choice", shipment.id),
        &[("choice", "keep")],
    )
    .await
    .assert_status(303);

    let back = w.reload(&shipment).await;
    assert_eq!(back.status, ShipmentStatus::Returning);
    assert_eq!(back.claim_choice.as_deref(), Some("keep"));
    assert!(w.invoices.drafts().is_empty());
    let charges = back.charges(w.db()).await.unwrap();
    assert!(charges.iter().any(|c| c.kind == charge_kind::RETURN));
}

/// A claim is settled only on a lost shipment, and `found` only for a lost one.
#[renox::test]
async fn found_and_a_settled_claim_need_a_lost_shipment() {
    let w = boot().await;
    let shipment = attempted(&w).await;
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/claim-settled", shipment.id),
        &[("amount", "400")],
    )
    .await
    .assert_status(422);
    w.post(
        &w.hub,
        &format!("/shipments/{}/found", shipment.id),
        &[("depot", "Depot B")],
    )
    .await
    .assert_status(422);
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::Attempted);
}

/// `mark_lost_system`, `mark_lost_approved` and `found_system` belong to the
/// system and the kernel: no user runs them.
#[renox::test]
async fn no_user_runs_the_system_transitions_of_a_loss() {
    let w = boot().await;
    let shipment = attempted(&w).await;
    for user in [&w.ops, &w.dispatcher, &w.hub, &w.amal] {
        for name in [MARK_LOST_SYSTEM, MARK_LOST_APPROVED, FOUND_SYSTEM] {
            assert_eq!(
                w.try_run(user, &shipment, name).await,
                "forbidden",
                "{name}"
            );
        }
    }
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::Attempted);
    let machine = Shipment::machine();
    assert!(machine.find(MARK_LOST_SYSTEM).unwrap().is_system());
    assert!(machine.find(MARK_LOST_APPROVED).unwrap().is_system());
    assert!(machine.find(FOUND_SYSTEM).unwrap().is_system());
}

/// The delivery found at a depot (`DeliveryFound`) brings the shipment back
/// to the hub too, as the system.
#[renox::test]
async fn a_found_delivery_brings_the_shipment_back_to_the_hub() {
    let w = boot().await;
    let shipment = lost(&w).await;
    let delivery = w.delivery_of(&shipment).await;

    w.post(
        &w.dispatcher,
        &format!("/deliveries/{}/found", delivery.id),
        &[("depot", "Depot C")],
    )
    .await
    .assert_ok();

    let found = w.reload(&shipment).await;
    assert_eq!(found.status, ShipmentStatus::AtHub);
    assert_eq!(found.found_depot.as_deref(), Some("Depot C"));
    assert!(
        transitions(&w, &found)
            .await
            .contains(&FOUND_SYSTEM.to_owned())
    );
}
