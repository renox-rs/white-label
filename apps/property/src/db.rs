//! Small database helpers shared by the lease code.

use renox::db::{ToDbValue, Transaction, now};
use renox::prelude::*;

use crate::lease::Lease;

pub(crate) use crate::building::refused;

/// Sets columns of lease `id` other than the status (the machine owns it)
/// and stamps `updated_at`.
pub(crate) async fn set_lease(
    tx: &mut Transaction,
    id: i64,
    columns: &[(&str, &(dyn ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn ToDbValue + Sync))> = columns.to_vec();
    let stamp = now();
    all.push(("updated_at", &stamp));
    Lease::query()
        .where_(Lease::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}
