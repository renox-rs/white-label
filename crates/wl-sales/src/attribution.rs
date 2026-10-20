//! The kernel contract of the module (issue #139): [`Attribution`] is the
//! app's [`CampaignAttribution`] and [`Conversion`] its [`LeadConversion`], so
//! verticals report their first sale and win their leads without depending on
//! this crate.

use renox::db::now;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::sales::{
    Attribute, AttributionTarget, CampaignAttribution, FirstSale, LeadConversion, LeadId,
    SalesFuture,
};
use wl_kernel::{PartyId, Subject};

use crate::acquisition::{self, AttributeParty};
use crate::campaign::Campaign;
use crate::config::config_of;
use crate::lead::{Lead, LeadStatus};
use crate::lead_machine::{WIN, WinInput};
use crate::lead_source::LeadSource;

/// The app's [`CampaignAttribution`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Attribution;

impl CampaignAttribution for Attribution {
    fn attribute<'a>(&'a self, state: &'a AppState, request: Attribute) -> SalesFuture<'a, bool> {
        Box::pin(async move {
            match request.target {
                AttributionTarget::Party(party) => {
                    let created = acquisition::attribute_party(
                        state,
                        AttributeParty {
                            party,
                            source: request.source,
                            campaign: request.campaign,
                            set_from: request.set_from,
                            ..AttributeParty::default()
                        },
                    )
                    .await?
                    .map_err(acquisition::AcquisitionRefused::into_error)?;
                    Ok(created.is_some())
                }
                AttributionTarget::Lead(lead) => attribute_lead(state, lead, request).await,
            }
        })
    }

    fn record_first_sale<'a>(
        &'a self,
        state: &'a AppState,
        sale: FirstSale,
    ) -> SalesFuture<'a, bool> {
        Box::pin(async move { acquisition::record_sale(state, sale).await })
    }

    fn sale_gone<'a>(&'a self, state: &'a AppState, sale: Subject) -> SalesFuture<'a, ()> {
        Box::pin(async move { acquisition::sale_gone(state, &sale).await })
    }
}

/// Sets the source and campaign of an open lead. A won lead is changed
/// through [`crate::acquisition_change`] (`SAL.ACL.09`), so this returns
/// `false` for it and for a lead that does not exist.
async fn attribute_lead(state: &AppState, id: LeadId, request: Attribute) -> Result<bool> {
    let Some(lead) = Lead::find(&state.db, id.0).await? else {
        return Ok(false);
    };
    if lead.status == LeadStatus::Won {
        return Ok(false);
    }
    let source = LeadSource::by_code(&state.db, &LeadSource::normalize_code(&request.source))
        .await?
        .filter(|s| s.active)
        .ok_or_else(|| acquisition::AcquisitionRefused::UnknownSource.into_error())?;
    let campaign = match request.campaign.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        Some(code) => Some(
            Campaign::by_code(&state.db, code)
                .await?
                .ok_or_else(|| acquisition::AcquisitionRefused::UnknownCampaign.into_error())?,
        ),
        None => None,
    };
    if campaign.is_none() && config_of(state).campaign_required_for(&source.code) {
        return Err(acquisition::AcquisitionRefused::CampaignRequired.into_error());
    }
    Lead::query()
        .where_(Lead::ID.eq(lead.id))
        .update(
            &state.db,
            &[
                ("source_id", &source.id),
                ("campaign_id", &campaign.as_ref().map(|c| c.id)),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(true)
}

/// The app's [`LeadConversion`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Conversion;

/// Runs `win` on `lead` when it is `contacted` or `qualified`.
async fn win(state: &AppState, lead: Lead, converted: Subject) -> Result<bool> {
    if !matches!(lead.status, LeadStatus::Contacted | LeadStatus::Qualified) {
        return Ok(false);
    }
    let _outcome = lead.transition(WIN)
        .by_system()
        .input(&WinInput { converted })
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    Ok(true)
}

impl LeadConversion for Conversion {
    fn win<'a>(
        &'a self,
        state: &'a AppState,
        lead: LeadId,
        converted: Subject,
    ) -> SalesFuture<'a, bool> {
        Box::pin(async move {
            match Lead::find(&state.db, lead.0).await? {
                Some(lead) => win(state, lead, converted).await,
                None => Ok(false),
            }
        })
    }

    fn win_for_party<'a>(
        &'a self,
        state: &'a AppState,
        party: PartyId,
        converted: Subject,
    ) -> SalesFuture<'a, bool> {
        Box::pin(async move {
            let open = Lead::query()
                .where_(Lead::PARTY_ID.eq(party.0))
                .where_(Lead::STATUS.is_in([LeadStatus::Contacted, LeadStatus::Qualified]))
                .order_by_desc(Lead::ID)
                .first(&state.db)
                .await?;
            match open {
                Some(lead) => win(state, lead, converted).await,
                None => Ok(false),
            }
        })
    }
}
