//! The idempotent installer of the nine lead sources (wl-sales §3, #138 A5).
//! [`install`] adds the sources the database lacks and keeps every row that
//! is there, so a client's renames and deactivations survive a re-run.

use renox::auth::User;
use renox::prelude::*;

use crate::lead_source::{DEFAULTS, LeadSource};

/// What [`install`] created.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Installed {
    /// Lead sources created.
    pub lead_sources: usize,
}

impl Installed {
    /// The number of rows of every kind created.
    pub fn total(&self) -> usize {
        self.lead_sources
    }
}

/// Stores the lead sources the database lacks, in one transaction.
pub async fn install(db: &Db) -> Result<Installed> {
    let mut tx = db.begin().await?;
    let mut installed = Installed::default();
    for (code, name) in DEFAULTS {
        if LeadSource::by_code(&mut tx, code).await?.is_none() {
            LeadSource::create(
                &mut tx,
                LeadSource {
                    code: code.to_owned(),
                    name_en: name.to_owned(),
                    active: true,
                    seeded: true,
                    ..LeadSource::default()
                },
            )
            .await?;
            installed.lead_sources += 1;
        }
    }
    tx.commit().await?;
    Ok(installed)
}

/// Stores two demo campaigns as drafts, where their codes are free: an
/// acquisition campaign on Meta with a budget and a promo code, and a
/// retention-style e-mail campaign without a budget. The owner is the first
/// user, if there is one. A re-run adds nothing. Returns how many it made.
pub async fn demo_campaigns(db: &Db) -> Result<usize> {
    use renox::chrono::NaiveDate;

    use crate::campaign::{Campaign, MarketingChannel, Purpose};

    let owner = User::query().order_by("id").first(db).await?.map(|u| u.id);
    let day = |m, d| NaiveDate::from_ymd_opt(2026, m, d);
    let demos = [
        Campaign {
            name: "Spring offer".into(),
            code: "SPRING26".into(),
            purpose: Some(Purpose::Acquisition),
            marketing_channel: Some(MarketingChannel::Meta),
            owner_id: owner,
            start_date: day(3, 1),
            end_date: day(3, 31),
            budget: Some(500_000),
            promo_code: Some("SPRING26".into()),
            ..Campaign::default()
        },
        Campaign {
            name: "Summer newsletter".into(),
            code: "SUMMER26".into(),
            purpose: Some(Purpose::Acquisition),
            marketing_channel: Some(MarketingChannel::Email),
            owner_id: owner,
            start_date: day(6, 1),
            end_date: day(8, 31),
            ..Campaign::default()
        },
    ];
    let mut made = 0;
    for demo in demos {
        if Campaign::by_code(db, &demo.code).await?.is_none() && demo.store(db).await?.is_ok() {
            made += 1;
        }
    }
    Ok(made)
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::testing::TestApp;

    #[renox::test]
    async fn installing_twice_leaves_nine_and_keeps_edits() {
        let app = TestApp::new(App::new().module(crate::WlSales::new())).await;
        let db = app.db();
        assert_eq!(install(db).await.unwrap(), Installed { lead_sources: 9 });
        assert!(
            LeadSource::ordered(db)
                .await
                .unwrap()
                .iter()
                .all(|s| s.active && s.seeded)
        );

        let mut walk_in = LeadSource::by_code(db, "walk_in").await.unwrap().unwrap();
        walk_in.name_en = "Walk in".into();
        walk_in.active = false;
        walk_in.save(db).await.unwrap();

        assert_eq!(install(db).await.unwrap(), Installed::default());
        assert_eq!(LeadSource::query().count(db).await.unwrap(), 9);
        let walk_in = LeadSource::by_code(db, "walk_in").await.unwrap().unwrap();
        assert_eq!(walk_in.name_en, "Walk in");
        assert!(!walk_in.active);
    }

    #[renox::test]
    async fn the_demo_campaigns_are_two_drafts_made_once() {
        use crate::campaign::{Campaign, CampaignStatus};
        let app = TestApp::new(
            App::new()
                .module(renox::auth::Auth::new())
                .module(crate::WlSales::new()),
        )
        .await;
        let db = app.db();
        assert_eq!(demo_campaigns(db).await.unwrap(), 2);
        assert_eq!(demo_campaigns(db).await.unwrap(), 0);
        let spring = Campaign::by_code(db, "spring26").await.unwrap().unwrap();
        assert_eq!(spring.status, CampaignStatus::Draft);
        assert_eq!(spring.budget, Some(500_000));
        assert_eq!(Campaign::query().count(db).await.unwrap(), 2);
    }

    #[renox::test]
    async fn a_source_the_client_added_is_not_seeded_and_survives() {
        let app = TestApp::new(App::new().module(crate::WlSales::new())).await;
        let db = app.db();
        install(db).await.unwrap();
        LeadSource::add(db, "TV_Ad", "TV ad", "")
            .await
            .unwrap()
            .unwrap();
        install(db).await.unwrap();
        let tv = LeadSource::by_code(db, "tv_ad").await.unwrap().unwrap();
        assert!(tv.active && !tv.seeded);
        assert_eq!(LeadSource::query().count(db).await.unwrap(), 10);
    }
}
