//! Small database helpers shared by the services and the machines.

use renox::db::{ToDbValue, Transaction, now};
use renox::prelude::*;

use crate::job_card::JobCard;

/// A 422 with `message`, like a refused guard.
pub(crate) fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// Sets columns of job card `id` other than the status (the machine owns it)
/// and stamps `updated_at`.
pub(crate) async fn set_job_card(
    tx: &mut Transaction,
    id: i64,
    columns: &[(&str, &(dyn ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn ToDbValue + Sync))> = columns.to_vec();
    let stamp = now();
    all.push(("updated_at", &stamp));
    JobCard::query()
        .where_(JobCard::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}
