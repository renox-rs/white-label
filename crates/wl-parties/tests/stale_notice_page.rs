//! The stale sanctions list line on the party page (#41 part 3): shown to
//! holders of a screening permission, hidden for everyone else.

mod support;

use std::sync::{Arc, Mutex};

use wl_parties::feed::{FeedError, FetchFuture};
use wl_parties::screening::{self, CheckFuture};
use wl_parties::{
    ListFeed, ListUsed, ListVersion, NewCandidate, Party, ProviderError, SanctionsList,
    ScreeningProvider, ScreeningTrigger, WlParties, permissions as perm, seed,
};

use support::{Booted, boot_with};

const AL_NOOR: &str = "Al Noor Trading LLC";
const NOTICE: &str = "screened on a stale list";

#[derive(Clone, Default)]
struct FakeFeed(Arc<Mutex<Option<String>>>);

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

async fn setup() -> (Booted, Party) {
    let module = WlParties::new()
        .screening(true)
        .list_feed(FakeFeed::default())
        .screening_provider(NoMatches);
    let t = boot_with(module, true).await;
    seed::lists(t.app.state()).await.unwrap();
    let party = t.party(AL_NOOR).await;
    (t, party)
}

#[renox::test]
async fn permitted_viewers_see_the_line_and_others_do_not() {
    let (t, party) = setup().await;
    let page = format!("/parties/{}", party.id);
    // Screened while the seeded lists are stale: the latest screening is flagged.
    screening::run(t.app.state(), &party, ScreeningTrigger::Onboarding)
        .await
        .unwrap();

    for (name, permission) in [
        ("Runner", perm::SCREENING_RUN),
        ("Reviewer", perm::SCREENING_REVIEW),
        ("Officer", perm::BLOCK_REASON_VIEW),
    ] {
        let viewer = t.user(name, &[perm::VIEW, permission]).await;
        t.app.acting_as(&viewer);
        t.app
            .get(&page)
            .await
            .assert_ok()
            .assert_see(NOTICE)
            .assert_see("UN Consolidated List");
    }

    let staff = t.user("Sami", &[perm::VIEW, perm::UPDATE]).await;
    t.app.acting_as(&staff);
    t.app
        .get(&page)
        .await
        .assert_ok()
        .assert_dont_see(NOTICE)
        .assert_dont_see("UN Consolidated List");
}

#[renox::test]
async fn no_line_without_a_flagged_screening() {
    let (t, party) = setup().await;
    let viewer = t
        .user("Compliance", &[perm::VIEW, perm::SCREENING_RUN])
        .await;
    t.app.acting_as(&viewer);
    t.app
        .get(&format!("/parties/{}", party.id))
        .await
        .assert_ok()
        .assert_dont_see(NOTICE);
}
