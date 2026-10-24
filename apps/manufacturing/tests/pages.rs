//! The screens of the work order (Base 07, `docs/UX.md` §2): the list and the
//! record page answer a page for a browser and JSON otherwise, name the sales
//! order by its number, draw an action only for who may run it and disabled
//! with the reason when a guard fails, and keep the tables inside their
//! wrappers for a phone.

mod support;

use manufacturing::{WorkOrder, WorkOrderSource, WorkOrderStatus};
use renox::prelude::*;
use support::{RESIN, boot};

#[renox::test]
async fn the_pages_name_the_order_and_draw_only_the_actions_of_the_person() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let planner = t.planner("Amal", &plant).await;
    let keeper = t.keeper("Karim", &plant).await;
    let order = t.fixture(&plant, &line, WorkOrderStatus::Released).await;
    WorkOrder::query()
        .where_(WorkOrder::ID.eq(order.id))
        .update(
            t.db(),
            &[
                ("source", &WorkOrderSource::SalesOrder),
                ("sales_order_id", &Some(50_i64)),
                ("sales_order_line_id", &Some(1_i64)),
            ],
        )
        .await
        .unwrap();
    t.lines.open(50, 1, 1_000_000, None);
    let path = format!("/manufacturing/work-orders/{}", order.id);

    let page = t.browse(&planner, &path).await;
    page.assert_ok();
    page.assert_see("Sales order SO-50");
    page.assert_see("cancel-order");
    page.assert_see("unlink-order");
    page.assert_dont_see("issue-order");
    page.assert_dont_see("abort-order");
    page.assert_see("rx-table-wrap");
    let list = t.browse(&planner, "/manufacturing/work-orders").await;
    list.assert_ok();
    list.assert_see("Sales order SO-50");
    list.assert_see("New work order");
    t.browse(&keeper, &path).await.assert_see("issue-order");
    t.browse(&keeper, "/manufacturing/work-orders")
        .await
        .assert_dont_see("New work order");

    // JSON for everything else; the costs are not in it for a planner.
    let json = t.get(&planner, &path).await;
    json.assert_ok();
    assert!(json.text().contains("\"status_key\":\"released\""));
    assert!(json.text().contains("\"show_costs\":false"));
}

/// A refusal is the sheet's 422 with the words; an abort that waits is drawn
/// as a disabled button with *Abort pending*; a stranger gets 403.
#[renox::test]
async fn a_waiting_abort_disables_the_actions_and_says_why() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let planner = t.planner("Amal", &plant).await;
    let keeper = t.keeper("Karim", &plant).await;
    let stranger = t.user("Nadia").await;
    t.stock.put(RESIN, Some(1), 100_000, 800);
    let order = t.released(&planner, &plant, &line).await;
    t.act(
        &keeper,
        &order,
        "issue",
        &[("item_id", &RESIN.to_string()), ("quantity", "100")],
    )
    .await
    .assert_status(303);
    let path = format!("/manufacturing/work-orders/{}", order.id);

    let before = t.browse(&planner, &path).await;
    before.assert_see("abort-order");
    before.assert_dont_see("Abort pending");
    t.act(&planner, &order, "abort", &[("reason", "wrong BOM")])
        .await
        .assert_status(303);

    let after = t.browse(&planner, &path).await;
    after.assert_see("Abort pending");
    after.assert_see("Waiting for approval: Abort a work order.");
    t.browse(&stranger, &path).await.assert_forbidden();
    let refused = t.act(&stranger, &order, "cancel", &[("reason", "x")]).await;
    refused.assert_forbidden();
}
