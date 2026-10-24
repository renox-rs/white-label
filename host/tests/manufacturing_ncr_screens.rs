//! The NCR screens of BL-MFG-08 (#389) as the demo roles open them: the list,
//! the record page with the disposition actions (disabled with the reason, or
//! not offered), a refusal as a 422 in the sheet, and the structure that fits
//! a phone.

mod manufacturing_support;

use host::roles::{PRODUCTION_MANAGER, QC_INSPECTOR, STOREKEEPER};
use manufacturing::Ncr;
use manufacturing_support::{boot, browse, floor, person, post};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_inventory::{Lot, LotStatus};

/// The list and the page of an NCR on opening stock (no supplier lot): QC sees
/// the number, the defect and the cause in words, *Return to supplier* is
/// disabled saying why, the others are offered; the production manager sees
/// the page; a storekeeper is refused; raw keys never show.
#[renox::test]
async fn the_ncr_pages_show_words_and_disable_what_cannot_be_done() {
    let app = boot().await;
    let f = floor(&app).await;
    let qc = person(&app, "Ines", QC_INSPECTOR, Some(f.plant.scope())).await;
    let manager = person(&app, "Mona", PRODUCTION_MANAGER, Some(f.plant.scope())).await;
    let keeper = person(&app, "Karim", STOREKEEPER, None).await;
    let mut lot = Lot {
        item_id: f.resin.id,
        number: "OLD-1".into(),
        status: LotStatus::Quarantine,
        ..Lot::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    lot.insert_in_state(&mut tx, LotStatus::Quarantine, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    wl_inventory::StockMovement::receive(
        f.resin.id,
        Some(lot.id),
        f.store.id,
        50_000,
        800,
        40_000,
        &wl_kernel::Subject::new("inventory.goods_receipt", 900),
        None,
    )
    .save(app.db())
    .await
    .unwrap();
    post(
        &app,
        &qc,
        "/manufacturing/ncrs",
        &[
            ("plant_id", &f.plant.id.to_string()),
            ("lot_id", &lot.id.to_string()),
            ("quantity", "10"),
            ("defect", "cracked pellets"),
        ],
    )
    .await
    .assert_status(303);
    let ncr = Ncr::query().first(app.db()).await.unwrap().unwrap();

    let list = browse(&app, &qc, "/manufacturing/ncrs").await;
    let page = browse(&app, &qc, &format!("/manufacturing/ncrs/{}", ncr.id)).await;
    let seen = browse(&app, &manager, &format!("/manufacturing/ncrs/{}", ncr.id)).await;
    let refused = browse(&app, &keeper, "/manufacturing/ncrs").await;
    let bad = post(
        &app,
        &qc,
        &format!("/manufacturing/ncrs/{}/disposition", ncr.id),
        &[("disposition", "return_to_supplier"), ("outcome", "credit")],
    )
    .await;

    list.assert_ok();
    for words in [ncr.number.as_str(), "Open", "Unknown"] {
        assert!(list.text().contains(words), "{words}: {}", list.text());
    }
    assert!(list.text().contains("rx-grid"), "the standard grid (#556)");
    page.assert_ok();
    for words in [
        "cracked pellets",
        "Rework",
        "Scrap",
        "Use as is",
        "supplier lot is not known",
    ] {
        assert!(page.text().contains(words), "{words}");
    }
    assert!(page.text().contains("rx-sheet"));
    seen.assert_ok();
    refused.assert_status(403);
    bad.assert_status(422);
    assert!(
        bad.text().contains("supplier lot is not known"),
        "the refusal is in the sheet in words"
    );
}
