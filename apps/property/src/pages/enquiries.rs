//! The enquiries (01 Property `PRP.LEA.01`, BL-PRP-27): who asked, which unit,
//! and where they came from (the lead source and the campaign of
//! `SAL.CRM.02`).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /property/enquiries` | `property.enquiries` | `units.view` |
//! | `POST /property/enquiries` | `property.enquiry_create` | `leases.create` |

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;

use super::{Texts, text};
use crate::enquiry::{Enquiry, NewEnquiry, record};
use crate::permissions;
use crate::seed::LEAD_SOURCES;
use crate::texts;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/property/enquiries", index)
        .name("property.enquiries")
        .post("/property/enquiries", create)
        .name("property.enquiry_create")
        .require_auth()
}

/// One enquiry of the list.
#[derive(Serialize, Debug, Clone)]
pub struct EnquiryRow {
    /// The primary key.
    pub id: i64,
    /// Who asked.
    pub prospect: String,
    /// How to reach them.
    pub contact: String,
    /// The source in the page's language.
    pub source: String,
    /// The campaign code, or a dash.
    pub campaign: String,
    /// The day it came in.
    pub received: String,
    /// Whether it converted to a tenant.
    pub converted: bool,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "property/enquiries.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    enquiries: Vec<EnquiryRow>,
    sources: Vec<(String, String)>,
    can_record: bool,
    today: String,
}

/// The form that records an enquiry.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct CreateForm {
    /// Who asked.
    #[serde(default)]
    pub prospect: String,
    /// How to reach them.
    #[serde(default)]
    pub contact: String,
    /// The lead source code.
    #[serde(default)]
    pub source: String,
    /// The campaign code.
    #[serde(default)]
    pub campaign: String,
    /// Notes.
    #[serde(default)]
    pub notes: String,
}

/// `GET /property/enquiries`.
async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    if !user.has_permission(permissions::UNITS_VIEW) {
        return Err(Error::Forbidden);
    }
    let source_name = |code: &str| text(&lang, &format!("enquiries.source_{code}"));
    let enquiries = Enquiry::recent(&state.db)
        .await?
        .into_iter()
        .map(|e| EnquiryRow {
            id: e.id,
            prospect: e.prospect_name.clone(),
            contact: e.contact.clone(),
            source: source_name(&e.source),
            campaign: e.campaign.clone().unwrap_or_else(|| "—".to_owned()),
            received: e.received_on.to_string(),
            converted: e.is_converted(),
        })
        .collect();
    let sources = LEAD_SOURCES
        .iter()
        .map(|code| ((*code).to_owned(), source_name(code)))
        .collect();
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        enquiries,
        sources,
        can_record: user.has_permission(permissions::LEASES_CREATE),
        today: today().to_string(),
    }))
}

/// `POST /property/enquiries`.
async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<CreateForm>,
) -> Result<Response> {
    if !user.has_permission(permissions::LEASES_CREATE) {
        return Err(Error::Forbidden);
    }
    record(
        &state.db,
        NewEnquiry {
            prospect_name: form.prospect,
            contact: form.contact,
            unit_id: None,
            source: form.source,
            campaign: Some(form.campaign),
            notes: form.notes,
            recorded_by: Some(user.id),
        },
    )
    .await?;
    Ok((
        Toast::success(text(&lang, "enquiries.created")),
        renox::HxRefresh,
    )
        .into_response())
}
