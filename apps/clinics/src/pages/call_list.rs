//! The call list while the SMS or WhatsApp connection is down (05 §8 *When a
//! connection is down*; BL-CLN-15).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /clinics/call-list` | `clinics.call_list` | `appointments.call` (the branches the user holds it in) |
//! | `POST /clinics/call-list/{booking}` | `clinics.call_list_tick` | `appointments.call` in the appointment's branch |
//!
//! The list is empty while neither connection is down. While one is, it shows
//! the `booked` and `confirmed` appointments starting in the 24 hours after it
//! went down, per branch, each with a *called* tick; the banner of the
//! connection says what to do.

use std::collections::BTreeMap;

use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::json;
use serde::Serialize;
use wl_kernel::documents::dubai_text;
use wl_kernel::integrations::web::banner_for;

use super::{Texts, text};
use crate::appointment::Appointment;
use crate::call_list::{self, SCREEN};
use crate::grid_lookup;
use crate::permissions;
use crate::texts;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/clinics/call-list", index)
        .name("clinics.call_list")
        .post("/clinics/call-list/{booking}", tick)
        .name("clinics.call_list_tick")
        .require_auth()
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "clinics/call_list.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    rows: GridPage<Appointment>,
    /// The bookings still to call: each has a confirm dialog after the grid.
    to_call: Vec<i64>,
    down: bool,
    since: String,
    integration_banner: Option<renox::serde_json::Value>,
}

/// The call list grid: the appointments of the next 24 hours of the user's
/// branches, a filter by branch (`branch_sql` and `branches`) and a search
/// that also finds the patient by name or phone (`patient_match`). The
/// patient, phone and the *called* tick are drawn by the template; no export
/// and no row link: the list names patients and phones, and the tick is the
/// row's action. It does not poll: the tick's dialog of a row that a reload
/// adds would not be on the page.
pub(crate) fn grid(
    lang: &Lang,
    patient_match: &str,
    branch_sql: &str,
    branches: Vec<(String, String)>,
) -> Grid {
    Grid::new("clinics-call-list")
        .title(&text(lang, "call_list.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::datetime("starts_at", &text(lang, "call_list.starts"))
                .frozen()
                .mobile(),
        )
        .column(
            Column::select("branch", &text(lang, "recall.branch"), branches)
                .computed(branch_sql)
                .mobile(),
        )
        .column(Column::custom("patient", &text(lang, "appointments.patient")).mobile())
        .column(Column::custom("phone", &text(lang, "call_list.phone")).mobile())
        .column(Column::custom("called", &text(lang, "call_list.called")))
        .column(Column::custom("tick", &text(lang, "call_list.tick")).frozen_right())
        // Search: the booking number and the service code (names and phones
        // are in the parties module).
        .column(
            Column::text("lookup", &text(lang, "call_list.starts"))
                .computed("CAST({T}.\"booking_id\" AS TEXT) || ' ' || {T}.\"service_code\"")
                .hidden()
                .searchable()
                .sortable(false)
                .filterable(false),
        )
        .column(
            Column::text("patient_match", &text(lang, "appointments.patient"))
                .computed(patient_match)
                .hidden()
                .searchable()
                .sortable(false)
                .filterable(false),
        )
        .sort_by("starts_at")
        .cards_on_mobile()
        .empty_state(
            &text(lang, "call_list.empty"),
            Some(&text(lang, "call_list.subtitle")),
        )
}

/// Whether `user` may use the list somewhere: globally or in a branch.
fn may_call(user: &AuthUser) -> bool {
    user.has_permission(permissions::APPOINTMENTS_CALL)
        || user.has_permission_anywhere(permissions::APPOINTMENTS_CALL)
}

/// `GET /clinics/call-list`.
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    session: Session,
    request: GridRequest,
) -> Result<View> {
    if !may_call(&user) {
        return Err(Error::Forbidden);
    }
    let dash = text(&lang, "none");
    let since = call_list::down_since(&state.db).await?;
    let entries = call_list::entries(&state, &user, &lang.locale).await?;
    // The grid pages the appointments the user may call (`-1`: none).
    let ids: Vec<i64> = entries
        .iter()
        .map(|entry| entry.appointment.booking_id)
        .collect();
    let query = Appointment::query().where_(Appointment::BOOKING_ID.is_in(if ids.is_empty() {
        vec![-1]
    } else {
        ids
    }));
    // The branch and the patient are worked out per entry: the filter and the
    // search read them through SQL of the booking ids.
    let branches: BTreeMap<i64, String> = entries
        .iter()
        .map(|entry| (entry.appointment.booking_id, entry.branch.clone()))
        .collect();
    let (branch_sql, branch_options) = grid_lookup::labelled("booking_id", &branches);
    let term = grid_lookup::searched(&request, &session, "clinics-call-list");
    let found: Vec<i64> = match term.as_deref() {
        Some(term) => {
            let wanted = term.to_lowercase();
            entries
                .iter()
                .filter(|entry| {
                    entry.patient.to_lowercase().contains(&wanted)
                        || entry
                            .phone
                            .as_deref()
                            .is_some_and(|phone| phone.contains(term))
                })
                .map(|entry| entry.appointment.booking_id)
                .collect()
        }
        None => Vec::new(),
    };
    let patient_match = grid_lookup::matched_sql(term.as_deref(), &["booking_id"], &found);
    let page = grid(&lang, &patient_match, &branch_sql, branch_options)
        .page(query, &request)
        .await?;
    let mut shown = BTreeMap::new();
    let mut to_call = Vec::new();
    for entry in &entries {
        let booking = entry.appointment.booking_id;
        if !page.items().iter().any(|a| a.booking_id == booking) {
            continue;
        }
        if entry.called.is_none() {
            to_call.push(booking);
        }
        shown.insert(
            booking,
            json!({
                "booking": booking,
                "branch": entry.branch,
                "patient": entry.patient,
                "phone": entry.phone.clone().unwrap_or_else(|| dash.clone()),
                "called": entry.called.is_some(),
                "called_text": match &entry.called {
                    Some(tick) => format!(
                        "{} — {}",
                        super::user_name(&state.db, tick.called_by.unwrap_or_default()).await,
                        dubai_text(tick.called_at)
                    ),
                    None => dash.clone(),
                },
            }),
        );
    }
    let page = page.extend(|appointment| {
        shown
            .get(&appointment.booking_id)
            .cloned()
            .unwrap_or_default()
    });
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        rows: page,
        to_call,
        down: since.is_some(),
        since: since.map_or_else(|| dash.clone(), dubai_text),
        integration_banner: banner_for(&state.db, SCREEN, &lang).await?,
    }))
}

/// `POST /clinics/call-list/{booking}`: ticks the appointment as called.
async fn tick(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(booking): Path<i64>,
) -> Result<Response> {
    if !may_call(&user) {
        return Err(Error::Forbidden);
    }
    call_list::tick(&state, &user, booking).await?;
    Ok((
        Toast::success(text(&lang, "call_list.ticked")),
        renox::HxRefresh,
    )
        .into_response())
}
