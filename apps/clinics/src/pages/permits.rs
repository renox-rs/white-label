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

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use super::{Texts, invalid, parse_date, parse_id, require, text, user_name};
use crate::permissions;
use crate::permits::{self, AdvertisingPermit, NewPermit};
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

/// One permit as the list shows it.
#[derive(Serialize, Debug, Clone)]
pub struct PermitRow {
    /// The wl-sales campaign, by its code and name (its number when the
    /// sales module is off).
    pub campaign: String,
    /// Whether the campaign promotes services, in words.
    pub promotes: String,
    /// The permit number, or a dash.
    pub number: String,
    /// The last valid day, or a dash.
    pub valid_to: String,
    /// Valid, lapsed or what is missing, in words.
    pub state: String,
    /// The badge colour of the state.
    pub state_kind: &'static str,
    /// Who approved it.
    pub approved_by: String,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "clinics/permits.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    permits: Vec<PermitRow>,
    can_manage: bool,
    promotes_options: Vec<[String; 2]>,
    today: NaiveDate,
}

/// `GET /clinics/campaign-permits`.
async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    if !user.has_permission(permissions::PERMITS_VIEW)
        && !user.has_permission(permissions::PERMITS_MANAGE)
    {
        return Err(Error::Forbidden);
    }
    let day = today();
    let dash = text(&lang, "none");
    let mut rows = Vec::new();
    for permit in AdvertisingPermit::listed(&state.db).await? {
        let problem = permit.problem_on(day);
        rows.push(PermitRow {
            campaign: wl_kernel::documents::subject_label(
                &state.db,
                &wl_kernel::Subject::new(
                    wl_kernel::sales::CAMPAIGN_SUBJECT_TYPE,
                    permit.campaign_id,
                ),
            )
            .await?
            .unwrap_or_else(|| format!("#{}", permit.campaign_id)),
            promotes: text(
                &lang,
                if permit.promotes_services {
                    "permit.promotes_yes"
                } else {
                    "permit.promotes_no"
                },
            ),
            number: if permit.permit_number.is_empty() {
                dash.clone()
            } else {
                permit.permit_number.clone()
            },
            valid_to: permit
                .valid_to
                .map_or_else(|| dash.clone(), |d| d.to_string()),
            state: problem.map_or_else(|| text(&lang, "permit.valid"), |p| p.message()),
            state_kind: if problem.is_some() {
                "error"
            } else {
                "success"
            },
            approved_by: match permit.approved_by {
                Some(id) => user_name(&state.db, id).await,
                None => dash.clone(),
            },
        });
    }
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        permits: rows,
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
