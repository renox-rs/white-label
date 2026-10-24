//! BL-SAL-28 (#471): marketing channels are a table seeded with the module's
//! list, extended and switched off per vertical. The vertical lists here are
//! the test's own (the 01 six and a 02 channel): wl-sales holds none.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_sales::costs::{self, CostInput};
use wl_sales::marketing_channel::CampaignChannel;
use wl_sales::permissions as perm;
use wl_sales::report::{self, Filter, GroupBy, GroupKey, Viewer};
use wl_sales::{Campaign, ChannelRefused, Purpose};

use support::{Booted, boot};

/// The six channels of the 01 seed as (key, English name, Arabic name).
const PROPERTY_CHANNELS: [(&str, &str, &str); 6] = [
    ("listing_portals", "Listing portals", ""),
    ("social_media", "Social media", ""),
    ("search_ads", "Search ads", ""),
    ("email", "Email", ""),
    ("signboards", "Signboards", ""),
    ("brokers", "Brokers", ""),
];

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, d).unwrap()
}

/// An active campaign `code` on the channel `channel`.
async fn campaign(t: &Booted, code: &str, channel: &str) -> Campaign {
    let stored = Campaign {
        name: format!("Campaign {code}"),
        code: code.into(),
        purpose: Some(Purpose::Acquisition),
        marketing_channel: Some(channel.into()),
        owner_id: Some(1),
        start_date: Some(day(10, 1)),
        end_date: Some(day(10, 31)),
        ..Campaign::default()
    }
    .store(t.db())
    .await
    .unwrap()
    .unwrap();
    Campaign::query()
        .where_(Campaign::ID.eq(stored.id))
        .update(t.db(), &[("status", &"active")])
        .await
        .unwrap();
    Campaign::find(t.db(), stored.id).await.unwrap().unwrap()
}

async fn manager(t: &Booted, name: &str) -> User {
    t.user(name, &[perm::CAMPAIGNS_VIEW, perm::CAMPAIGNS_MANAGE])
        .await
}

#[renox::test]
async fn the_module_list_is_seeded_once_with_stable_keys() {
    let t = boot().await;
    let db = t.db();
    assert_eq!(CampaignChannel::query().count(db).await.unwrap(), 12);
    let meta = CampaignChannel::by_code(db, "meta").await.unwrap().unwrap();
    assert!(meta.active && meta.seeded);
    assert_eq!(meta.name_en, "Meta");
    t.app
        .kernel()
        .call("sales:install", Vec::<String>::new())
        .await
        .unwrap();
    assert_eq!(CampaignChannel::query().count(db).await.unwrap(), 12);
}

#[renox::test]
async fn with_the_property_seed_the_form_offers_the_six_channels_and_the_report_names_them() {
    let t = boot().await;
    let db = t.db();
    let seeded = CampaignChannel::seed_vertical(db, &PROPERTY_CHANNELS, true)
        .await
        .unwrap();
    assert_eq!(seeded.added, 5, "e-mail is the module's own");
    assert_eq!(seeded.deactivated, 11);
    let active: Vec<String> = CampaignChannel::ordered(db)
        .await
        .unwrap()
        .into_iter()
        .filter(|c| c.active)
        .map(|c| c.code)
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
    // Nothing is deleted, and a re-run changes nothing.
    assert_eq!(CampaignChannel::query().count(db).await.unwrap(), 17);
    let again = CampaignChannel::seed_vertical(db, &PROPERTY_CHANNELS, true)
        .await
        .unwrap();
    assert_eq!((again.added, again.deactivated), (0, 0));

    let boss = manager(&t, "Mona").await;
    t.app.acting_as(&boss);
    t.app
        .get("/campaigns/new")
        .await
        .assert_ok()
        .assert_see("Listing portals")
        .assert_see("Signboards")
        .assert_dont_see("Other online")
        .assert_dont_see("TikTok");

    // A campaign on listing portals reports under its own name.
    let portals = campaign(&t, "BAYUT26", "listing_portals").await;
    costs::save(
        t.state(),
        CostInput {
            campaign_id: portals.id,
            entry_date: day(10, 5),
            amount: 300_000,
            description: "Listing fee".into(),
            requested_by: 1,
            manages_costs: true,
        },
    )
    .await
    .unwrap()
    .unwrap();
    let config = wl_sales::SalesConfig::default();
    let facts = report::load(db, &config).await.unwrap();
    let r = report::compute(
        &facts,
        &Filter::default().month(2026, 10),
        GroupBy::Channel,
        &Viewer::all(),
    );
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0].key.value.as_deref(), Some("listing_portals"));
    let label = report::label_of(db, &r.rows[0].key, "en").await.unwrap();
    assert_eq!(label, "Listing portals");
    assert_ne!(label, "Other online");
}

#[renox::test]
async fn a_campaign_from_before_the_change_keeps_its_channel() {
    let t = boot().await;
    let db = t.db();
    // The column holds the old list's stored name; the migration made it a
    // row key without touching the campaign.
    let old = campaign(&t, "OLD26", "meta").await;
    assert_eq!(old.marketing_channel.as_deref(), Some("meta"));
    let key = GroupKey {
        by: GroupBy::Channel,
        value: Some("meta".into()),
    };
    assert_eq!(report::label_of(db, &key, "en").await.unwrap(), "Meta");
    let boss = manager(&t, "Mona").await;
    t.app.acting_as(&boss);
    t.app
        .get(&format!("/campaigns/{}", old.id))
        .await
        .assert_ok()
        .assert_see("Meta");
}

#[renox::test]
async fn a_channel_in_use_is_not_deleted_and_deactivating_hides_it_from_new_campaigns_only() {
    let t = boot().await;
    let db = t.db();
    let used = campaign(&t, "TIK26", "tiktok").await;
    let tiktok = CampaignChannel::by_code(db, "tiktok")
        .await
        .unwrap()
        .unwrap();
    assert!(tiktok.in_use(db).await.unwrap());
    assert_eq!(
        tiktok.clone().delete_unused(db).await.unwrap(),
        Err(ChannelRefused::InUse)
    );

    let boss = manager(&t, "Mona").await;
    t.app.acting_as(&boss);
    t.app
        .post(
            &format!("/sales/marketing-channels/{}/delete", tiktok.id),
            &[("_", "")],
        )
        .await
        .assert_status(422);
    assert!(
        CampaignChannel::by_code(db, "tiktok")
            .await
            .unwrap()
            .is_some()
    );

    t.app
        .post(
            &format!("/sales/marketing-channels/{}/deactivate", tiktok.id),
            &[("_", "")],
        )
        .await
        .assert_ok();
    t.app
        .get("/campaigns/new")
        .await
        .assert_ok()
        .assert_dont_see("TikTok");
    // The old campaign keeps it, on its form, its page and in reports.
    t.app
        .get(&format!("/campaigns/{}/edit", used.id))
        .await
        .assert_ok()
        .assert_see("TikTok");
    t.app
        .get(&format!("/campaigns/{}", used.id))
        .await
        .assert_ok()
        .assert_see("TikTok");
    // A new campaign cannot pick it.
    t.app
        .htmx()
        .post(
            "/campaigns",
            &[
                ("name", "New one"),
                ("code", "NEW26"),
                ("purpose", "acquisition"),
                ("marketing_channel", "tiktok"),
            ],
        )
        .await
        .assert_invalid("marketing_channel");

    // An unused channel can be deleted.
    let sms = CampaignChannel::by_code(db, "sms").await.unwrap().unwrap();
    sms.delete_unused(db).await.unwrap().unwrap();
    assert!(CampaignChannel::by_code(db, "sms").await.unwrap().is_none());
}

#[renox::test]
async fn a_vertical_seed_adds_tender_invitations_without_code_in_wl_sales() {
    let t = boot().await;
    let db = t.db();
    let done = CampaignChannel::seed_vertical(
        db,
        &[(
            "tender_invitations",
            "Tender invitations",
            "دعوات المناقصات",
        )],
        false,
    )
    .await
    .unwrap();
    assert_eq!((done.added, done.deactivated), (1, 0));
    let tender = CampaignChannel::by_code(db, "tender_invitations")
        .await
        .unwrap()
        .unwrap();
    assert!(tender.active);
    assert_eq!(tender.name("ar"), "دعوات المناقصات");
    let offered = CampaignChannel::offered(db, "en", None).await.unwrap();
    assert!(offered.iter().any(|(_, name)| name == "Tender invitations"));
    // The page lists it for the sales manager.
    let boss = manager(&t, "Mona").await;
    t.app.acting_as(&boss);
    t.app
        .get("/sales/marketing-channels")
        .await
        .assert_ok()
        .assert_see("Tender invitations");
}
