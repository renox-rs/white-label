//! Enquiries (01 Property `PRP.LEA.01`, BL-PRP-27): the minimal record the
//! lead source and the campaign (`SAL.CRM.02`) are recorded on.
//!
//! An enquiry is a prospect's first contact: a name, how to reach them, the
//! unit they asked about and **where they came from**. The source is one of
//! the eight 01 uses ([`crate::seed::LEAD_SOURCES`]); the campaign is
//! optional, because no source requires one in 01. When the enquiry converts,
//! the source and the campaign reach the tenant's party through the kernel's
//! sales contract ([`wl_kernel::sales::CampaignAttribution`]): wl-sales holds
//! the party's acquisition and, on the activation of the first lease, its
//! converted value. A tenant who never made an enquiry gets the source on its
//! party by staff, as for any customer. Viewings and the pipeline around the
//! enquiry are not built here.

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;
use wl_kernel::documents::today;
use wl_kernel::sales::{Attribute, campaign_attribution};

use crate::seed::LEAD_SOURCES;
use crate::texts::english;

/// The subject type of an enquiry.
pub const ENQUIRY_SUBJECT_TYPE: &str = "property.enquiry";

/// What set the attribution of a party that came from an enquiry.
pub const SET_FROM: &str = "lead";

/// A prospect's enquiry.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "property_enquiries", index(source), index(converted_party_id))]
pub struct Enquiry {
    /// The primary key.
    pub id: i64,
    /// Who asked.
    pub prospect_name: String,
    /// Phone or e-mail.
    pub contact: String,
    /// The unit they asked about.
    #[model(references = "property_units")]
    pub unit_id: Option<i64>,
    /// The code of the lead source.
    pub source: String,
    /// The code of the campaign, if any.
    pub campaign: Option<String>,
    /// Anything staff noted.
    pub notes: String,
    /// The user who recorded it.
    pub recorded_by: Option<i64>,
    /// The day it came in.
    pub received_on: NaiveDate,
    /// The tenant's party once the enquiry converted.
    pub converted_party_id: Option<i64>,
    /// The day it converted.
    pub converted_on: Option<NaiveDate>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Enquiry {
    /// The subject.
    pub fn subject(&self) -> wl_kernel::Subject {
        wl_kernel::Subject::new(ENQUIRY_SUBJECT_TYPE, self.id)
    }

    /// Whether it converted.
    pub fn is_converted(&self) -> bool {
        self.converted_party_id.is_some()
    }

    /// The enquiries, newest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn recent(db: &Db) -> Result<Vec<Enquiry>> {
        Enquiry::query().order_by_desc(Enquiry::ID).get(db).await
    }
}

/// What is given to record an enquiry.
#[derive(Debug, Clone, Default)]
pub struct NewEnquiry {
    /// Who asked.
    pub prospect_name: String,
    /// Phone or e-mail.
    pub contact: String,
    /// The unit they asked about.
    pub unit_id: Option<i64>,
    /// The code of the lead source (one of [`LEAD_SOURCES`]).
    pub source: String,
    /// The code of the campaign, if any.
    pub campaign: Option<String>,
    /// Anything staff noted.
    pub notes: String,
    /// The user who records it.
    pub recorded_by: Option<i64>,
}

/// 422 with the English text `key`.
fn refused(key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key, &[]))
}

/// Records an enquiry received today.
///
/// # Errors
///
/// 422 for a blank name or a source 01 does not use; the database's errors.
pub async fn record(db: &Db, new: NewEnquiry) -> Result<Enquiry> {
    let name = new.prospect_name.trim().to_owned();
    if name.is_empty() {
        return Err(refused("property.enquiries.name_required"));
    }
    let source = new.source.trim().to_owned();
    if !LEAD_SOURCES.contains(&source.as_str()) {
        return Err(refused("property.enquiries.source_unknown"));
    }
    let campaign = new
        .campaign
        .map(|c| c.trim().to_owned())
        .filter(|c| !c.is_empty());
    Enquiry::create(
        db,
        Enquiry {
            prospect_name: name,
            contact: new.contact.trim().to_owned(),
            unit_id: new.unit_id,
            source,
            campaign,
            notes: new.notes.trim().to_owned(),
            recorded_by: new.recorded_by,
            received_on: today(),
            ..Enquiry::default()
        },
    )
    .await
}

/// Marks enquiry `id` converted to the tenant's party and hands its source and
/// campaign to the party's acquisition (wl-sales, through the kernel). Without
/// wl-sales nothing is attributed and the enquiry still converts.
///
/// # Errors
///
/// `NotFound` for an unknown enquiry; 422 when it already converted; the
/// database's and wl-sales' errors.
pub async fn convert(state: &AppState, id: i64, tenant: PartyId) -> Result<Enquiry> {
    let mut enquiry = Enquiry::find_or_404(&state.db, id).await?;
    if enquiry.is_converted() {
        return Err(refused("property.enquiries.already_converted"));
    }
    let mut attribute = Attribute::party(tenant, &enquiry.source).set_from(SET_FROM);
    if let Some(campaign) = &enquiry.campaign {
        attribute = attribute.campaign(campaign);
    }
    campaign_attribution(state)
        .attribute(state, attribute)
        .await?;
    enquiry.converted_party_id = Some(tenant.0);
    enquiry.converted_on = Some(today());
    enquiry.save(&state.db).await?;
    Ok(enquiry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sources_are_the_codes_of_wl_sales() {
        for source in LEAD_SOURCES {
            assert!(
                source.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{source}"
            );
        }
    }

    #[test]
    fn an_enquiry_is_a_subject_of_the_vertical() {
        let enquiry = Enquiry {
            id: 3,
            ..Enquiry::default()
        };
        assert_eq!(enquiry.subject().to_string(), "property.enquiry:3");
        assert!(!enquiry.is_converted());
    }
}
