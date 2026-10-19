//! The sales contract (wl-sales §6): what other modules may know about
//! campaigns without depending on the `wl-sales` crate.
//!
//! `wl-sales` emits [`CampaignActivated`] and [`CampaignEnded`]; `wl-notify`
//! and the verticals may listen. Campaigns are referred to by [`CampaignId`]
//! or as a [`Subject`] of type [`CAMPAIGN_SUBJECT_TYPE`].
//!
//! The purpose and the marketing channel are the stored names of the
//! `wl-sales` enums (e.g. `acquisition`, `meta`), so the kernel holds no copy
//! of those lists.

use std::fmt;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::subject::Subject;

/// The subject type of a campaign, for [`Subject`] references.
pub const CAMPAIGN_SUBJECT_TYPE: &str = "sales.campaign";

/// The subject type of a campaign cost entry, for [`Subject`] references.
pub const CAMPAIGN_COST_ENTRY_SUBJECT_TYPE: &str = "sales.campaign_cost_entry";

/// The id of a campaign.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct CampaignId(pub i64);

impl CampaignId {
    /// The campaign as a [`Subject`]: `sales.campaign:12`.
    pub fn subject(self) -> Subject {
        Subject::new(CAMPAIGN_SUBJECT_TYPE, self.0)
    }
}

impl fmt::Display for CampaignId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<i64> for CampaignId {
    fn from(id: i64) -> Self {
        CampaignId(id)
    }
}

/// A campaign became active (`activate`, by hand or on its start date).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampaignActivated {
    /// The campaign.
    pub campaign: CampaignId,
    /// `acquisition` or `retention`.
    pub purpose: String,
    /// The marketing channel's stored name.
    pub marketing_channel: String,
    /// The first day.
    pub start_date: NaiveDate,
    /// The last day, if set.
    pub end_date: Option<NaiveDate>,
    /// The budget in fils of AED, if set.
    pub budget: Option<i64>,
}

impl Event for CampaignActivated {}

/// A campaign ended (`end`, by hand or after its end date).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampaignEnded {
    /// The campaign.
    pub campaign: CampaignId,
    /// `acquisition` or `retention`.
    pub purpose: String,
    /// The marketing channel's stored name.
    pub marketing_channel: String,
    /// The first day.
    pub start_date: NaiveDate,
    /// The last day, if set.
    pub end_date: Option<NaiveDate>,
    /// The budget in fils of AED, if set.
    pub budget: Option<i64>,
}

impl Event for CampaignEnded {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_campaign_id_is_a_subject_and_a_plain_number() {
        let id = CampaignId::from(12);
        assert_eq!(id.to_string(), "12");
        assert_eq!(id.subject(), Subject::new("sales.campaign", 12));
        assert_eq!(renox::serde_json::to_string(&id).unwrap(), "12");
        assert_eq!(CampaignId::default(), CampaignId(0));
    }

    #[test]
    fn the_subject_types_are_stable() {
        assert_eq!(CAMPAIGN_SUBJECT_TYPE, "sales.campaign");
        assert_eq!(
            CAMPAIGN_COST_ENTRY_SUBJECT_TYPE,
            "sales.campaign_cost_entry"
        );
    }

    #[test]
    fn the_campaign_events_round_trip_through_json() {
        let day = |m, d| NaiveDate::from_ymd_opt(2026, m, d).unwrap();
        let activated = CampaignActivated {
            campaign: CampaignId(1),
            purpose: "acquisition".into(),
            marketing_channel: "meta".into(),
            start_date: day(3, 1),
            end_date: Some(day(3, 31)),
            budget: Some(500_000),
        };
        let json = renox::serde_json::to_string(&activated).unwrap();
        assert_eq!(
            renox::serde_json::from_str::<CampaignActivated>(&json).unwrap(),
            activated
        );
        let ended = CampaignEnded {
            campaign: CampaignId(1),
            purpose: "acquisition".into(),
            marketing_channel: "meta".into(),
            start_date: day(3, 1),
            end_date: None,
            budget: None,
        };
        let json = renox::serde_json::to_string(&ended).unwrap();
        assert_eq!(
            renox::serde_json::from_str::<CampaignEnded>(&json).unwrap(),
            ended
        );
    }
}
