//! BL-SAL-11 (#130) point 2, BL-MFG-02 (#383) point 5 and BL-MFG-03 (#384)
//! with the real wl-sales, wl-inventory and manufacturing: stock reserved for
//! the lines of a released order, the output lot of a work order reserved for
//! the line it serves and free again when the line goes, and the output
//! "for stock" of a started work order. The orders are entered, amended and
//! cancelled through the pages of wl-sales, the reports and completions are
//! the routes of the app, the events and the listeners are the real ones.

mod support;

use host::roles::{
    ADMIN, FINANCE_MANAGER, PRODUCTION_MANAGER, PRODUCTION_PLANNER, SALES, SALES_MANAGER,
    STOREKEEPER,
};
use manufacturing::{Line, Plant, WorkOrder, WorkOrderSource, WorkOrderStatus};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use wl_inventory::reservation::{SOURCE_ORDER, SOURCE_WORK_ORDER, StockReservation};
use wl_inventory::{Item, Location, Lot, LotStatus, StockMovement};
use wl_kernel::invoicing::VatCategory;
use wl_sales::customer_account::{self, Terms};
use wl_sales::{SaleLine, SalesOrder, SalesOrderStatus};

use support::{company, day, issued_invoice, manufacturing_booted, person};

/// The shop floor: a plant with one line, a finished goods location, the
/// lot-tracked item PANEL and a customer.
struct Floor {
    plant: Plant,
    line: Line,
    panel: Item,
    finished: Location,
}

async fn floor(app: &TestApp) -> Floor {
    let plant = Plant::create(
        app.db(),
        Plant {
            name: "Plant 1".into(),
            active: true,
            ..Plant::default()
        },
    )
    .await
    .unwrap();
    let mut finished = Location::new("FG-STORE", "Finished goods");
    finished.save(app.db()).await.unwrap();
    let mut panel = Item::new("PANEL", "Panel", "pcs").lot_tracked();
    panel.save(app.db()).await.unwrap();
    let line = Line::create(
        app.db(),
        Line {
            plant_id: plant.id,
            code: "L1".into(),
            name: "Line 1".into(),
            hourly_rate: 12_000,
            finished_location_id: Some(finished.id),
            active: true,
            ..Line::default()
        },
    )
    .await
    .unwrap();
    Floor {
        plant,
        line,
        panel,
        finished,
    }
}

/// A confirmed order of `customer` for `quantity` panels entered through the
/// pages by `sales`, and its line.
async fn place(
    app: &TestApp,
    sales: &User,
    customer: wl_kernel::PartyId,
    quantity: &str,
) -> (SalesOrder, SaleLine) {
    let order = submitted(app, sales, customer, quantity).await;
    let order = SalesOrder::find_or_404(app.db(), order.id).await.unwrap();
    assert_eq!(order.status, SalesOrderStatus::Confirmed);
    let line = SaleLine::of(app.db(), &order.subject())
        .await
        .unwrap()
        .remove(0);
    (order, line)
}

/// An order on account entered and submitted (it may be held).
async fn submitted(
    app: &TestApp,
    sales: &User,
    customer: wl_kernel::PartyId,
    quantity: &str,
) -> SalesOrder {
    app.acting_as(sales);
    app.htmx()
        .post(
            "/sales-orders",
            &[
                ("party_id", &customer.0.to_string()),
                ("channel", "phone"),
                ("payment_mode", "on_account"),
            ],
        )
        .await
        .assert_status(303);
    let order = SalesOrder::query()
        .order_by_desc(SalesOrder::ID)
        .first(app.db())
        .await
        .unwrap()
        .expect("the order");
    app.htmx()
        .post(
            &format!("/sales-orders/{}/lines", order.id),
            &[
                ("item_code", "PANEL"),
                ("quantity", quantity),
                ("unit_price", "10.00"),
                ("vat", "standard"),
            ],
        )
        .await
        .assert_status(303);
    app.htmx()
        .post(&format!("/sales-orders/{}/submit", order.id), &[("x", "")])
        .await
        .assert_status(303);
    SalesOrder::find_or_404(app.db(), order.id).await.unwrap()
}

/// A work order of `quantity_milli` panels for the order line, `in_progress`
/// on the floor's line.
async fn running(
    app: &TestApp,
    f: &Floor,
    order: &SalesOrder,
    line: &SaleLine,
    quantity_milli: i64,
) -> WorkOrder {
    let mut made = WorkOrder {
        number: format!("WO-{}-{}", order.id, line.id),
        plant_id: f.plant.id,
        line_id: Some(f.line.id),
        item_id: f.panel.id,
        quantity_milli,
        source: WorkOrderSource::SalesOrder,
        sales_order_id: Some(order.id),
        sales_order_line_id: Some(line.id),
        status: WorkOrderStatus::InProgress,
        std_material_cost: 1_800,
        std_unit_cost: 2_400,
        ..WorkOrder::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    made.insert_in_state(&mut tx, WorkOrderStatus::InProgress, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    made
}

async fn reload(app: &TestApp, id: i64) -> WorkOrder {
    WorkOrder::find_or_404(app.db(), id).await.unwrap()
}

/// The production manager reports `good` panels on the work order.
async fn report(app: &TestApp, by: &User, order: &WorkOrder, good: &str) {
    app.acting_as(by);
    app.htmx()
        .post(
            &format!("/manufacturing/work-orders/{}/report", order.id),
            &[("good", good)],
        )
        .await
        .assert_status(303);
}

/// What is reserved, and what is free, of the output lot of `order` at the
/// finished goods location.
async fn figures(app: &TestApp, f: &Floor, order: &WorkOrder) -> (i64, i64) {
    let lot = reload(app, order.id)
        .await
        .output_lot_id
        .expect("an output lot");
    let found = wl_inventory::balance::balance(app.db(), f.panel.id, f.finished.id, Some(lot))
        .await
        .unwrap();
    (found.reserved_milli, found.available_milli())
}

/// BL-SAL-11 (#130) point 2: an order held for credit reserves nothing; when
/// it is released and confirmed, the stock of its line is reserved (once);
/// the stock page says how much is reserved for orders; cancelling the order
/// gives the stock back.
#[renox::test]
async fn a_released_order_reserves_its_stock_and_cancelling_it_gives_it_back() {
    let app = manufacturing_booted().await;
    let f = floor(&app).await;
    let sales = person(&app, "Sara", SALES).await;
    let finance = person(&app, "Alia", ADMIN).await;
    let approver = person(&app, "Fadi", FINANCE_MANAGER).await;
    let keeper = person(&app, "Karim", STOREKEEPER).await;
    let customer = company(&app, "Al Noor Trading LLC").await;
    customer_account::save(app.state(), customer, Terms::default().limit(50_000_000))
        .await
        .unwrap();
    issued_invoice(
        &app,
        &finance,
        customer,
        100_000,
        VatCategory::Standard,
        day(-31),
    )
    .await;
    let mut lot = Lot {
        item_id: f.panel.id,
        number: "P-1".into(),
        status: LotStatus::Released,
        ..Lot::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    lot.insert_in_state(&mut tx, LotStatus::Released, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    StockMovement::receive(
        f.panel.id,
        Some(lot.id),
        f.finished.id,
        30_000,
        300,
        9_000,
        &wl_kernel::Subject::new("inventory.goods_receipt", 1),
        None,
    )
    .save(app.db())
    .await
    .unwrap();

    let held = submitted(&app, &sales, customer, "10").await;
    assert_eq!(held.status, SalesOrderStatus::CreditHold);
    assert!(
        StockReservation::of_order(app.db(), held.id)
            .await
            .unwrap()
            .is_empty(),
        "a held order reserves nothing"
    );

    // Released by a holder of `credit.release` who did not enter it, with an
    // approval: the order is confirmed.
    app.acting_as(&finance);
    app.htmx()
        .post(
            &format!("/sales-orders/{}/release", held.id),
            &[("note", "The customer paid")],
        )
        .await
        .assert_status(303);
    let waiting = wl_kernel::approvals::ApprovalRequest::pending_about(app.db(), &held.subject())
        .await
        .unwrap();
    assert_eq!(waiting.len(), 1, "the release waits for an approver");
    wl_kernel::approvals::approve(app.state(), waiting[0].id, &approver, "paid")
        .await
        .unwrap();
    let released = SalesOrder::find_or_404(app.db(), held.id).await.unwrap();
    assert_eq!(released.status, SalesOrderStatus::Confirmed);

    let line = SaleLine::of(app.db(), &held.subject())
        .await
        .unwrap()
        .remove(0);
    let rows = StockReservation::of_line(app.db(), held.id, line.id)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "one reservation for the line");
    assert_eq!(rows[0].source, SOURCE_ORDER);
    assert!(rows[0].is_active());
    // What an order reserves is by item and location, not by lot.
    assert_eq!(
        (rows[0].lot_id, rows[0].location_id, rows[0].quantity_milli),
        (None, f.finished.id, 10_000)
    );
    let found = wl_inventory::balance::balance(app.db(), f.panel.id, f.finished.id, None)
        .await
        .unwrap();
    assert_eq!(
        (found.reserved_milli, found.available_milli()),
        (10_000, 20_000)
    );

    // The stock page says it in words.
    app.acting_as(&keeper);
    let page = app
        .request()
        .header("accept", "text/html,application/xhtml+xml,*/*;q=0.8")
        .get(&format!("/stock?location={}", f.finished.id))
        .await;
    page.assert_ok();
    page.assert_see("Reserved for orders");
    page.assert_see("10 pcs");

    // The cancel flow of the order gives the stock back.
    app.acting_as(&sales);
    app.htmx()
        .post(
            &format!("/sales-orders/{}/cancel", held.id),
            &[("reason", "The customer withdrew the order")],
        )
        .await
        .assert_status(303);
    let rows = StockReservation::of_line(app.db(), held.id, line.id)
        .await
        .unwrap();
    assert!(!rows[0].is_active(), "given back");
    let found = wl_inventory::balance::balance(app.db(), f.panel.id, f.finished.id, None)
        .await
        .unwrap();
    assert_eq!((found.reserved_milli, found.available_milli()), (0, 30_000));
}

/// BL-MFG-02 (#383) point 5 with the real stock: the output lot of a work
/// order is reserved for the sale line when the output is reported; the
/// order is cancelled with the outcome *finish to stock*, the work order is
/// unlinked, and the lot is free stock.
#[renox::test]
async fn the_output_lot_reserved_for_a_cancelled_line_is_free_stock_after_the_unlink() {
    let app = manufacturing_booted().await;
    let f = floor(&app).await;
    let sales = person(&app, "Sara", SALES).await;
    let manager = person(&app, "Sam", SALES_MANAGER).await;
    let boss = person(&app, "Mona", PRODUCTION_MANAGER).await;
    let customer = company(&app, "Al Noor Trading LLC").await;
    let (order, line) = place(&app, &sales, customer, "10").await;
    let made = running(&app, &f, &order, &line, 10_000).await;

    report(&app, &boss, &made, "10").await;

    let rows: Vec<_> = StockReservation::of_line(app.db(), order.id, line.id)
        .await
        .unwrap()
        .into_iter()
        .filter(|row| row.is_active())
        .collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].source, SOURCE_WORK_ORDER);
    assert_eq!(rows[0].lot_id, reload(&app, made.id).await.output_lot_id);
    assert_eq!(figures(&app, &f, &made).await, (10_000, 0), "reserved");

    app.acting_as(&sales);
    app.htmx()
        .post(
            &format!("/sales-orders/{}/cancel", order.id),
            &[
                ("reason", "The customer withdrew the order"),
                (&format!("line_{}_outcome", line.id), "finish_to_stock"),
            ],
        )
        .await
        .assert_status(303);
    let waiting = wl_kernel::approvals::ApprovalRequest::pending_about(app.db(), &order.subject())
        .await
        .unwrap();
    assert!(!waiting.is_empty());
    assert_eq!(
        figures(&app, &f, &made).await,
        (10_000, 0),
        "still reserved while the cancellation waits"
    );
    for request in waiting {
        wl_kernel::approvals::approve(app.state(), request.id, &manager, "agreed")
            .await
            .unwrap();
    }

    let after = reload(&app, made.id).await;
    assert_eq!(after.source, WorkOrderSource::Stock, "unlinked");
    assert_eq!(after.status, WorkOrderStatus::InProgress);
    assert_eq!(figures(&app, &f, &made).await, (0, 10_000), "free stock");
}

/// BL-MFG-02 (#383) edge case: the planner unlinks a work order whose line is
/// still open; the lot stays reserved for the line, and what the unlinked
/// work order makes later is not reserved.
#[renox::test]
async fn unlinking_for_a_line_that_is_still_open_keeps_the_reservation() {
    let app = manufacturing_booted().await;
    let f = floor(&app).await;
    let sales = person(&app, "Sara", SALES).await;
    let planner = person(&app, "Amal", PRODUCTION_PLANNER).await;
    let boss = person(&app, "Mona", PRODUCTION_MANAGER).await;
    let customer = company(&app, "Al Noor Trading LLC").await;
    let (order, line) = place(&app, &sales, customer, "10").await;
    let made = running(&app, &f, &order, &line, 20_000).await;
    report(&app, &boss, &made, "10").await;

    app.acting_as(&planner);
    app.htmx()
        .post(
            &format!("/manufacturing/work-orders/{}/unlink", made.id),
            &[("reason", "Made for stock now")],
        )
        .await
        .assert_status(303);
    assert_eq!(figures(&app, &f, &made).await, (10_000, 0));

    report(&app, &boss, &made, "10").await;
    assert_eq!(
        figures(&app, &f, &made).await,
        (10_000, 10_000),
        "the later output is free stock"
    );
}

/// BL-MFG-03 (#384): 4 of the line's 10 are cancelled (outcome finish to
/// stock) after 10 panels were reported; the work order keeps its quantity and 4 are flagged for stock;
/// the reservation shrinks to the 6 the line needs, the 4 are free stock, and
/// completing the work order keeps it so.
#[renox::test]
async fn the_output_for_stock_of_a_started_work_order_is_not_reserved() {
    let app = manufacturing_booted().await;
    let f = floor(&app).await;
    let sales = person(&app, "Sara", SALES).await;
    let manager = person(&app, "Sam", SALES_MANAGER).await;
    let boss = person(&app, "Mona", PRODUCTION_MANAGER).await;
    let customer = company(&app, "Al Noor Trading LLC").await;
    let (order, line) = place(&app, &sales, customer, "10").await;
    let made = running(&app, &f, &order, &line, 10_000).await;
    report(&app, &boss, &made, "10").await;
    assert_eq!(figures(&app, &f, &made).await, (10_000, 0));

    app.acting_as(&sales);
    let cancelled = app
        .htmx()
        .post(
            &format!("/sales-orders/{}/cancel-line", order.id),
            &[
                ("line_no", "1"),
                ("quantity", "4"),
                ("reason", "The customer needs fewer"),
                ("outcome", "finish_to_stock"),
            ],
        )
        .await;
    assert_eq!(cancelled.status.as_u16(), 303, "{}", cancelled.text());
    let waiting = wl_kernel::approvals::ApprovalRequest::pending_about(app.db(), &order.subject())
        .await
        .unwrap();
    for request in waiting {
        wl_kernel::approvals::approve(app.state(), request.id, &manager, "agreed")
            .await
            .unwrap();
    }

    let flagged = reload(&app, made.id).await;
    assert_eq!(flagged.quantity_milli, 10_000, "keeps its quantity");
    assert_eq!(flagged.quantity_for_stock_milli, 4_000, "4 are for stock");
    assert_eq!(figures(&app, &f, &made).await, (6_000, 4_000));

    app.acting_as(&boss);
    let done = app
        .htmx()
        .post(
            &format!("/manufacturing/work-orders/{}/complete", made.id),
            &[("x", "")],
        )
        .await;
    assert_eq!(done.status.as_u16(), 303, "{}", done.text());
    assert_eq!(
        reload(&app, made.id).await.status,
        WorkOrderStatus::Completed
    );
    assert_eq!(figures(&app, &f, &made).await, (6_000, 4_000));
}
