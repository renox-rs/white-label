//! The test drives and the vehicle incidents of a job card (`AUT.ACL.22`,
//! BL-AUT-06). The job card page shows them and the forms; these are the
//! routes the forms post to.
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `POST /auto/job-cards/{id}/test-drives` | `auto.job_card_test_drive` | `job_cards.work` (own) or `job_cards.advise` |
//! | `POST /auto/test-drives/{id}/finish` | `auto.test_drive_finish` | the same |
//! | `POST /auto/job-cards/{id}/incidents` | `auto.job_card_incident` | the same |
//! | `POST /auto/incidents/{id}/decide` | `auto.incident_decide` | `job_cards.liability` |

use std::collections::BTreeMap;

use renox::db::DateTime;
use renox::prelude::*;
use serde::Serialize;
use wl_kernel::documents::dubai_text;

use super::{parse_km, text, ticked};
use crate::incident::{Incident, NOT_WORKSHOP, WORKSHOP};
use crate::incidents::{self, NewIncident};
use crate::job_card::JobCard;
use crate::job_cards::may_work;
use crate::permissions;
use crate::test_drive::TestDrive;
use crate::test_drives::{self, NewDrive};

/// The routes of the test drives and the incidents.
pub fn routes() -> Routes {
    Routes::new()
        .post("/auto/job-cards/{id}/test-drives", record_drive)
        .name("auto.job_card_test_drive")
        .post("/auto/test-drives/{id}/finish", finish_drive)
        .name("auto.test_drive_finish")
        .post("/auto/job-cards/{id}/incidents", report_incident)
        .name("auto.job_card_incident")
        .post("/auto/incidents/{id}/decide", decide_incident)
        .name("auto.incident_decide")
        .require_auth()
}

/// A test drive as a row.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct DriveRow {
    pub id: i64,
    pub driver: String,
    pub out: String,
    pub back: String,
    pub km_out: String,
    pub km_in: String,
    pub distance: String,
    pub open: bool,
}

/// An incident as a row.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct IncidentRow {
    pub id: i64,
    pub number: String,
    pub description: String,
    pub road_accident: String,
    pub ticket: String,
    pub liability: String,
    pub reason: String,
    pub internal: String,
    pub internal_id: Option<i64>,
    pub can_decide: bool,
}

/// What the job card page shows of the drives and the incidents.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct IncidentView {
    pub drives: Vec<DriveRow>,
    pub incidents: Vec<IncidentRow>,
    pub can_work: bool,
    pub can_back: bool,
    pub is_internal: bool,
    pub liability_options: Vec<[String; 2]>,
}

/// What the page shows for `card` and `user`.
pub(crate) async fn view_of(
    state: &AppState,
    lang: &Lang,
    user: &AuthUser,
    card: &JobCard,
) -> Result<IncidentView> {
    let none = || text(lang, "none");
    let mut drives = Vec::new();
    for drive in TestDrive::of_job_card(&state.db, card.id).await? {
        drives.push(DriveRow {
            id: drive.id,
            driver: super::user_name(&state.db, drive.driver_user_id).await,
            out: dubai_text(drive.out_at),
            back: drive
                .in_at
                .map_or_else(|| text(lang, "drives.still_out"), dubai_text),
            km_out: crate::number_text(drive.odometer_out),
            km_in: drive.odometer_in.map_or_else(none, crate::number_text),
            distance: drive
                .distance()
                .map_or_else(none, |km| format!("{} km", crate::number_text(km))),
            open: drive.is_open(),
        });
    }
    let can_decide_any = user.has_permission(permissions::JOB_CARDS_LIABILITY);
    let mut rows = Vec::new();
    for incident in Incident::of_job_card(&state.db, card.id).await? {
        let involved = incidents::involved(&state.db, &incident).await?;
        rows.push(row_of(
            lang,
            &incident,
            can_decide_any && !involved.contains(&user.id),
        ));
    }
    let open = card.status.is_open();
    Ok(IncidentView {
        drives,
        incidents: rows,
        can_work: open && may_work(user, card),
        can_back: open && may_work(user, card),
        is_internal: card.internal,
        liability_options: [WORKSHOP, NOT_WORKSHOP]
            .into_iter()
            .map(|l| {
                [
                    l.to_owned(),
                    text(lang, &format!("incidents.liability.{l}")),
                ]
            })
            .collect(),
    })
}

fn row_of(lang: &Lang, incident: &Incident, can_decide: bool) -> IncidentRow {
    let none = || text(lang, "none");
    IncidentRow {
        id: incident.id,
        number: incident.number(),
        description: incident.description.clone(),
        road_accident: text(
            lang,
            if incident.road_accident {
                "jobs.yes"
            } else {
                "jobs.no"
            },
        ),
        ticket: incident.ticket_number.clone().unwrap_or_else(none),
        liability: match &incident.liability {
            Some(l) => text(lang, &format!("incidents.liability.{l}")),
            None => text(lang, "incidents.undecided"),
        },
        reason: incident.liability_reason.clone().unwrap_or_else(none),
        internal: incident
            .internal_job_card_id
            .map_or_else(none, |id| format!("JOB-{id:05}")),
        internal_id: incident.internal_job_card_id,
        can_decide: can_decide && !incident.is_decided(),
    }
}

/// A field of the form, trimmed; `None` when blank.
fn field(form: &BTreeMap<String, String>, name: &str) -> Option<String> {
    form.get(name)
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

/// `POST /auto/job-cards/{id}/test-drives`: `odometer`.
async fn record_drive(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let km = parse_km(&field(&form, "odometer").unwrap_or_default())?;
    test_drives::record(&state, &user, id, NewDrive::out(km)).await?;
    Ok((
        Toast::success(text(&lang, "drives.saved")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /auto/test-drives/{id}/finish`: `odometer`.
async fn finish_drive(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let km = parse_km(&field(&form, "odometer").unwrap_or_default())?;
    let at: Option<DateTime> = None;
    test_drives::finish(&state, &user, id, km, at).await?;
    Ok((
        Toast::success(text(&lang, "drives.closed")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /auto/job-cards/{id}/incidents`: `description`, `road_accident`.
async fn report_incident(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let mut new = NewIncident::new(&field(&form, "description").unwrap_or_default());
    if ticked(form.get("road_accident").map_or("", String::as_str)) {
        new = new.road_accident();
    }
    incidents::report(&state, &user, id, new).await?;
    Ok((
        Toast::success(text(&lang, "incidents.reported")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /auto/incidents/{id}/decide`: `liability`, `reason`.
async fn decide_incident(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    incidents::decide(
        &state,
        &user,
        id,
        &field(&form, "liability").unwrap_or_default(),
        &field(&form, "reason").unwrap_or_default(),
    )
    .await?;
    Ok((
        Toast::success(text(&lang, "incidents.decided")),
        renox::HxRefresh,
    )
        .into_response())
}
