//! The hourly sanctions list check (wl-parties §3, `PTY.RUL.02`, issue #40
//! part 2): download, same version, failure, stale and current transitions
//! and the two events, with a fake feed and travelled time.

mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use renox::prelude::*;
use wl_kernel::{RuleRun, RuleRunStatus};
use wl_parties::feed::{FeedError, FetchFuture};
use wl_parties::freshness::{self, RULE};
use wl_parties::{
    ListFeed, ListStatus, ListVersion, SanctionsList, SanctionsListStale, SanctionsListUpdated,
    WlParties, seed,
};

use support::{Booted, boot, boot_with};

/// A feed that answers what the test sets.
#[derive(Clone, Default)]
struct FakeFeed(Arc<Mutex<Option<String>>>);

impl FakeFeed {
    fn serve(&self, version: &str) {
        *self.0.lock().unwrap() = Some(version.to_owned());
    }

    fn fail(&self) {
        *self.0.lock().unwrap() = None;
    }
}

impl ListFeed for FakeFeed {
    fn fetch<'a>(&'a self, _list: &'a SanctionsList) -> FetchFuture<'a> {
        let answer = self.0.lock().unwrap().clone();
        Box::pin(async move {
            answer
                .map(|version| ListVersion {
                    version,
                    published_at: None,
                })
                .ok_or_else(|| FeedError("source unreachable".to_owned()))
        })
    }
}

async fn setup() -> (Booted, FakeFeed) {
    let feed = FakeFeed::default();
    let t = boot_with(
        WlParties::new().screening(true).list_feed(feed.clone()),
        false,
    )
    .await;
    seed::lists(t.app.state()).await.unwrap();
    t.app.fake_events();
    (t, feed)
}

async fn run(t: &Booted) -> freshness::Report {
    t.app
        .at_travelled_time(freshness::run(t.app.state()))
        .await
        .unwrap()
}

fn hours(t: &Booted, n: u64) {
    t.app.travel(Duration::from_secs(n * 3600));
}

async fn un(t: &Booted) -> SanctionsList {
    SanctionsList::all(t.db())
        .await
        .unwrap()
        .into_iter()
        .find(|l| l.name == "UN Consolidated List")
        .unwrap()
}

#[renox::test]
async fn a_first_download_makes_the_lists_current_with_one_update_each() {
    let (t, feed) = setup().await;
    feed.serve("v1");
    let report = run(&t).await;
    assert_eq!((report.checked, report.updated, report.stale), (2, 2, 0));
    let list = un(&t).await;
    assert_eq!(list.status, ListStatus::Current);
    assert_eq!(list.version.as_deref(), Some("v1"));
    assert!(list.stale_since.is_none() && list.last_error.is_none());
    assert!(list.last_updated_at.is_some() && list.last_attempt_at.is_some());
    assert_eq!(t.app.emitted::<SanctionsListUpdated>().len(), 2);
    assert!(t.app.emitted::<SanctionsListStale>().is_empty());
}

#[renox::test]
async fn a_second_run_in_the_same_hour_does_nothing() {
    let (t, feed) = setup().await;
    feed.serve("v1");
    run(&t).await;
    let again = run(&t).await;
    assert_eq!((again.skipped, again.updated), (2, 0));
    let runs = RuleRun::query().get(t.db()).await.unwrap();
    assert_eq!(runs.len(), 2);
    assert!(runs.iter().all(|r| r.rule == RULE));
    assert!(runs.iter().all(|r| r.status == RuleRunStatus::Succeeded));
}

#[renox::test]
async fn a_failing_feed_turns_the_list_stale_once_per_period() {
    let (t, feed) = setup().await;
    feed.serve("v1");
    run(&t).await;
    feed.fail();
    hours(&t, 1);
    let first = run(&t).await;
    assert_eq!((first.failed_downloads, first.stale), (0, 0), "still fresh");
    // Not due yet: a fresh list is downloaded again after 12 h.
    let list = un(&t).await;
    assert_eq!(list.status, ListStatus::Current);
    hours(&t, 12);
    let failed = run(&t).await;
    assert_eq!((failed.failed_downloads, failed.stale), (2, 2));
    let list = un(&t).await;
    assert_eq!(list.status, ListStatus::Stale);
    assert!(list.stale_since.is_some());
    assert_eq!(list.last_error.as_deref(), Some("source unreachable"));
    assert_eq!(t.app.emitted::<SanctionsListStale>().len(), 2);
    // Retried hourly, no second event.
    let since = list.stale_since;
    for _ in 0..3 {
        hours(&t, 1);
        let again = run(&t).await;
        assert_eq!((again.failed_downloads, again.stale), (2, 0));
    }
    assert_eq!(un(&t).await.stale_since, since);
    assert_eq!(t.app.emitted::<SanctionsListStale>().len(), 2);
    // Recovered, then failing again: a new period, a new event.
    feed.serve("v1");
    hours(&t, 1);
    let same = run(&t).await;
    assert_eq!((same.updated, same.stale), (0, 0));
    let list = un(&t).await;
    assert_eq!(list.status, ListStatus::Current);
    assert!(list.stale_since.is_none() && list.last_error.is_none());
    feed.fail();
    hours(&t, 12);
    run(&t).await;
    assert_eq!(t.app.emitted::<SanctionsListStale>().len(), 4);
}

#[renox::test]
async fn fresh_lists_are_not_downloaded_before_the_retry_hours() {
    let (t, feed) = setup().await;
    feed.serve("v1");
    run(&t).await;
    feed.fail();
    hours(&t, 11);
    let before = run(&t).await;
    assert_eq!(before.failed_downloads, 0, "fresh lists are not retried");
}

#[renox::test]
async fn a_new_version_emits_the_update_and_the_same_version_does_not() {
    let (t, feed) = setup().await;
    feed.serve("v1");
    run(&t).await;
    hours(&t, 12);
    let same = run(&t).await;
    assert_eq!(same.updated, 0);
    let refreshed = un(&t).await;
    assert_eq!(refreshed.version.as_deref(), Some("v1"));
    feed.serve("v2");
    hours(&t, 12);
    let newer = run(&t).await;
    assert_eq!(newer.updated, 2);
    assert_eq!(un(&t).await.version.as_deref(), Some("v2"));
    t.app
        .assert_emitted::<SanctionsListUpdated>(|e| e.version == "v2");
    assert_eq!(t.app.emitted::<SanctionsListUpdated>().len(), 4);
}

#[renox::test]
async fn without_a_feed_the_lists_turn_stale_and_alert_once() {
    let t = boot_with(WlParties::new().screening(true), false).await;
    seed::lists(t.app.state()).await.unwrap();
    t.app.fake_events();
    let first = run(&t).await;
    assert_eq!((first.failed_downloads, first.stale), (2, 2));
    hours(&t, 1);
    let second = run(&t).await;
    assert_eq!((second.failed_downloads, second.stale), (2, 0));
    let list = un(&t).await;
    assert_eq!(list.status, ListStatus::Stale);
    assert_eq!(list.last_error.as_deref(), Some(wl_parties::feed::NO_FEED));
}

#[renox::test]
async fn with_screening_off_the_check_does_nothing() {
    let t = boot().await;
    let report = run(&t).await;
    assert_eq!(report, freshness::Report::default());
    assert!(RuleRun::query().get(t.db()).await.unwrap().is_empty());
}
