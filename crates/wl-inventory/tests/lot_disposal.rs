//! What a vertical's NCR does with a failed lot through the kernel's
//! `LotDisposal` and `DispatchLots` (07 Manufacturing §7.1 *NCR*, BL-MFG-08
//! #389, BL-MFG-13 #394): the facts of a lot from a goods receipt, the draft
//! Return to supplier from the quarantine location at the receipt's cost,
//! the refusal for a lot with no supplier lot, the scrap (the lot's discard
//! or a wastage of the quantity), the release, the reject of an emptied lot
//! and the lots behind a dispatch line.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_inventory::movement::MovementType;
use wl_inventory::supplier_return::{self, ReturnStatus};
use wl_inventory::{DraftLine, Item, Location, LocationType, Lot, LotStatus, NewDispatchLine};
use wl_kernel::Subject;
use wl_kernel::inventory::{LOT_SUBJECT_TYPE, ReturnOutcome};
use wl_kernel::lot_disposal::{ReturnLot, ScrapLot, dispatch_lots, lot_disposal};

use support::{Booted, SUPPLIER, boot};

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn ncr() -> Subject {
    Subject::new("manufacturing.ncr", 5)
}

/// Resin at AED 28 a kg on average; 100 kg of lot `L7` received from the
/// supplier at AED 30 a kg.
async fn received(t: &Booted) -> (Item, Location, Lot) {
    let mut item = t.tracked_item("RESIN").await;
    item.average_cost = 2_800;
    item.save(t.db()).await.unwrap();
    let store = t.named_location("STORE-1", "Store 1").await;
    let (order, line) = t.order(&store, &item, 100_000, 3_000).await;
    let receipt = t
        .receipt(
            &order,
            &store,
            vec![DraftLine::on(line.id, 100_000, 100_000).lot("L7", day(2027, 12, 31))],
        )
        .await;
    let keeper = t.storekeeper("Amal").await;
    t.post_as(&keeper, &receipt).await.assert_ok();
    let lot = Lot::query()
        .where_(Lot::ITEM_ID.eq(item.id))
        .first(t.db())
        .await
        .unwrap()
        .expect("the receipt made a lot");
    (item, store, lot)
}

fn lot_subject(lot: &Lot) -> Subject {
    Subject::new(LOT_SUBJECT_TYPE, lot.id)
}

/// The facts of a lot from a goods receipt: its supplier, the supplier's lot
/// number, what is left and where, the receipt's cost and order line.
#[renox::test]
async fn the_facts_of_a_received_lot_name_its_supplier_cost_and_place() {
    let t = boot().await;
    let (item, store, lot) = received(&t).await;

    let facts = lot_disposal(t.state())
        .facts(t.state(), &lot_subject(&lot))
        .await
        .unwrap()
        .expect("the lot is known");

    assert_eq!(facts.supplier, Some(SUPPLIER));
    assert_eq!(facts.supplier_lot.as_deref(), Some("L7"));
    assert_eq!(facts.quantity_milli, 100_000);
    assert_eq!(facts.location, Some(store.subject()));
    assert_eq!(facts.unit_cost, 3_000);
    assert!(facts.po_line.is_some());
    assert!(facts.produced_by.is_none());
    assert_eq!(facts.item, item.subject());
}

/// A return of 30 kg of the lot moves it to the quarantine location and lists
/// it on a draft Return to supplier for the supplier at the receipt's cost,
/// the NCR being its source; cancelling the draft emits
/// `SupplierReturnCancelled`.
#[renox::test]
async fn a_return_moves_the_quantity_to_quarantine_and_a_cancelled_draft_is_told() {
    let t = boot().await;
    let (item, store, lot) = received(&t).await;
    let service = lot_disposal(t.state());

    let made = service
        .create_return(
            t.state(),
            ReturnLot {
                ncr: ncr(),
                lot: lot_subject(&lot),
                quantity_milli: 30_000,
                outcome: ReturnOutcome::Credit,
                reason: "colour off".into(),
                user: None,
            },
        )
        .await
        .unwrap()
        .expect("a draft return");

    let ret = supplier_return::SupplierReturn::find_or_404(t.db(), made.subject_id)
        .await
        .unwrap();
    assert_eq!(ret.status, ReturnStatus::Draft);
    assert_eq!(ret.supplier_party_id, SUPPLIER.0);
    assert_eq!(ret.source(), ncr());
    let quarantine = Location::query()
        .where_(Location::LOCATION_TYPE.eq(LocationType::Quarantine))
        .first(t.db())
        .await
        .unwrap()
        .expect("the quarantine location");
    assert_eq!(ret.location_id, Some(quarantine.id));
    let lines = supplier_return::lines_of(t.db(), ret.id).await.unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].lot_id, Some(lot.id));
    assert_eq!(lines[0].quantity_milli, 30_000);
    assert_eq!(lines[0].unit_cost, 3_000);
    assert_eq!(
        wl_inventory::balance::on_hand_at(t.db(), item.id, store.id, Some(lot.id))
            .await
            .unwrap(),
        70_000
    );
    assert_eq!(
        service.return_status(t.state(), &made).await.unwrap(),
        Some("draft".to_owned())
    );

    let procurement = t.procurement("Cara").await;
    t.cancel_as(&procurement, &ret, "supplier refuses collection", None)
        .await
        .assert_ok();
    assert_eq!(
        service.return_status(t.state(), &made).await.unwrap(),
        Some("cancelled".to_owned())
    );
    let told = t.cancelled_returns.all();
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].source, ncr());
    assert_eq!(told[0].supplier_return, made);
}

/// A lot with no goods receipt behind it has no supplier lot, and a quantity
/// that is not there is refused: 422 with words.
#[renox::test]
async fn a_lot_without_a_supplier_lot_or_without_the_quantity_is_refused() {
    let t = boot().await;
    let (_, _, lot) = received(&t).await;
    let service = lot_disposal(t.state());
    let item = t.tracked_item("OPENING").await;
    let store = t.named_location("STORE-2", "Store 2").await;
    let old = t.lot(&item, "OLD-1", LotStatus::Quarantine).await;
    t.stock_in(&item, &store, Some(&old), 10_000, 500).await;
    let ask = |lot: &Lot, quantity_milli| ReturnLot {
        ncr: ncr(),
        lot: lot_subject(lot),
        quantity_milli,
        outcome: ReturnOutcome::Replace,
        reason: "cracked".into(),
        user: None,
    };

    let no_supplier = service
        .create_return(t.state(), ask(&old, 5_000))
        .await
        .unwrap_err();
    let too_much = service
        .create_return(t.state(), ask(&lot, 100_001))
        .await
        .unwrap_err();

    match no_supplier {
        Error::Status(code, message) => {
            assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
            assert!(message.contains("supplier lot"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    match too_much {
        Error::Status(code, message) => {
            assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
            assert!(message.contains("Only 100"), "{message}");
        }
        other => panic!("{other:?}"),
    }
}

/// Scrap of the whole remaining quantity of a held lot is the lot's discard
/// (the lot is `rejected`); scrap of part of a lot is a `wastage` movement of
/// that quantity and the lot keeps its state.
#[renox::test]
async fn scrap_discards_a_whole_held_lot_and_wastes_part_of_a_lot() {
    let t = boot().await;
    let (item, store, lot) = received(&t).await;
    let service = lot_disposal(t.state());
    let held = t.lot(&item, "H-1", LotStatus::OnHold).await;
    t.stock_in(&item, &store, Some(&held), 10_000, 2_800).await;

    let part = service
        .scrap(
            t.state(),
            ScrapLot {
                ncr: ncr(),
                lot: lot_subject(&lot),
                quantity_milli: 20_000,
                reason: "contaminated".into(),
                user: None,
            },
        )
        .await
        .unwrap()
        .expect("scrapped");
    let whole = service
        .scrap(
            t.state(),
            ScrapLot {
                ncr: ncr(),
                lot: lot_subject(&held),
                quantity_milli: 10_000,
                reason: "contaminated".into(),
                user: None,
            },
        )
        .await
        .unwrap()
        .expect("scrapped");

    assert!(!part.discarded);
    assert_eq!(part.value, 20 * t.item_now(&item).await.average_cost);
    let wasted: Vec<_> = t
        .movements_of(&item)
        .await
        .into_iter()
        .filter(|m| m.movement_type == MovementType::Wastage && m.lot_id == Some(lot.id))
        .collect();
    assert_eq!(wasted.len(), 1);
    assert_eq!(wasted[0].quantity_milli, 20_000);
    assert_eq!(wasted[0].from_location_id, Some(store.id));
    assert_ne!(t.lot_now(&lot).await.status, LotStatus::Rejected);
    assert!(whole.discarded);
    assert_eq!(t.lot_now(&held).await.status, LotStatus::Rejected);
}

/// `release` makes a quarantined lot usable; `close_out` rejects a lot that a
/// return emptied and leaves one with stock alone.
#[renox::test]
async fn a_lot_is_released_and_an_emptied_one_is_rejected() {
    let t = boot().await;
    let (item, store, lot) = received(&t).await;
    let service = lot_disposal(t.state());
    let emptied = t.lot(&item, "E-1", LotStatus::Quarantine).await;
    t.stock_in(&item, &store, Some(&emptied), 10_000, 2_800)
        .await;
    t.stock_out(&item, &store, Some(&emptied), 10_000).await;

    assert!(
        !service
            .close_out(t.state(), &lot_subject(&lot))
            .await
            .unwrap()
    );
    assert!(
        service
            .close_out(t.state(), &lot_subject(&emptied))
            .await
            .unwrap()
    );
    assert!(
        service
            .release(t.state(), &lot_subject(&lot))
            .await
            .unwrap()
    );

    assert_eq!(t.lot_now(&emptied).await.status, LotStatus::Rejected);
    assert_eq!(t.lot_now(&lot).await.status, LotStatus::Released);
}

/// A dispatch line leads to its lot, and a dispatch to the lots of all its
/// lines; any other record leads to none.
#[renox::test]
async fn a_dispatch_line_leads_to_its_lot() {
    let t = boot().await;
    let (item, store, lot) = received(&t).await;
    let other = t.lot(&item, "F5", LotStatus::Released).await;
    t.stock_in(&item, &store, Some(&other), 40_000, 2_800).await;
    lot_disposal(t.state())
        .release(t.state(), &lot_subject(&lot))
        .await
        .unwrap();
    let dispatcher = t.dispatcher("Dan").await;
    let sent = t
        .sent_dispatch(
            &dispatcher,
            &store,
            50,
            vec![
                NewDispatchLine::of(item.id, 10_000).lot(lot.id),
                NewDispatchLine::of(item.id, 5_000).lot(other.id),
            ],
        )
        .await;
    let lines = t.dispatch_lines(&sent).await;
    let service = dispatch_lots(t.state());

    let one = service
        .lots_of(t.state(), &lines[0].subject())
        .await
        .unwrap();
    let all = service.lots_of(t.state(), &sent.subject()).await.unwrap();
    let none = service
        .lots_of(t.state(), &Subject::new("sales.sales_order", 50))
        .await
        .unwrap();

    assert_eq!(one.len(), 1);
    assert_eq!(one[0].lot, Some(lot_subject(&lot)));
    assert_eq!(one[0].quantity_milli, 10_000);
    assert_eq!(all.len(), 2);
    assert_eq!(all[1].lot, Some(lot_subject(&other)));
    assert!(none.is_empty());
}
