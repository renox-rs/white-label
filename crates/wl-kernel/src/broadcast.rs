//! The broadcast contract (wl-notify §4.3, §6): a marketing message to a
//! segment, asked for by a campaign.
//!
//! A module asks `notify::notifier(&state).broadcast(&state, request)` with a
//! [`BroadcastRequest`]. `wl-notify` owns the broadcast, its estimate, its
//! approval above a size and the send. Who may ask is decided by the module
//! that owns the requester (a campaign in `wl-sales`): it provides a
//! [`BroadcastRequesters`] with [`provide_requesters`], and `wl-notify` asks
//! it at the request and again at the send time. Without a provider,
//! [`NoRequesters`] answers [`RequesterCheck::Ended`], which refuses every
//! marketing request (`NTF.ACL.04`: nobody starts a broadcast outside a
//! campaign).
//!
//! `wl-notify` emits [`BroadcastCompleted`] and [`BroadcastCancelled`], so the
//! requester's module can listen. Neither carries a party name.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::notify::NotifyChannel;
use crate::segments::SegmentId;
use crate::subject::Subject;

/// The subject type of a broadcast, for [`Subject`] references.
pub const BROADCAST_SUBJECT_TYPE: &str = "notify.broadcast";

/// The id of a broadcast `wl-notify` recorded.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct BroadcastId(pub i64);

impl BroadcastId {
    /// The broadcast as a [`Subject`]: `notify.broadcast:42`.
    pub fn subject(self) -> Subject {
        Subject::new(BROADCAST_SUBJECT_TYPE, self.0)
    }
}

impl fmt::Display for BroadcastId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<i64> for BroadcastId {
    fn from(id: i64) -> Self {
        BroadcastId(id)
    }
}

/// A request to send one marketing message to a segment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BroadcastRequest {
    /// The notification type key; its category must be `marketing`. The
    /// template is the type's one for `channel` and the recipient's language.
    pub type_key: String,
    /// The requester, e.g. a campaign. It must be active.
    pub requester: Subject,
    /// The user who asks. Required: a broadcast is never anonymous, and the
    /// approver may not be this user.
    pub requested_by: i64,
    /// The channel it goes out on.
    pub channel: NotifyChannel,
    /// The audience.
    pub segment: SegmentId,
    /// When to send.
    pub send_at: DateTime,
}

impl BroadcastRequest {
    /// A request by user `requested_by` for `requester` to send `type_key`
    /// to `segment` on `channel` at `send_at`.
    pub fn new(
        type_key: &str,
        requester: Subject,
        requested_by: i64,
        channel: NotifyChannel,
        segment: SegmentId,
        send_at: DateTime,
    ) -> Self {
        BroadcastRequest {
            type_key: type_key.to_owned(),
            requester,
            requested_by,
            channel,
            segment,
            send_at,
        }
    }
}

/// Whether a requester may still start a broadcast.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequesterCheck {
    /// It is active.
    Active,
    /// It ended, was cancelled or does not exist.
    Ended,
    /// A guard of its module refuses the send (e.g. a missing permit); the
    /// reason is a code for the requester, never a party name.
    GuardFailed(String),
}

impl RequesterCheck {
    /// Whether the broadcast may go on.
    pub fn is_active(&self) -> bool {
        matches!(self, RequesterCheck::Active)
    }
}

/// The future a [`BroadcastRequesters`] method returns.
pub type RequesterFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Checks the requester of a broadcast.
pub trait BroadcastRequesters: Send + Sync + 'static {
    /// Whether `subject` may start a broadcast at `at`.
    fn check<'a>(
        &'a self,
        state: &'a AppState,
        subject: &'a Subject,
        at: DateTime,
    ) -> RequesterFuture<'a, RequesterCheck>;
}

/// The requesters when no module provides them: every requester has ended,
/// so every marketing request is refused.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoRequesters;

impl BroadcastRequesters for NoRequesters {
    fn check<'a>(
        &'a self,
        _state: &'a AppState,
        _subject: &'a Subject,
        _at: DateTime,
    ) -> RequesterFuture<'a, RequesterCheck> {
        Box::pin(async { Ok(RequesterCheck::Ended) })
    }
}

/// Makes `requesters` the app's [`BroadcastRequesters`]; the module that owns
/// campaigns calls it in `register`.
pub fn provide_requesters(app: &mut Registry, requesters: impl BroadcastRequesters) {
    let requesters: Arc<dyn BroadcastRequesters> = Arc::new(requesters);
    app.provide(requesters);
}

/// The app's [`BroadcastRequesters`], or [`NoRequesters`] when none is
/// provided.
pub fn requesters(state: &AppState) -> Arc<dyn BroadcastRequesters> {
    match state.provided::<Arc<dyn BroadcastRequesters>>() {
        Some(requesters) => Arc::clone(&requesters),
        None => Arc::new(NoRequesters),
    }
}

/// Why a broadcast was cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BroadcastCancelReason {
    /// An approver rejected the request.
    #[default]
    Rejected,
    /// The requester or a holder of `notifications.marketing.send` cancelled
    /// it, or the approval request was withdrawn.
    Cancelled,
    /// The approval expired before anyone decided.
    ApprovalExpired,
    /// The requester ended or a guard of its module failed before the send.
    RequesterGuardFailed,
}

impl BroadcastCancelReason {
    /// Every reason, in order.
    pub const ALL: [BroadcastCancelReason; 4] = [
        BroadcastCancelReason::Rejected,
        BroadcastCancelReason::Cancelled,
        BroadcastCancelReason::ApprovalExpired,
        BroadcastCancelReason::RequesterGuardFailed,
    ];

    /// The stored name.
    pub fn as_str(self) -> &'static str {
        match self {
            BroadcastCancelReason::Rejected => "rejected",
            BroadcastCancelReason::Cancelled => "cancelled",
            BroadcastCancelReason::ApprovalExpired => "approval_expired",
            BroadcastCancelReason::RequesterGuardFailed => "requester_guard_failed",
        }
    }
}

/// A broadcast finished sending: every recipient has a message or an
/// exclusion. Emitted once per broadcast.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BroadcastCompleted {
    /// The broadcast.
    pub broadcast: BroadcastId,
    /// Its requester.
    pub requester: Subject,
    /// The channel it went out on.
    pub channel: NotifyChannel,
    /// The messages queued.
    pub recipients: i64,
    /// The members left out, counted by reason (`no_consent`, `opted_out`,
    /// `suppressed`, `blocked_party`, `no_contact_point`). No names.
    pub excluded: BTreeMap<String, i64>,
}

impl Event for BroadcastCompleted {}

/// A broadcast was cancelled and nothing more will be sent. Emitted once per
/// broadcast.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BroadcastCancelled {
    /// The broadcast.
    pub broadcast: BroadcastId,
    /// Its requester.
    pub requester: Subject,
    /// Why.
    pub reason: BroadcastCancelReason,
}

impl Event for BroadcastCancelled {}

/// The 422 of a broadcast request while the notify module is off.
pub(crate) fn broadcasts_off() -> Error {
    Error::Status(
        StatusCode::UNPROCESSABLE_ENTITY,
        "broadcasts need the notify module".to_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_broadcast_is_a_subject() {
        assert_eq!(BroadcastId(42).subject().to_string(), "notify.broadcast:42");
    }

    #[test]
    fn only_active_lets_a_broadcast_go_on() {
        assert!(RequesterCheck::Active.is_active());
        assert!(!RequesterCheck::Ended.is_active());
        assert!(!RequesterCheck::GuardFailed("permit".into()).is_active());
    }

    #[test]
    fn cancel_reasons_have_stable_names() {
        let names: Vec<_> = BroadcastCancelReason::ALL
            .iter()
            .map(|r| r.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "rejected",
                "cancelled",
                "approval_expired",
                "requester_guard_failed"
            ]
        );
        let json = renox::serde_json::to_string(&BroadcastCancelReason::ApprovalExpired).unwrap();
        assert_eq!(json, "\"approval_expired\"");
    }

    #[test]
    fn the_off_fallback_is_a_422() {
        match broadcasts_off() {
            Error::Status(code, message) => {
                assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
                assert!(message.contains("notify module"), "{message}");
            }
            other => panic!("unexpected error {other:?}"),
        }
    }
}
