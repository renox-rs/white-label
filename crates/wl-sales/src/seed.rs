//! The idempotent installer of the nine lead sources (wl-sales §3, #138 A5).
//! [`install`] adds the sources the database lacks and keeps every row that
//! is there, so a client's renames and deactivations survive a re-run.

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
