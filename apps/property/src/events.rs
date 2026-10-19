//! The events the vertical emits for the client's own integrations and for
//! the other modules (01 Property §7.1; `PLT.HOK.01`). Emitted after the
//! commit of the transition they follow.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;

use crate::lease::VersionKind;

/// A lease version was activated (`activate`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseActivated {
    /// The version that was activated.
    pub lease: i64,
    /// The lease all versions belong to.
    pub root: i64,
    /// The version number.
    pub version: i32,
    /// New, renewal or amendment.
    pub kind: VersionKind,
    /// The unit.
    pub unit: i64,
    /// The building.
    pub building: i64,
    /// The tenant.
    pub tenant: PartyId,
    /// The first day of the term.
    pub start_date: NaiveDate,
    /// The last day of the term.
    pub end_date: NaiveDate,
    /// The annual rent, in fils.
    pub annual_rent: i64,
}

impl Event for LeaseActivated {}

/// A lease version was cancelled and its money effects ran (`cancel`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseCancelled {
    /// The version.
    pub lease: i64,
    /// The lease all versions belong to.
    pub root: i64,
    /// The tenant.
    pub tenant: PartyId,
    /// Why.
    pub reason: String,
    /// The part of the deposit forfeited, in fils.
    pub forfeit: i64,
    /// Whether any money or cheque had been received.
    pub money_received: bool,
}

impl Event for LeaseCancelled {}

/// A notice to end the lease was withdrawn (`withdraw_notice`, `PRP.ACL.20`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseNoticeWithdrawn {
    /// The version.
    pub lease: i64,
    /// The lease all versions belong to.
    pub root: i64,
    /// The tenant.
    pub tenant: PartyId,
    /// The state the dates gave: `active`, `renewal_due` or `expired`.
    pub status: String,
}

impl Event for LeaseNoticeWithdrawn {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_events_serialize_with_plain_ids() {
        let event = LeaseActivated {
            lease: 4,
            root: 2,
            version: 2,
            kind: VersionKind::Renewal,
            unit: 9,
            building: 1,
            tenant: PartyId(7),
            start_date: NaiveDate::from_ymd_opt(2027, 1, 1).unwrap(),
            end_date: NaiveDate::from_ymd_opt(2027, 12, 31).unwrap(),
            annual_rent: 9_000_000,
        };
        let json = renox::serde_json::to_value(&event).unwrap();
        assert_eq!(json["tenant"], 7);
        assert_eq!(json["kind"], "renewal");
        assert_eq!(json["start_date"], "2027-01-01");
    }
}
