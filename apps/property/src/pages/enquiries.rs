//! The enquiries (01 Property `PRP.LEA.01`, BL-PRP-27): who asked, which unit,
//! and where they came from (the lead source and the campaign of
//! `SAL.CRM.02`).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /property/enquiries` | `property.enquiries` | `units.view`, in some building |
//! | `POST /property/enquiries` | `property.enquiry_create` | `leases.create` |

use renox::grid::{Column, Grid, GridPage, GridRequest};
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

/// The enquiries grid: search on the prospect, contact and campaign, a
/// filter by lead source. The contact is exported for those who record
/// enquiries only.
pub(crate) fn grid(lang: &Lang, can_export: bool) -> Grid {
    let t = |key: &str| text(lang, key);
    let sources: Vec<(String, String)> = LEAD_SOURCES
        .iter()
        .map(|code| ((*code).to_owned(), t(&format!("enquiries.source_{code}"))))
        .collect();
    let grid = Grid::new("property-enquiries")
        .title(&t("enquiries.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("prospect_name", &t("enquiries.prospect"))
                .frozen()
                .mobile()
                .searchable(),
        )
        .column(
            Column::text("contact", &t("enquiries.contact"))
                .mobile()
                .searchable(),
        )
        .column(Column::select("source", &t("enquiries.source"), sources).mobile())
        .column(Column::text("campaign", &t("enquiries.campaign")).searchable())
        .column(Column::date("received_on", &t("enquiries.received")).mobile())
        .column(
            Column::select(
                "converted",
                &t("enquiries.converted"),
                vec![
                    ("yes".to_owned(), t("enquiries.converted_yes")),
                    ("no".to_owned(), t("enquiries.converted_no")),
                ],
            )
            .computed("CASE WHEN {T}.\"converted_party_id\" IS NULL THEN 'no' ELSE 'yes' END")
            .badges(&[("yes", "success")])
            .hidden(),
        )
        .sort_by("-received_on")
        .cards_on_mobile()
        .empty_state(&t("enquiries.empty"), Some(&t("enquiries.empty_hint")));
    if can_export { grid.exports() } else { grid }
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "property/enquiries.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    enquiries: GridPage<Enquiry>,
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
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    // The enquiries about the units of the user's buildings, and those that
    // name no unit (a prospect's first call belongs to no building yet).
    let reach = wl_kernel::scopes::reach(&state.db, &user, permissions::UNITS_VIEW).await?;
    let units = reach.ids(&state.db, crate::unit::UNIT_SUBJECT_TYPE).await?;
    let query = match units {
        None => Enquiry::query(),
        Some(units) => Enquiry::query().where_any(|group| {
            group
                .where_null("unit_id")
                .where_in("unit_id", units.iter().copied())
        }),
    };
    let can_record = user.has_permission_anywhere(permissions::LEASES_CREATE);
    let grid = grid(&lang, can_record);
    if request.param("export").is_some() && !can_record {
        return Err(Error::Forbidden);
    }
    if let Some(file) = grid.export(query.clone(), &request).await? {
        return Ok(file);
    }
    let enquiries = grid.page(query, &request).await?;
    let sources = LEAD_SOURCES
        .iter()
        .map(|code| {
            (
                (*code).to_owned(),
                text(&lang, &format!("enquiries.source_{code}")),
            )
        })
        .collect();
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        enquiries,
        sources,
        can_record,
        today: today().to_string(),
    })
    .into_response())
}

/// `POST /property/enquiries`.
async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<CreateForm>,
) -> Result<Response> {
    if !user.has_permission_anywhere(permissions::LEASES_CREATE) {
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
