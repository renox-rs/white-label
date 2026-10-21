//! The freight job, as far as a blocked client needs it (03 Logistics §6
//! *Freight job*, checklist row *Freight job*: *new jobs refused; open jobs
//! continue*; BL-LOG-19).
//!
//! A job is opened for a client by a user with `freight.manage` and starts
//! `open`. Opening one is new business: it asks the block guard
//! ([`crate::block::refuse_start`]) and a blocked client's job is refused with
//! the neutral text. A job that is open continues whatever happens to the
//! client: [`add_note`] is not guarded. The lifecycle of the job (departure,
//! cancel, hold, reopen for late costs, close) belongs to the freight issues
//! and is not built here.

use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;
use wl_kernel::numbering::{Reset, Series};
use wl_kernel::documents::today;

use crate::block;
use crate::pages::{Fields, field};
use crate::permissions;
use crate::texts::{english, t};

/// `FRJ-{0000}`: the number of a freight job.
pub const FREIGHT_SERIES: Series = Series {
    key: "freight_job",
    pattern: "{prefix}-{0000}",
    prefix: "FRJ",
    reset: Reset::Never,
    gapless: false,
    per_entity: false,
};

/// Where a freight job is.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FreightStatus {
    /// Opened, not closed: it continues even when the client is blocked.
    #[default]
    Open,
}

/// A freight job of a client.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "logistics_freight_jobs", unique(number), index(client_party_id))]
pub struct FreightJob {
    /// The primary key.
    pub id: i64,
    /// The number from the `freight_job` series.
    pub number: String,
    /// The client.
    pub client_party_id: i64,
    /// What is shipped.
    pub description: String,
    /// The user who opened it.
    pub opened_by: Option<i64>,
    /// The notes added, one per line.
    pub notes: String,
    /// Where it is; only `open` is built here.
    pub status: FreightStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

fn guard(message: String) -> TransitionError {
    TransitionError::Guard(message)
}

/// `user` (`freight.manage`) opens a freight job for `client`.
///
/// # Errors
///
/// `Forbidden` without `freight.manage`; `Guard` (422) without a description
/// and, with the neutral text, for a blocked client.
pub async fn open(
    state: &AppState,
    user: &AuthUser,
    client: PartyId,
    description: &str,
) -> std::result::Result<FreightJob, TransitionError> {
    if !user.has_permission(permissions::FREIGHT_MANAGE) {
        return Err(TransitionError::Forbidden);
    }
    if description.trim().is_empty() {
        return Err(guard(english("logistics.freight.description_needed", &[])));
    }
    block::refuse_start(state, client).await?;
    let number = FREIGHT_SERIES
        .next_allowing_gaps(&state.db, None, None, today())
        .await?;
    let mut job = FreightJob {
        number,
        client_party_id: client.0,
        description: description.trim().to_owned(),
        opened_by: Some(user.id),
        status: FreightStatus::Open,
        ..FreightJob::default()
    };
    job.insert(&state.db).await?;
    Ok(job)
}

/// `user` (`freight.manage`) adds a note to the open job `id`. The block does
/// not stop it: an open job continues.
///
/// # Errors
///
/// `Forbidden` without `freight.manage`; `Guard` (422) for a blank note.
pub async fn add_note(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    note: &str,
) -> std::result::Result<FreightJob, TransitionError> {
    if !user.has_permission(permissions::FREIGHT_MANAGE) {
        return Err(TransitionError::Forbidden);
    }
    if note.trim().is_empty() {
        return Err(guard(english("logistics.freight.note_needed", &[])));
    }
    let job = FreightJob::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    let notes = if job.notes.is_empty() {
        note.trim().to_owned()
    } else {
        format!("{}\n{}", job.notes, note.trim())
    };
    FreightJob::query()
        .where_(FreightJob::ID.eq(id))
        .update(&state.db, &[("notes", &notes)])
        .await?;
    FreightJob::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))
}

/// The routes.
pub fn routes() -> Routes {
    Routes::new()
        .post("/freight", open_job)
        .name("logistics.freight_open")
        .post("/freight/{id}/note", note_job)
        .name("logistics.freight_note")
        .require_auth()
}

/// `POST /freight`: `client_party_id` and `description`.
async fn open_job(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Fields,
) -> Result<Response> {
    let client = field(&form, "client_party_id")
        .trim()
        .parse::<i64>()
        .map_err(|_| {
            Error::Status(StatusCode::UNPROCESSABLE_ENTITY, t(&lang, "logistics.bad_input"))
        })?;
    open(&state, &user, PartyId(client), &field(&form, "description"))
        .await
        .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(t(&lang, "logistics.freight_opened")),
        Redirect::route("logistics.shipments", &[])?,
    )
        .into_response())
}

/// `POST /freight/{id}/note`: `note`.
async fn note_job(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    add_note(&state, &user, id, &field(&form, "note"))
        .await
        .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(t(&lang, "logistics.freight_noted")),
        Redirect::route("logistics.shipments", &[])?,
    )
        .into_response())
}
