//! BL-MFG-02 (#383): the unlink of a work order from its sales order on the
//! record page (07 §7.1 *Work order* `unlink`).

mod manufacturing_support;

use host::roles::{PRODUCTION_PLANNER, QC_INSPECTOR};
use manufacturing::WorkOrder;
use manufacturing_support::{boot, browse, floor, person, post};
use renox::prelude::*;

/// BL-MFG-02 (#383): the planner unlinks a work order from its sales order on
/// the record page; the page then says it is for stock, the activity tab has
/// the reason, and the action is no longer offered.
#[renox::test]
async fn unlink_is_offered_while_linked_and_its_reason_shows_in_the_activity() {
    let app = boot().await;
    let f = floor(&app).await;
    let planner = person(&app, "Amal", PRODUCTION_PLANNER, Some(f.plant.scope())).await;
    let inspector = person(&app, "Qasim", QC_INSPECTOR, Some(f.plant.scope())).await;
    WorkOrder::query()
        .where_(WorkOrder::ID.eq(f.order.id))
        .update(
            app.db(),
            &[
                ("source", &manufacturing::WorkOrderSource::SalesOrder),
                ("sales_order_id", &Some(50_i64)),
                ("sales_order_line_id", &Some(1_i64)),
            ],
        )
        .await
        .unwrap();
    let path = format!("/manufacturing/work-orders/{}", f.order.id);

    let before = browse(&app, &planner, &path).await;
    before.assert_see("unlink-order");
    before.assert_see("Sales order 50");
    browse(&app, &inspector, &path)
        .await
        .assert_dont_see("unlink-order");
    // A reason is needed; the refusal is the sheet's 422.
    post(&app, &planner, &format!("{path}/unlink"), &[("reason", "")])
        .await
        .assert_status(422);
    post(
        &app,
        &planner,
        &format!("{path}/unlink"),
        &[("reason", "customer cancelled, standard item")],
    )
    .await
    .assert_status(303);

    let after = browse(&app, &planner, &path).await;
    after.assert_ok();
    after.assert_see("Stock");
    after.assert_see("customer cancelled, standard item");
    after.assert_dont_see("unlink-order");
    after.assert_see("Released");
}
