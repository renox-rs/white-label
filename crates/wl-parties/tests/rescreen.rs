//! The stale flag on screening runs and the re-run after a list update
//! (wl-parties §3, issue #40 part 4): `screening::run` records the lists
//! used, `SanctionsListUpdated` re-runs the flagged screenings once.

mod support;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use renox::prelude::*;
use wl_kernel::ScreeningStatus;
use wl_parties::feed::{FeedError, FetchFuture};
use wl_parties::screening::{self, CheckFuture};
use wl_parties::{
    ListFeed, ListUsed, ListVersion, NewCandidate, Party, ProviderError, SanctionsList, Screening,
    ScreeningProvider, ScreeningTrigger, WlParties, freshness, rescreen, seed,
};

use support::{Booted, boot_with};

const AL_NOOR: &str = "Al Noor Trading LLC";

#[derive(Clone, Default)]
struct FakeFeed(Arc<Mutex<Option<String>>>);

impl FakeFeed {
    fn serve(&self, version: &str) {
        *self.0.lock().unwrap() = Some(version.to_owned());
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

/// A provider that finds nothing, or fails while its flag is set.
#[derive(Clone, Default)]
struct NoMatches(Arc<AtomicBool>);

impl ScreeningProvider for NoMatches {
    fn check<'a>(&'a self, _party: &'a Party, _lists: &'a [ListUsed]) -> CheckFuture<'a> {
        let down = self.0.load(Ordering::SeqCst);
        Box::pin(async move {
            if down {
                Err(ProviderError("provider down".to_owned()))
            } else {
                Ok(Vec::<NewCandidate>::new())
            }
        })
    }
}

async fn setup(provider: bool) -> (Booted, FakeFeed) {
    let (t, feed, _) = setup_with(provider).await;
    (t, feed)
}

async fn setup_with(provider: bool) -> (Booted, FakeFeed, Arc<AtomicBool>) {
    let flag = Arc::new(AtomicBool::new(false));
    let feed = FakeFeed::default();
    let mut module = WlParties::new().screening(true).list_feed(feed.clone());
    if provider {
        module = module.screening_provider(NoMatches(Arc::clone(&flag)));
    }
    let t = boot_with(module, true).await;
    seed::lists(t.app.state()).await.unwrap();
    (t, feed, flag)
}

async fn check_lists(t: &Booted) -> freshness::Report {
    t.app
        .at_travelled_time(freshness::run(t.app.state()))
        .await
        .unwrap()
}

async fn all(t: &Booted, party: &Party) -> Vec<Screening> {
    let mut rows = Screening::for_party(t.db(), party.id).await.unwrap();
    rows.sort_by_key(|s| s.id);
    rows
}

#[renox::test]
async fn a_run_on_stale_lists_is_flagged_and_records_the_lists() {
    let (t, _feed) = setup(false).await;
    let party = t.party(AL_NOOR).await;
    let s = screening::run(t.app.state(), &party, ScreeningTrigger::Onboarding)
        .await
        .unwrap();
    assert_eq!(s.status, ScreeningStatus::Pending, "no provider");
    assert!(s.lists_stale);
    assert_eq!(s.lists_used.0.len(), 2);
    assert!(
        s.lists_used
            .0
            .iter()
            .all(|l| l.stale && l.version.is_none())
    );
    assert_eq!((s.rerun_of, s.superseded_by), (None, None));
}

#[renox::test]
async fn a_run_on_current_lists_is_not_flagged_and_a_provider_ends_it() {
    let (t, feed) = setup(true).await;
    feed.serve("v1");
    check_lists(&t).await;
    let party = t.party(AL_NOOR).await;
    let s = screening::run(t.app.state(), &party, ScreeningTrigger::Manual)
        .await
        .unwrap();
    assert_eq!(s.status, ScreeningStatus::Clear);
    assert!(!s.lists_stale);
    assert!(s.lists_used.0.iter().all(|l| !l.stale));
    assert!(
        s.lists_used
            .0
            .iter()
            .all(|l| l.version.as_deref() == Some("v1"))
    );
    assert!(s.lists_used.0.iter().all(|l| l.last_updated_at.is_some()));
}

#[renox::test]
async fn a_run_with_screening_off_is_refused() {
    let t = boot_with(WlParties::new(), true).await;
    let party = t.party(AL_NOOR).await;
    assert!(
        screening::run(t.app.state(), &party, ScreeningTrigger::Onboarding)
            .await
            .is_err()
    );
    assert!(all(&t, &party).await.is_empty());
}

#[renox::test]
async fn updates_rerun_a_flagged_screening_until_every_list_is_current() {
    let (t, feed) = setup(true).await;
    let party = t.party(AL_NOOR).await;
    // The first run: both lists stale.
    let first = screening::run(t.app.state(), &party, ScreeningTrigger::Onboarding)
        .await
        .unwrap();
    assert!(first.lists_stale);
    // One feed run updates both lists in turn; the first update still sees
    // the second list stale, so the re-run is flagged again and re-run once
    // more after the second update.
    feed.serve("v1");
    let report = check_lists(&t).await;
    assert_eq!(report.updated, 2);
    let rows = all(&t, &party).await;
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[1].rerun_of, Some(rows[0].id));
    assert_eq!(rows[2].rerun_of, Some(rows[1].id));
    assert_eq!(rows[0].superseded_by, Some(rows[1].id));
    assert_eq!(rows[1].superseded_by, Some(rows[2].id));
    assert!(rows[1].lists_stale && !rows[2].lists_stale);
    assert_eq!(rows[2].superseded_by, None);
    assert!(
        rows[1..]
            .iter()
            .all(|s| s.run_trigger == ScreeningTrigger::ListUpdate)
    );
    assert!(
        rows[2]
            .lists_used
            .0
            .iter()
            .all(|l| l.version.as_deref() == Some("v1"))
    );
    // The same version later: no event, nothing re-run.
    t.app.travel(Duration::from_secs(3600));
    check_lists(&t).await;
    assert_eq!(all(&t, &party).await.len(), 3);
    // A second update for a list re-runs nothing: nothing waits.
    let list = SanctionsList::all(t.db()).await.unwrap().remove(0);
    let report = rescreen::rerun_for_list(t.app.state(), list.id)
        .await
        .unwrap();
    assert_eq!(report, rescreen::Report::default());
}

#[renox::test]
async fn archived_parties_are_skipped_and_a_failure_is_counted_without_stopping() {
    let (t, _feed, fail) = setup_with(true).await;
    let a = t.party(AL_NOOR).await;
    let others = Party::query()
        .where_(Party::ID.ne(a.id))
        .limit(2)
        .get(t.db())
        .await
        .unwrap();
    let (b, c) = (&others[0], &others[1]);
    for party in [&a, b, c] {
        screening::run(t.app.state(), party, ScreeningTrigger::Onboarding)
            .await
            .unwrap();
    }
    renox::db::sql("UPDATE wl_parties_parties SET status = 'archived' WHERE id = ?")
        .bind(a.id)
        .execute(t.db())
        .await
        .unwrap();
    fail.store(true, Ordering::SeqCst);
    let lists = SanctionsList::all(t.db()).await.unwrap();
    let report = rescreen::rerun_for_list(t.app.state(), lists[0].id)
        .await
        .unwrap();
    assert_eq!((report.rerun, report.skipped, report.failed), (0, 1, 2));
    // Nothing was superseded by the failures, so the next update retries.
    fail.store(false, Ordering::SeqCst);
    let report = rescreen::rerun_for_list(t.app.state(), lists[0].id)
        .await
        .unwrap();
    assert_eq!((report.rerun, report.skipped, report.failed), (2, 1, 0));
    assert_eq!(
        all(&t, &a).await.len(),
        1,
        "the archived party is not re-screened"
    );
    assert_eq!(all(&t, b).await.len(), 2);
}
