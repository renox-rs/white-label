//! Deliveries and runs in the search index and the Related panel
//! (BL-AGT-11): a delivery is found by its number and by its source's
//! tracking number, a run by the tracking numbers of its deliveries, the
//! documents follow the transitions, the phone is stored only as a hash, and
//! both record pages load their Related panel with the groups declared.

mod support;

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::documents::today;
use wl_kernel::search::{self, SearchIdentifierRow, kinds};

use support::boot;
use wl_delivery::search::{DELIVERY_SUBJECT_TYPE, RUN_SUBJECT_TYPE};

/// The source of the deliveries of this test, as the logistics app indexes
/// a shipment: its number is an identifier of the shipment.
async fn shipment_numbered(db: &Db, id: i64, number: &str) {
    let mut row = SearchIdentifierRow {
        subject_type: "logistics.shipment".to_owned(),
        subject_id: id,
        kind: kinds::NUMBER.to_owned(),
        value: search::normalize_identifier(kinds::NUMBER, number),
        hashed: false,
        ..Default::default()
    };
    row.insert(db).await.unwrap();
}

#[renox::test]
async fn a_tracking_number_finds_the_delivery_and_then_its_run() {
    let world = boot().await;
    shipment_numbered(world.db(), 1_000_001, "SHP-7001").await;
    let request = wl_kernel::DeliveryRequest::new(
        Subject::new("logistics.shipment", 1_000_001),
        "Mariam Al Hosani",
    )
    .address("Villa 12, Al Barsha 2", "Al Barsha")
    .phone("+971500000000")
    .item(wl_kernel::DeliveryItem::new("Parcel 1", 1_000));
    let delivery = wl_delivery::Deliveries::request(world.state(), request)
        .await
        .unwrap();
    let delivery_subject = Subject::new(DELIVERY_SUBJECT_TYPE, delivery.id);

    // The document: the number, the recipient, the status.
    let document = search::document_of(world.db(), &delivery_subject)
        .await
        .unwrap()
        .expect("the request indexed the delivery");
    assert_eq!(document.title, delivery.number);
    assert_eq!(document.status.as_deref(), Some("pending"));
    assert!(document.secondary.contains("Mariam Al Hosani"));
    // The phone is a hash, never in the text.
    let identifiers = search::identifiers_of(world.db(), &delivery_subject)
        .await
        .unwrap();
    let phone = identifiers
        .iter()
        .find(|row| row.kind == kinds::PHONE)
        .expect("the phone is an identifier");
    assert!(phone.hashed);
    assert!(!document.keywords.contains("971500000000"));

    // The tracking number, typed loosely, finds the delivery.
    let found = search::lookup_identifier(world.db(), "shp 7001")
        .await
        .unwrap();
    assert!(found.iter().any(|m| m.subject == delivery_subject));
    let by_number = search::lookup_identifier(world.db(), &delivery.number)
        .await
        .unwrap();
    assert!(by_number.iter().any(|m| m.subject == delivery_subject));

    // On a run, the same tracking number finds the run too, and the
    // delivery's document follows the transition.
    world.valid_documents("2099-12-31");
    let run = world.run_on(&today().format("%Y-%m-%d").to_string()).await;
    let run_subject = Subject::new(RUN_SUBJECT_TYPE, run.id);
    let run_document = search::document_of(world.db(), &run_subject)
        .await
        .unwrap()
        .expect("a new run is indexed");
    assert_eq!(run_document.title, run.number);
    world
        .assign(&world.dispatcher, &delivery, &run)
        .await
        .assert_ok();
    let found = search::lookup_identifier(world.db(), "SHP-7001")
        .await
        .unwrap();
    assert!(found.iter().any(|m| m.subject == delivery_subject));
    assert!(found.iter().any(|m| m.subject == run_subject));
    let document = search::document_of(world.db(), &delivery_subject)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(document.status.as_deref(), Some("assigned"));

    // The rebuild gives the same documents as the writes.
    let before = search::identifiers_of(world.db(), &run_subject)
        .await
        .unwrap();
    search::rebuild_type(world.db(), RUN_SUBJECT_TYPE)
        .await
        .unwrap();
    let after = search::identifiers_of(world.db(), &run_subject)
        .await
        .unwrap();
    let values = |rows: &[SearchIdentifierRow]| {
        rows.iter()
            .map(|r| (r.kind.clone(), r.value.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(values(&before), values(&after));
}

#[renox::test]
async fn the_delivery_and_run_pages_load_their_related_panels() {
    let world = boot().await;
    let delivery = world.simple_delivery().await;
    world.valid_documents("2099-12-31");
    let run = world.run_on(&today().format("%Y-%m-%d").to_string()).await;
    world
        .assign(&world.dispatcher, &delivery, &run)
        .await
        .assert_ok();

    world.app.acting_as(&world.dispatcher);
    world
        .app
        .get(&format!("/deliveries/{}", delivery.id))
        .await
        .assert_ok()
        .assert_see(&format!(
            "hx-get=\"/related/delivery.delivery:{}/panel\"",
            delivery.id
        ));
    world
        .app
        .get(&format!("/related/delivery.delivery:{}/panel", delivery.id))
        .await
        .assert_ok()
        .assert_see("data-group=\"run\" data-count=\"1\"")
        .assert_see(&run.number)
        .assert_see("data-group=\"attempts\" data-count=\"0\"")
        .assert_see("data-group=\"proofs\" data-count=\"0\"");
    world
        .app
        .get(&format!("/runs/{}", run.id))
        .await
        .assert_ok()
        .assert_see(&format!(
            "hx-get=\"/related/delivery.run:{}/panel\"",
            run.id
        ));
    world
        .app
        .get(&format!("/related/delivery.run:{}/panel", run.id))
        .await
        .assert_ok()
        .assert_see("data-group=\"deliveries\" data-count=\"1\"")
        .assert_see(&delivery.number);
}
