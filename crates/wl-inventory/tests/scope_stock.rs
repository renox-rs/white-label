//! What a vertical asks of the stock and the purchasing of a scope when the
//! scope stops (the kernel's `ScopeStock`; 02 Construction §7.1 *Project*
//! `terminate`): the open orders, the purchase requests without an order and
//! the count of the stores.

mod support;

use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_inventory::{
    PurchaseRequest, PurchaseRequestStatus, StockCount, StockCountStatus, stock_count,
};
use wl_kernel::Subject;
use wl_kernel::inventory::scope_stock;

use support::{Booted, boot};

fn project(id: i64) -> Subject {
    Subject::new("construction.project", id)
}

/// A purchase request at `location` in `status`, inserted as a fixture.
async fn request(t: &Booted, location: i64, status: PurchaseRequestStatus) -> PurchaseRequest {
    let mut request = PurchaseRequest::new(location);
    request.status = status;
    let mut tx = t.db().begin().await.unwrap();
    request
        .insert_in_state(&mut tx, status, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    request
}

async fn reload(t: &Booted, id: i64) -> PurchaseRequest {
    PurchaseRequest::find_or_404(t.db(), id).await.unwrap()
}

/// The requests that were not ordered are cancelled with the reason; an
/// ordered one and another project's stay.
#[renox::test]
async fn open_requests_without_an_order_are_cancelled() {
    let t = boot().await;
    let site = t.location("SITE-4", Some(&project(4))).await;
    let other = t.location("SITE-5", Some(&project(5))).await;
    let draft = request(&t, site.id, PurchaseRequestStatus::Draft).await;
    let approved = request(&t, site.id, PurchaseRequestStatus::Approved).await;
    let ordered = request(&t, site.id, PurchaseRequestStatus::Ordered).await;
    let elsewhere = request(&t, other.id, PurchaseRequestStatus::Submitted).await;

    let cancelled = scope_stock(t.state())
        .cancel_open_requests(t.state(), &project(4), "project terminated")
        .await
        .unwrap();

    assert_eq!(cancelled, [draft.subject(), approved.subject()]);
    let status = |id: i64| reload(&t, id);
    let draft = status(draft.id).await;
    assert_eq!(draft.status, PurchaseRequestStatus::Cancelled);
    assert_eq!(draft.cancel_reason.as_deref(), Some("project terminated"));
    assert_eq!(
        status(approved.id).await.status,
        PurchaseRequestStatus::Cancelled
    );
    assert_eq!(
        status(ordered.id).await.status,
        PurchaseRequestStatus::Ordered
    );
    assert_eq!(
        status(elsewhere.id).await.status,
        PurchaseRequestStatus::Submitted
    );
}

/// The open orders delivering to the scope are listed, with whether goods
/// were accepted on them; nothing is cancelled.
#[renox::test]
async fn the_open_orders_of_a_scope_are_listed() {
    let t = boot().await;
    let site = t.location("SITE-4", Some(&project(4))).await;
    let other = t.location("SITE-5", Some(&project(5))).await;
    let item = t.item("CEMENT").await;
    let (first, _) = t.order(&site, &item, 10_000, 1_800).await;
    let (second, line) = t.order(&site, &item, 5_000, 1_800).await;
    t.order(&other, &item, 5_000, 1_800).await;
    wl_inventory::PurchaseOrderLine::query()
        .where_(wl_inventory::PurchaseOrderLine::ID.eq(line.id))
        .update(t.db(), &[("accepted_qty_milli", &2_000_i64)])
        .await
        .unwrap();

    let open = scope_stock(t.state())
        .open_orders(t.state(), &project(4))
        .await
        .unwrap();

    assert_eq!(open.len(), 2);
    assert_eq!(open[0].order, first.subject());
    assert!(!open[0].received);
    assert_eq!(open[1].order, second.subject());
    assert!(open[1].received);
    let order = t.order_now(&first).await;
    assert_eq!(order.status, wl_inventory::PurchaseOrderStatus::Issued);
}

/// One count per location of the scope, none twice, and a cancelled count
/// does not count as open.
#[renox::test]
async fn a_count_is_opened_for_each_store_of_the_scope_once() {
    let t = boot().await;
    let site = t.location("SITE-4", Some(&project(4))).await;
    let yard = t.location("YARD-4", Some(&project(4))).await;
    t.location("SITE-5", Some(&project(5))).await;

    let stock = scope_stock(t.state());
    let opened = stock
        .open_stock_counts(t.state(), &project(4), "project terminated")
        .await
        .unwrap();
    assert_eq!(opened.len(), 2);
    let again = stock
        .open_stock_counts(t.state(), &project(4), "project terminated")
        .await
        .unwrap();
    assert!(again.is_empty());

    let counts = stock_count::unfinished_at(t.db(), site.id).await.unwrap();
    assert_eq!(counts.len(), 1);
    assert_eq!(counts[0].status, StockCountStatus::Open);
    assert_eq!(counts[0].reason.as_deref(), Some("project terminated"));
    assert_eq!(counts[0].source.as_deref(), Some("construction.project:4"));
    assert_eq!(
        stock
            .open_counts(t.state(), &project(4))
            .await
            .unwrap()
            .len(),
        2
    );

    // A cancelled count is no longer open; the store can be counted again.
    let manager = t.user_with("Mona", "stock_manager").await;
    let found = StockCount::find_or_404(t.db(), counts[0].id).await.unwrap();
    let _outcome = found
        .transition(stock_count::CANCEL)
        .by(&manager)
        .reason("counted by hand")
        .run(t.state())
        .await
        .unwrap();
    assert_eq!(
        stock
            .open_counts(t.state(), &project(4))
            .await
            .unwrap()
            .len(),
        1
    );
    let reopened = stock
        .open_stock_counts(t.state(), &project(4), "project terminated")
        .await
        .unwrap();
    assert_eq!(reopened.len(), 1);
    assert_eq!(
        stock_count::unfinished_at(t.db(), yard.id)
            .await
            .unwrap()
            .len(),
        1
    );
}
