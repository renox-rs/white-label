//! BL-MFG-11 (#392) with the real wl-inventory dispatch, wl-documents and
//! tickets: a regulated item leaves the plant only while one of its accepted
//! certificates is valid (the day of the expiry still counts, the day after
//! does not, a renewal in review does not, a renewal accepted does), no
//! override is offered, an export only item is not delivered in the UAE, the
//! nightly look-ahead lists the orders promised after the expiry once, and an
//! expired industrial licence blocks nothing.

mod manufacturing_support;

use host::roles::{PRODUCTION_MANAGER, PRODUCTION_PLANNER, STOREKEEPER};
use manufacturing::ItemFlag;
use manufacturing::conformity::{ECAS, EQM, FOOD_CONTACT};
use manufacturing_support::{
    boot_with_orders, browse, dispatch_of, document, floor, messages_to, person, post, stocked,
    words,
};
use renox::chrono::Duration;
use renox::prelude::*;
use wl_documents::DocumentStatus;
use wl_inventory::DispatchStatus;
use wl_kernel::Subject;
use wl_kernel::documents::today;
use wl_kernel::sales::order_line_rules;

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

/// Done when 7: LED fitting X has an order line promised in 40 days and an ECAS
/// certificate that expires in 29 days. The nightly run of wl-documents tells
/// sales and the planner, in one message, about the line; the next night
/// nothing new is sent. A line promised before the expiry is not listed.
#[renox::test]
async fn the_look_ahead_lists_the_orders_promised_after_the_expiry_once() {
    let (app, orders) = boot_with_orders().await;
    let f = floor(&app).await;
    let manager = person(&app, "Mona", PRODUCTION_MANAGER, None).await;
    renox::auth::permissions::define_role(app.db(), manufacturing::permissions::SALES_ROLE, &[])
        .await
        .unwrap();
    let planner = person(&app, "Pia", PRODUCTION_PLANNER, None).await;
    let sales = person(&app, "Sam", manufacturing::permissions::SALES_ROLE, None).await;
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
    orders.promise(80, 1, led.id, today() + Duration::days(40));
    orders.promise(81, 1, led.id, today() + Duration::days(10));

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
    assert!(text.contains("SO-80"), "the late line is listed: {text}");
    assert!(!text.contains("SO-81"), "the early line is not: {text}");

    // The next night nothing is new.
    app.travel(a_day());
    wl_documents::expiry::run(app.state(), today())
        .await
        .unwrap();
    assert_eq!(messages_to(&app, kind, &planner).await.len(), 1);
}
