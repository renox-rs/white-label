//! BL-LOG-06 (#304), 03 §3.3 `LOG.ACL.14`, §4.1 row *Parcel damaged at the
//! hub*, §7.1 row `dispose`, §7.2 `LOG.RUL.07`: a held shipment is destroyed
//! on the client's written instruction (or because the goods are dangerous)
//! after an operations manager other than the requester approves; a claim
//! opens when the company is liable and a disposal charge is made only when
//! the client caused it. Approvals run through the kernel, the delivery
//! module is the real wl-delivery.

mod support;

use logistics::notices::CLIENT_NOTICE;
use logistics::shipment::charge_kind;
use logistics::{Shipment, ShipmentStatus, billing, rates};
use renox::auth::permissions;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_delivery::DeliveryStatus;
use wl_kernel::approvals::{self, ApprovalRequest};

use logistics::permissions as perm;
use support::{SOUQ, World, boot};

/// A holder of the operations manager role who approves a disposal.
async fn manager(w: &World, name: &str) -> User {
    permissions::define_role(w.db(), perm::OPERATIONS_MANAGER_ROLE, &[perm::SHIPMENTS_VIEW])
        .await
        .unwrap();
    let user = w
        .user_with(
            name,
            &[
                perm::SHIPMENTS_VIEW,
                perm::SHIPMENTS_MANAGE,
                perm::SHIPMENTS_APPROVE_DISPOSAL,
            ],
        )
        .await;
    user.assign_role(w.db(), perm::OPERATIONS_MANAGER_ROLE)
        .await
        .unwrap();
    user
}

/// A shipment held for *damaged* with photos.
async fn held(w: &World, reference: &str) -> Shipment {
    let booked = w.book_as(&w.noor, SOUQ, reference).await;
    let at_hub = w.at_hub(&w.pick_up(&booked).await).await;
    w.post(
        &w.hub,
        &format!("/shipments/{}/hold", at_hub.id),
        &[("reason", "damaged"), ("photos", "damage-1.jpg")],
    )
    .await
    .assert_status(303);
    let shipment = w.reload(&at_hub).await;
    assert_eq!(shipment.status, ShipmentStatus::OnHold);
    shipment
}

/// The request of `by` for `shipment` with the e-mail attached.
async fn ask(
    w: &World,
    by: &User,
    shipment: &Shipment,
    liability: &str,
) -> renox::testing::TestResponse {
    w.post(
        by,
        &format!("/shipments/{}/dispose", shipment.id),
        &[
            ("reason", "Leaking, the client wants it destroyed"),
            ("photos", "leak-1.jpg"),
            ("instruction", "client-email-2026-11-02.eml"),
            ("liability", liability),
        ],
    )
    .await
}

async fn pending(w: &World, shipment: &Shipment) -> ApprovalRequest {
    ApprovalRequest::pending_about(w.db(), &shipment.subject())
        .await
        .unwrap()
        .pop()
        .expect("a request waits for approval")
}

fn disposal_notices(w: &World) -> usize {
    w.notices
        .of_type(CLIENT_NOTICE)
        .into_iter()
        .filter(|n| n.data["title"].as_str().unwrap().contains("was disposed of"))
        .count()
}

/// AC1: given an `on_hold` shipment (reason *damaged*), when user A requests
/// `dispose` with reason, photo and the client's e-mail attached, and
/// operations manager B approves, then it is `disposed` and the client is
/// notified.
#[renox::test]
async fn an_approved_disposal_destroys_the_held_shipment_and_tells_the_client() {
    let w = boot().await;
    let boss = manager(&w, "Mona").await;
    let shipment = held(&w, "ORD-1").await;
    let delivery = w.delivery_of(&shipment).await;

    ask(&w, &w.dispatcher, &shipment, "none")
        .await
        .assert_status(303);
    // It waits for B; the shipment stays held and nobody is told yet.
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::OnHold);
    let request = pending(&w, &shipment).await;
    assert_eq!(request.action, "logistics.dispose");
    assert_eq!(request.requester_id, w.dispatcher.id);
    assert_eq!(disposal_notices(&w), 0);

    approvals::approve(w.state(), request.id, &boss, "agreed")
        .await
        .unwrap()
        .expect("the request ends");

    let disposed = w.reload(&shipment).await;
    assert_eq!(disposed.status, ShipmentStatus::Disposed);
    assert_eq!(
        disposed.disposal_reason.as_deref(),
        Some("Leaking, the client wants it destroyed")
    );
    assert_eq!(disposed.disposal_photos.as_deref(), Some("leak-1.jpg"));
    assert_eq!(
        disposed.disposal_instruction.as_deref(),
        Some("client-email-2026-11-02.eml")
    );
    assert!(disposed.disposed_at.is_some());
    assert_eq!(disposal_notices(&w), 1);
    // The delivery is cancelled with the shipment, and the history shows the
    // approved run.
    assert_eq!(w.delivery_of(&disposed).await.status, DeliveryStatus::Cancelled);
    let history = renox::state_machine::history(w.db(), Shipment::TABLE, shipment.id)
        .await
        .unwrap();
    assert!(history.iter().any(|row| row.transition == "dispose_approved"));
    // The delivery module knew the shipment's delivery all along.
    assert_eq!(delivery.id, w.delivery_of(&disposed).await.id);
}

/// AC2: A cannot approve their own request; without an attachment and
/// without *dangerous* the request is refused (and so is a request without a
/// photo, a reason, a known liability, from another state or by a user who
/// cannot manage shipments).
#[renox::test]
async fn the_requester_cannot_approve_and_a_request_without_proof_is_refused() {
    let w = boot().await;
    let shipment = held(&w, "ORD-1").await;
    let path = format!("/shipments/{}/dispose", shipment.id);

    let refused = w
        .post(
            &w.dispatcher,
            &path,
            &[
                ("reason", "Leaking"),
                ("photos", "leak-1.jpg"),
                ("instruction", " "),
                ("liability", "none"),
            ],
        )
        .await;
    refused.assert_status(422);
    refused.assert_see("written instruction, or tick that the goods are dangerous");
    w.post(
        &w.dispatcher,
        &path,
        &[("reason", ""), ("photos", "a.jpg"), ("instruction", "x"), ("liability", "none")],
    )
    .await
    .assert_status(422);
    w.post(
        &w.dispatcher,
        &path,
        &[("reason", "Leaking"), ("photos", " "), ("instruction", "x"), ("liability", "none")],
    )
    .await
    .assert_status(422);
    w.post(
        &w.dispatcher,
        &path,
        &[("reason", "Leaking"), ("photos", "a.jpg"), ("instruction", "x"), ("liability", "driver")],
    )
    .await
    .assert_status(422);
    // A client user cannot request it.
    ask(&w, &w.noor, &shipment, "none").await.assert_forbidden();
    assert!(
        ApprovalRequest::pending_about(w.db(), &shipment.subject())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::OnHold);

    // A manager who requests cannot approve their own request; nor can a
    // dispatcher who holds no approval permission.
    let boss = manager(&w, "Mona").await;
    ask(&w, &boss, &shipment, "none").await.assert_status(303);
    let request = pending(&w, &shipment).await;
    assert!(
        approvals::approve(w.state(), request.id, &boss, "mine")
            .await
            .is_err()
    );
    assert!(
        approvals::approve(w.state(), request.id, &w.dispatcher, "mine")
            .await
            .is_err()
    );
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::OnHold);
    // A second request while one is pending is refused.
    ask(&w, &boss, &shipment, "none").await.assert_status(422);

    // Disposal is only from `on_hold`.
    let other = w.book_as(&w.noor, SOUQ, "ORD-2").await;
    let at_hub = w.at_hub(&w.pick_up(&other).await).await;
    ask(&w, &w.dispatcher, &at_hub, "none")
        .await
        .assert_status(422);
    assert_eq!(w.reload(&at_hub).await.status, ShipmentStatus::AtHub);
}

/// AC3: given *dangerous* ticked and no attachment, the request is accepted
/// and still needs B's approval.
#[renox::test]
async fn dangerous_goods_need_no_instruction_but_still_need_the_approval() {
    let w = boot().await;
    let boss = manager(&w, "Mona").await;
    let shipment = held(&w, "ORD-1").await;

    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/dispose", shipment.id),
        &[
            ("reason", "Leaking acid"),
            ("photos", "acid.jpg"),
            ("dangerous", "1"),
            ("liability", "none"),
        ],
    )
    .await
    .assert_status(303);

    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::OnHold);
    assert_eq!(disposal_notices(&w), 0);
    let request = pending(&w, &shipment).await;
    approvals::approve(w.state(), request.id, &boss, "agreed")
        .await
        .unwrap();
    let disposed = w.reload(&shipment).await;
    assert_eq!(disposed.status, ShipmentStatus::Disposed);
    assert!(disposed.disposal_dangerous);
    assert!(disposed.disposal_instruction.is_none());
    // The client is told after the disposal.
    assert_eq!(disposal_notices(&w), 1);
}

/// AC4 (business): a claim ticket is opened and the shipment is not billed.
#[renox::test]
async fn a_disposal_the_business_caused_opens_a_claim_and_is_not_billed() {
    let w = boot().await;
    let boss = manager(&w, "Mona").await;
    rates::set_price(w.db(), SOUQ, charge_kind::DISPOSAL, 1_500)
        .await
        .unwrap();
    let shipment = held(&w, "ORD-1").await;
    ask(&w, &w.dispatcher, &shipment, "business")
        .await
        .assert_status(303);
    let request = pending(&w, &shipment).await;
    approvals::approve(w.state(), request.id, &boss, "agreed")
        .await
        .unwrap();

    let disposed = w.reload(&shipment).await;
    assert_eq!(disposed.status, ShipmentStatus::Disposed);
    assert_eq!(disposed.liability.as_deref(), Some("business"));
    assert!(disposed.claim_ticket_id.is_some());
    assert_eq!(
        w.tickets
            .opened()
            .iter()
            .filter(|t| t.category_key == "claim")
            .count(),
        1
    );
    assert!(
        disposed
            .charges(w.db())
            .await
            .unwrap()
            .iter()
            .all(|c| c.kind != charge_kind::DISPOSAL)
    );
    assert!(Shipment::billable(w.db(), SOUQ).await.unwrap().is_empty());
}

/// AC4 (client): a disposal charge from the rate card is recorded and the
/// shipment is on the next monthly draft; without a disposal line the charge
/// is AED 0 and the accountant gets a task.
#[renox::test]
async fn a_disposal_the_client_caused_is_charged_and_billed() {
    let w = boot().await;
    let boss = manager(&w, "Mona").await;
    rates::set_price(w.db(), SOUQ, charge_kind::DISPOSAL, 1_500)
        .await
        .unwrap();
    let shipment = held(&w, "ORD-1").await;
    ask(&w, &w.dispatcher, &shipment, "client")
        .await
        .assert_status(303);
    let request = pending(&w, &shipment).await;
    approvals::approve(w.state(), request.id, &boss, "agreed")
        .await
        .unwrap();

    let disposed = w.reload(&shipment).await;
    assert_eq!(disposed.status, ShipmentStatus::Disposed);
    assert_eq!(disposed.liability.as_deref(), Some("client"));
    assert!(disposed.claim_ticket_id.is_none());
    let charges = disposed.charges(w.db()).await.unwrap();
    let charge = charges
        .iter()
        .find(|c| c.kind == charge_kind::DISPOSAL)
        .expect("a disposal charge");
    assert_eq!(charge.amount, 1_500);
    let unbilled = billing::unbilled_disposals(w.db(), SOUQ).await.unwrap();
    assert_eq!(unbilled.len(), 1);
    let billable = Shipment::billable(w.db(), SOUQ).await.unwrap();
    assert_eq!(billable.len(), 1);
    assert_eq!(billable[0].id, shipment.id);

    // A client whose prices have no disposal line pays AED 0.
    let second = held(&w, "ORD-2").await;
    clear_disposal_price(&w).await;
    ask(&w, &w.dispatcher, &second, "client")
        .await
        .assert_status(303);
    let request = pending(&w, &second).await;
    approvals::approve(w.state(), request.id, &boss, "agreed")
        .await
        .unwrap();
    let charges = w.reload(&second).await.charges(w.db()).await.unwrap();
    let charge = charges
        .iter()
        .find(|c| c.kind == charge_kind::DISPOSAL)
        .expect("a disposal charge of 0");
    assert_eq!(charge.amount, 0);
    assert!(charge.note.contains("No disposal price"));
    assert!(
        w.tickets
            .opened()
            .iter()
            .any(|t| t.title.starts_with("No disposal price for shipment"))
    );
}

/// Takes the disposal line off the client's prices.
async fn clear_disposal_price(w: &World) {
    logistics::rates::ClientRate::query()
        .where_(logistics::rates::ClientRate::KIND.eq(charge_kind::DISPOSAL))
        .delete(w.db())
        .await
        .unwrap();
}

/// AC4 (none): no claim ticket, no disposal charge, and the shipment is not
/// on the monthly draft; the liability is recorded.
#[renox::test]
async fn a_disposal_nobody_caused_has_no_claim_no_charge_and_is_not_billed() {
    let w = boot().await;
    let boss = manager(&w, "Mona").await;
    rates::set_price(w.db(), SOUQ, charge_kind::DISPOSAL, 1_500)
        .await
        .unwrap();
    let shipment = held(&w, "ORD-1").await;
    ask(&w, &w.dispatcher, &shipment, "none")
        .await
        .assert_status(303);
    let request = pending(&w, &shipment).await;
    approvals::approve(w.state(), request.id, &boss, "agreed")
        .await
        .unwrap();

    let disposed = w.reload(&shipment).await;
    assert_eq!(disposed.status, ShipmentStatus::Disposed);
    assert_eq!(disposed.liability.as_deref(), Some("none"));
    assert!(disposed.claim_ticket_id.is_none());
    assert!(
        w.tickets
            .opened()
            .iter()
            .all(|t| t.category_key != "claim")
    );
    assert!(
        disposed
            .charges(w.db())
            .await
            .unwrap()
            .iter()
            .all(|c| c.kind != charge_kind::DISPOSAL)
    );
    assert!(billing::unbilled_disposals(w.db(), SOUQ).await.unwrap().is_empty());
    assert!(Shipment::billable(w.db(), SOUQ).await.unwrap().is_empty());
}

/// AC5: when B rejects, the shipment stays `on_hold`, and a new request can
/// be made.
#[renox::test]
async fn a_rejected_disposal_leaves_the_shipment_on_hold() {
    let w = boot().await;
    let boss = manager(&w, "Mona").await;
    let shipment = held(&w, "ORD-1").await;
    ask(&w, &w.dispatcher, &shipment, "none")
        .await
        .assert_status(303);
    let request = pending(&w, &shipment).await;

    approvals::reject(w.state(), request.id, &boss, "the client has not confirmed")
        .await
        .unwrap();

    let after = w.reload(&shipment).await;
    assert_eq!(after.status, ShipmentStatus::OnHold);
    assert!(after.disposed_at.is_none());
    assert_eq!(disposal_notices(&w), 0);
    assert!(Shipment::billable(w.db(), SOUQ).await.unwrap().iter().all(|s| s.status != ShipmentStatus::Disposed));
    ask(&w, &w.dispatcher, &shipment, "none")
        .await
        .assert_status(303);
}

/// The kernel runs `dispose_approved`; no user does.
#[renox::test]
async fn no_user_runs_the_approved_disposal() {
    let w = boot().await;
    let shipment = held(&w, "ORD-1").await;
    for user in [&w.ops, &w.dispatcher, &w.hub, &w.amal] {
        let result = shipment
            .transition("dispose_approved")
            .by(user)
            .run(w.state())
            .await;
        assert!(
            matches!(result, Err(TransitionError::Forbidden)),
            "dispose_approved"
        );
    }
    assert_eq!(w.reload(&shipment).await.status, ShipmentStatus::OnHold);
}

/// The path of found goods: after a settled claim the client keeps the
/// settlement and tells staff to dispose of the goods; the disposal needs
/// the same approval, and a request without its proof records no choice.
#[renox::test]
async fn keeping_the_settlement_can_dispose_of_the_goods_with_an_approval() {
    let w = boot().await;
    let boss = manager(&w, "Mona").await;
    let booked = w.book_as(&w.noor, SOUQ, "ORD-1").await;
    let at_hub = w.at_hub(&w.pick_up(&booked).await).await;
    let id = at_hub.id;
    // Lost off a delivery with an approval, the claim settled, then found.
    ask_lost(&w, &at_hub).await;
    w.post(
        &w.dispatcher,
        &format!("/shipments/{id}/claim-settled"),
        &[("amount", "400")],
    )
    .await
    .assert_status(303);
    w.post(&w.hub, &format!("/shipments/{id}/found"), &[("depot", "Depot B")])
        .await
        .assert_status(303);
    assert_eq!(w.shipment(id).await.status, ShipmentStatus::OnHold);
    let path = format!("/shipments/{id}/claim-choice");

    // No instruction and not dangerous: refused, and the choice stays open.
    w.post(
        &w.dispatcher,
        &path,
        &[
            ("choice", "keep"),
            ("then", "dispose"),
            ("reason", "Client keeps the settlement"),
            ("photos", "goods.jpg"),
            ("liability", "none"),
        ],
    )
    .await
    .assert_status(422);
    assert!(w.shipment(id).await.claim_choice.is_none());

    w.post(
        &w.dispatcher,
        &path,
        &[
            ("choice", "keep"),
            ("then", "dispose"),
            ("reason", "Client keeps the settlement"),
            ("photos", "goods.jpg"),
            ("instruction", "client-email.eml"),
            ("liability", "none"),
        ],
    )
    .await
    .assert_status(303);
    let waiting = w.shipment(id).await;
    assert_eq!(waiting.claim_choice.as_deref(), Some("keep"));
    assert_eq!(waiting.status, ShipmentStatus::OnHold);
    let request = pending(&w, &waiting).await;
    assert_eq!(request.action, "logistics.dispose");
    approvals::approve(w.state(), request.id, &boss, "agreed")
        .await
        .unwrap();
    let disposed = w.shipment(id).await;
    assert_eq!(disposed.status, ShipmentStatus::Disposed);
    // The claim of the loss is the one linked; no second ticket.
    assert_eq!(
        w.tickets
            .opened()
            .iter()
            .filter(|t| t.category_key == "claim")
            .count(),
        1
    );
}

/// The shipment marked lost by one manager and approved by another.
async fn ask_lost(w: &World, shipment: &Shipment) {
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/mark-lost", shipment.id),
        &[("reason", "Not at the hub"), ("liability", "business")],
    )
    .await
    .assert_status(303);
    let request = pending(w, shipment).await;
    let approver_with_loss = w
        .user_with(
            "Lena",
            &[perm::SHIPMENTS_VIEW, perm::SHIPMENTS_APPROVE_LOSS],
        )
        .await;
    approvals::approve(w.state(), request.id, &approver_with_loss, "agreed")
        .await
        .unwrap();
    assert_eq!(w.reload(shipment).await.status, ShipmentStatus::Lost);
}
