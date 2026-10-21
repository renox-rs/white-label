//! The broadcast record (wl-notify §3 *Broadcast*, #61 A3): one marketing
//! message to a segment, asked for by a campaign. Its status moves only
//! through the machine in [`crate::broadcast_machine`].

use std::collections::BTreeMap;
use std::sync::Arc;

use renox::db::Json;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::broadcast::BroadcastId;
use wl_kernel::notify::NotifyChannel;
use wl_kernel::{SegmentId, Subject};

/// Where a broadcast is in its lifecycle (wl-notify §4.3).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BroadcastStatus {
    /// Recorded, not yet requested: the state a new row starts in.
    #[default]
    Draft,
    /// Above the size limit; waiting for a second holder of
    /// `notifications.marketing.send`.
    PendingApproval,
    /// Asked for (and approved when needed); goes out at its send time.
    Scheduled,
    /// Queueing its messages.
    Sending,
    /// Every recipient has a message or an exclusion.
    Completed,
    /// Rejected, withdrawn, cancelled, expired or its requester ended.
    Cancelled,
}

/// The members left out of a broadcast, counted by reason (`no_consent`,
/// `opted_out`, `blocked_party`, ...). It holds no names.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExcludedCounts(pub BTreeMap<String, i64>);

impl ExcludedCounts {
    /// Counts one more member left out for `reason`.
    pub fn add(&mut self, reason: &str) {
        *self.0.entry(reason.to_owned()).or_insert(0) += 1;
    }

    /// How many were left out for `reason`.
    pub fn of(&self, reason: &str) -> i64 {
        self.0.get(reason).copied().unwrap_or(0)
    }

    /// How many were left out in all.
    pub fn total(&self) -> i64 {
        self.0.values().sum()
    }
}

/// How the messages of a broadcast ended up, computed from the messages.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BroadcastResults {
    /// Accepted by the provider (delivered and read ones included).
    #[serde(default)]
    pub sent: i64,
    /// Delivered to the device or mailbox.
    #[serde(default)]
    pub delivered: i64,
    /// Failed or abandoned.
    #[serde(default)]
    pub failed: i64,
    /// Recipients who opted out of this broadcast.
    #[serde(default)]
    pub opted_out: i64,
}

/// One marketing broadcast.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_notify_broadcasts",
    state_machine,
    index(status, send_at),
    index(requester_type, requester_id)
)]
pub struct Broadcast {
    /// The primary key.
    pub id: i64,
    /// The notification type's key (category `marketing`).
    pub type_key: String,
    /// The type of the requester, e.g. a campaign.
    pub requester_type: String,
    /// The id of the requester.
    pub requester_id: i64,
    /// The user who asked.
    pub requested_by: i64,
    /// The channel it goes out on.
    pub channel: NotifyChannel,
    /// The audience.
    pub segment_id: i64,
    /// When to send.
    pub send_at: DateTime,
    /// The recipients who would pass the checks when it was asked.
    pub estimate: i64,
    /// The recipients resolved when it started.
    pub recipients_resolved: Option<i64>,
    /// The members left out, by reason.
    pub excluded: Json<ExcludedCounts>,
    /// The results, once there are messages.
    pub results: Json<BroadcastResults>,
    /// The user who gave the last yes, when approval was needed.
    pub approved_by: Option<i64>,
    /// The kernel approval request, when approval was needed.
    pub approval_request_id: Option<i64>,
    /// Why it was cancelled (the approver's comment, or `cancelled`,
    /// `approval_expired`, `requester_guard_failed`).
    pub cancel_reason: Option<String>,
    /// When it started sending.
    pub started_at: Option<DateTime>,
    /// When it completed.
    pub completed_at: Option<DateTime>,
    /// The user who stopped it (`NTF.RUL.03`); the state stays.
    pub stopped_by: Option<i64>,
    /// When it was stopped.
    pub stopped_at: Option<DateTime>,
    /// Bumped to take the broadcast lock when it starts.
    pub lock_version: i64,
    /// The lifecycle state; changed only by a transition.
    pub status: BroadcastStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Broadcast {
    /// The kernel's id of this broadcast.
    pub fn broadcast_id(&self) -> BroadcastId {
        BroadcastId(self.id)
    }

    /// The requester as a [`Subject`].
    pub fn requester(&self) -> Subject {
        Subject::new(&self.requester_type, self.requester_id)
    }

    /// The audience.
    pub fn segment(&self) -> SegmentId {
        SegmentId(self.segment_id)
    }
}

/// The estimate above which a broadcast needs approval, unless set.
pub const DEFAULT_APPROVAL_ABOVE: i64 = 200;

/// The module's broadcast settings (#61 A6): client defaults kept in code,
/// set on [`WlNotify`](crate::WlNotify) with builder calls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BroadcastSettings {
    /// A broadcast whose estimate is above this needs approval.
    pub approval_above: i64,
    /// The registered alphanumeric sender ID of marketing SMS, `AD-...`.
    pub sms_sender_id: Option<String>,
}

impl Default for BroadcastSettings {
    fn default() -> Self {
        BroadcastSettings {
            approval_above: DEFAULT_APPROVAL_ABOVE,
            sms_sender_id: None,
        }
    }
}

impl BroadcastSettings {
    /// Whether a broadcast of `estimate` recipients needs approval.
    pub fn needs_approval(&self, estimate: i64) -> bool {
        estimate > self.approval_above
    }

    /// The SMS sender ID when it is a valid advertising one (`AD-` and a
    /// name, 11 characters at most in all, letters and digits).
    pub fn valid_sms_sender_id(&self) -> Option<&str> {
        self.sms_sender_id.as_deref().filter(|id| {
            id.len() <= 11
                && id.len() > 3
                && id.starts_with("AD-")
                && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
    }
}

/// The app's broadcast settings, or the defaults when none are provided.
pub fn settings(state: &AppState) -> Arc<BroadcastSettings> {
    state
        .provided::<BroadcastSettings>()
        .unwrap_or_else(|| Arc::new(BroadcastSettings::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_limit_is_200_and_approval_is_needed_only_above_it() {
        let settings = BroadcastSettings::default();
        assert!(!settings.needs_approval(200));
        assert!(settings.needs_approval(201));
    }

    #[test]
    fn only_an_ad_sender_id_of_11_characters_or_less_is_valid() {
        let with = |id: Option<&str>| BroadcastSettings {
            sms_sender_id: id.map(str::to_owned),
            ..Default::default()
        };
        assert_eq!(
            with(Some("AD-SCHOOL")).valid_sms_sender_id(),
            Some("AD-SCHOOL")
        );
        assert_eq!(with(Some("SCHOOL")).valid_sms_sender_id(), None);
        assert_eq!(with(Some("AD-")).valid_sms_sender_id(), None);
        assert_eq!(with(Some("AD-TOOLONGNAME")).valid_sms_sender_id(), None);
        assert_eq!(with(None).valid_sms_sender_id(), None);
    }

    #[test]
    fn excluded_counts_add_by_reason_and_hold_no_names() {
        let mut counts = ExcludedCounts::default();
        counts.add("no_consent");
        counts.add("no_consent");
        counts.add("opted_out");
        assert_eq!(counts.of("no_consent"), 2);
        assert_eq!(counts.of("blocked_party"), 0);
        assert_eq!(counts.total(), 3);
        assert_eq!(
            renox::serde_json::to_string(&counts).unwrap(),
            r#"{"no_consent":2,"opted_out":1}"#
        );
    }

    #[test]
    fn results_read_back_with_missing_fields() {
        let results: BroadcastResults = renox::serde_json::from_str("{}").unwrap();
        assert_eq!(results, BroadcastResults::default());
    }
}
