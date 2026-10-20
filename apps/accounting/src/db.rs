//! Small database helpers shared by the services.

use renox::db::{ToDbValue, Transaction, now};
use renox::prelude::*;

use crate::cdd::CddFile;
use crate::engagement::Engagement;
use crate::tax_return::Return;

/// A 422 with `message`, like a refused guard.
pub(crate) fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// Sets columns of engagement `id` other than the status (the machine owns
/// it) and stamps `updated_at`.
pub(crate) async fn set_engagement(
    tx: &mut Transaction,
    id: i64,
    columns: &[(&str, &(dyn ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn ToDbValue + Sync))> = columns.to_vec();
    let stamp = now();
    all.push(("updated_at", &stamp));
    Engagement::query()
        .where_(Engagement::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}

/// Sets columns of CDD file `id` other than the status and stamps
/// `updated_at`.
pub(crate) async fn set_cdd_file(
    tx: &mut Transaction,
    id: i64,
    columns: &[(&str, &(dyn ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn ToDbValue + Sync))> = columns.to_vec();
    let stamp = now();
    all.push(("updated_at", &stamp));
    CddFile::query()
        .where_(CddFile::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}

/// Sets columns of return `id` other than the status and stamps
/// `updated_at`.
pub(crate) async fn set_return(
    tx: &mut Transaction,
    id: i64,
    columns: &[(&str, &(dyn ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn ToDbValue + Sync))> = columns.to_vec();
    let stamp = now();
    all.push(("updated_at", &stamp));
    Return::query()
        .where_(Return::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}
