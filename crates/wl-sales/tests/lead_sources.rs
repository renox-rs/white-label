//! Lead sources: the seeds, the in-use check and the back-office page
//! (#138 part 3).

mod support;

use renox::prelude::Model;
use wl_sales::{LeadSource, LeadSourceRefused, permissions as perm};

use support::boot;

#[renox::test]
async fn the_installer_stored_the_nine_sources_once() {
    let t = boot().await;
    let db = t.db();
    assert_eq!(LeadSource::query().count(db).await.unwrap(), 9);
    t.app
        .kernel()
        .call("sales:install", Vec::<String>::new())
        .await
        .unwrap();
    assert_eq!(LeadSource::query().count(db).await.unwrap(), 9);
}

#[renox::test]
async fn a_source_nothing_names_is_deleted_and_a_taken_code_is_refused() {
    let t = boot().await;
    let db = t.db();
    let tv = LeadSource::add(db, " TV_Ad ", "TV ad", "")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(tv.code, "tv_ad");
    assert!(matches!(
        LeadSource::add(db, "tv_ad", "Again", "").await.unwrap(),
        Err(LeadSourceRefused::CodeTaken)
    ));
    assert!(matches!(
        LeadSource::add(db, "no way", "Bad", "").await.unwrap(),
        Err(LeadSourceRefused::BadCode)
    ));
    assert!(!tv.in_use(db).await.unwrap());
    tv.delete_unused(db).await.unwrap().unwrap();
    assert!(LeadSource::by_code(db, "tv_ad").await.unwrap().is_none());
}

#[renox::test]
async fn the_page_needs_campaigns_manage_and_lists_the_sources() {
    let t = boot().await;
    let viewer = t.user("Vera", &[perm::CAMPAIGNS_VIEW]).await;
    t.app.acting_as(&viewer);
    t.app.get("/sales/lead-sources").await.assert_forbidden();

    let manager = t.user("Mona", &[perm::CAMPAIGNS_MANAGE]).await;
    t.app.acting_as(&manager);
    t.app
        .get("/sales/lead-sources")
        .await
        .assert_ok()
        .assert_see("Lead sources")
        .assert_see("walk_in")
        .assert_see("Walk-in")
        .assert_see("paid_ads");
}

#[renox::test]
async fn the_page_adds_renames_and_switches_a_source() {
    let t = boot().await;
    let db = t.db();
    let manager = t.user("Mona", &[perm::CAMPAIGNS_MANAGE]).await;
    t.app.acting_as(&manager);

    t.app
        .post(
            "/sales/lead-sources",
            &[("code", "Radio"), ("name_en", "Radio"), ("name_ar", "")],
        )
        .await
        .assert_ok();
    let radio = LeadSource::by_code(db, "radio").await.unwrap().unwrap();
    assert!(radio.active && !radio.seeded);

    // A taken code is a 422.
    t.app
        .htmx()
        .post(
            "/sales/lead-sources",
            &[("code", "radio"), ("name_en", "Radio 2")],
        )
        .await
        .assert_invalid("code");

    let path = format!("/sales/lead-sources/{}", radio.id);
    t.app
        .post(&path, &[("name_en", "Radio ads"), ("name_ar", "إذاعة")])
        .await
        .assert_ok();
    t.app
        .htmx()
        .post(&path, &[("name_en", "  "), ("name_ar", "")])
        .await
        .assert_invalid("name_en");
    let renamed = LeadSource::by_code(db, "radio").await.unwrap().unwrap();
    assert_eq!(
        (renamed.name_en.as_str(), renamed.name_ar.as_str()),
        ("Radio ads", "إذاعة")
    );

    t.app
        .post(&format!("{path}/deactivate"), &[("x", "y")])
        .await
        .assert_ok();
    assert!(
        !LeadSource::by_code(db, "radio")
            .await
            .unwrap()
            .unwrap()
            .active
    );
    t.app
        .post(&format!("{path}/activate"), &[("x", "y")])
        .await
        .assert_ok();
    assert!(
        LeadSource::by_code(db, "radio")
            .await
            .unwrap()
            .unwrap()
            .active
    );
    t.app
        .post("/sales/lead-sources/9999", &[("name_en", "Nope")])
        .await
        .assert_not_found();
}
