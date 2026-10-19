//! The sanctions list base (wl-parties §3, issue #40 part 1): seeding, the
//! stored defaults and the default feed.

mod support;

use wl_parties::feed::{FeedError, NO_FEED, feed};
use wl_parties::{ListStatus, SanctionsList, WlParties, seed};

use support::{boot, boot_with};

#[renox::test]
async fn with_screening_on_the_seed_adds_two_stale_lists_once() {
    let t = boot_with(WlParties::new().screening(true), false).await;
    assert_eq!(seed::lists(t.app.state()).await.unwrap(), 2);
    let lists = SanctionsList::all(t.db()).await.unwrap();
    let names: Vec<_> = lists.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["UAE Local Terrorist List", "UN Consolidated List"]);
    for list in &lists {
        assert_eq!(list.status, ListStatus::Stale);
        assert!(list.version.is_none() && list.last_updated_at.is_none());
        assert!(list.stale_since.is_none() && list.last_error.is_none());
        assert!(list.is_stale_at(renox::db::now(), 24));
    }
    assert_eq!(seed::lists(t.app.state()).await.unwrap(), 0);
    assert_eq!(SanctionsList::all(t.db()).await.unwrap().len(), 2);
}

#[renox::test]
async fn with_screening_off_no_list_is_seeded() {
    let t = boot().await;
    assert!(SanctionsList::all(t.db()).await.unwrap().is_empty());
    assert_eq!(seed::lists(t.app.state()).await.unwrap(), 0);
}

#[renox::test]
async fn without_a_feed_every_download_fails() {
    let t = boot_with(WlParties::new().screening(true), false).await;
    seed::lists(t.app.state()).await.unwrap();
    let list = SanctionsList::all(t.db()).await.unwrap().remove(0);
    let result = feed(t.app.state()).fetch(&list).await;
    assert_eq!(result, Err(FeedError(NO_FEED.to_owned())));
}
