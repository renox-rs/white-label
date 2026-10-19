//! The list feed: how a sanctions list is downloaded (wl-parties §3,
//! `PTY.RUL.02`, issue #40).
//!
//! The module does not download anything itself. An app provides a
//! [`ListFeed`] with `WlParties::new().list_feed(..)`; without one,
//! [`NoListFeed`] fails every fetch with [`NO_FEED`], so a screening app
//! without a feed shows its lists as stale and alerts, and never passes a
//! list silently.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::db::DateTime;
use renox::prelude::*;

use crate::sanctions_lists::SanctionsList;

/// The error of the default feed.
pub const NO_FEED: &str = "no list feed configured";

/// What a successful download reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListVersion {
    /// The version the publisher gives the list.
    pub version: String,
    /// When the publisher published it.
    pub published_at: Option<DateTime>,
}

/// A download that failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedError(pub String);

impl fmt::Display for FeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for FeedError {}

/// The future of a fetch.
pub type FetchFuture<'a> =
    Pin<Box<dyn Future<Output = std::result::Result<ListVersion, FeedError>> + Send + 'a>>;

/// Downloads a sanctions list and reports the version it got.
pub trait ListFeed: Send + Sync + 'static {
    /// Downloads `list`. The default fails with [`NO_FEED`].
    fn fetch<'a>(&'a self, list: &'a SanctionsList) -> FetchFuture<'a> {
        let _ = list;
        Box::pin(async { Err(FeedError(NO_FEED.to_owned())) })
    }
}

impl fmt::Debug for dyn ListFeed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ListFeed")
    }
}

/// The feed an app gets when it provides none: every fetch fails.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoListFeed;

impl ListFeed for NoListFeed {}

/// The app's feed, or [`NoListFeed`].
pub fn feed(state: &AppState) -> Arc<dyn ListFeed> {
    match state.provided::<Arc<dyn ListFeed>>() {
        Some(feed) => Arc::clone(&feed),
        None => Arc::new(NoListFeed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[renox::test]
    async fn the_default_feed_fails() {
        let list = SanctionsList::default();
        let result = NoListFeed.fetch(&list).await;
        assert_eq!(result, Err(FeedError(NO_FEED.to_owned())));
    }
}
