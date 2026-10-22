//! BL-SAL-01 (#120): an aggregator or website may send the same order twice;
//! the external order number is unique per channel (trimmed, case-insensitive),
//! so a second import is refused and names the existing order. Capability
//! `SAL.RUL` order intake (wl-sales §3 *Sales order*, §4.2 row `submit`,
//! §7 *Unique external order number per channel*).

mod support;

use renox::prelude::*;
use wl_kernel::PartyId;
use wl_sales::sales_orders::{self, Imported, NewSalesOrder};
use wl_sales::{SalesOrder, SalesOrderStatus, WlSales};

use support::online::{boot_online, boot_online_with, draft_order, seller};

/// An import of `number` on `channel` for party 7.
fn order(channel: &str, number: &str) -> NewSalesOrder {
    NewSalesOrder::new(PartyId(7))
        .channel(channel)
        .external_number(number)
}

async fn count(db: &Db) -> i64 {
    SalesOrder::query().count(db).await.unwrap()
}

/// AC1: Given order SO-2026-0101 on channel *Talabat* with external number
/// "TB-555", when an import with "tb-555 " arrives on *Talabat*, then it is
/// refused and the response names SO-2026-0101.
#[renox::test]
async fn a_second_import_of_the_number_on_the_channel_is_refused_and_names_the_first() {
    let (t, _) = boot_online().await;
    let Imported::Created(first) = sales_orders::import(t.state(), None, order("talabat", "TB-555"))
        .await
        .unwrap()
    else {
        panic!("the first import makes the order");
    };
    let again = sales_orders::import(t.state(), None, order("talabat", "tb-555 "))
        .await
        .unwrap();
    match again {
        Imported::Duplicate { id, number } => {
            assert_eq!((id, number), (first.id, first.number.clone()));
        }
        Imported::Created(_) => panic!("the duplicate was accepted"),
    }
    // No second order, and the existing one keeps its data.
    assert_eq!(count(t.db()).await, 1);
    let kept = SalesOrder::find_or_404(t.db(), first.id).await.unwrap();
    assert_eq!(kept.external_number.as_deref(), Some("TB-555"));
    assert_eq!(kept.status, SalesOrderStatus::Draft);
    // The service that returns an order answers 409 with the existing number.
    let refused = sales_orders::create(t.state(), None, order("talabat", " Tb-555"))
        .await
        .unwrap_err();
    assert!(refused.to_string().contains(&first.number), "{refused}");
}

/// AC1 (the screen): the entry form refuses the number and says which order
/// has it; the screen shows no second order.
#[renox::test]
async fn the_order_form_refuses_a_number_the_channel_used() {
    let (t, _) = boot_online().await;
    t.app.acting_as(&seller(&t).await);
    let first = draft_order(&t, "talabat", "on_account", &[("external_number", "TB-555")]).await;
    t.app
        .post(
            "/sales-orders",
            &[
                ("party_id", "7"),
                ("channel", "talabat"),
                ("external_number", "tb-555 "),
            ],
        )
        .await
        .assert_status(422)
        .assert_see(&first.number);
    assert_eq!(count(t.db()).await, 1);
}

/// AC2: Given "TB-555" on channel *Deliveroo*, the import is accepted.
#[renox::test]
async fn the_same_number_on_another_channel_is_accepted() {
    let (t, _) = boot_online().await;
    sales_orders::import(t.state(), None, order("talabat", "TB-555"))
        .await
        .unwrap();
    let other = sales_orders::import(t.state(), None, order("deliveroo", "TB-555"))
        .await
        .unwrap();
    assert!(matches!(other, Imported::Created(_)));
    assert_eq!(count(t.db()).await, 2);
}

/// AC3: Given two imports of "TB-777" at the same moment, exactly one order
/// exists.
#[renox::test]
async fn two_imports_at_the_same_moment_make_one_order() {
    let (t, _) = boot_online().await;
    let (a, b) = renox::tokio::join!(
        sales_orders::import(t.state(), None, order("talabat", "TB-777")),
        sales_orders::import(t.state(), None, order("talabat", "tb-777")),
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    let made = [&a, &b]
        .iter()
        .filter(|r| matches!(r, Imported::Created(_)))
        .count();
    assert_eq!(made, 1, "exactly one import makes the order");
    assert_eq!(count(t.db()).await, 1);
    // The loser is told which order won.
    let winner = SalesOrder::query().first(t.db()).await.unwrap().unwrap();
    for result in [&a, &b] {
        if let Imported::Duplicate { id, number } = result {
            assert_eq!((*id, number.as_str()), (winner.id, winner.number.as_str()));
        }
    }
}

/// AC4: Given SO with "TB-555" `cancelled`, a new import "TB-555" is refused.
#[renox::test]
async fn a_cancelled_order_still_counts_as_used() {
    let (t, _) = boot_online().await;
    t.app.acting_as(&seller(&t).await);
    let first = draft_order(&t, "talabat", "on_account", &[("external_number", "TB-555")]).await;
    t.app
        .post(
            &format!("/sales-orders/{}/cancel", first.id),
            &[("reason", "Customer called")],
        )
        .await
        .assert_status(303);
    let cancelled = SalesOrder::find_or_404(t.db(), first.id).await.unwrap();
    assert_eq!(cancelled.status, SalesOrderStatus::Cancelled);
    let again = sales_orders::import(t.state(), None, order("talabat", "TB-555"))
        .await
        .unwrap();
    assert!(
        matches!(again, Imported::Duplicate { id, .. } if id == first.id),
        "a re-import of a cancelled order's number is refused"
    );
}

/// Edge: an empty external number (counter, phone) is not checked.
#[renox::test]
async fn orders_without_an_external_number_are_not_checked() {
    let (t, _) = boot_online().await;
    for _ in 0..2 {
        let made = sales_orders::import(
            t.state(),
            None,
            NewSalesOrder::new(PartyId(7)).channel("phone").external_number("  "),
        )
        .await
        .unwrap();
        assert!(matches!(made, Imported::Created(_)));
    }
    assert_eq!(count(t.db()).await, 2);
}

/// Edge: with the setting off no check is made.
#[renox::test]
async fn with_the_setting_off_a_repeated_number_is_accepted() {
    let (t, _) = boot_online_with(WlSales::new().unique_external_order_number(false)).await;
    for _ in 0..2 {
        let made = sales_orders::import(t.state(), None, order("talabat", "TB-555"))
            .await
            .unwrap();
        assert!(matches!(made, Imported::Created(_)));
    }
    assert_eq!(count(t.db()).await, 2);
}
