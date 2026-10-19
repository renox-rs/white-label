//! The segment contract (wl-notify §4.3): who is in a named audience, without
//! `wl-notify` depending on the module that owns the segments (`wl-sales`).
//!
//! The module that owns segments provides a [`Segments`] with
//! [`provide_segments`]; `wl-notify` resolves a broadcast's audience through
//! [`segments`]. Without a provider, [`NoSegments`] answers that every
//! segment is empty, so a broadcast reaches nobody.
//!
//! A segment is resolved when asked, never cached: the estimate at the
//! request and the list at the send time may differ.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::parties::PartyId;

/// The id of a segment of the module that owns segments.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct SegmentId(pub i64);

impl fmt::Display for SegmentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<i64> for SegmentId {
    fn from(id: i64) -> Self {
        SegmentId(id)
    }
}

/// The future a [`Segments`] method returns.
pub type SegmentFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Resolves segments to parties.
pub trait Segments: Send + Sync + 'static {
    /// The parties in `segment` as of `at`, each once, in a stable order.
    /// An unknown segment has no members. It lists parties only: whether
    /// they may be written to is the caller's check.
    fn members<'a>(
        &'a self,
        db: &'a Db,
        segment: SegmentId,
        at: DateTime,
    ) -> SegmentFuture<'a, Vec<PartyId>>;
}

/// The segments when no module provides them: every segment is empty.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoSegments;

impl Segments for NoSegments {
    fn members<'a>(
        &'a self,
        _db: &'a Db,
        _segment: SegmentId,
        _at: DateTime,
    ) -> SegmentFuture<'a, Vec<PartyId>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// Makes `segments` the app's [`Segments`]; the module that owns segments
/// calls it in `register`.
pub fn provide_segments(app: &mut Registry, segments: impl Segments) {
    let segments: Arc<dyn Segments> = Arc::new(segments);
    app.provide(segments);
}

/// The app's [`Segments`], or [`NoSegments`] when none is provided.
pub fn segments(state: &AppState) -> Arc<dyn Segments> {
    match state.provided::<Arc<dyn Segments>>() {
        Some(segments) => Arc::clone(&segments),
        None => Arc::new(NoSegments),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_segment_id_displays_and_converts() {
        assert_eq!(SegmentId::from(7).to_string(), "7");
        assert_eq!(SegmentId::default(), SegmentId(0));
    }

    #[test]
    fn a_segment_id_is_a_plain_number_in_json() {
        let json = renox::serde_json::to_string(&SegmentId(12)).unwrap();
        assert_eq!(json, "12");
        let back: SegmentId = renox::serde_json::from_str("12").unwrap();
        assert_eq!(back, SegmentId(12));
    }
}
