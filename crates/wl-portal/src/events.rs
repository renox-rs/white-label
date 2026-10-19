//! The events of the access grant lifecycle (wl-portal §6). Their payloads
//! carry plain ids, so a listener need not depend on `wl-portal`.

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;

/// A grant became `active` (`accept`, or a self-registration).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortalAccessActivated {
    /// The grant.
    pub grant: i64,
    /// The portal user.
    pub user: i64,
    /// The party the user acts for.
    pub party: PartyId,
}

impl Event for PortalAccessActivated {}

/// Staff suspended a grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortalAccessSuspended {
    /// The grant.
    pub grant: i64,
    /// Why.
    pub reason: String,
}

impl Event for PortalAccessSuspended {}

/// A grant became `read_only` (`end`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortalAccessReadOnly {
    /// The grant.
    pub grant: i64,
    /// Why, when given.
    pub reason: Option<String>,
}

impl Event for PortalAccessReadOnly {}

/// A grant was closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortalAccessClosed {
    /// The grant.
    pub grant: i64,
    /// Why, when given.
    pub reason: Option<String>,
}

impl Event for PortalAccessClosed {}
