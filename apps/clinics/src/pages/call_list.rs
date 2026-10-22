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

use renox::prelude::*;
use serde::Serialize;
use wl_kernel::documents::dubai_text;
use wl_kernel::integrations::web::banner_for;

use super::{Texts, text};
use crate::call_list::{self, SCREEN};
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

/// One row of the list.
#[derive(Serialize, Debug, Clone)]
pub struct CallRow {
    /// The booking.
    pub booking: i64,
    /// The branch.
    pub branch: String,
    /// The patient.
    pub patient: String,
    /// The patient's mobile number, or a dash.
    pub phone: String,
    /// When the appointment starts.
    pub starts: String,
    /// Whether the front desk has called.
    pub called: bool,
    /// Who called and when, or a dash.
    pub called_text: String,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "clinics/call_list.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    rows: Vec<CallRow>,
    down: bool,
    since: String,
    integration_banner: Option<renox::serde_json::Value>,
}

/// Whether `user` may use the list somewhere: globally or in a branch.
fn may_call(user: &AuthUser) -> bool {
    user.has_permission(permissions::APPOINTMENTS_CALL)
        || user.has_permission_anywhere(permissions::APPOINTMENTS_CALL)
}

/// `GET /clinics/call-list`.
async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    if !may_call(&user) {
        return Err(Error::Forbidden);
    }
    let dash = text(&lang, "none");
    let since = call_list::down_since(&state.db).await?;
    let mut rows = Vec::new();
    for entry in call_list::entries(&state, &user, &lang.locale).await? {
        rows.push(CallRow {
            booking: entry.appointment.booking_id,
            branch: entry.branch.clone(),
            patient: entry.patient.clone(),
            phone: entry.phone.clone().unwrap_or_else(|| dash.clone()),
            starts: dubai_text(entry.appointment.starts_at),
            called: entry.called.is_some(),
            called_text: match &entry.called {
                Some(tick) => format!(
                    "{} — {}",
                    super::user_name(&state.db, tick.called_by.unwrap_or_default()).await,
                    dubai_text(tick.called_at)
                ),
                None => dash.clone(),
            },
        });
    }
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        rows,
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
