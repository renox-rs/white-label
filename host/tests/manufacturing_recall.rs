//! BL-MFG-10 (#391) with the real stock, recall and tickets of the host: a
//! raw lot issued to work orders is traced through the lots they made to the
//! dispatches and the customers; the recall owner reads the customers, a
//! dispatch that has not left is flagged for a re-pick with a task, the
//! screens answer in words and a person without the right is refused.

mod manufacturing_support;
mod support;

use host::roles::{PRODUCTION_MANAGER, QC_INSPECTOR, STOREKEEPER};
use manufacturing::recalls::RepickFlag;
use manufacturing::{WorkOrder, WorkOrderSource};
use manufacturing_support::{boot_tuned, boot_with_orders, browse, floor, person, post};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use support::BouncingMail;
use wl_inventory::movement::{MovementType, StockMovement};
use wl_inventory::{Lot, LotStatus, NewDispatch, NewDispatchLine, create_dispatch};
use wl_kernel::{PartyId, Subject};

const C1: i64 = 4101;
const C2: i64 = 4102;
const C3: i64 = 4103;

/// A lot of `item` in `status` (a fixture).
async fn lot(db: &Db, item: i64, number: &str, status: LotStatus) -> Lot {
    let mut lot = Lot {
        item_id: item,
        number: number.into(),
        status,
        ..Lot::default()
    };
    let mut tx = db.begin().await.unwrap();
    lot.insert_in_state(&mut tx, status, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    lot
}

/// Done when 1 to 3 with the real recall: raw lot R1 issued to WO-HOST-1
/// (stock) that made lot F1, F1 sent on a dispatch to C1 and picked on another
/// for C2, and WO-701 for the order SO-81 of C3 issued from R1 too. The trace
/// lists C1 shipped, C2 not shipped and C3 not shipped; the picked dispatch
/// is flagged with a task; the work order for stock has no line.
#[renox::test]
async fn a_recalled_raw_lot_is_traced_to_the_customers_through_the_work_orders() {
    let (app, orders) = boot_with_orders().await;
    let f = floor(&app).await;
    let keeper = person(&app, "Karim", STOREKEEPER, None).await;
    let manager = person(&app, "Mona", PRODUCTION_MANAGER, None).await;
    let r1 = lot(app.db(), f.resin.id, "R1", LotStatus::Released).await;
    let f1 = lot(app.db(), f.panel.id, "F1", LotStatus::Released).await;
    StockMovement::receive(
        f.resin.id,
        Some(r1.id),
        f.store.id,
        200_000,
        800,
        160_000,
        &Subject::new("inventory.goods_receipt", 1),
        None,
    )
    .save(app.db())
    .await
    .unwrap();

    // WO-HOST-1 (for stock) is issued 40 kg of R1 through the page and made
    // 100 panels of F1.
    let path = format!("/manufacturing/work-orders/{}", f.order.id);
    let issued = post(
        &app,
        &keeper,
        &format!("{path}/issue"),
        &[
            ("item_id", &f.resin.id.to_string()),
            ("quantity", "40"),
            ("lot", "R1"),
        ],
    )
    .await;
    if issued.status.as_u16() != 303 {
        let mut plain = String::new();
        let mut inside = false;
        for c in issued.text().chars() {
            match c {
                '<' => inside = true,
                '>' => inside = false,
                _ if !inside => plain.push(c),
                _ => {}
            }
        }
        let plain: Vec<&str> = plain
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        panic!("the issue was refused: {plain:?}");
    }
    StockMovement::moving(
        MovementType::Produce,
        f.panel.id,
        Some(f1.id),
        None,
        Some(f.finished.id),
        100_000,
        0,
        0,
        &f.order.subject(),
        None,
    )
    .save(app.db())
    .await
    .unwrap();

    // WO-701 for line 1 of SO-81 (customer C3) is issued 10 kg of R1 too.
    orders.open(81, 1, 50_000, C3);
    let mut served = WorkOrder {
        number: "WO-701".into(),
        plant_id: f.plant.id,
        line_id: Some(f.line.id),
        item_id: f.panel.id,
        quantity_milli: 50_000,
        status: manufacturing::WorkOrderStatus::InProgress,
        source: WorkOrderSource::SalesOrder,
        sales_order_id: Some(81),
        sales_order_line_id: Some(1),
        ..WorkOrder::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    served
        .insert_in_state(
            &mut tx,
            manufacturing::WorkOrderStatus::InProgress,
            "fixture",
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let served_subject = served.subject();
    let mut consumed = StockMovement::moving(
        MovementType::Consume,
        f.resin.id,
        Some(r1.id),
        Some(f.store.id),
        None,
        10_000,
        0,
        0,
        &served_subject,
        None,
    );
    consumed.cost_object_type = Some(served_subject.subject_type.clone());
    consumed.cost_object_id = Some(served_subject.subject_id);
    consumed.save(app.db()).await.unwrap();

    // F1 goes to C1 on DN-1 (sent) and is picked for C2 on DN-2.
    let dispatch = |order: i64, party: i64, quantity: i64| NewDispatch {
        source: Subject::new("sales.sales_order", order),
        recipient: Some(PartyId(party)),
        recipient_name: "Customer".into(),
        address: "Dubai".into(),
        location_id: f.finished.id,
        lines: vec![NewDispatchLine::of(f.panel.id, quantity).lot(f1.id)],
    };
    let first = create_dispatch(app.db(), None, dispatch(80, C1, 30_000))
        .await
        .unwrap();
    post(
        &app,
        &keeper,
        &format!("/dispatches/{}/pick", first.id),
        &[],
    )
    .await
    .assert_ok();
    post(
        &app,
        &keeper,
        &format!("/dispatches/{}/dispatch", first.id),
        &[("handover", "courier"), ("tracking_number", "TRK-1")],
    )
    .await
    .assert_ok();
    let second = create_dispatch(app.db(), None, dispatch(82, C2, 20_000))
        .await
        .unwrap();
    post(
        &app,
        &keeper,
        &format!("/dispatches/{}/pick", second.id),
        &[],
    )
    .await
    .assert_ok();

    // The production manager opens a recall on R1.
    post(
        &app,
        &manager,
        "/recalls",
        &[
            ("source", "supplier"),
            ("reason", "Contaminated resin"),
            ("items", &f.resin.id.to_string()),
            ("lot_numbers", "R1"),
        ],
    )
    .await
    .assert_ok();
    let recall = wl_inventory::Recall::query()
        .first(app.db())
        .await
        .unwrap()
        .expect("a recall");

    // The customers, as the recall owner reads them.
    let page = browse(
        &app,
        &manager,
        &format!("/manufacturing/recalls/{}", recall.id),
    )
    .await;
    page.assert_ok();
    let html = page.text();
    for word in [
        "Shipped",
        "Not shipped",
        "To bring back",
        "To re-pick",
        "R1",
        "F1",
    ] {
        assert!(html.contains(word), "the page says `{word}`: {html}");
    }
    let rows = manufacturing::recalls::trace(app.state(), recall.id)
        .await
        .unwrap();
    let customer = |id: i64| {
        rows.iter()
            .filter(move |row| row.customer == Some(id))
            .collect::<Vec<_>>()
    };
    assert_eq!(customer(C1).len(), 1);
    assert!(customer(C1)[0].shipped && customer(C1)[0].bring_back);
    assert_eq!(customer(C2).len(), 1);
    assert!(!customer(C2)[0].shipped && customer(C2)[0].repick);
    assert_eq!(customer(C3).len(), 1);
    assert_eq!(customer(C3)[0].label, "WO-701");
    assert!(!customer(C3)[0].shipped);
    assert!(
        rows.iter().all(|row| row.label != f.order.number),
        "the work order for stock has no line"
    );

    // The picked dispatch is flagged once, with a task for the storekeeper.
    let flags = RepickFlag::query().get(app.db()).await.unwrap();
    assert_eq!(flags.len(), 1);
    assert_eq!(flags[0].dispatch_id, second.id);
    assert!(flags[0].ticket_id.is_some(), "the storekeeper has the task");

    // The list and the JSON.
    let list = browse(&app, &manager, "/manufacturing/recalls").await;
    list.assert_ok();
    assert!(
        list.text()
            .contains(recall.number.as_deref().unwrap_or("RCL"))
    );
    assert!(!html.contains("None"), "{html}");

    // Somebody without the right is refused.
    let inspector = person(&app, "Ines", QC_INSPECTOR, Some(f.plant.scope())).await;
    browse(
        &app,
        &inspector,
        &format!("/manufacturing/recalls/{}", recall.id),
    )
    .await
    .assert_status(403);
}

/// Customer C4 with a main contact e-mail and no portal buyer, a lot F4 sent
/// to it on a dispatch, and a recall on F4 opened by the production manager:
/// the manager and the recall line of C4.
async fn recalled_for_c4(
    app: &TestApp,
) -> (
    wl_kernel::PartyId,
    User,
    wl_inventory::Recall,
    wl_inventory::RecallLine,
) {
    let f = floor(app).await;
    let keeper = person(app, "Karim", STOREKEEPER, None).await;
    let manager = person(app, "Mona", PRODUCTION_MANAGER, None).await;
    let c4 = wl_parties::create(
        app.state(),
        wl_parties::NewParty {
            kind: wl_kernel::PartyKind::Company,
            legal_name_en: "Bin Rashid Contracting LLC".into(),
            company: Some(wl_parties::CompanyDetails::default()),
            contacts: vec![wl_parties::ContactPoint {
                contact_type: wl_parties::ContactType::Email,
                value: "accounts@binrashid.example".into(),
                is_primary: true,
                ..Default::default()
            }],
            source: "back_office".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .party_id();
    let f1 = lot(app.db(), f.panel.id, "F4", LotStatus::Released).await;
    StockMovement::receive(
        f.panel.id,
        Some(f1.id),
        f.finished.id,
        50_000,
        800,
        40_000,
        &Subject::new("inventory.goods_receipt", 2),
        None,
    )
    .save(app.db())
    .await
    .unwrap();
    let sent = create_dispatch(
        app.db(),
        None,
        NewDispatch {
            source: Subject::new("sales.sales_order", 90),
            recipient: Some(c4),
            recipient_name: "Bin Rashid".into(),
            address: "Dubai".into(),
            location_id: f.finished.id,
            lines: vec![NewDispatchLine::of(f.panel.id, 30_000).lot(f1.id)],
        },
    )
    .await
    .unwrap();
    post(app, &keeper, &format!("/dispatches/{}/pick", sent.id), &[])
        .await
        .assert_ok();
    post(
        app,
        &keeper,
        &format!("/dispatches/{}/dispatch", sent.id),
        &[("handover", "courier"), ("tracking_number", "TRK-4")],
    )
    .await
    .assert_ok();
    post(
        app,
        &manager,
        "/recalls",
        &[
            ("source", "internal"),
            ("reason", "Wrong resin grade"),
            ("items", &f.panel.id.to_string()),
            ("lot_numbers", "F4"),
        ],
    )
    .await
    .assert_ok();
    let recall = wl_inventory::Recall::query()
        .first(app.db())
        .await
        .unwrap()
        .expect("a recall");
    let line = wl_inventory::RecallLine::query()
        .where_(wl_inventory::RecallLine::RECALL_ID.eq(recall.id))
        .where_(wl_inventory::RecallLine::PARTY_ID.eq(Some(c4.0)))
        .first(app.db())
        .await
        .unwrap()
        .expect("the line of C4");

    (c4, manager, recall, line)
}

/// BL-MFG-10 AC4 (#391): customer C4 has no buyer with a portal login; the
/// recall notice sent from the recall page goes to its main contact: a
/// message under the 07 type `mfg.recall_notice` is queued for the party's
/// primary e-mail address, and none for a buyer.
#[renox::test]
async fn a_customer_without_a_portal_buyer_is_told_at_its_main_contact_email() {
    let (app, _orders) = boot_with_orders().await;
    let (c4, manager, recall, line) = recalled_for_c4(&app).await;
    post(
        &app,
        &manager,
        &format!("/recalls/{}/notices", recall.id),
        &[("template", "customer"), ("lines", &line.id.to_string())],
    )
    .await
    .assert_ok();

    let messages = wl_notify::Message::query()
        .where_(wl_notify::Message::TYPE_KEY.eq("mfg.recall_notice"))
        .get(app.db())
        .await
        .unwrap();
    assert_eq!(messages.len(), 1, "one notice for the one customer");
    assert_eq!(
        messages[0].address.as_deref(),
        Some("accounts@binrashid.example"),
        "to the main contact e-mail"
    );
    assert_eq!(messages[0].party_id, Some(c4.0));
    assert!(messages[0].user_id.is_none(), "there is no buyer to tell");
}

/// BL-MFG-14 *Notifications* (#395): the 07 recall notice (`mfg.recall_notice`,
/// urgent, no group) that bounces for good gives up: the line is `pending`
/// again and the recall owner has the task to tell the customer another way,
/// the company admin has the staff task and the in-app alert, which links to
/// that task.
#[renox::test]
async fn a_recall_notice_that_gives_up_gives_the_owner_a_task_and_the_admin_an_alert() {
    let mail = BouncingMail::default();
    let (app, _orders) = boot_tuned(host::Tuning::default().notify({
        let mail = mail.clone();
        move |n| n.provider(mail)
    }))
    .await;
    let admin = person(&app, "Ada", host::roles::ADMIN, None).await;
    let (_c4, manager, recall, line) = recalled_for_c4(&app).await;

    post(
        &app,
        &manager,
        &format!("/recalls/{}/notices", recall.id),
        &[("template", "customer"), ("lines", &line.id.to_string())],
    )
    .await
    .assert_ok();
    app.run_all_jobs().await;

    assert!(!mail.attempts().is_empty(), "the notice was e-mailed");
    let again = wl_inventory::RecallLine::find_or_404(app.db(), line.id)
        .await
        .unwrap();
    assert_eq!(
        again.notice_status,
        wl_inventory::NoticeStatus::Pending,
        "the customer was not told"
    );
    let tasks = wl_kernel::tickets::task_creator(app.state());
    assert!(
        tasks
            .find_open(app.state(), &wl_inventory::recall_notice::task_key(line.id))
            .await
            .unwrap()
            .is_some(),
        "the recall owner has the task"
    );
    let alerts: Vec<_> = admin
        .notifications(app.db(), 50)
        .await
        .unwrap()
        .into_iter()
        .filter(|n| n.kind == "notify-give-up-alert")
        .collect();
    assert_eq!(
        alerts.len(),
        1,
        "an urgent notice alerts the admin in the app"
    );
    assert!(alerts[0].data.to_string().contains("/tickets/"));
}
