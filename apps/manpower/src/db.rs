//! Small database helpers shared by the services.

use renox::db::{ToDbValue, Transaction, now};
use renox::prelude::*;
use renox::state_machine::TransitionError;

use crate::contract::ClientContract;
use crate::deployment::Deployment;
use crate::request::ManpowerRequest;

/// A 422 with `message`, like a refused guard.
pub(crate) fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// A transition's refusal as an answer: a guard's message is a 422.
pub(crate) fn answer(error: TransitionError) -> Error {
    match error {
        TransitionError::Guard(message) => refused(message),
        other => other.into_error(),
    }
}

/// Sets columns of client contract `id` other than the status (the machine
/// owns it) and stamps `updated_at`.
pub(crate) async fn set_contract(
    tx: &mut Transaction,
    id: i64,
    columns: &[(&str, &(dyn ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn ToDbValue + Sync))> = columns.to_vec();
    let stamp = now();
    all.push(("updated_at", &stamp));
    ClientContract::query()
        .where_(ClientContract::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}

/// Client contract `id` as it is in `tx`, locked until the transaction ends
/// (PostgreSQL): the state a guard of `MPW.ACL.14` reads, so a `hold`
/// committed first wins.
pub(crate) async fn lock_contract(tx: &mut Transaction, id: i64) -> Result<ClientContract> {
    ClientContract::query()
        .where_(ClientContract::ID.eq(id))
        .lock_for_update()
        .first(&mut *tx)
        .await?
        .ok_or(Error::NotFound)
}

/// Sets columns of manpower request `id` other than the status and stamps
/// `updated_at`.
pub(crate) async fn set_request(
    tx: &mut Transaction,
    id: i64,
    columns: &[(&str, &(dyn ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn ToDbValue + Sync))> = columns.to_vec();
    let stamp = now();
    all.push(("updated_at", &stamp));
    ManpowerRequest::query()
        .where_(ManpowerRequest::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}

/// Sets columns of deployment `id` other than the status and stamps
/// `updated_at`.
pub(crate) async fn set_deployment(
    tx: &mut Transaction,
    id: i64,
    columns: &[(&str, &(dyn ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn ToDbValue + Sync))> = columns.to_vec();
    let stamp = now();
    all.push(("updated_at", &stamp));
    Deployment::query()
        .where_(Deployment::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}
