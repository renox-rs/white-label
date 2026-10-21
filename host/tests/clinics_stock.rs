//! BL-CLN-16 (#364): the 05 values for the shared stock mechanisms, proved
//! through the real host of the demo clinic: every branch store blocks
//! negative stock and an issue beyond stock is a pending issue (no
//! `allow_consumption`, `STK.RUL.06`: the storekeeper at once, the clinic
//! manager after 24 hours), a recall hold is released only by a second holder
//! of `lots.release` and a *quality* hold of a suspect consumable by one of
//! the storekeeper or the clinic manager (`STK.ACL.05`, `STK.RUL.07`; 05
//! section 4.6, 7.2 *Settings for the shared negative flows*).

mod support;

use std::time::Duration;

use host::roles::{CLINIC_MANAGER, OWNER, STOREKEEPER};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use support::{clinics_booted, messages_to, person};
use wl_inventory::item::value_of;
use wl_inventory::movement::StockMovement;
use wl_inventory::pending_issue::open_of;
use wl_inventory::shortage_alert::{self, OVERDUE_NOTICE, SHORT_NOTICE};
use wl_inventory::stock_hold::lines_of;
use wl_inventory::{
    HoldLineStatus, HoldStatus, Item, Location, Lot, LotStatus, NegativeStockPolicy, StockHold,
};
use wl_kernel::Subject;
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::inventory::{ConsumeRequest, stock_service};
use wl_kernel::tickets::task_creator;
use wl_ticketing::Ticket;

/// Done when (stock), 1: an issue to a visit beyond the stock of a branch
/// store creates a pending issue (the store blocks negative stock and the
/// balance stays at zero); the storekeeper is told at once, and nobody else
/// before 24 hours; after 24 hours the storekeeper has the task and the
/// clinic manager is told, once.
#[renox::test]
async fn an_issue_beyond_stock_is_a_pending_issue_and_the_clinic_manager_is_told_after_24_hours() {
    let app = clinics_booted().await;
    let kim = person(&app, "Kim", STOREKEEPER).await;
    let mona = person(&app, "Mona", CLINIC_MANAGER).await;
    let mut item = Item::new("GAUZE", "Gauze", "box");
    item.save(app.db()).await.unwrap();
    let mut branch = Location::new("BRANCH-1", "Branch 1");
    branch.save(app.db()).await.unwrap();
    assert_eq!(
        branch.negative_stock_policy,
        NegativeStockPolicy::Block,
        "a branch store blocks negative stock"
    );

    let visit = Subject::new("clinics.visit", 1);
    stock_service(app.state())
        .consume(
            app.state(),
            ConsumeRequest::new(item.subject(), branch.subject(), 3_000, visit.clone())
                .cost_object(visit.clone()),
        )
        .await
        .unwrap();

    let open = open_of(app.db()).await.unwrap();
    assert_eq!(open.len(), 1, "the short quantity is a pending issue");
    assert_eq!(
        wl_inventory::balance::on_hand_at(app.db(), item.id, branch.id, None)
            .await
            .unwrap(),
        0,
        "the balance never goes below zero"
    );
    for location in Location::query().get(app.db()).await.unwrap() {
        assert_eq!(
            location.negative_stock_policy,
            NegativeStockPolicy::Block,
            "{}: no allow_consumption",
            location.code
        );
    }
    assert_eq!(messages_to(&app, SHORT_NOTICE, &kim).await.len(), 1);
    assert!(messages_to(&app, SHORT_NOTICE, &mona).await.is_empty());

    // 23 hours later: not yet.
    let key = shortage_alert::task_key(branch.id, item.id);
    app.travel(Duration::from_secs(23 * 3600));
    app.run_scheduled(shortage_alert::TASK).await.unwrap();
    assert!(messages_to(&app, OVERDUE_NOTICE, &mona).await.is_empty());
    assert!(
        task_creator(app.state())
            .find_open(app.state(), &key)
            .await
            .unwrap()
            .is_none()
    );

    // 24 hours after the pending issue: the task and the manager's notice.
    app.travel(Duration::from_secs(3600));
    app.run_scheduled(shortage_alert::TASK).await.unwrap();
    assert_eq!(messages_to(&app, OVERDUE_NOTICE, &mona).await.len(), 1);
    let task = task_creator(app.state())
        .find_open(app.state(), &key)
        .await
        .unwrap()
        .expect("the storekeeper has the task");
    let ticket = Ticket::find_or_404(app.db(), task.id.0).await.unwrap();
    assert_eq!(ticket.assignee_user_id, Some(kim.id));

    // Once.
    app.travel(Duration::from_secs(3600));
    app.run_scheduled(shortage_alert::TASK).await.unwrap();
    assert_eq!(messages_to(&app, OVERDUE_NOTICE, &mona).await.len(), 1);
}

/// A lot of `item` with `quantity_milli` at `branch`.
async fn held_lot(
    app: &renox::testing::TestApp,
    item: &Item,
    branch: &Location,
    number: &str,
) -> Lot {
    let mut lot = Lot {
        item_id: item.id,
        number: number.to_owned(),
        status: LotStatus::Released,
        ..Lot::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    lot.insert_in_state(&mut tx, LotStatus::Released, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let mut received = StockMovement::receive(
        item.id,
        Some(lot.id),
        branch.id,
        12_000,
        1_000,
        value_of(12_000, 1_000),
        &Subject::new("inventory.goods_receipt", 900),
        None,
    );
    received.save(app.db()).await.unwrap();
    lot
}

/// Places a hold of `reason` on `lot` as `by`; the hold placed.
async fn place_hold(
    app: &renox::testing::TestApp,
    by: &User,
    lot: &Lot,
    reason: &str,
) -> StockHold {
    app.acting_as(by);
    let ids = lot.id.to_string();
    app.htmx()
        .post(
            "/stock-holds",
            &[
                ("reason", reason),
                ("reference", "ticketing.ticket:9"),
                ("lots", ids.as_str()),
            ],
        )
        .await
        .assert_ok();
    StockHold::query()
        .order_by_desc(StockHold::ID)
        .first(app.db())
        .await
        .unwrap()
        .expect("a hold")
}

/// Done when (stock), 2: releasing a recall-held lot needs a second
/// `lots.release` holder: the storekeeper decides *release*, the request
/// waits, the storekeeper cannot approve it, the clinic manager does and the
/// lot is released. A *quality* hold of a suspect consumable needs one
/// decider only (the storekeeper or the clinic manager).
#[renox::test]
async fn releasing_a_recall_held_lot_needs_a_second_holder_of_lots_release() {
    let app = clinics_booted().await;
    let hal = person(&app, "Hal", STOREKEEPER).await;
    let ann = person(&app, "Ann", STOREKEEPER).await;
    let mona = person(&app, "Mona", CLINIC_MANAGER).await;
    let mut item = Item::new("FILLER", "Dermal filler", "syringe").lot_tracked();
    item.save(app.db()).await.unwrap();
    let mut branch = Location::new("BRANCH-1", "Branch 1");
    branch.save(app.db()).await.unwrap();

    // A recall hold.
    let lot = held_lot(&app, &item, &branch, "L9").await;
    let hold = place_hold(&app, &hal, &lot, "recall").await;
    let line = lines_of(app.db(), hold.id).await.unwrap().remove(0);
    app.acting_as(&ann);
    app.htmx()
        .post(&format!("/stock-hold-lines/{}/release", line.id), &[])
        .await
        .assert_ok();
    let waiting = ApprovalRequest::pending_about(app.db(), &line.subject())
        .await
        .unwrap();
    assert_eq!(waiting.len(), 1, "the release waits for a second holder");
    assert_eq!(
        lines_of(app.db(), hold.id).await.unwrap()[0].status,
        HoldLineStatus::Undecided
    );
    assert_eq!(
        Lot::find_or_404(app.db(), lot.id).await.unwrap().status,
        LotStatus::OnHold
    );
    let own = approvals::approve(app.state(), waiting[0].id, &ann, "").await;
    assert!(matches!(own, Err(Error::Forbidden)), "{own:?}");
    approvals::approve(app.state(), waiting[0].id, &mona, "checked")
        .await
        .unwrap();
    assert_eq!(
        Lot::find_or_404(app.db(), lot.id).await.unwrap().status,
        LotStatus::Released
    );

    // A suspect consumable: a quality hold, one decider is enough.
    let suspect = held_lot(&app, &item, &branch, "L10").await;
    let hold = place_hold(&app, &mona, &suspect, "quality").await;
    let line = lines_of(app.db(), hold.id).await.unwrap().remove(0);
    app.acting_as(&ann);
    app.htmx()
        .post(&format!("/stock-hold-lines/{}/release", line.id), &[])
        .await
        .assert_ok();
    assert!(
        ApprovalRequest::pending_about(app.db(), &line.subject())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        lines_of(app.db(), hold.id).await.unwrap()[0].status,
        HoldLineStatus::Released
    );
    assert_eq!(
        Lot::find_or_404(app.db(), suspect.id).await.unwrap().status,
        LotStatus::Released
    );
    let closed = StockHold::find_or_404(app.db(), hold.id).await.unwrap();
    assert_eq!(closed.status, HoldStatus::Closed);
}

/// The recall owner is the owner / medical director and the notices are sent
/// by the clinic manager (05 section 7.2): the roles hold the permissions.
#[test]
fn the_owner_opens_the_recall_and_the_clinic_manager_sends_the_notices() {
    let holds = |role: &str, permission: &str| {
        host::roles::ROLES
            .iter()
            .find(|(name, _)| *name == role)
            .is_some_and(|(_, grants)| grants.contains(&permission))
    };
    assert!(holds(OWNER, wl_inventory::permissions::RECALLS_MANAGE));
    assert!(holds(CLINIC_MANAGER, clinics::permissions::RECALLS_NOTIFY));
    assert!(holds(
        CLINIC_MANAGER,
        wl_inventory::permissions::LOTS_RELEASE
    ));
    assert!(holds(STOREKEEPER, wl_inventory::permissions::LOTS_RELEASE));
    // `STK.RUL.06` tells the clinic manager through `stock.approve`.
    assert!(holds(
        CLINIC_MANAGER,
        wl_inventory::permissions::STOCK_APPROVE
    ));
}
