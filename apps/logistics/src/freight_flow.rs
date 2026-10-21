//! The services that move a freight job along its lifecycle (03 Logistics
//! §4.2, §7.1 *Freight job*): request documents, depart, arrive, clear,
//! deliver, close and cancel before departure. Each runs the machine of
//! [`crate::freight_machine`] as `user`; what the guards need from the
//! database is worked out here and carried in the [`FreightInput`]; the client
//! is told after the commit.

use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};

use crate::freight::FreightJob;
use crate::freight_costing;
use crate::freight_machine::{
    ARRIVE, CLEAR, CLOSE, DELIVER, DEPART, FreightInput, REQUEST_DOCUMENTS,
};
use crate::notices::tell_party;
use crate::texts::english;

pub(crate) async fn load(
    state: &AppState,
    id: i64,
) -> std::result::Result<FreightJob, TransitionError> {
    FreightJob::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))
}

/// Runs the transition `name` of job `id` as `user` with `input` and answers
/// the job as it is after.
pub(crate) async fn run(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    name: &'static str,
    input: &FreightInput,
) -> std::result::Result<FreightJob, TransitionError> {
    let job = load(state, id).await?;
    let _outcome = job
        .transition(name)
        .by(user)
        .input(input)
        .run(state)
        .await?;
    load(state, id).await
}

/// Tells the client of `job` with `title` and `body`.
pub(crate) async fn tell_client(
    state: &AppState,
    job: &FreightJob,
    title: &str,
    body: &str,
) -> Result {
    tell_party(state, job.subject(), &job.number, job.client(), title, body).await
}

/// `request_documents` (`jobs.manage`): the client is told which documents
/// are missing.
///
/// # Errors
///
/// `Forbidden` without `jobs.manage`; `NotFrom` unless the job is `booked`.
pub async fn request_documents(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    missing: &str,
) -> std::result::Result<FreightJob, TransitionError> {
    let input = FreightInput {
        note: Some(missing.trim().to_owned()),
        ..FreightInput::default()
    };
    let job = run(state, user, id, REQUEST_DOCUMENTS, &input).await?;
    let title = english("logistics.freight.documents_title", &[("number", &job.number)]);
    let body = english(
        "logistics.freight.documents_body",
        &[("number", &job.number), ("missing", &missing.trim())],
    );
    tell_client(state, &job, &title, &body).await?;
    Ok(job)
}

/// `depart` (`jobs.manage`): the bill of lading or air waybill is recorded,
/// the client is told.
///
/// # Errors
///
/// `Forbidden` without `jobs.manage`; `NotFrom` unless the job is `booked` or
/// `documents_pending`; `Guard` (422) without a BL/AWB.
pub async fn depart(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    bl_awb: &str,
) -> std::result::Result<FreightJob, TransitionError> {
    let input = FreightInput {
        bl_awb: Some(bl_awb.trim().to_owned()),
        ..FreightInput::default()
    };
    let job = run(state, user, id, DEPART, &input).await?;
    let title = english("logistics.freight.departed_title", &[("number", &job.number)]);
    let body = english(
        "logistics.freight.departed_body",
        &[
            ("number", &job.number),
            ("bl", &job.bl_awb.clone().unwrap_or_default()),
        ],
    );
    tell_client(state, &job, &title, &body).await?;
    Ok(job)
}

/// `arrive` (`jobs.manage`): the job is in customs clearance.
///
/// # Errors
///
/// `Forbidden` without `jobs.manage`; `NotFrom` unless the job is
/// `in_transit`.
pub async fn arrive(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<FreightJob, TransitionError> {
    run(state, user, id, ARRIVE, &FreightInput::default()).await
}

/// `clear` (`jobs.manage`): the declaration number, the duties (fils) and the
/// delivery order are recorded.
///
/// # Errors
///
/// `Forbidden` without `jobs.manage`; `NotFrom` unless the job is in
/// `customs_clearance`; `Guard` (422) while any of the three is missing.
pub async fn clear(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    declaration_no: &str,
    duties: Option<i64>,
    delivery_order: &str,
) -> std::result::Result<FreightJob, TransitionError> {
    let input = FreightInput {
        declaration_no: Some(declaration_no.trim().to_owned()),
        duties,
        delivery_order: Some(delivery_order.trim().to_owned()),
        ..FreightInput::default()
    };
    run(state, user, id, CLEAR, &input).await
}

/// `deliver` (`jobs.deliver`): the e-POD is captured.
///
/// # Errors
///
/// `Forbidden` without `jobs.deliver`; `NotFrom` unless the job is
/// `ready_for_delivery`; `Guard` (422) without an e-POD.
pub async fn deliver(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    pod_reference: &str,
) -> std::result::Result<FreightJob, TransitionError> {
    let input = FreightInput {
        pod_reference: Some(pod_reference.trim().to_owned()),
        ..FreightInput::default()
    };
    run(state, user, id, DELIVER, &input).await
}

/// What `close` needs to know about the lines of the job.
pub(crate) async fn closing_input(
    state: &AppState,
    id: i64,
) -> std::result::Result<FreightInput, TransitionError> {
    let costs_open = freight_costing::open_costs(&state.db, id).await?;
    let (revenue_unissued, has_revenue) = freight_costing::invoicing_state(&state.db, id).await?;
    Ok(FreightInput {
        costs_open,
        revenue_unissued,
        has_revenue,
        ..FreightInput::default()
    })
}

/// `close` (`jobs.close`): every supplier bill and disbursement is allocated
/// and the invoice is issued; the margin is kept.
///
/// # Errors
///
/// `Forbidden` without `jobs.close`; `NotFrom` unless the job is `delivered`;
/// `Guard` (422) while a cost has no liability or no invoice is issued.
pub async fn close(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<FreightJob, TransitionError> {
    let input = closing_input(state, id).await?;
    run(state, user, id, CLOSE, &input).await
}

/// `cancel` (`jobs.manage`) before departure, with a reason; the quotation's
/// cancellation charges and the incurred costs are drafted on one invoice
/// ([`crate::freight_cancel`]).
///
/// # Errors
///
/// `Forbidden` without `jobs.manage`; `NotFrom` unless the job is `booked` or
/// `documents_pending`; `Guard` (422) without a reason.
pub async fn cancel(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    reason: &str,
) -> std::result::Result<FreightJob, TransitionError> {
    crate::freight_cancel::cancel(state, user, id, reason).await
}
