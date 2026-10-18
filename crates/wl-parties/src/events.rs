//! The events of the sanctions list check (wl-parties §3, `PTY.RUL.02`,
//! issue #40).
//!
//! Both are defined here, not in the kernel, so only compliance-side code
//! (which depends on this crate) can listen: other modules read staleness
//! through the onboarding guard instead.

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

/// A sanctions list turned stale: once per stale period.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SanctionsListStale {
    /// The list's id.
    pub list: i64,
    /// The list's name.
    pub name: String,
    /// When a download last succeeded; `None` if it never did.
    pub last_updated_at: Option<DateTime>,
    /// Why the last download failed; `None` when the list is only old.
    pub last_error: Option<String>,
}

impl Event for SanctionsListStale {}

/// A sanctions list got a new version. A download of the version already
/// held does not emit it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SanctionsListUpdated {
    /// The list's id.
    pub list: i64,
    /// The list's name.
    pub name: String,
    /// The new version.
    pub version: String,
}

impl Event for SanctionsListUpdated {}
