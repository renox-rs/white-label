//! The module's `ScreeningGuard` (#41 part 2): stale evaluation from the
//! latest screening and the lists, and the provider registered only when
//! screening is on. The full acceptance checks are in `stale_onboarding.rs`.

mod support;

use std::sync::{Arc, Mutex};

use renox::prelude::Model;
use wl_kernel::PartyId;
use wl_kernel::screening_guard::{ScreeningOutcome, StaleListPolicy, screening_guard};
use wl_parties::feed::{FeedError, FetchFuture};
use wl_parties::screening::{self, CheckFuture};
use wl_parties::{
    ListFeed, ListUsed, ListVersion, NewCandidate, Party, ProviderError, SanctionsList,
    ScreeningProvider, ScreeningTrigger, WlParties, freshness, seed,
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

struct NoMatches;

impl ScreeningProvider for NoMatches {
    fn check<'a>(&'a self, _party: &'a Party, _lists: &'a [ListUsed]) -> CheckFuture<'a> {
        Box::pin(async { Ok::<Vec<NewCandidate>, ProviderError>(Vec::new()) })
    }
}

async fn setup(policy: StaleListPolicy) -> (Booted, FakeFeed, Party) {
    let feed = FakeFeed::default();
    let module = WlParties::new()
        .screening(true)
        .list_feed(feed.clone())
        .screening_provider(NoMatches)
        .stale_list_at_onboarding(policy);
    let t = boot_with(module, true).await;
    seed::lists(t.app.state()).await.unwrap();
    let party = t.party(AL_NOOR).await;
    (t, feed, party)
}

async fn check(t: &Booted, party: &Party) -> ScreeningOutcome {
    let mut tx = t.app.state().db.begin().await.unwrap();
    screening_guard(t.app.state())
        .check_in(&mut tx, PartyId(party.id))
        .await
        .unwrap()
}

async fn notice(t: &Booted, party: &Party) -> usize {
    let mut tx = t.app.state().db.begin().await.unwrap();
    screening_guard(t.app.state())
        .latest_notice_in(&mut tx, PartyId(party.id))
        .await
        .unwrap()
        .len()
}

#[renox::test]
async fn warn_names_the_stale_lists_and_never_refuses() {
    let (t, _feed, party) = setup(StaleListPolicy::Warn).await;
    let outcome = check(&t, &party).await;
    let lists = outcome.warned();
    assert_eq!(lists.len(), 2, "{outcome:?}");
    assert!(lists.iter().all(|l| l.list_date.is_none()));
    assert!(lists.iter().any(|l| l.name == "UN Consolidated List"));
    // Nothing was written: the party has no screening and no notice.
    assert_eq!(notice(&t, &party).await, 0);
}

#[renox::test]
async fn block_refuses_naming_the_lists_until_they_are_current_and_rerun() {
    let (t, feed, party) = setup(StaleListPolicy::Block).await;
    let outcome = check(&t, &party).await;
    let ScreeningOutcome::Blocked(refusal) = outcome else {
        panic!("expected a refusal, got {outcome:?}");
    };
    assert_eq!(refusal.code(), "stale_list");
    assert!(refusal.message_en().contains("UN Consolidated List"));

    // A screening on the stale lists is flagged; its notice shows.
    screening::run(t.app.state(), &party, ScreeningTrigger::Onboarding)
        .await
        .unwrap();
    assert_eq!(notice(&t, &party).await, 2);

    // The lists update; the re-run is not flagged, so the notice is gone and
    // the party passes.
    feed.serve("v1");
    t.app
        .at_travelled_time(freshness::run(t.app.state()))
        .await
        .unwrap();
    assert_eq!(notice(&t, &party).await, 0);
    assert_eq!(check(&t, &party).await, ScreeningOutcome::Allowed);
}

#[renox::test]
async fn a_flagged_latest_screening_warns_even_when_the_lists_are_current() {
    let (t, feed, party) = setup(StaleListPolicy::Warn).await;
    screening::run(t.app.state(), &party, ScreeningTrigger::Onboarding)
        .await
        .unwrap();
    feed.serve("v1");
    // Update the lists without the re-run: only the lists change.
    for list in SanctionsList::all(t.db()).await.unwrap() {
        let mut list = list;
        list.last_updated_at = Some(renox::db::now());
        list.status = wl_parties::ListStatus::Current;
        list.last_error = None;
        list.save(t.db()).await.unwrap();
    }
    let outcome = check(&t, &party).await;
    assert_eq!(outcome.warned().len(), 2, "{outcome:?}");
}

#[renox::test]
async fn with_screening_off_the_guard_is_the_no_op() {
    let t = boot_with(
        WlParties::new().stale_list_at_onboarding(StaleListPolicy::Block),
        true,
    )
    .await;
    seed::lists(t.app.state()).await.unwrap();
    let party = t.party(AL_NOOR).await;
    assert_eq!(check(&t, &party).await, ScreeningOutcome::Allowed);
    assert_eq!(notice(&t, &party).await, 0);
}
