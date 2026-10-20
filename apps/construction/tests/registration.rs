//! BL-CON-16 (#295): `CON.ACL.13` holds a purchase order to a subcontractor
//! without a valid Dubai Municipality registration on a Dubai project for the
//! approval of the project manager; the order stays a draft until then. The
//! check runs on `issue` and again on each `revise`.

mod support;

use construction::registration::APPROVAL_ACTION;
use construction::{Project, ProjectStatus};
use renox::auth::permissions::define_role;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_inventory::{Item, PurchaseOrder, PurchaseOrderLine, PurchaseOrderStatus};
use wl_kernel::PartyId;
use wl_kernel::approvals::{self, ApprovalRequest, ApprovalStatus};

use support::{Booted, boot, date};

const SUBCONTRACTOR: PartyId = PartyId(77);

/// A draft order of AED 4,000 (10 units at AED 400) to `supplier`, delivering
/// to the store of `project`.
async fn draft(
    t: &Booted,
    project: &Project,
    supplier: PartyId,
) -> (PurchaseOrder, PurchaseOrderLine) {
    let mut item = Item::new("PARTS", "PARTS", "kg");
    item.save(t.db()).await.unwrap();
    let store = t.store(project, "STORE-1").await;
    let mut order = PurchaseOrder::new(supplier, store.id);
    let mut tx = t.db().begin().await.unwrap();
    order
        .insert_in_state(&mut tx, PurchaseOrderStatus::Draft, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let mut line = PurchaseOrderLine::new(order.id, 1, item.id, 10_000, 40_000);
    line.save(t.db()).await.unwrap();
    (order, line)
}

async fn procurement(t: &Booted) -> User {
    define_role(
        t.db(),
        "procurement",
        &[wl_inventory::permissions::PURCHASE_ORDERS_ISSUE],
    )
    .await
    .unwrap();
    t.user_with("Prisha", "procurement").await
}

async fn order_now(t: &Booted, order: &PurchaseOrder) -> PurchaseOrder {
    PurchaseOrder::find_or_404(t.db(), order.id).await.unwrap()
}

async fn pending(t: &Booted, order: &PurchaseOrder) -> Vec<ApprovalRequest> {
    ApprovalRequest::pending_about(t.db(), &order.subject())
        .await
        .unwrap()
}

async fn issue(t: &Booted, by: &User, order: &PurchaseOrder) {
    t.post(by, &format!("/purchase-orders/{}/issue", order.id), &[])
        .await
        .assert_ok();
}

/// Done when 1 and 2: a Dubai project and a subcontractor whose registration
/// expired: issuing creates an approval request for the project manager and
/// the order stays a draft; the issuer cannot approve; after the project
/// manager approves the order is issued.
#[renox::test]
async fn an_expired_registration_holds_the_order_for_the_project_manager() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let project = t.project(ProjectStatus::InProgress, Some(&pm)).await;
    let buyer = procurement(&t).await;
    t.registrations.subcontractor(SUBCONTRACTOR);
    t.registrations.register(SUBCONTRACTOR, date(2026, 12, 31));
    let (order, _line) = draft(&t, &project, SUBCONTRACTOR).await;

    t.on(date(2027, 1, 15), async {
        issue(&t, &buyer, &order).await;
    })
    .await;
    let waiting = pending(&t, &order).await;
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].action, APPROVAL_ACTION);
    assert_eq!(waiting[0].transition, "issue");
    assert_eq!(waiting[0].requester_id, buyer.id);
    let held = order_now(&t, &order).await;
    assert_eq!(held.status, PurchaseOrderStatus::Draft);
    assert_eq!(held.number, None);

    // The issuer never approves their own order.
    let refused = approvals::approve(t.state(), waiting[0].id, &buyer, "ok").await;
    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");
    assert_eq!(
        order_now(&t, &order).await.status,
        PurchaseOrderStatus::Draft
    );

    t.on(date(2027, 1, 15), async {
        approvals::approve(t.state(), waiting[0].id, &pm, "registration chased")
            .await
            .unwrap();
    })
    .await;
    let issued = order_now(&t, &order).await;
    assert_eq!(issued.status, PurchaseOrderStatus::Issued);
    assert!(
        issued
            .number
            .as_deref()
            .is_some_and(|n| n.starts_with("PO-"))
    );
    let done = ApprovalRequest::find_or_404(t.db(), waiting[0].id)
        .await
        .unwrap();
    assert_eq!(done.status, ApprovalStatus::Approved);
}

/// A rejection leaves the order a draft; a subcontractor without any
/// registration on file is held too.
#[renox::test]
async fn a_rejected_request_leaves_the_order_a_draft() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let project = t.project(ProjectStatus::InProgress, Some(&pm)).await;
    let buyer = procurement(&t).await;
    t.registrations.subcontractor(SUBCONTRACTOR);
    let (order, _line) = draft(&t, &project, SUBCONTRACTOR).await;

    issue(&t, &buyer, &order).await;
    let waiting = pending(&t, &order).await;
    assert_eq!(waiting.len(), 1, "no registration at all is held too");
    approvals::reject(t.state(), waiting[0].id, &pm, "not registered")
        .await
        .unwrap();
    assert!(pending(&t, &order).await.is_empty());
    let kept = order_now(&t, &order).await;
    assert_eq!(kept.status, PurchaseOrderStatus::Draft);
    assert_eq!(kept.number, None);
}

/// Done when 3: with a valid registration on the order's date the order is
/// issued without approval; a supplier that is not a subcontractor is not
/// checked.
#[renox::test]
async fn a_valid_registration_or_another_supplier_is_issued_without_approval() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let project = t.project(ProjectStatus::InProgress, Some(&pm)).await;
    let buyer = procurement(&t).await;
    t.registrations.subcontractor(SUBCONTRACTOR);
    t.registrations.register(SUBCONTRACTOR, date(2030, 12, 31));
    let (registered, _) = draft(&t, &project, SUBCONTRACTOR).await;
    issue(&t, &buyer, &registered).await;
    assert!(pending(&t, &registered).await.is_empty());
    assert_eq!(
        order_now(&t, &registered).await.status,
        PurchaseOrderStatus::Issued
    );

    // A material supplier has no registration and needs none.
    let (materials, _) = draft(&t, &project, PartyId(78)).await;
    issue(&t, &buyer, &materials).await;
    assert!(pending(&t, &materials).await.is_empty());
    assert_eq!(
        order_now(&t, &materials).await.status,
        PurchaseOrderStatus::Issued
    );
}

/// Done when 4: on an Abu Dhabi project there is no check.
#[renox::test]
async fn an_abu_dhabi_project_is_not_checked() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let project = t.project(ProjectStatus::InProgress, Some(&pm)).await;
    Project::query()
        .where_(Project::ID.eq(project.id))
        .update(t.db(), &[("emirate", &"abu_dhabi".to_owned())])
        .await
        .unwrap();
    let buyer = procurement(&t).await;
    t.registrations.subcontractor(SUBCONTRACTOR);
    let (order, _line) = draft(&t, &project, SUBCONTRACTOR).await;

    issue(&t, &buyer, &order).await;
    assert!(pending(&t, &order).await.is_empty());
    assert_eq!(
        order_now(&t, &order).await.status,
        PurchaseOrderStatus::Issued
    );
}

/// A blocked supplier is refused before any request of the rule is made.
#[renox::test]
async fn a_blocked_subcontractor_is_refused_before_the_rule_asks() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let project = t.project(ProjectStatus::InProgress, Some(&pm)).await;
    let buyer = procurement(&t).await;
    t.registrations.subcontractor(SUBCONTRACTOR);
    t.blocked.block(SUBCONTRACTOR);
    let (order, _line) = draft(&t, &project, SUBCONTRACTOR).await;

    let refused = t
        .post(&buyer, &format!("/purchase-orders/{}/issue", order.id), &[])
        .await;
    refused.assert_status(422);
    assert!(refused.text().contains("This party is on hold"));
    assert!(pending(&t, &order).await.is_empty());
    assert_eq!(
        order_now(&t, &order).await.status,
        PurchaseOrderStatus::Draft
    );
}

/// The check runs again on each `revise`: an order issued while the
/// registration was valid is revised after it expired and waits for the
/// project manager; the revision in force stays until then.
#[renox::test]
async fn a_revision_is_checked_again() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let project = t.project(ProjectStatus::InProgress, Some(&pm)).await;
    let buyer = procurement(&t).await;
    t.registrations.subcontractor(SUBCONTRACTOR);
    t.registrations.register(SUBCONTRACTOR, date(2027, 2, 1));
    let (order, line) = draft(&t, &project, SUBCONTRACTOR).await;
    issue(&t, &buyer, &order).await;
    assert_eq!(
        order_now(&t, &order).await.status,
        PurchaseOrderStatus::Issued
    );

    let revise = |price: &'static str| {
        let t = &t;
        let buyer = &buyer;
        let order = &order;
        let line = &line;
        async move {
            t.post(
                buyer,
                &format!("/purchase-orders/{}/revise", order.id),
                &[
                    ("reason", "price rise"),
                    (&format!("qty_{}", line.id), "10"),
                    (&format!("price_{}", line.id), price),
                ],
            )
            .await
            .assert_ok();
        }
    };

    // Still registered on 2027-01-20: revised at once.
    t.on(date(2027, 1, 20), revise("420")).await;
    assert_eq!(order_now(&t, &order).await.revision, 2);
    assert!(pending(&t, &order).await.is_empty());

    // Expired on 2027-03-01: the revision waits.
    t.on(date(2027, 3, 1), revise("450")).await;
    let waiting = pending(&t, &order).await;
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].action, APPROVAL_ACTION);
    assert_eq!(waiting[0].transition, "revise");
    let now = order_now(&t, &order).await;
    assert_eq!(now.revision, 2, "the revision in force stays");
    let kept = PurchaseOrderLine::find_or_404(t.db(), line.id)
        .await
        .unwrap();
    assert_eq!(kept.unit_price, 42_000);

    t.on(date(2027, 3, 1), async {
        approvals::approve(t.state(), waiting[0].id, &pm, "ok")
            .await
            .unwrap();
    })
    .await;
    assert_eq!(order_now(&t, &order).await.revision, 3);
    let revised = PurchaseOrderLine::find_or_404(t.db(), line.id)
        .await
        .unwrap();
    assert_eq!(revised.unit_price, 45_000);
}
