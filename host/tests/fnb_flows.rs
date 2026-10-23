//! BL-FNB-16 (#380), 06 §3.3, §4.2, §7.2: the 06 deployment's values for the
//! shared delivery, sales and payment mechanisms are in force when the real
//! mechanism runs. The world is the F&B test world of `apps/fnb` (the real
//! wl-delivery, wl-sales, wl-payments, wl-inventory and wl-parties, driven
//! through their routes, transitions and scheduled jobs) booted with the
//! module settings the host seeds for a F&B client (`host::fnb_seed`), so a
//! change of a 06 value that breaks the story breaks these tests. The read-back
//! of the values stays in `fnb_money.rs`.

#[path = "../../apps/fnb/tests/support/mod.rs"]
mod world;

use fnb::FnbOrderStatus;
use fnb::order::payment;
use renox::chrono::TimeDelta;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_delivery::{DeliveryStatus, WlDelivery};
use wl_inventory::WlInventory;
use wl_kernel::Subject;
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::sales::SALES_RETURN_SUBJECT_TYPE;
use wl_payments::WlPayments;
use wl_payments::gateway::FakeGateway;
use wl_payments::payment_link::{self, NewLink, PaymentLink};
use wl_sales::permissions as sperm;
use wl_sales::{
    ReturnResolution, ReturnStatus, SaleLine, SalesOrder, SalesOrderStatus, SalesReturn, WlSales,
};

use world::stock;
use world::{World, make_user};

/// The world with the module settings of the 06 client.
async fn deployed() -> (World, FakeGateway) {
    host::fnb_seed::register_approvals();
    let gateway = FakeGateway::new();
    let w = world::boot_with_sales(
        host::fnb_seed::delivery(WlDelivery::new()),
        host::fnb_seed::module(),
        host::fnb_seed::payments(WlPayments::new().gateway(gateway.clone())),
        host::fnb_seed::inventory(WlInventory::new()),
        host::fnb_seed::sales(WlSales::new()),
    )
    .await;
    (w, gateway)
}

const CASH: [(&str, &str, &str); 1] = [("shawarma-plate", "1", "80.00")];

/// AC (Delivery 2): COD AED 80.00. The rider collects AED 76.00: within the
/// AED 5.00 tolerance, it is delivered (with the reason) without an approval. AED 74.00 is short
/// by AED 6.00: it needs a reason and waits for the kitchen manager, and the
/// rider cannot approve it.
#[renox::test]
async fn cash_short_by_aed_4_is_accepted_and_short_by_aed_6_waits_for_the_kitchen_manager() {
    let (w, _) = deployed().await;
    let c = w.customer("Mariam Al Hosani").await;

    let (within, delivery, _run) = w.dispatched(c, payment::CASH_ON_DELIVERY, &CASH).await;
    w.deliver_stop_with(
        &delivery,
        "76.00",
        &[("difference_reason", "recipient_short")],
    )
    .await
    .assert_status(303);
    assert_eq!(w.reload(&within).await.status, FnbOrderStatus::Completed);
    assert!(
        ApprovalRequest::pending_about(w.db(), &delivery.delivery_id().subject())
            .await
            .unwrap()
            .is_empty(),
        "AED 4.00 is within the tolerance: no approval"
    );

    let (short, delivery, _run) = w.dispatched(c, payment::CASH_ON_DELIVERY, &CASH).await;
    w.deliver_stop(&delivery, "74.00")
        .await
        .assert_status(422)
        .assert_see("short of the AED 80.00 expected");
    w.deliver_stop_with(
        &delivery,
        "74.00",
        &[("difference_reason", "recipient_short")],
    )
    .await
    .assert_status(303);
    assert_eq!(
        w.delivery(&delivery).await.status,
        DeliveryStatus::OutForDelivery
    );
    assert_eq!(w.reload(&short).await.status, FnbOrderStatus::Dispatched);
    let request = ApprovalRequest::pending_about(w.db(), &delivery.delivery_id().subject())
        .await
        .unwrap()
        .pop()
        .expect("the difference waits for a kitchen manager");

    let approve = format!("/deliveries/{}/cod-difference/approve", delivery.id);
    w.post(&w.rider, &approve, &[("absorbed_by", "business")])
        .await
        .assert_forbidden();
    w.post(&w.manager, &approve, &[("absorbed_by", "business")])
        .await
        .assert_ok();
    assert_eq!(w.reload(&short).await.status, FnbOrderStatus::Completed);
    assert_eq!(
        ApprovalRequest::find(w.db(), request.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        wl_kernel::approvals::ApprovalStatus::Approved
    );
}

/// A user who files and decides the returns of the sales module.
async fn returns_desk(w: &World, name: &str, approves: bool) -> User {
    let mut grants = vec![
        sperm::SALES_ORDERS_VIEW,
        sperm::RETURNS_REQUEST,
        sperm::RETURNS_HANDLE,
    ];
    if approves {
        grants.push(sperm::RETURNS_APPROVE);
    }
    make_user(w.db(), name, &grants).await
}

/// AC (Sales S2): a prepaid order of AED 60.00 delivered; a no-goods refund of
/// AED 50.00 is resolved at once, one of AED 51.00 waits for the kitchen
/// manager and the clerk who filed it cannot approve it.
#[renox::test]
async fn a_no_goods_refund_above_aed_50_waits_for_a_manager_other_than_the_clerk() {
    let (w, _) = deployed().await;
    let clerk = returns_desk(&w, "Cleo", false).await;
    let manager = returns_desk(&w, "Basem", true).await;

    let sixty = [("shawarma-plate", "1", "60.00")];
    let mut raised = Vec::new();
    for amount in ["50.00", "51.00"] {
        let c = w.customer(&format!("Customer {amount}")).await;
        let (order, delivery, _run) = w.dispatched(c, "online", &sixty).await;
        w.deliver_stop(&delivery, "0").await.assert_status(303);
        let sales = SalesOrder::find_or_404(w.db(), order.sales_order_id)
            .await
            .unwrap();
        let line = SaleLine::of(w.db(), &sales.subject())
            .await
            .unwrap()
            .remove(0);
        let field = format!("line_{}_refund", line.id);
        w.post(
            &clerk,
            &format!("/sales-orders/{}/refund", sales.id),
            &[
                ("reason", "arrived cold"),
                ("category", "cold"),
                ("party", "business"),
                (field.as_str(), amount),
            ],
        )
        .await
        .assert_status(303);
        let ret = SalesReturn::query()
            .where_(SalesReturn::SALE_ID.eq(sales.id))
            .first(w.db())
            .await
            .unwrap()
            .expect("a return was raised");
        raised.push(ret);
    }

    assert_eq!(
        raised[0].status,
        ReturnStatus::Resolved,
        "AED 50.00 is within the limit"
    );
    assert_eq!(raised[0].resolution, Some(ReturnResolution::Refund));
    let waiting = ApprovalRequest::pending_about(
        w.db(),
        &Subject::new(SALES_RETURN_SUBJECT_TYPE, raised[1].id),
    )
    .await
    .unwrap();
    assert_eq!(waiting.len(), 1, "AED 51.00 needs a manager");
    assert_eq!(raised[1].status, ReturnStatus::Requested);
    assert!(
        approvals::approve(w.state(), waiting[0].id, &clerk, "")
            .await
            .is_err(),
        "the clerk cannot approve their own refund"
    );
    approvals::approve(w.state(), waiting[0].id, &manager, "")
        .await
        .expect("the manager approves");
    let done = SalesReturn::find_or_404(w.db(), raised[1].id)
        .await
        .unwrap();
    assert_eq!(done.status, ReturnStatus::Resolved);
    assert_eq!(done.gross_value, 5_100);
}

/// AC (Sales S3): after the kitchen started an order, the cancel form offers
/// *return to stock* and *scrap* only: *finish to stock* is refused, and
/// *return to stock* waits for a manager.
#[renox::test]
async fn cancelling_after_the_kitchen_started_offers_only_return_to_stock_or_scrap() {
    let (w, _) = deployed().await;
    let c = w.customer("Mariam Al Hosani").await;
    let placed = w
        .place(c, "online", &[("shawarma-plate", "1", "40.00")])
        .await;
    let accepted = w.accept(&placed).await;
    for ticket in fnb::KitchenTicket::of_order(w.db(), accepted.id)
        .await
        .unwrap()
    {
        w.post(
            &w.cook,
            &format!("/tickets/{}/start", ticket.id),
            &[("x", "")],
        )
        .await
        .assert_status(303);
    }
    let ticket = fnb::KitchenTicket::of_order(w.db(), accepted.id)
        .await
        .unwrap()
        .remove(0);
    let path = format!("/orders/{}/cancel", accepted.id);
    let outcome = format!("outcome_{}", ticket.sale_line_id);
    let form = |value: &'static str| {
        vec![
            ("reason".to_owned(), "kitchen_issue".to_owned()),
            ("party".to_owned(), "business".to_owned()),
            ("note".to_owned(), String::new()),
            (outcome.clone(), value.to_owned()),
        ]
    };
    let post = |fields: Vec<(String, String)>| {
        let w = &w;
        let path = path.clone();
        async move {
            let borrowed: Vec<(&str, &str)> = fields
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            w.post(&w.cashier, &path, &borrowed).await
        }
    };

    post(form("finish_to_stock")).await.assert_status(422);
    assert!(
        ApprovalRequest::pending_about(
            w.db(),
            &Subject::new(
                wl_kernel::sales::SALES_ORDER_SUBJECT_TYPE,
                accepted.sales_order_id
            )
        )
        .await
        .unwrap()
        .is_empty()
    );
    post(form("return_to_stock")).await.assert_status(303);
    let waiting = ApprovalRequest::pending_about(
        w.db(),
        &Subject::new(
            wl_kernel::sales::SALES_ORDER_SUBJECT_TYPE,
            accepted.sales_order_id,
        ),
    )
    .await
    .unwrap();
    assert_eq!(waiting.len(), 1, "the outcome waits for a manager");
    assert_eq!(w.reload(&accepted).await.status, FnbOrderStatus::Preparing);
}

/// AC (Payments P1): an online order not paid within 15 minutes is cancelled
/// as *payment failed* by the scheduled job; at 14 minutes it still waits.
#[renox::test]
async fn an_online_order_unpaid_after_fifteen_minutes_is_cancelled_by_the_job() {
    let (w, _) = deployed().await;
    let c = w.customer("Mariam Al Hosani").await;
    let party = c.0.to_string();
    w.post(
        &w.cashier,
        "/sales-orders",
        &[
            ("party_id", party.as_str()),
            ("channel", "website"),
            ("payment_mode", "online"),
        ],
    )
    .await
    .assert_status(303);
    let sales = SalesOrder::query()
        .order_by_desc(SalesOrder::ID)
        .first(w.db())
        .await
        .unwrap()
        .expect("the sales order was entered");
    w.post(
        &w.cashier,
        &format!("/sales-orders/{}/lines", sales.id),
        &[
            ("item_code", "shawarma-plate"),
            ("quantity", "1"),
            ("unit_price", "40.00"),
            ("vat", "zero_rated"),
        ],
    )
    .await
    .assert_status(303);
    w.post(
        &w.cashier,
        &format!("/sales-orders/{}/submit", sales.id),
        &[("x", "")],
    )
    .await
    .assert_status(303);
    let status = |w: &World| {
        let db = w.db().clone();
        async move { SalesOrder::find_or_404(&db, sales.id).await.unwrap().status }
    };
    assert_eq!(status(&w).await, SalesOrderStatus::AwaitingPayment);

    w.travel(TimeDelta::minutes(14));
    w.app
        .at_travelled_time(w.app.run_scheduled(wl_sales::online_payment::TASK))
        .await
        .unwrap();
    assert_eq!(status(&w).await, SalesOrderStatus::AwaitingPayment);

    w.travel(TimeDelta::minutes(2));
    w.app
        .at_travelled_time(w.app.run_scheduled(wl_sales::online_payment::TASK))
        .await
        .unwrap();
    assert_eq!(status(&w).await, SalesOrderStatus::Cancelled);
}

/// AC (Payments P1, `FIN.RUL.19`): an online payment still pending is not
/// asked of the gateway at 9 minutes; at 11 minutes it is, and a payment the
/// gateway has no final answer for fails with the reason *timeout*.
#[renox::test]
async fn a_pending_online_payment_is_asked_of_the_gateway_after_ten_minutes() {
    use wl_kernel::payments::{PaymentFailureKind, PaymentStatus};
    let (w, _) = deployed().await;
    let c = w.customer("Mariam Al Hosani").await;
    let entity = wl_kernel::LegalEntity::default_entity(w.db())
        .await
        .unwrap();
    let link: PaymentLink =
        payment_link::create(w.state(), NewLink::new(c, entity.id, 4_000, "AED"))
            .await
            .expect("the link is made");
    w.app.logout();
    w.app
        .htmx()
        .post(
            &format!("{}/start", payment_link::path(w.state(), link.id)),
            &[],
        )
        .await;
    let payment = wl_payments::Payment::query()
        .where_(wl_payments::Payment::PAYMENT_LINK_ID.eq(link.id))
        .first(w.db())
        .await
        .unwrap()
        .expect("a payment on the link");
    assert_eq!(payment.status, PaymentStatus::Pending);
    let reload = |w: &World| {
        let db = w.db().clone();
        let id = payment.id;
        async move { wl_payments::Payment::find_or_404(&db, id).await.unwrap() }
    };

    w.travel(TimeDelta::minutes(9));
    w.app
        .at_travelled_time(w.app.run_scheduled(wl_payments::online::TASK))
        .await
        .unwrap();
    assert_eq!(reload(&w).await.status, PaymentStatus::Pending);

    w.travel(TimeDelta::minutes(2));
    w.app
        .at_travelled_time(w.app.run_scheduled(wl_payments::online::TASK))
        .await
        .unwrap();
    let failed = reload(&w).await;
    assert_eq!(failed.status, PaymentStatus::Failed);
    assert_eq!(failed.failure_kind, Some(PaymentFailureKind::Timeout));
}

// ------------------------------------------------------------------ stock

/// An issued purchase order of 100 kg of chicken from a dairy supplier for
/// the outlet's store (a fixture: the order is not what is tested).
async fn issued_order(w: &World) -> (wl_inventory::PurchaseOrder, wl_inventory::PurchaseOrderLine) {
    let supplier = w.customer("Al Ain Dairy").await;
    let mut order = wl_inventory::PurchaseOrder::new(supplier, w.store.id);
    order.status = wl_inventory::PurchaseOrderStatus::Issued;
    let mut tx = w.db().begin().await.unwrap();
    order
        .insert_in_state(
            &mut tx,
            wl_inventory::PurchaseOrderStatus::Issued,
            "fixture",
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let mut line = wl_inventory::PurchaseOrderLine::new(order.id, 1, w.chicken.id, 100_000, 1_200);
    line.save(w.db()).await.unwrap();
    (order, line)
}

/// A draft receipt of the order with one line of 100 kg received, 80 kg
/// accepted and the rest rejected for `temperature`, with `evidence`.
async fn temperature_receipt(
    w: &World,
    order: &wl_inventory::PurchaseOrder,
    line: &wl_inventory::PurchaseOrderLine,
    evidence: Option<&str>,
) -> wl_inventory::GoodsReceipt {
    let mut draft = wl_inventory::DraftLine::on(line.id, 100_000, 80_000).rejected_for(
        "temperature",
        wl_inventory::supplier_return::RejectedHandling::RefusedAtDoor,
    );
    if let Some(evidence) = evidence {
        draft = draft.evidence(evidence);
    }
    wl_inventory::goods_receipt::create_draft(
        w.db(),
        None,
        wl_inventory::DraftReceipt {
            purchase_order_id: order.id,
            location_id: w.store.id,
            delivery_note: Some("DN-1".to_owned()),
            lines: vec![draft],
        },
    )
    .await
    .expect("the draft receipt")
}

/// AC (Stock S2): a goods receipt line rejected for `temperature` without the
/// reading is refused when it is posted; with the reading it posts and the
/// rejected quantity goes back to the supplier.
#[renox::test]
async fn a_temperature_rejection_without_the_reading_is_refused_at_posting() {
    let (w, _) = deployed().await;
    let keeper = make_user(
        w.db(),
        "Amal",
        &[
            wl_inventory::permissions::GOODS_RECEIPTS_POST,
            wl_inventory::permissions::STOCK_VIEW,
        ],
    )
    .await;
    let (order, line) = issued_order(&w).await;

    let bare = temperature_receipt(&w, &order, &line, None).await;
    let refused = w
        .post(&keeper, &format!("/goods-receipts/{}/post", bare.id), &[])
        .await;
    refused.assert_status(422);
    assert_eq!(
        wl_inventory::GoodsReceipt::find_or_404(w.db(), bare.id)
            .await
            .unwrap()
            .status,
        wl_inventory::GoodsReceiptStatus::Draft,
        "nothing was posted"
    );

    let read = temperature_receipt(&w, &order, &line, Some("9 C")).await;
    w.post(&keeper, &format!("/goods-receipts/{}/post", read.id), &[])
        .await
        .assert_ok();
    assert_eq!(
        wl_inventory::GoodsReceipt::find_or_404(w.db(), read.id)
            .await
            .unwrap()
            .status,
        wl_inventory::GoodsReceiptStatus::Posted
    );
}

/// AC (Stock S3): the release of a line of a recall hold needs a second
/// holder of `lots.release`: the head chef's decision waits, the kitchen
/// manager approves it; the head chef releases the line of a temperature hold
/// alone.
#[renox::test]
async fn a_recall_hold_needs_a_second_holder_and_a_temperature_hold_does_not() {
    let (w, _) = deployed().await;
    let k = stock::kitchen(&w).await;
    let t7 = stock::lot(&w, &k.tomato, "T7", &k.central, 40_000, 20).await;
    let t8 = stock::lot(&w, &k.tomato, "T8", &k.central, 40_000, 20).await;
    let recall = stock::hold(&w, &t7, "recall").await;
    let temperature = stock::hold(&w, &t8, "temperature").await;
    let line_of = |hold: &wl_inventory::StockHold| {
        let db = w.db().clone();
        let id = hold.id;
        async move {
            wl_inventory::stock_hold::lines_of(&db, id)
                .await
                .unwrap()
                .remove(0)
        }
    };

    let alone = line_of(&temperature).await;
    w.post(
        &k.chef,
        &format!("/stock-hold-lines/{}/release", alone.id),
        &[],
    )
    .await
    .assert_ok();
    assert_eq!(
        wl_inventory::StockHoldLine::find_or_404(w.db(), alone.id)
            .await
            .unwrap()
            .status,
        wl_inventory::HoldLineStatus::Released,
        "a temperature hold is released by the chef alone"
    );

    let line = line_of(&recall).await;
    w.post(
        &k.chef,
        &format!("/stock-hold-lines/{}/release", line.id),
        &[],
    )
    .await
    .assert_ok();
    let waiting = wl_inventory::StockHoldLine::find_or_404(w.db(), line.id)
        .await
        .unwrap();
    assert_eq!(waiting.status, wl_inventory::HoldLineStatus::Undecided);
    let request = ApprovalRequest::pending_about(w.db(), &waiting.subject())
        .await
        .unwrap()
        .pop()
        .expect("the release of a recall hold waits for a second holder");
    assert!(
        approvals::approve(w.state(), request.id, &k.chef, "")
            .await
            .is_err(),
        "the chef cannot approve their own release"
    );
    approvals::approve(w.state(), request.id, &k.manager, "")
        .await
        .expect("the kitchen manager approves");
    assert_eq!(
        wl_inventory::StockHoldLine::find_or_404(w.db(), line.id)
            .await
            .unwrap()
            .status,
        wl_inventory::HoldLineStatus::Released
    );
}

/// AC (Suppliers R2, `STK.RUL.09`): a supplier whose monthly score is 69 gets
/// a review task and procurement is told; one whose score is 70 gets none.
#[renox::test]
async fn a_supplier_score_of_69_opens_a_review_and_70_does_not() {
    let (w, _) = deployed().await;
    let settings = w
        .state()
        .provided::<wl_inventory::InventoryConfig>()
        .expect("the stock settings")
        .score
        .clone();
    assert_eq!(settings.threshold, 70);
    let mut snapshots = Vec::new();
    for (name, score) in [("Weak Supplier", 69), ("Fair Supplier", 70)] {
        let supplier = w.customer(name).await;
        let date = renox::chrono::NaiveDate::from_ymd_opt(2026, 12, 1).unwrap();
        let mut row = wl_inventory::SupplierScore {
            supplier_party_id: supplier.0,
            snapshot_date: date,
            window_from: date
                .checked_sub_months(renox::chrono::Months::new(1))
                .unwrap(),
            window_to: date.pred_opt().unwrap(),
            receipts: 5,
            on_time: 3,
            on_time_bp: Some(6_000),
            rejected_bp: Some(300),
            score: Some(score),
            band: wl_inventory::supplier_score::band_of(Some(score), &settings),
            computed_at: Some(renox::db::now()),
            ..wl_inventory::SupplierScore::default()
        };
        row.save(w.db()).await.unwrap();
        snapshots.push((supplier, row));
    }
    let rows: Vec<_> = snapshots.iter().map(|(_, row)| row.clone()).collect();

    wl_inventory::supplier_review::on_snapshots(w.state(), &rows)
        .await
        .unwrap();

    let reviews = wl_inventory::SupplierReview::query()
        .get(w.db())
        .await
        .unwrap();
    assert_eq!(reviews.len(), 1, "only the supplier at 69 is reviewed");
    assert_eq!(reviews[0].supplier_party_id, snapshots[0].0.0);
    assert_eq!(reviews[0].status, wl_inventory::ReviewStatus::Open);
    let tasks = w.tickets.opened();
    assert_eq!(tasks.len(), 1);
    assert!(
        tasks[0].description.contains("Score 69"),
        "{}",
        tasks[0].description
    );
}
