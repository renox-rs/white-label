//! BL-MFG-11 (#392) with the real wl-inventory dispatch, wl-documents and
//! tickets: a regulated item leaves the plant only while one of its accepted
//! certificates is valid (the day of the expiry still counts, the day after
//! does not, a renewal in review does not, a renewal accepted does), no
//! override is offered, an export only item is not delivered in the UAE, the
//! nightly look-ahead lists the orders promised after the expiry once, and an
//! expired industrial licence blocks nothing. Points 6 and 7 also run through
//! the real wl-sales (`submit` and `amend` of the order pages, the order's
//! delivery destination, the promised day of a line) on the manufacturing
//! deployment.

mod manufacturing_support;
mod support;

use host::roles::{PRODUCTION_MANAGER, PRODUCTION_PLANNER, STOREKEEPER};
use manufacturing::ItemFlag;
use manufacturing::conformity::{ECAS, EQM, FOOD_CONTACT};
use manufacturing_support::{
    boot_with_orders, browse, dispatch_of, document, floor, messages_to, person, post, stocked,
    words,
};
use renox::chrono::Duration;
use renox::prelude::*;
use renox::testing::TestApp;
use wl_documents::DocumentStatus;
use wl_inventory::DispatchStatus;
use wl_kernel::Subject;
use wl_kernel::documents::today;
use wl_kernel::sales::order_line_rules;
use wl_sales::{SalesOrder, SalesOrderStatus};

fn item_subject(id: i64) -> Subject {
    Subject::new("inventory.item", id)
}

/// A day of travel: the pick of tomorrow.
fn a_day() -> std::time::Duration {
    std::time::Duration::from_secs(24 * 3600)
}

/// Done when 1, 2, 3 and 5: LED fitting X (regulated) has an ECAS certificate
/// valid through today. A dispatch of X picked today passes. The next day a
/// pick is refused naming the certificate and its expiry, with no override;
/// the dispatch picked yesterday is refused at `dispatch`, offers `cancel`,
/// and can be cancelled. A renewal uploaded and still in review is named in
/// the refusal; once it is accepted the pick passes again.
#[renox::test]
async fn a_regulated_item_is_picked_only_while_its_certificate_is_valid() {
    let (app, _orders) = boot_with_orders().await;
    let f = floor(&app).await;
    let keeper = person(&app, "Karim", STOREKEEPER, None).await;
    let manager = person(&app, "Mona", PRODUCTION_MANAGER, None).await;
    let led = stocked(&app, "LED-X", &f.finished, 100).await;
    post(
        &app,
        &manager,
        "/manufacturing/conformity",
        &[("code", "LED-X"), ("regulated", "1")],
    )
    .await
    .assert_status(303);
    let flag = ItemFlag::of(app.db(), led.id).await.unwrap().unwrap();
    assert!(flag.regulated && !flag.export_only);
    document(
        &app,
        &item_subject(led.id),
        ECAS,
        Some(today()),
        DocumentStatus::Accepted,
    )
    .await;
    let first = dispatch_of(&app, &led, &f.finished, 60, 4001, 10).await;
    let second = dispatch_of(&app, &led, &f.finished, 61, 4002, 10).await;

    // On the expiry day the pick succeeds.
    post(
        &app,
        &keeper,
        &format!("/dispatches/{}/pick", first.id),
        &[],
    )
    .await
    .assert_ok();

    // The day after, it is refused with the certificate and its expiry.
    app.travel(a_day());
    let refused = post(
        &app,
        &keeper,
        &format!("/dispatches/{}/pick", second.id),
        &[],
    )
    .await;
    refused.assert_status(422);
    let text = refused.text();
    assert!(text.contains("ECAS certificate"), "{}", words(&text));
    assert!(text.contains(&today().to_string()), "{}", words(&text));
    assert!(
        !text.contains("/documents/overrides/new"),
        "no override is offered: {text}"
    );

    // The dispatch picked yesterday is refused today, and can be cancelled.
    let leaving = post(
        &app,
        &keeper,
        &format!("/dispatches/{}/dispatch", first.id),
        &[("handover", "courier"), ("tracking_number", "TRK-1")],
    )
    .await;
    leaving.assert_status(422);
    assert!(
        leaving.text().contains("ECAS certificate"),
        "{}",
        leaving.text()
    );
    let page = browse(&app, &keeper, &format!("/dispatches/{}", first.id)).await;
    page.assert_ok();
    assert!(
        page.text()
            .contains(&format!("/dispatches/{}/cancel", first.id)),
        "the refused dispatch can be cancelled"
    );
    post(
        &app,
        &keeper,
        &format!("/dispatches/{}/cancel", first.id),
        &[("reason", "certificate expired")],
    )
    .await
    .assert_ok();
    assert_eq!(
        wl_inventory::Dispatch::find_or_404(app.db(), first.id)
            .await
            .unwrap()
            .status,
        DispatchStatus::Cancelled
    );

    // A renewal in review is named, and does not pass.
    let renewal = document(
        &app,
        &item_subject(led.id),
        ECAS,
        Some(today() + Duration::days(365)),
        DocumentStatus::PendingReview,
    )
    .await;
    let waiting = post(
        &app,
        &keeper,
        &format!("/dispatches/{}/pick", second.id),
        &[],
    )
    .await;
    waiting.assert_status(422);
    assert!(
        waiting.text().contains("renewal pending review since"),
        "{}",
        waiting.text()
    );

    // Accepted, it passes.
    wl_documents::Document::query()
        .where_(wl_documents::Document::ID.eq(renewal.id))
        .update(app.db(), &[("status", &DocumentStatus::Accepted)])
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
}

/// Done when 4 and 9: item Z (regulated, accepted kind food-contact only) with
/// a valid ECAS certificate and no food-contact certificate is refused; an
/// item that names no kind passes with only an EQM licence; an expired
/// industrial licence blocks nothing; an item that is not regulated needs no
/// certificate.
#[renox::test]
async fn any_one_accepted_kind_passes_and_other_documents_block_nothing() {
    let (app, _orders) = boot_with_orders().await;
    let f = floor(&app).await;
    let keeper = person(&app, "Karim", STOREKEEPER, None).await;
    let manager = person(&app, "Mona", PRODUCTION_MANAGER, None).await;
    let z = stocked(&app, "ZZ", &f.finished, 100).await;
    let q = stocked(&app, "QQ", &f.finished, 100).await;
    let plain = stocked(&app, "PLAIN", &f.finished, 100).await;
    post(
        &app,
        &manager,
        "/manufacturing/conformity",
        &[("code", "ZZ"), ("regulated", "1"), ("kind", FOOD_CONTACT)],
    )
    .await
    .assert_status(303);
    post(
        &app,
        &manager,
        "/manufacturing/conformity",
        &[("code", "QQ"), ("regulated", "1")],
    )
    .await
    .assert_status(303);
    let valid = Some(today() + Duration::days(200));
    document(
        &app,
        &item_subject(z.id),
        ECAS,
        valid,
        DocumentStatus::Accepted,
    )
    .await;
    document(
        &app,
        &item_subject(q.id),
        EQM,
        valid,
        DocumentStatus::Accepted,
    )
    .await;
    // The company's industrial licence expired a month ago.
    document(
        &app,
        &Subject::new("manufacturing.plant", f.plant.id),
        "industrial_licence",
        Some(today() - Duration::days(30)),
        DocumentStatus::Accepted,
    )
    .await;

    let for_z = dispatch_of(&app, &z, &f.finished, 62, 4003, 5).await;
    let refused = post(
        &app,
        &keeper,
        &format!("/dispatches/{}/pick", for_z.id),
        &[],
    )
    .await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("Food-contact certificate"),
        "{}",
        refused.text()
    );

    for item in [&q, &plain] {
        let dispatch = dispatch_of(&app, item, &f.finished, 63, 4004, 5).await;
        post(
            &app,
            &keeper,
            &format!("/dispatches/{}/pick", dispatch.id),
            &[],
        )
        .await
        .assert_ok();
    }
}

/// Done when 6: cup lids Y (export only) are refused for a domestic delivery
/// at the pick and accepted for an export order; the rule sales asks of an
/// order line says the same (domestic refused, export accepted); a person
/// without `conformity.manage` cannot flag.
#[renox::test]
async fn an_export_only_item_is_not_delivered_in_the_uae() {
    let (app, orders) = boot_with_orders().await;
    let f = floor(&app).await;
    let keeper = person(&app, "Karim", STOREKEEPER, None).await;
    let manager = person(&app, "Mona", PRODUCTION_MANAGER, None).await;
    let lids = stocked(&app, "LIDS", &f.finished, 100).await;
    post(
        &app,
        &keeper,
        "/manufacturing/conformity",
        &[("code", "LIDS"), ("export_only", "1")],
    )
    .await
    .assert_status(403);
    post(
        &app,
        &manager,
        "/manufacturing/conformity",
        &[("code", "LIDS"), ("export_only", "1")],
    )
    .await
    .assert_status(303);
    orders.export(70, false);
    orders.export(71, true);

    let home = dispatch_of(&app, &lids, &f.finished, 70, 4005, 5).await;
    let refused = post(&app, &keeper, &format!("/dispatches/{}/pick", home.id), &[]).await;
    refused.assert_status(422);
    assert!(refused.text().contains("export only"), "{}", refused.text());
    let abroad = dispatch_of(&app, &lids, &f.finished, 71, 4006, 5).await;
    post(
        &app,
        &keeper,
        &format!("/dispatches/{}/pick", abroad.id),
        &[],
    )
    .await
    .assert_ok();

    let rules = order_line_rules(app.state());
    assert!(
        rules
            .refuse_line(app.state(), &item_subject(lids.id), true)
            .await
            .unwrap()
            .is_some_and(|words| words.contains("export only"))
    );
    assert!(
        rules
            .refuse_line(app.state(), &item_subject(lids.id), false)
            .await
            .unwrap()
            .is_none()
    );
}

/// The deployment as the demo has it: wl-sales, wl-inventory and the
/// vertical, with the real order pages and no fake of the order book.
async fn deployment() -> TestApp {
    let app = support::manufacturing_booted().await;
    host::roles::define(app.db()).await.unwrap();
    app
}

/// A draft order of `customer` through the pages of wl-sales: `lines` are
/// (item code, promised day or ""), `delivery` the delivery fields.
async fn draft_order(
    app: &TestApp,
    sales: &User,
    customer: wl_kernel::PartyId,
    lines: &[(&str, &str)],
    delivery: &[(&str, &str)],
) -> SalesOrder {
    app.acting_as(sales);
    let customer = customer.0.to_string();
    let mut form = vec![
        ("party_id", customer.as_str()),
        ("channel", "phone"),
        ("payment_mode", "on_account"),
    ];
    form.extend_from_slice(delivery);
    app.htmx()
        .post("/sales-orders", &form)
        .await
        .assert_status(303);
    let order = SalesOrder::query()
        .order_by_desc(SalesOrder::ID)
        .first(app.db())
        .await
        .unwrap()
        .expect("the order");
    for (code, promised) in lines {
        app.htmx()
            .post(
                &format!("/sales-orders/{}/lines", order.id),
                &[
                    ("item_code", code),
                    ("quantity", "10"),
                    ("unit_price", "10.00"),
                    ("promised", promised),
                ],
            )
            .await
            .assert_status(303);
    }
    SalesOrder::find_or_404(app.db(), order.id).await.unwrap()
}

async fn submit_order(
    app: &TestApp,
    sales: &User,
    order: &SalesOrder,
) -> renox::testing::TestResponse {
    app.acting_as(sales);
    app.htmx()
        .post(&format!("/sales-orders/{}/submit", order.id), &[("x", "")])
        .await
}

async fn amend_order(
    app: &TestApp,
    sales: &User,
    order: &SalesOrder,
    fields: &[(&str, &str)],
) -> renox::testing::TestResponse {
    app.acting_as(sales);
    let order = SalesOrder::find_or_404(app.db(), order.id).await.unwrap();
    let revision = order.revision.to_string();
    let mut form = vec![
        ("revision", revision.as_str()),
        ("reason", "The buyer asked"),
    ];
    form.extend_from_slice(fields);
    app.htmx()
        .post(&format!("/sales-orders/{}/amend", order.id), &form)
        .await
}

async fn status_of(app: &TestApp, order: &SalesOrder) -> SalesOrderStatus {
    SalesOrder::find_or_404(app.db(), order.id)
        .await
        .unwrap()
        .status
}

/// Done when 6 through the real wl-sales: cup lids Y (export only) are
/// refused on a domestic line at the order page's `submit` and at `amend`
/// with a 422 in words, whether the delivery is in the UAE or not stated;
/// they are placed on an order delivered abroad, which cannot be moved to the
/// UAE while it holds them; another item passes everywhere; and the dispatch
/// guard reads the same destination from wl-sales (a pick of lids for the
/// order in the UAE is refused, for the export order it passes).
#[renox::test]
async fn an_export_only_item_is_refused_on_a_domestic_order_at_submit_and_amend() {
    let app = deployment().await;
    let f = floor(&app).await;
    let manager = person(&app, "Mona", PRODUCTION_MANAGER, None).await;
    let keeper = person(&app, "Karim", STOREKEEPER, None).await;
    let sales = person(&app, "Sara", host::roles::SALES, None).await;
    let customer = support::company(&app, "Al Noor Trading LLC").await;
    let lids = stocked(&app, "LIDS", &f.finished, 100).await;
    stocked(&app, "CUPS", &f.finished, 100).await;
    post(
        &app,
        &manager,
        "/manufacturing/conformity",
        &[("code", "LIDS"), ("export_only", "1")],
    )
    .await
    .assert_status(303);

    // Submit: the UAE and an unstated destination are refused, in words.
    let home = draft_order(
        &app,
        &sales,
        customer,
        &[("CUPS", ""), ("LIDS", "")],
        &[("delivery_country", "AE"), ("delivery_emirate", "Dubai")],
    )
    .await;
    let refused = submit_order(&app, &sales, &home).await;
    refused.assert_status(422);
    assert!(refused.text().contains("export only"), "{}", refused.text());
    assert!(refused.text().contains("LIDS"), "{}", refused.text());
    assert_eq!(status_of(&app, &home).await, SalesOrderStatus::Draft);
    let unsaid = draft_order(&app, &sales, customer, &[("LIDS", "")], &[]).await;
    submit_order(&app, &sales, &unsaid).await.assert_status(422);
    assert_eq!(status_of(&app, &unsaid).await, SalesOrderStatus::Draft);

    // Abroad the same lids are placed; the order of cups in the UAE too.
    let abroad = draft_order(
        &app,
        &sales,
        customer,
        &[("LIDS", "")],
        &[
            ("delivery_country", "OM"),
            ("delivery_address", "Sohar port"),
        ],
    )
    .await;
    submit_order(&app, &sales, &abroad).await.assert_status(303);
    assert_eq!(status_of(&app, &abroad).await, SalesOrderStatus::Confirmed);
    let cups = draft_order(
        &app,
        &sales,
        customer,
        &[("CUPS", "")],
        &[("delivery_country", "AE")],
    )
    .await;
    submit_order(&app, &sales, &cups).await.assert_status(303);
    assert_eq!(status_of(&app, &cups).await, SalesOrderStatus::Confirmed);

    // Amend: lids cannot be added to the order in the UAE, cups can; the
    // export order cannot be moved to the UAE.
    let added = amend_order(
        &app,
        &sales,
        &cups,
        &[
            ("add_item", "LIDS"),
            ("add_quantity", "5"),
            ("add_price", "1.00"),
        ],
    )
    .await;
    added.assert_status(422);
    assert!(added.text().contains("export only"), "{}", added.text());
    amend_order(
        &app,
        &sales,
        &cups,
        &[
            ("add_item", "CUPS"),
            ("add_quantity", "5"),
            ("add_price", "1.00"),
        ],
    )
    .await
    .assert_status(303);
    let moved = amend_order(
        &app,
        &sales,
        &abroad,
        &[
            ("delivery_destination", "domestic"),
            ("delivery_country", "AE"),
        ],
    )
    .await;
    moved.assert_status(422);
    assert!(moved.text().contains("export only"), "{}", moved.text());
    amend_order(
        &app,
        &sales,
        &abroad,
        &[
            ("add_item", "LIDS"),
            ("add_quantity", "5"),
            ("add_price", "1.00"),
        ],
    )
    .await
    .assert_status(303);

    // The dispatch guard reads the destination wl-sales now provides.
    let to_home = dispatch_of(&app, &lids, &f.finished, cups.id, customer.0, 5).await;
    let pick = post(
        &app,
        &keeper,
        &format!("/dispatches/{}/pick", to_home.id),
        &[],
    )
    .await;
    pick.assert_status(422);
    assert!(pick.text().contains("export only"), "{}", pick.text());
    let to_sea = dispatch_of(&app, &lids, &f.finished, abroad.id, customer.0, 5).await;
    post(
        &app,
        &keeper,
        &format!("/dispatches/{}/pick", to_sea.id),
        &[],
    )
    .await
    .assert_ok();
}

/// Done when 7 through the real wl-sales: LED fitting X has orders promised in
/// 40 days (placed) and 10 days (placed), and in 45 days (still a draft), and
/// an ECAS certificate that expires in 29 days. The nightly run of
/// wl-documents, scheduled for 01:00 Asia/Dubai, tells sales, the planner and
/// the production manager, in one message each, about the order promised after
/// the expiry only; the next night nothing new is sent. The orders and their
/// promised days are entered through the order pages, no fake supplies them.
#[renox::test]
async fn the_look_ahead_lists_the_orders_promised_after_the_expiry_once() {
    let app = deployment().await;
    let f = floor(&app).await;
    let manager = person(&app, "Mona", PRODUCTION_MANAGER, None).await;
    let planner = person(&app, "Pia", PRODUCTION_PLANNER, None).await;
    let sales = person(&app, "Sam", host::roles::SALES, None).await;
    let customer = support::company(&app, "Al Noor Trading LLC").await;
    let led = stocked(&app, "LED-Y", &f.finished, 100).await;
    post(
        &app,
        &manager,
        "/manufacturing/conformity",
        &[("code", "LED-Y"), ("regulated", "1")],
    )
    .await
    .assert_status(303);
    document(
        &app,
        &item_subject(led.id),
        ECAS,
        Some(today() + Duration::days(29)),
        DocumentStatus::Accepted,
    )
    .await;
    let day = |n: i64| (today() + Duration::days(n)).format("%Y-%m-%d").to_string();
    let delivery = [("delivery_country", "AE")];
    let late = draft_order(&app, &sales, customer, &[("LED-Y", &day(40))], &delivery).await;
    let early = draft_order(&app, &sales, customer, &[("LED-Y", &day(10))], &delivery).await;
    let draft = draft_order(&app, &sales, customer, &[("LED-Y", &day(45))], &delivery).await;
    submit_order(&app, &sales, &late).await.assert_status(303);
    submit_order(&app, &sales, &early).await.assert_status(303);

    let report = wl_documents::expiry::run(app.state(), today())
        .await
        .unwrap();
    assert_eq!(report.failed, 0, "{report:?}");
    let kind = wl_documents::expiry::run::NOTIFICATION_TYPE;
    for user in [&planner, &sales, &manager] {
        let told = messages_to(&app, kind, user).await;
        assert_eq!(told.len(), 1, "{} is told once", user.name);
    }
    let told = messages_to(&app, kind, &planner).await;
    let text = format!("{:?}", told[0]);
    assert!(
        text.contains(&late.number),
        "the late line is listed: {text}"
    );
    assert!(
        !text.contains(&early.number),
        "the early line is not: {text}"
    );
    assert!(!text.contains(&draft.number), "a draft is not: {text}");

    // The next night nothing is new.
    app.travel(a_day());
    wl_documents::expiry::run(app.state(), today())
        .await
        .unwrap();
    assert_eq!(messages_to(&app, kind, &planner).await.len(), 1);
}

/// Done when 7 (schedule): the nightly run of the look-ahead is a scheduled
/// task of wl-documents at 01:00 in the Asia/Dubai zone, found in the list of
/// scheduled tasks the way `schedule:list` shows it.
#[renox::test]
async fn the_look_ahead_is_scheduled_for_one_in_the_morning_dubai_time() {
    use renox::Module;
    let mut registry = renox::Registry::default();
    wl_documents::WlDocuments.register(&mut registry);
    let run = registry
        .schedule()
        .upcoming(renox::timezone::Zone::UTC)
        .into_iter()
        .find(|run| run.name == wl_documents::expiry::run::TASK)
        .expect("the nightly expiry run is scheduled");
    assert_eq!(run.zone.to_string(), "Asia/Dubai");
    let at = run.at.expect("it has a next run");
    assert_eq!(
        run.zone.local(at.timestamp()).format("%H:%M").to_string(),
        "01:00"
    );
}
