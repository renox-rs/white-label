//! BL-DLV-07 (#163), wl-inventory §5.2 step 3, `STK.ACL.02`: the real
//! `InventoryWastage` the module provides to `wl-delivery`. It estimates what
//! the discarded goods of a delivery are worth at cost (the figure compared
//! with the limit for a second person) and records the wastage with the order
//! as reference and the driver's reason: the exploded components of the
//! quantity when the consumption was not yet posted, the value of the line
//! (no second movement) when it was, and nothing for a line it cannot explain.

mod support;

use renox::prelude::Model;

use wl_inventory::movement::MovementType;
use wl_inventory::{Item, Location};
use wl_kernel::Subject;
use wl_kernel::delivery::{DeliveryId, DiscardLine, DiscardWastage, delivery_wastage};
use wl_kernel::inventory::{LineComponent, SaleLineStock};
use wl_kernel::sales::SalesOrderId;

use support::{Booted, boot};

const ORDER: i64 = 7;

/// Chicken and buns at the outlet with their cost; one meal is 0.2 kg of
/// chicken (AED 10.00 a kg) and one bun (AED 0.50).
async fn kitchen(t: &Booted) -> (Item, Item, Location) {
    let mut chicken = t.item("CHICKEN").await;
    let mut bun = t.item("BUN").await;
    let outlet = t.location("OUTLET-1", None).await;
    t.stock_in(&chicken, &outlet, None, 20_000, 1_000).await;
    t.stock_in(&bun, &outlet, None, 20_000, 500).await;
    chicken.average_cost = 1_000;
    chicken.save(t.db()).await.unwrap();
    bun.average_cost = 500;
    bun.save(t.db()).await.unwrap();
    (chicken, bun, outlet)
}

fn recipe(chicken: &Item, bun: &Item, outlet: &Location, consumed: bool) -> SaleLineStock {
    SaleLineStock {
        location: outlet.subject(),
        components: vec![
            LineComponent {
                item: chicken.subject(),
                per_unit_milli: 200,
            },
            LineComponent {
                item: bun.subject(),
                per_unit_milli: 1_000,
            },
        ],
        quantity_milli: 3_000,
        consumed,
        value: 9_000,
    }
}

fn discard(lines: Vec<DiscardLine>) -> DiscardWastage {
    DiscardWastage {
        delivery: DeliveryId(5),
        source: SalesOrderId(ORDER).subject(),
        reason: "Spoilt in the heat".to_owned(),
        photo: Some("photo-1".to_owned()),
        lines,
    }
}

fn line(sale_line: Option<i64>, quantity_milli: i64) -> DiscardLine {
    DiscardLine {
        source_line: sale_line.map(|id| Subject::new("sales.line", id)),
        description: "Chicken burger".to_owned(),
        quantity_milli,
    }
}

async fn wastage_of(t: &Booted, item: &Item) -> Vec<wl_inventory::movement::StockMovement> {
    t.movements_of(item)
        .await
        .into_iter()
        .filter(|m| m.movement_type == MovementType::Wastage)
        .collect()
}

/// The estimate of three meals not yet consumed is the cost of their
/// components (0.6 kg chicken AED 6.00 and 3 buns AED 1.50), and it posts
/// nothing.
#[renox::test]
async fn the_estimate_is_the_cost_of_the_components_and_posts_nothing() {
    let t = boot().await;
    let (chicken, bun, outlet) = kitchen(&t).await;
    t.sale_lines.set(21, recipe(&chicken, &bun, &outlet, false));

    let worth = delivery_wastage(t.state())
        .estimate(t.state(), &discard(vec![line(Some(21), 3_000)]))
        .await
        .unwrap();

    assert_eq!(worth, 600 + 1_500);
    assert!(wastage_of(&t, &chicken).await.is_empty());
}

/// Recording posts one wastage movement per component with the order as the
/// reference and the driver's reason, and returns the value and the first
/// movement as the record.
#[renox::test]
async fn the_record_posts_the_components_as_wastage_with_the_order_and_the_drivers_reason() {
    let t = boot().await;
    let (chicken, bun, outlet) = kitchen(&t).await;
    t.sale_lines.set(21, recipe(&chicken, &bun, &outlet, false));

    let recorded = delivery_wastage(t.state())
        .record(t.state(), &discard(vec![line(Some(21), 3_000)]))
        .await
        .unwrap()
        .expect("something was recorded");

    assert_eq!(recorded.value, 2_100);
    let chicken_waste = wastage_of(&t, &chicken).await;
    let bun_waste = wastage_of(&t, &bun).await;
    assert_eq!(chicken_waste.len(), 1);
    assert_eq!(chicken_waste[0].quantity_milli, 600);
    assert_eq!(bun_waste[0].quantity_milli, 3_000);
    for moved in chicken_waste.iter().chain(&bun_waste) {
        assert_eq!(moved.reason.as_deref(), Some("Spoilt in the heat"));
        assert_eq!(moved.source(), SalesOrderId(ORDER).subject());
    }
    let reference = recorded
        .reference
        .expect("the first movement is the record");
    assert_eq!(reference.subject_type, "inventory.stock_movement");
    assert_eq!(reference.subject_id, chicken_waste[0].id);
}

/// A line whose consumption was posted already makes no second movement: the
/// share of the discarded quantity is the value reported (and estimated).
#[renox::test]
async fn a_consumed_line_makes_no_second_movement_and_reports_its_share() {
    let t = boot().await;
    let (chicken, bun, outlet) = kitchen(&t).await;
    t.sale_lines.set(21, recipe(&chicken, &bun, &outlet, true));
    let request = discard(vec![line(Some(21), 1_000)]);

    let worth = delivery_wastage(t.state())
        .estimate(t.state(), &request)
        .await
        .unwrap();
    let recorded = delivery_wastage(t.state())
        .record(t.state(), &request)
        .await
        .unwrap()
        .expect("the value is reported");

    assert_eq!(worth, 3_000);
    assert_eq!(recorded.value, 3_000);
    assert!(recorded.reference.is_none(), "no movement was made");
    assert!(wastage_of(&t, &chicken).await.is_empty());
    assert!(wastage_of(&t, &bun).await.is_empty());
}

/// A line the stock source cannot explain (a dispatch note's, or one with no
/// source line) is worth nothing and records nothing.
#[renox::test]
async fn a_line_without_a_recipe_estimates_zero_and_records_nothing() {
    let t = boot().await;
    let (chicken, bun, outlet) = kitchen(&t).await;
    t.sale_lines.set(21, recipe(&chicken, &bun, &outlet, false));
    let request = discard(vec![line(None, 2_000), line(Some(99), 2_000)]);

    let worth = delivery_wastage(t.state())
        .estimate(t.state(), &request)
        .await
        .unwrap();
    let recorded = delivery_wastage(t.state())
        .record(t.state(), &request)
        .await
        .unwrap();

    assert_eq!(worth, 0);
    assert!(recorded.is_none());
    assert!(wastage_of(&t, &chicken).await.is_empty());
}
