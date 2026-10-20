//! The sanctions list download reports to the integration health (wl-parties
//! §3 `PTY.RUL.02`, wl-kernel integrations): a failed download degrades the
//! connection the app registered under `SANCTIONS_DOWNLOAD`, a good one brings
//! it back `up`. Its own test file: the registry of connections is process
//! wide.

mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use wl_kernel::integrations::SANCTIONS_DOWNLOAD;
use wl_kernel::{ConnectionStatus, Delivery, IntegrationConnection, IntegrationDef};
use wl_parties::feed::{FeedError, FetchFuture};
use wl_parties::{ListFeed, ListVersion, SanctionsList, WlParties, freshness, seed};

use support::{Booted, boot_with};

/// A feed that answers what the test sets.
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

async fn status(t: &Booted) -> Option<ConnectionStatus> {
    IntegrationConnection::by_key(t.db(), SANCTIONS_DOWNLOAD)
        .await
        .unwrap()
        .map(|connection| connection.status)
}

async fn run(t: &Booted) {
    t.app
        .at_travelled_time(freshness::run(t.app.state()))
        .await
        .unwrap();
}

#[renox::test]
async fn a_failed_download_degrades_the_connection_and_a_good_one_recovers_it() {
    IntegrationDef::of(SANCTIONS_DOWNLOAD, "parties", |_state, _call| {
        Box::pin(async { Ok(Delivery::Sent) })
    })
    .fallback("Screen new clients by hand.", None)
    .register();
    let feed = FakeFeed::default();
    let t = boot_with(
        WlParties::new().screening(true).list_feed(feed.clone()),
        false,
    )
    .await;
    seed::lists(t.app.state()).await.unwrap();

    run(&t).await;
    assert_eq!(status(&t).await, Some(ConnectionStatus::Degraded));

    *feed.0.lock().unwrap() = Some("v1".to_owned());
    t.app.travel(Duration::from_secs(3600));
    run(&t).await;
    assert_eq!(status(&t).await, Some(ConnectionStatus::Up));
}
