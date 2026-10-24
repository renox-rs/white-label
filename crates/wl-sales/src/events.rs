//! The campaign events (wl-sales §6, #138 A7). A listener on the machine's
//! `Transitioned<Campaign>`, which the engine emits after the commit (by
//! hand or by the daily run alike), emits [`CampaignActivated`] or
//! [`CampaignEnded`] from the committed row, and on `end` tells the owner.

use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::sales::{CampaignActivated, CampaignEnded};

use crate::campaign::Campaign;
use crate::machine::{ACTIVATE, ACTIVATE_ON_START, END, END_BY_RULE, END_ON_END_DATE};
use crate::notices;

/// The stored name of the purpose, `acquisition` if (wrongly) unset.
fn purpose_of(campaign: &Campaign) -> String {
    campaign.purpose.unwrap_or_default().to_string()
}

/// The stored name of the marketing channel, empty if unset.
fn channel_of(campaign: &Campaign) -> String {
    campaign.marketing_channel.clone().unwrap_or_default()
}

/// The `CampaignActivated` event for `campaign`.
pub fn activated(campaign: &Campaign) -> CampaignActivated {
    CampaignActivated {
        campaign: campaign.campaign_id(),
        purpose: purpose_of(campaign),
        marketing_channel: channel_of(campaign),
        start_date: campaign.start_date.unwrap_or_default(),
        end_date: campaign.end_date,
        budget: campaign.budget,
    }
}

/// The `CampaignEnded` event for `campaign`.
pub fn ended(campaign: &Campaign) -> CampaignEnded {
    CampaignEnded {
        campaign: campaign.campaign_id(),
        purpose: purpose_of(campaign),
        marketing_channel: channel_of(campaign),
        start_date: campaign.start_date.unwrap_or_default(),
        end_date: campaign.end_date,
        budget: campaign.budget,
    }
}

/// Registers the listener.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: Transitioned<Campaign>, state: AppState| async move {
            let name = event.transition.as_str();
            let is_start = [ACTIVATE, ACTIVATE_ON_START].contains(&name);
            let is_end = [END, END_ON_END_DATE, END_BY_RULE].contains(&name);
            if !is_start && !is_end {
                return Ok(());
            }
            let Some(campaign) = Campaign::find(&state.db, event.key).await? else {
                return Ok(());
            };
            if is_start {
                state.emit(activated(&campaign)).await
            } else {
                notices::campaign_ended(&state, &campaign).await?;
                state.emit(ended(&campaign)).await
            }
        },
    );
}
