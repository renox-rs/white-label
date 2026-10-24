//! Items, purchase orders, goods receipts, lots and dispatches in the search
//! index and the Related panel (BL-AGT-11): each type's document is written
//! by its write path or its transitions, found by its identifiers, and its
//! relations are produced (a lot's item, movements, holds and the dispatches
//! that shipped it; an order's receipts; a receipt's lots; a dispatch's lots).

mod support;

use renox::prelude::*;
use wl_inventory::{DraftLine, Lot, NewDispatchLine, permissions as perm};
use wl_kernel::Subject;
use wl_kernel::inventory::{
    DISPATCH_SUBJECT_TYPE, GOODS_RECEIPT_SUBJECT_TYPE, ITEM_SUBJECT_TYPE, LOT_SUBJECT_TYPE,
    PURCHASE_ORDER_SUBJECT_TYPE,
};
use wl_kernel::related::{self, RelatedGroup};
use wl_kernel::scopes::Reach;
use wl_kernel::search::{self, Access};

use support::{ORDER, boot};

/// Someone who may see every inventory type, everywhere.
fn everyone() -> Access {
    let permissions = [
        perm::ITEMS_VIEW,
        perm::STOCK_VIEW,
        perm::PURCHASE_ORDERS_VIEW,
        perm::RECALLS_VIEW,
        "parties.view",
    ];
    Access::with(
        1,
        permissions
            .iter()
            .map(|p| ((*p).to_owned(), Reach::everything())),
    )
}

/// The count of group `name`, if the group is shown.
fn count(groups: &[RelatedGroup], name: &str) -> Option<u64> {
    groups.iter().find(|g| g.name == name).map(|g| g.count)
}

async fn found(db: &Db, text: &str, subject: &Subject) -> bool {
    search::lookup_identifier(db, text)
        .await
        .unwrap()
        .iter()
        .any(|m| &m.subject == subject)
}

#[renox::test]
async fn every_inventory_type_is_found_and_shows_its_related_records() {
    let t = boot().await;
    let item = t.tracked_item("FLOUR-25").await;
    let store = t.location("STORE-1", None).await;
    let (order, po_line) = t.order(&store, &item, 100_000, 1_200).await;
    let expiry = renox::chrono::NaiveDate::from_ymd_opt(2027, 6, 30).unwrap();
    let receipt = t
        .receipt(
            &order,
            &store,
            vec![DraftLine::on(po_line.id, 100_000, 100_000).lot("L-2026-77", expiry)],
        )
        .await;
    let receipt_subject = Subject::new(GOODS_RECEIPT_SUBJECT_TYPE, receipt.id);
    // The draft is indexed as it is created.
    assert!(
        search::document_of(t.db(), &receipt_subject)
            .await
            .unwrap()
            .is_some()
    );
    let amal = t.storekeeper("Amal").await;
    t.post_as(&amal, &receipt).await.assert_ok();
    let receipt = t.reload(&receipt).await;
    let number = receipt
        .number
        .clone()
        .expect("a posted receipt has a number");
    // The post (a transition) wrote the number into the document.
    assert!(found(t.db(), &number, &receipt_subject).await);

    // The lot, made by the post, is found by its number typed loosely.
    let lot = Lot::query()
        .where_eq("number", "L-2026-77")
        .first(t.db())
        .await
        .unwrap()
        .expect("the post made the lot");
    let lot_subject = Subject::new(LOT_SUBJECT_TYPE, lot.id);
    assert!(found(t.db(), "l 2026 77", &lot_subject).await);
    let document = search::document_of(t.db(), &lot_subject)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(document.title, "L-2026-77");
    assert!(document.secondary.contains("FLOUR-25"));

    // Items and orders the fixtures inserted come in with the rebuild the
    // seeds end with; the item is found by its code.
    search::rebuild(t.db()).await.unwrap();
    let item_subject = Subject::new(ITEM_SUBJECT_TYPE, item.id);
    assert!(found(t.db(), "flour 25", &item_subject).await);
    let order_subject = Subject::new(PURCHASE_ORDER_SUBJECT_TYPE, order.id);
    assert!(
        search::document_of(t.db(), &order_subject)
            .await
            .unwrap()
            .is_some()
    );

    // 10 kg of the lot leave on a dispatch with a courier number.
    let dispatcher = t.dispatcher("Dina").await;
    let sent = t
        .sent_dispatch(
            &dispatcher,
            &store,
            ORDER,
            vec![NewDispatchLine::of(item.id, 10_000).lot(lot.id)],
        )
        .await;
    let dispatch_subject = Subject::new(DISPATCH_SUBJECT_TYPE, sent.id);
    assert!(found(t.db(), "TRK-1", &dispatch_subject).await);
    let dispatch_number = sent.number.clone().unwrap();
    assert!(found(t.db(), &dispatch_number, &dispatch_subject).await);
    let document = search::document_of(t.db(), &dispatch_subject)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(document.status.as_deref(), Some("dispatched"));

    // The rest of the lot is held: the lot's document follows its `hold`.
    let holder = t.holder("Hana").await;
    let lot_id = lot.id.to_string();
    t.hold_as(
        &holder,
        &[
            ("reason", "quality"),
            ("reference", "ticketing.ticket:9"),
            ("lots", lot_id.as_str()),
        ],
    )
    .await
    .assert_ok();
    let document = search::document_of(t.db(), &lot_subject)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(document.status.as_deref(), Some("on_hold"));

    let access = everyone();
    let lot_groups = related::related(t.db(), &access, &lot_subject, 1)
        .await
        .unwrap();
    assert_eq!(count(&lot_groups, "item"), Some(1));
    assert_eq!(count(&lot_groups, "holds"), Some(1));
    assert_eq!(count(&lot_groups, "dispatches"), Some(1));
    assert_eq!(count(&lot_groups, "recalls"), Some(0));
    assert!(count(&lot_groups, "movements").unwrap_or(0) >= 2);

    let item_groups = related::related(t.db(), &access, &item_subject, 1)
        .await
        .unwrap();
    assert_eq!(count(&item_groups, "lots"), Some(1));
    assert!(count(&item_groups, "suppliers").is_some());

    let order_groups = related::related(t.db(), &access, &order_subject, 1)
        .await
        .unwrap();
    assert_eq!(count(&order_groups, "receipts"), Some(1));
    assert_eq!(count(&order_groups, "supplier"), Some(1));

    let receipt_groups = related::related(t.db(), &access, &receipt_subject, 1)
        .await
        .unwrap();
    assert_eq!(count(&receipt_groups, "purchase_order"), Some(1));
    assert_eq!(count(&receipt_groups, "lots"), Some(1));

    let dispatch_groups = related::related(t.db(), &access, &dispatch_subject, 1)
        .await
        .unwrap();
    assert_eq!(count(&dispatch_groups, "lots"), Some(1));

    // The lot's page loads its panel.
    let finance = t.finance("Fay").await;
    t.browse(&finance, &format!("/stock/lots/{}", lot.id))
        .await
        .assert_ok()
        .assert_see(&format!(
            "hx-get=\"/related/inventory.lot:{}/panel\"",
            lot.id
        ));
}

#[renox::test]
async fn a_lot_number_shared_by_two_items_finds_both_lots() {
    let t = boot().await;
    let flour = t.tracked_item("FLOUR").await;
    let sugar = t.tracked_item("SUGAR").await;
    let one = t
        .lot(&flour, "B-100", wl_inventory::LotStatus::Released)
        .await;
    let two = t
        .lot(&sugar, "B-100", wl_inventory::LotStatus::Released)
        .await;
    search::rebuild_type(t.db(), LOT_SUBJECT_TYPE)
        .await
        .unwrap();
    assert!(found(t.db(), "B-100", &Subject::new(LOT_SUBJECT_TYPE, one.id)).await);
    assert!(found(t.db(), "B-100", &Subject::new(LOT_SUBJECT_TYPE, two.id)).await);
}
