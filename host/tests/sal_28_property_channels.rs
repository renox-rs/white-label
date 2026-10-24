//! BL-SAL-28 (#471) on the Property client: the 01 seed adds its six
//! campaign channels and switches off the module's others, which stay in the
//! table for old campaigns and reports.

use renox::prelude::*;
use renox::testing::TestApp;
use wl_sales::CampaignChannel;

/// The app, with `wl:install` and the seeds run as the demo does.
async fn booted() -> TestApp {
    let app = TestApp::new(host::app()).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    app
}

#[renox::test]
async fn the_property_seed_offers_the_six_01_channels() {
    let app = booted().await;
    app.kernel()
        .call("property:install", Vec::<String>::new())
        .await
        .unwrap();
    let channels = CampaignChannel::ordered(app.db()).await.unwrap();
    let active: Vec<&str> = channels
        .iter()
        .filter(|c| c.active)
        .map(|c| c.code.as_str())
        .collect();
    assert_eq!(
        active,
        [
            "brokers",
            "email",
            "listing_portals",
            "search_ads",
            "signboards",
            "social_media"
        ]
    );
    let other_online = channels
        .iter()
        .find(|c| c.code == "other_online")
        .expect("the module's channel is kept");
    assert!(!other_online.active);
    let portals = channels
        .iter()
        .find(|c| c.code == "listing_portals")
        .unwrap();
    assert_eq!(portals.name_en, "Listing portals");

    // A second run changes nothing.
    let count = CampaignChannel::query().count(app.db()).await.unwrap();
    app.kernel()
        .call("property:install", Vec::<String>::new())
        .await
        .unwrap();
    assert!(
        CampaignChannel::by_code(app.db(), "listing_portals")
            .await
            .unwrap()
            .is_some_and(|c| c.active)
    );
    assert_eq!(
        CampaignChannel::query().count(app.db()).await.unwrap(),
        count
    );
}
