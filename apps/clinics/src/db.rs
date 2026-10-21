//! Small database helpers shared by the services.

use renox::db::{ToDbValue, Transaction, now};
use renox::prelude::*;

use crate::visit::Visit;

/// A 422 with `message`, like a refused guard.
pub(crate) fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// Sets columns of visit `id` other than the status (the machine owns it)
/// and stamps `updated_at`.
pub(crate) async fn set_visit(
    tx: &mut Transaction,
    id: i64,
    columns: &[(&str, &(dyn ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn ToDbValue + Sync))> = columns.to_vec();
    let stamp = now();
    all.push(("updated_at", &stamp));
    Visit::query()
        .where_(Visit::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}
