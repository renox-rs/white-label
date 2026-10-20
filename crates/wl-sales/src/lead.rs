//! The lead (wl-sales §3 *Lead*, §4.1, §5.1, §5.8 step 3, issue #139): an
//! enquiry not yet converted. Its status moves only through the machine
//! ([`crate::lead_machine`]); [`capture`] creates a lead with its source and
//! campaign, whether staff enter it or a web form link with a campaign code
//! sends it.
//!
//! Attribution at capture:
//!
//! - The campaign comes from the code. An `active` campaign counts; an
//!   `ended` one still counts for the late-response days after its end date
//!   ([`crate::config::SalesConfig::late_response_days`]), then the lead gets
//!   the source only; the code of a `draft` campaign, or an unknown one, is
//!   ignored.
//! - The source is the one given, else the one the campaign implies, else
//!   *website* for a web form.
//! - A source that needs a campaign (`SAL.CRM.02` setting) is refused for a
//!   lead entered by staff without one; a web form lead is kept, since
//!   refusing it would lose the enquiry.
//! - A web form lead for a party that already has an open lead on the same
//!   campaign is added to that lead as a note and creates no lead.
//! - The branch is the campaign's, else the one given (the web form's or the
//!   owner's).

use renox::chrono::NaiveDate;
use renox::db::{DateTime, Executor};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::sales::{LeadCreated, LeadId};
use wl_kernel::{PartyId, Subject};

use crate::campaign::{Campaign, CampaignStatus};
use crate::config::{LOST_DUPLICATE, LOST_SPAM, config_of};
use crate::lead_source::LeadSource;

/// Where a lead stands (§4.1).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LeadStatus {
    /// Recorded, not yet contacted.
    #[default]
    New,
    /// First contact made.
    Contacted,
    /// Need, budget or level and timing known.
    Qualified,
    /// Converted to the vertical's conversion record.
    Won,
    /// Not converting, with a reason; can be reopened.
    Lost,
}

impl LeadStatus {
    /// The states a lead is open in: it can still be contacted, qualified,
    /// won or lost.
    pub const OPEN: [LeadStatus; 3] = [
        LeadStatus::New,
        LeadStatus::Contacted,
        LeadStatus::Qualified,
    ];
}

/// A lead.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[allow(clippy::duplicated_attributes)] // each `index(..)` is a separate index; `status` repeats by design
#[model(
    table = "wl_sales_leads",
    state_machine,
    index(party_id, status),
    index(campaign_id, status),
    index(source_id)
)]
pub struct Lead {
    /// The primary key.
    pub id: i64,
    /// The party, once there is one.
    pub party_id: Option<i64>,
    /// Who the lead is, while there is no party.
    pub contact_name: String,
    /// The lead source.
    #[model(references = "wl_sales_lead_sources")]
    pub source_id: i64,
    /// The campaign, if known.
    #[model(references = "wl_sales_campaigns")]
    pub campaign_id: Option<i64>,
    /// The legal entity of the lead in reports.
    pub legal_entity_id: Option<i64>,
    /// The branch scope's type; empty for global.
    pub scope_type: Option<String>,
    /// The branch scope's id; empty for global.
    pub scope_id: Option<String>,
    /// Catalogue items or free text.
    pub interest: String,
    /// The staff member who owns it.
    pub owner_id: Option<i64>,
    /// When to follow up.
    pub next_action_date: Option<NaiveDate>,
    /// The lost reason code, once lost.
    pub lost_reason: Option<String>,
    /// The type of the conversion record, once won.
    pub converted_type: Option<String>,
    /// The id of the conversion record, once won.
    pub converted_id: Option<i64>,
    /// When it was won.
    pub won_at: Option<DateTime>,
    /// Where it stands; changed only by the machine.
    pub status: LeadStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A note on a lead: the contact note, a web form submission for an open
/// lead, or staff's own.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_sales_lead_notes", index(lead_id))]
pub struct LeadNote {
    /// The primary key.
    pub id: i64,
    /// The lead.
    #[model(references = "wl_sales_leads")]
    pub lead_id: i64,
    /// `contact`, `web_form` or `staff`.
    pub kind: String,
    /// The text.
    pub note: String,
    /// The user who wrote it.
    pub user_id: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Lead {
    /// The lead as an id.
    pub fn lead_id(&self) -> LeadId {
        LeadId(self.id)
    }

    /// The lead as a kernel subject.
    pub fn subject(&self) -> Subject {
        self.lead_id().subject()
    }

    /// The party, if any.
    pub fn party(&self) -> Option<PartyId> {
        self.party_id.map(PartyId)
    }

    /// The conversion record, once won.
    pub fn converted(&self) -> Option<Subject> {
        match (&self.converted_type, self.converted_id) {
            (Some(kind), Some(id)) => Some(Subject::new(kind, id)),
            _ => None,
        }
    }

    /// The branch scope as a subject, if the lead has one.
    pub fn branch(&self) -> Option<Subject> {
        match (&self.scope_type, &self.scope_id) {
            (Some(kind), Some(id)) => id.parse().ok().map(|id| Subject::new(kind, id)),
            _ => None,
        }
    }

    /// Whether the lead is left out of every lead count: it was closed as
    /// *duplicate* or *spam or invalid* (§5.8).
    pub fn left_out_of_counts(&self) -> bool {
        self.status == LeadStatus::Lost
            && self
                .lost_reason
                .as_deref()
                .is_some_and(|r| r == LOST_DUPLICATE || r == LOST_SPAM)
    }

    /// The notes of the lead, oldest first.
    pub async fn notes(&self, db: &Db) -> Result<Vec<LeadNote>> {
        LeadNote::query()
            .where_(LeadNote::LEAD_ID.eq(self.id))
            .order_by(LeadNote::ID)
            .get(db)
            .await
    }
}

/// What a lead is captured from.
#[derive(Debug, Clone, Default)]
pub struct NewLead {
    /// The party, if there is one.
    pub party: Option<PartyId>,
    /// Who the lead is, while there is no party.
    pub contact_name: String,
    /// The lead source code, if the enquirer or staff named one.
    pub source: Option<String>,
    /// The campaign code of the web form link (`?c=SPRING26`) or the promo
    /// code.
    pub campaign_code: Option<String>,
    /// Catalogue items or free text.
    pub interest: String,
    /// The staff member who owns it.
    pub owner_id: Option<i64>,
    /// The web form's branch, else the owner's.
    pub branch: Option<Subject>,
    /// The legal entity, when the campaign has none.
    pub legal_entity_id: Option<i64>,
    /// Whether a web form sent it (a staff entry is `false`).
    pub from_web_form: bool,
}

impl NewLead {
    /// A lead from a web form.
    pub fn web_form(party: Option<PartyId>, contact_name: &str) -> Self {
        NewLead {
            party,
            contact_name: contact_name.to_owned(),
            from_web_form: true,
            ..NewLead::default()
        }
    }

    /// A lead entered by staff, from `source`.
    pub fn staff(party: Option<PartyId>, contact_name: &str, source: &str) -> Self {
        NewLead {
            party,
            contact_name: contact_name.to_owned(),
            source: Some(source.to_owned()),
            ..NewLead::default()
        }
    }

    /// With the campaign code.
    pub fn campaign_code(mut self, code: &str) -> Self {
        self.campaign_code = Some(code.to_owned());
        self
    }

    /// With the interest.
    pub fn interest(mut self, interest: &str) -> Self {
        self.interest = interest.to_owned();
        self
    }

    /// With the owner.
    pub fn owner(mut self, owner: i64) -> Self {
        self.owner_id = Some(owner);
        self
    }

    /// At `branch`.
    pub fn branch(mut self, branch: Subject) -> Self {
        self.branch = Some(branch);
        self
    }
}

/// What [`capture`] did.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)] // boxing `Lead` would change the public API used by callers and tests
pub enum Captured {
    /// A lead was created.
    Created(Lead),
    /// The party had an open lead on the campaign: the submission is a note
    /// on it (`lead` is its id) and nothing was created.
    Noted(i64),
}

/// Why a lead could not be captured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeadRefused {
    /// No source was given and the campaign implies none.
    SourceRequired,
    /// There is no such lead source, or it is switched off.
    UnknownSource,
    /// The source needs a campaign (`SAL.CRM.02`) and none was given.
    CampaignRequired,
}

impl LeadRefused {
    /// The English explanation.
    pub fn message(self) -> String {
        let key = match self {
            LeadRefused::SourceRequired => "wl_sales.leads.source_required",
            LeadRefused::UnknownSource => "wl_sales.leads.unknown_source",
            LeadRefused::CampaignRequired => "wl_sales.leads.campaign_required",
        };
        crate::texts::english(key, &[])
    }

    /// The refusal as an HTTP 422.
    pub fn into_error(self) -> Error {
        Error::Status(StatusCode::UNPROCESSABLE_ENTITY, self.message())
    }
}

/// The campaign `code` attributes a lead or sale arriving on `on`: an
/// `active` campaign; an `ended` one until `late_days` after its end date;
/// never a `draft` one, nor an unknown code.
pub async fn campaign_for_code<'c>(
    db: impl Executor<'c>,
    code: &str,
    on: NaiveDate,
    late_days: u32,
) -> Result<Option<Campaign>> {
    let Some(campaign) = Campaign::by_code(db, code).await? else {
        return Ok(None);
    };
    Ok(match campaign.status {
        CampaignStatus::Draft => None,
        CampaignStatus::Active => Some(campaign),
        CampaignStatus::Ended => {
            let ended_on = campaign
                .end_date
                .or_else(|| campaign.updated_at.map(wl_kernel::documents::dubai_date));
            ended_on
                .is_some_and(|end| (on - end).num_days() <= i64::from(late_days))
                .then_some(campaign)
        }
    })
}

/// Captures a lead on `on` (today in Asia/Dubai for a live caller), with the
/// attribution rules of the module comment. The event [`LeadCreated`] follows
/// the commit.
pub async fn capture(
    state: &AppState,
    input: NewLead,
    on: NaiveDate,
) -> Result<std::result::Result<Captured, LeadRefused>> {
    let config = config_of(state);
    let campaign = match input.campaign_code.as_deref().map(str::trim) {
        Some(code) if !code.is_empty() => {
            campaign_for_code(&state.db, code, on, config.late_response_days).await?
        }
        _ => None,
    };
    let source = match input
        .source
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
    {
        Some(code) => {
            match LeadSource::by_code(&state.db, &LeadSource::normalize_code(code)).await? {
                Some(source) if source.active => source,
                _ => return Ok(Err(LeadRefused::UnknownSource)),
            }
        }
        None => {
            let implied = match campaign.as_ref().and_then(|c| c.lead_source_id) {
                Some(id) => LeadSource::find(&state.db, id).await?,
                None => None,
            };
            match implied {
                Some(source) => source,
                None if input.from_web_form => {
                    match LeadSource::by_code(&state.db, "website").await? {
                        Some(source) => source,
                        None => return Ok(Err(LeadRefused::SourceRequired)),
                    }
                }
                None => return Ok(Err(LeadRefused::SourceRequired)),
            }
        }
    };
    if !input.from_web_form && campaign.is_none() && config.campaign_required_for(&source.code) {
        return Ok(Err(LeadRefused::CampaignRequired));
    }
    // A web form lead for a party with an open lead on the same campaign is
    // a note on that lead.
    if let (true, Some(party), Some(campaign)) =
        (input.from_web_form, input.party, campaign.as_ref())
    {
        let open = Lead::query()
            .where_(Lead::PARTY_ID.eq(party.0))
            .where_(Lead::CAMPAIGN_ID.eq(campaign.id))
            .where_(Lead::STATUS.is_in(LeadStatus::OPEN))
            .order_by(Lead::ID)
            .first(&state.db)
            .await?;
        if let Some(lead) = open {
            LeadNote::create(
                &state.db,
                LeadNote {
                    lead_id: lead.id,
                    kind: "web_form".to_owned(),
                    note: crate::texts::english(
                        "wl_sales.leads.web_form_note",
                        &[("code", &campaign.code)],
                    ),
                    ..LeadNote::default()
                },
            )
            .await?;
            return Ok(Ok(Captured::Noted(lead.id)));
        }
    }
    let (scope_type, scope_id, entity) = match &campaign {
        Some(c) if c.scope_type.is_some() => (
            c.scope_type.clone(),
            c.scope_id.clone(),
            c.legal_entity_id.or(input.legal_entity_id),
        ),
        Some(c) => (
            input.branch.as_ref().map(|b| b.subject_type.clone()),
            input.branch.as_ref().map(|b| b.subject_id.to_string()),
            c.legal_entity_id.or(input.legal_entity_id),
        ),
        None => (
            input.branch.as_ref().map(|b| b.subject_type.clone()),
            input.branch.as_ref().map(|b| b.subject_id.to_string()),
            input.legal_entity_id,
        ),
    };
    let lead = Lead::create(
        &state.db,
        Lead {
            party_id: input.party.map(|p| p.0),
            contact_name: input.contact_name.trim().to_owned(),
            source_id: source.id,
            campaign_id: campaign.as_ref().map(|c| c.id),
            legal_entity_id: entity,
            scope_type,
            scope_id,
            interest: input.interest.trim().to_owned(),
            owner_id: input.owner_id,
            ..Lead::default()
        },
    )
    .await?;
    state
        .emit(LeadCreated {
            lead: lead.lead_id(),
            party: lead.party(),
            source: source.code.clone(),
            campaign: campaign.as_ref().map(Campaign::campaign_id),
            owner: lead.owner_id,
        })
        .await?;
    Ok(Ok(Captured::Created(lead)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lead_closed_as_duplicate_or_spam_is_left_out_of_the_counts() {
        let mut lead = Lead::default();
        assert!(!lead.left_out_of_counts());
        lead.status = LeadStatus::Lost;
        lead.lost_reason = Some("price".into());
        assert!(!lead.left_out_of_counts());
        for reason in ["duplicate", "spam_or_invalid"] {
            lead.lost_reason = Some(reason.into());
            assert!(lead.left_out_of_counts());
        }
        lead.status = LeadStatus::New;
        assert!(!lead.left_out_of_counts());
    }

    #[test]
    fn the_converted_record_and_the_branch_need_both_parts() {
        let mut lead = Lead::default();
        assert!(lead.converted().is_none() && lead.branch().is_none());
        lead.converted_type = Some("property.lease".into());
        lead.converted_id = Some(4);
        assert_eq!(lead.converted(), Some(Subject::new("property.lease", 4)));
        lead.scope_type = Some("property.site".into());
        lead.scope_id = Some("2".into());
        assert_eq!(lead.branch(), Some(Subject::new("property.site", 2)));
    }

    #[test]
    fn the_builders_set_the_origin() {
        let web = NewLead::web_form(Some(PartyId(1)), " Omar ").campaign_code("SPRING26");
        assert!(web.from_web_form && web.source.is_none());
        assert_eq!(web.campaign_code.as_deref(), Some("SPRING26"));
        let staff = NewLead::staff(None, "Omar", "referral").owner(3);
        assert!(!staff.from_web_form);
        assert_eq!(staff.owner_id, Some(3));
    }
}
