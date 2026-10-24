//! The freight job (03 Logistics §4.2, §6 *Freight job*, §7.1 *Freight job*,
//! `LOG.JOB.01`; BL-LOG-19 and the freight issues BL-LOG-21 to BL-LOG-24).
//!
//! A job is opened for a client by a user with `freight.manage` and starts
//! `booked`. Opening one is new business: it asks the block guard
//! ([`crate::block::refuse_start`]) and a blocked client's job is refused with
//! the neutral text. A job that is open continues whatever happens to the
//! client: [`add_note`] is not guarded.
//!
//! The lifecycle is [`crate::freight_machine`] (booked, documents pending, in
//! transit, customs clearance, ready for delivery, on hold, delivered, closed,
//! cancelled) and the services that run it are in [`crate::freight_flow`]; the
//! screen is [`crate::freight_pages`]. A job keeps its milestones as
//! [`FreightEvent`]s, its containers with their free-time end dates
//! ([`FreightContainer`]) and its revenue and cost lines
//! ([`crate::freight_costing`], the stand-in for job costing `FIN.CST.01`).

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;
use wl_kernel::numbering::{Reset, Series};
use wl_kernel::{PartyId, Subject};

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

/// The subject type of a freight job.
pub const SUBJECT_TYPE: &str = "logistics.freight_job";

/// Where a freight job is (§7.1).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FreightStatus {
    /// Opened from an accepted quotation; it continues even when the client
    /// is blocked.
    #[default]
    Booked,
    /// Waiting for shipping documents.
    DocumentsPending,
    /// Departed origin.
    InTransit,
    /// Arrived; the declaration is in progress.
    CustomsClearance,
    /// Cleared; the delivery order is received.
    ReadyForDelivery,
    /// Stopped for a reason outside the company's control (BL-LOG-21).
    OnHold,
    /// Delivered with e-POD.
    Delivered,
    /// All costs recorded and invoiced; reopened only for a late cost.
    Closed,
    /// Cancelled before departure (terminal).
    Cancelled,
}

/// The ways a freight job moves (`mode`).
pub const MODES: [&str; 3] = ["sea", "air", "road"];

/// A freight job of a client.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "logistics_freight_jobs",
    state_machine,
    unique(number),
    index(client_party_id)
)]
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
    /// `sea`, `air` or `road` ([`MODES`]).
    pub mode: String,
    /// Where it starts.
    pub origin: String,
    /// Where it goes.
    pub destination: String,
    /// The bill of lading or air waybill number (needed to `depart`).
    pub bl_awb: Option<String>,
    /// The customs declaration number (needed to `clear`).
    pub declaration_no: Option<String>,
    /// The duties recorded, in fils (needed to `clear`).
    pub duties: Option<i64>,
    /// The reference of the delivery order attached (needed to `clear`).
    pub delivery_order_ref: Option<String>,
    /// The date the job is expected to arrive (`LOG.RUL.09`).
    pub eta: Option<NaiveDate>,
    /// Marked international with the client contracted for the whole move
    /// (the VAT of §9).
    pub international: bool,
    /// The reference of the e-POD (needed to `deliver`).
    pub pod_ref: Option<String>,
    /// Why it was cancelled.
    pub cancel_reason: Option<String>,
    /// What documents are missing, when it is `documents_pending`.
    pub documents_note: Option<String>,
    /// Why it is (or was last) on hold: one of [`crate::freight_hold::reason`].
    pub hold_reason: Option<String>,
    /// The note of the hold, shown to the client.
    pub hold_note: Option<String>,
    /// The state it was in when it was last held: `in_transit`,
    /// `customs_clearance` or `ready_for_delivery`.
    pub held_from: Option<String>,
    /// When it was last held.
    pub held_at: Option<DateTime>,
    /// When it was last closed.
    pub closed_at: Option<DateTime>,
    /// The evidence for zero-rating is attached (the VAT of §9).
    pub zero_rate_evidence: bool,
    /// The ETA `LOG.RUL.09` last told the documentation officer about.
    pub eta_alerted_for: Option<NaiveDate>,
    /// Where it is; changed only by a transition.
    pub status: FreightStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl FreightJob {
    /// The job as a [`Subject`]: `logistics.freight_job:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// The client.
    pub fn client(&self) -> PartyId {
        PartyId(self.client_party_id)
    }

    /// The milestones and other events, oldest first.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn events(&self, db: &Db) -> Result<Vec<FreightEvent>> {
        FreightEvent::query()
            .where_(FreightEvent::JOB_ID.eq(self.id))
            .order_by(FreightEvent::ID)
            .get(db)
            .await
    }

    /// The containers, in the order they were added.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn containers(&self, db: &Db) -> Result<Vec<FreightContainer>> {
        FreightContainer::query()
            .where_(FreightContainer::JOB_ID.eq(self.id))
            .order_by(FreightContainer::ID)
            .get(db)
            .await
    }
}

/// A milestone or other event of a freight job.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "logistics_freight_events", index(job_id))]
pub struct FreightEvent {
    /// The primary key.
    pub id: i64,
    /// The job.
    #[model(references = "logistics_freight_jobs")]
    pub job_id: i64,
    /// `booked`, `documents_requested`, `departed`, `arrived`, `cleared`,
    /// `held`, `released`, `delivered`, `closed`, `reopened`, `cancelled` or
    /// `note`.
    pub kind: String,
    /// A note.
    pub note: String,
    /// When it happened.
    pub occurred_at: DateTime,
    /// The user who did it; none for the system.
    pub actor_user_id: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A container of a freight job with its free-time end dates (port storage,
/// demurrage, detention; `LOG.RUL.14` reads them).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "logistics_freight_containers", index(job_id))]
pub struct FreightContainer {
    /// The primary key.
    pub id: i64,
    /// The job.
    #[model(references = "logistics_freight_jobs")]
    pub job_id: i64,
    /// The container number.
    pub number: String,
    /// The size, `40HC`.
    pub size: String,
    /// The last free day of port storage.
    pub port_storage_end: Option<NaiveDate>,
    /// The last free day before demurrage.
    pub demurrage_end: Option<NaiveDate>,
    /// The last free day before detention.
    pub detention_end: Option<NaiveDate>,
    /// The day the container was collected: port storage and demurrage stop
    /// after it (`LOG.RUL.14`).
    pub collected_on: Option<NaiveDate>,
    /// The day the container was returned empty: detention stops after it.
    pub returned_empty_on: Option<NaiveDate>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// Writes an event of `kind` for the job `job_id`.
///
/// # Errors
///
/// Database errors.
pub async fn track<'c, E: renox::db::Executor<'c>>(
    executor: E,
    job_id: i64,
    kind: &str,
    note: &str,
    actor: Option<i64>,
) -> Result {
    let mut event = FreightEvent {
        job_id,
        kind: kind.to_owned(),
        note: note.to_owned(),
        occurred_at: renox::db::now(),
        actor_user_id: actor,
        ..FreightEvent::default()
    };
    event.insert(executor).await?;
    Ok(())
}

/// What is entered when a job is opened, besides the client and what is
/// shipped.
#[derive(Debug, Clone, Default)]
pub struct NewJob {
    /// `sea`, `air` or `road`; `road` when empty.
    pub mode: String,
    /// Where it starts.
    pub origin: String,
    /// Where it goes.
    pub destination: String,
    /// The date it is expected to arrive.
    pub eta: Option<NaiveDate>,
    /// International with the client contracted for the whole move.
    pub international: bool,
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
    open_with(state, user, client, description, &NewJob::default()).await
}

/// [`open`] with the mode, route, ETA and the international mark.
///
/// # Errors
///
/// As [`open`], and `Guard` (422) for a mode that is not `sea`, `air` or
/// `road`.
pub async fn open_with(
    state: &AppState,
    user: &AuthUser,
    client: PartyId,
    description: &str,
    new: &NewJob,
) -> std::result::Result<FreightJob, TransitionError> {
    if !user.has_permission(permissions::FREIGHT_MANAGE) {
        return Err(TransitionError::Forbidden);
    }
    create(state, Some(user.id), client, description, new).await
}

/// Opens a freight job for `client` as `opened_by` (a user, or none for the
/// system's seed), with the checks of [`open_with`] except the permission.
///
/// # Errors
///
/// As [`open_with`] without `Forbidden`.
pub async fn create(
    state: &AppState,
    opened_by: Option<i64>,
    client: PartyId,
    description: &str,
    new: &NewJob,
) -> std::result::Result<FreightJob, TransitionError> {
    if description.trim().is_empty() {
        return Err(guard(english("logistics.freight.description_needed", &[])));
    }
    let mode = if new.mode.trim().is_empty() {
        "road".to_owned()
    } else {
        new.mode.trim().to_ascii_lowercase()
    };
    if !MODES.contains(&mode.as_str()) {
        return Err(guard(english("logistics.freight.mode_unknown", &[])));
    }
    block::refuse_start(state, client).await?;
    let number = FREIGHT_SERIES
        .next_allowing_gaps(&state.db, None, None, today())
        .await?;
    let mut job = FreightJob {
        number,
        client_party_id: client.0,
        description: description.trim().to_owned(),
        opened_by,
        mode,
        origin: new.origin.trim().to_owned(),
        destination: new.destination.trim().to_owned(),
        eta: new.eta,
        international: new.international,
        status: FreightStatus::Booked,
        ..FreightJob::default()
    };
    job.insert(&state.db).await?;
    track(&state.db, job.id, "booked", &job.description, opened_by).await?;
    crate::search::index(&state.db, &job.subject()).await?;
    Ok(job)
}

/// `user` (`freight.manage`) adds a container to the job `id`.
///
/// # Errors
///
/// `Forbidden` without `freight.manage`; `Guard` (422) for a blank number or
/// one the job has already.
pub async fn add_container(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    number: &str,
    size: &str,
    dates: [Option<NaiveDate>; 3],
) -> std::result::Result<FreightContainer, TransitionError> {
    if !user.has_permission(permissions::FREIGHT_MANAGE) {
        return Err(TransitionError::Forbidden);
    }
    insert_container(state, id, number, size, dates).await
}

/// Adds a container to the job `id` without a permission check (the seed's).
///
/// # Errors
///
/// As [`add_container`] without `Forbidden`.
pub async fn insert_container(
    state: &AppState,
    id: i64,
    number: &str,
    size: &str,
    dates: [Option<NaiveDate>; 3],
) -> std::result::Result<FreightContainer, TransitionError> {
    let job = FreightJob::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    let number = number.trim().to_ascii_uppercase();
    if number.is_empty() {
        return Err(guard(english("logistics.freight.container_needed", &[])));
    }
    let known = FreightContainer::query()
        .where_(FreightContainer::JOB_ID.eq(id))
        .where_(FreightContainer::NUMBER.eq(number.as_str()))
        .first(&state.db)
        .await?;
    if known.is_some() {
        return Err(guard(english("logistics.freight.container_used", &[])));
    }
    let [port_storage_end, demurrage_end, detention_end] = dates;
    let mut container = FreightContainer {
        job_id: job.id,
        number,
        size: size.trim().to_owned(),
        port_storage_end,
        demurrage_end,
        detention_end,
        ..FreightContainer::default()
    };
    container.insert(&state.db).await?;
    // The container number finds the job (BL-AGT-12).
    crate::search::index(&state.db, &Subject::new(SUBJECT_TYPE, container.job_id)).await?;
    Ok(container)
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
        .post("/freight/{id}/containers", container_job)
        .name("logistics.freight_container")
        .require_auth()
}

/// A date typed as `2026-11-20`; none when blank or not a date.
pub(crate) fn date_in(text: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d").ok()
}

/// `POST /freight`: `client_party_id` and `description`, and optionally
/// `mode`, `origin`, `destination`, `eta` and `international`.
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
            Error::Status(
                StatusCode::UNPROCESSABLE_ENTITY,
                t(&lang, "logistics.bad_input"),
            )
        })?;
    let new = NewJob {
        mode: field(&form, "mode"),
        origin: field(&form, "origin"),
        destination: field(&form, "destination"),
        eta: date_in(&field(&form, "eta")),
        international: matches!(field(&form, "international").trim(), "1" | "on" | "true"),
    };
    let job = open_with(
        &state,
        &user,
        PartyId(client),
        &field(&form, "description"),
        &new,
    )
    .await
    .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(t(&lang, "logistics.freight_opened")),
        Redirect::route("logistics.freight_job", &[&job.id])?,
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

/// `POST /freight/{id}/containers`: `number`, `size` and the three free-time
/// end dates `port_storage_end`, `demurrage_end` and `detention_end`.
async fn container_job(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    add_container(
        &state,
        &user,
        id,
        &field(&form, "number"),
        &field(&form, "size"),
        [
            date_in(&field(&form, "port_storage_end")),
            date_in(&field(&form, "demurrage_end")),
            date_in(&field(&form, "detention_end")),
        ],
    )
    .await
    .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(t(&lang, "logistics.freight_container_added")),
        Redirect::route("logistics.freight_job", &[&id])?,
    )
        .into_response())
}
