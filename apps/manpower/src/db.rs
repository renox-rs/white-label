//! Small database helpers shared by the services.

use renox::db::{ToDbValue, Transaction, now};
use renox::prelude::*;
use renox::state_machine::TransitionError;

use crate::contract::ClientContract;

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
