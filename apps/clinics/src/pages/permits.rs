//! The advertising permits of campaigns (05 §9 *Advertising*, `CLN.ACL.13`;
//! BL-CLN-15).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /clinics/campaign-permits` | `clinics.campaign_permits` | `advertising_permits.view` or `.manage` |
//! | `POST /clinics/campaign-permits` | `clinics.campaign_permit_record` | `advertising_permits.manage` (the medical director) |
//!
//! The list shows each campaign's permit number, its last valid day and who
//! approved it. A campaign that promotes services is activated and sends only
//! while its permit is valid; the guard is wl-sales' `CampaignGuard`, wired
//! by the host.

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};

use super::{Texts, invalid, parse_date, parse_id, require, text};
use crate::permissions;
use crate::permits::{self, AdvertisingPermit, NewPermit, Problem};
use crate::texts;
use wl_kernel::documents::today;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/clinics/campaign-permits", index)
        .name("clinics.campaign_permits")
        .post("/clinics/campaign-permits", record)
        .name("clinics.campaign_permit_record")
        .require_auth()
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "clinics/permits.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    permits: GridPage<AdvertisingPermit>,
    can_manage: bool,
    promotes_options: Vec<[String; 2]>,
    today: NaiveDate,
}

/// The permits grid: search on the permit number, filters on the validity and
/// on what is missing. The campaign is drawn by the template (`custom`).
pub(crate) fn grid(lang: &Lang, day: NaiveDate) -> Grid {
    // Valid, or the first thing [`AdvertisingPermit::problem_on`] finds wrong.
    let state = format!(
        "CASE WHEN NOT {{T}}.\"promotes_services\" THEN 'valid' \
         WHEN TRIM({{T}}.\"permit_number\") = '' THEN 'no_number' \
         WHEN {{T}}.\"valid_to\" IS NULL THEN 'no_validity' \
         WHEN {{T}}.\"valid_to\" < '{day}' THEN 'lapsed' ELSE 'valid' END"
    );
    Grid::new("clinics-campaign-permits")
        .title(&text(lang, "permit.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::custom("campaign", &text(lang, "permit.campaign"))
                .frozen()
                .mobile(),
        )
        .column(Column::bool("promotes_services", &text(lang, "permit.promotes")).hidden())
        .column(
            Column::text("permit_number", &text(lang, "permit.number"))
                .mobile()
                .searchable(),
        )
        .column(Column::date("valid_to", &text(lang, "permit.valid_to")).mobile())
        .column(
            Column::related(
                "approved_by_name",
                &text(lang, "permit.approved_by"),
                "users",
                "approved_by",
                "name",
            )
            .searchable(),
        )
        .column(
            Column::select(
                "state",
                &text(lang, "fields.status"),
                vec![
                    ("valid".to_owned(), text(lang, "permit.valid")),
                    ("no_number".to_owned(), Problem::NoNumber.message()),
                    ("no_validity".to_owned(), Problem::NoValidity.message()),
                    ("lapsed".to_owned(), Problem::Lapsed.message()),
                ],
            )
            .computed(&state)
            .badges(&[
                ("valid", "success"),
                ("no_number", "danger"),
                ("no_validity", "danger"),
                ("lapsed", "danger"),
            ])
            .mobile(),
        )
        .sort_by("campaign_id")
        .cards_on_mobile()
        .empty_state(
            &text(lang, "permit.empty"),
            Some(&text(lang, "permit.subtitle")),
        )
}

/// `GET /clinics/campaign-permits`.
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<View> {
    if !user.has_permission(permissions::PERMITS_VIEW)
        && !user.has_permission(permissions::PERMITS_MANAGE)
    {
        return Err(Error::Forbidden);
    }
    let day = today();
    let page = grid(&lang, day)
        .page(AdvertisingPermit::query(), &request)
        .await?;
    let mut campaigns = BTreeMap::new();
    for permit in page.items() {
        let label = wl_kernel::documents::subject_label(
            &state.db,
            &wl_kernel::Subject::new(wl_kernel::sales::CAMPAIGN_SUBJECT_TYPE, permit.campaign_id),
        )
        .await?
        .unwrap_or_else(|| format!("#{}", permit.campaign_id));
        campaigns.insert(permit.id, label);
    }
    let page = page.extend(|permit| json!({ "campaign": campaigns.get(&permit.id) }));
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        permits: page,
        can_manage: user.has_permission(permissions::PERMITS_MANAGE),
        promotes_options: vec![
            ["yes".to_owned(), text(&lang, "permit.promotes_yes")],
            ["no".to_owned(), text(&lang, "permit.promotes_no")],
        ],
        today: day,
    }))
}

/// The form that records a permit.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct PermitForm {
    /// The wl-sales campaign's id.
    #[serde(default)]
    pub campaign: String,
    /// `yes` when the campaign promotes services, `no` when it does not.
    #[serde(default)]
    pub promotes: String,
    /// The regulator's permit number.
    #[serde(default)]
    pub permit_number: String,
    /// The last valid day, `2026-12-31`.
    #[serde(default)]
    pub valid_to: String,
}

/// `POST /clinics/campaign-permits`.
async fn record(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<PermitForm>,
) -> Result<Response> {
    require(&user, permissions::PERMITS_MANAGE)?;
    let campaign =
        parse_id(&form.campaign).ok_or_else(|| invalid(&text(&lang, "permit.problem.campaign")))?;
    let new = if form.promotes.trim() == "no" {
        NewPermit::no_services(campaign)
    } else {
        NewPermit {
            campaign_id: campaign,
            promotes_services: true,
            permit_number: form.permit_number.trim().to_owned(),
            valid_to: parse_date(&form.valid_to)?,
        }
    };
    permits::record(&state, &user, new).await?;
    Ok((
        Toast::success(text(&lang, "permit.recorded")),
        renox::HxRefresh,
    )
        .into_response())
}
