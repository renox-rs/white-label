//! Sanctions lists and their freshness (wl-parties §3, `PTY.RUL.02`, issue
//! #40).
//!
//! A [`SanctionsList`] records the last successful update and the version of
//! one list. [`is_stale`] decides whether it is stale: its last download
//! failed, it was never updated, or the freshness limit has passed. The
//! limit is elapsed time, so a list updated exactly `limit` hours ago is
//! stale.

use renox::chrono::Duration;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

/// The freshness limit when the app sets none, in hours (wl-parties §7).
pub const DEFAULT_FRESHNESS_HOURS: i64 = 24;

/// Whether a list can be relied on.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ListStatus {
    /// Updated within the freshness limit and the last download worked.
    Current,
    /// Never updated, too old, or the last download failed.
    #[default]
    Stale,
}

/// How the last successful update came in.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum UpdateSource {
    /// The list feed downloaded it.
    #[default]
    Download,
    /// A reviewer uploaded it.
    Manual,
}

/// One sanctions list.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_parties_sanctions_lists", index(status))]
pub struct SanctionsList {
    /// The primary key.
    pub id: i64,
    /// The list's name; unique.
    pub name: String,
    /// Where it comes from.
    pub source: String,
    /// The version last downloaded; `None` before the first download.
    pub version: Option<String>,
    /// When a download last succeeded.
    pub last_updated_at: Option<DateTime>,
    /// How that update came in.
    pub update_source: Option<UpdateSource>,
    /// Whether the list can be relied on.
    pub status: ListStatus,
    /// When the list turned stale; `None` while current.
    pub stale_since: Option<DateTime>,
    /// The last download attempt, successful or not.
    pub last_attempt_at: Option<DateTime>,
    /// Why the last attempt failed; `None` after a success.
    pub last_error: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl SanctionsList {
    /// All lists, by name.
    pub async fn all<'c>(db: impl renox::db::Executor<'c>) -> Result<Vec<SanctionsList>> {
        SanctionsList::query()
            .order_by(SanctionsList::NAME)
            .get(db)
            .await
    }

    /// Whether this list is stale at `now` under a limit of `limit_hours`.
    pub fn is_stale_at(&self, now: DateTime, limit_hours: i64) -> bool {
        is_stale(
            self.last_updated_at,
            self.last_error.is_some(),
            now,
            limit_hours,
        )
    }
}

/// Whether a list is stale at `now`: its last download failed, it has no
/// successful update, or `limit_hours` have passed since the last
/// successful update (exactly the limit counts).
pub fn is_stale(
    last_updated_at: Option<DateTime>,
    last_download_failed: bool,
    now: DateTime,
    limit_hours: i64,
) -> bool {
    if last_download_failed {
        return true;
    }
    match last_updated_at {
        None => true,
        Some(updated) => now - updated >= Duration::hours(limit_hours),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(hours: i64) -> DateTime {
        renox::chrono::DateTime::from_timestamp(1_800_000_000, 0).unwrap() + Duration::hours(hours)
    }

    #[test]
    fn a_list_never_updated_is_stale() {
        assert!(is_stale(None, false, at(0), 24));
    }

    #[test]
    fn a_list_updated_within_the_limit_is_current() {
        assert!(!is_stale(Some(at(0)), false, at(23), 24));
        assert!(!is_stale(Some(at(0)), false, at(0), 24));
    }

    #[test]
    fn exactly_the_limit_is_stale() {
        assert!(is_stale(Some(at(0)), false, at(24), 24));
        assert!(is_stale(Some(at(0)), false, at(30), 24));
    }

    #[test]
    fn a_failed_download_is_stale_whatever_the_age() {
        assert!(is_stale(Some(at(0)), true, at(1), 24));
    }

    #[test]
    fn the_limit_is_the_settings() {
        assert!(!is_stale(Some(at(0)), false, at(47), 48));
        assert!(is_stale(Some(at(0)), false, at(48), 48));
    }
}
