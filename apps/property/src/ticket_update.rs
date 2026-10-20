//! Field updates of a ticket entered after the fact (01 Property §7.2 settings
//! table row *Ticket*, `PLT.ACL.10`; BL-PRP-27).
//!
//! A technician who could not report from the site (no signal, a dead phone)
//! enters the update later. The update keeps the **work time as business
//! time**: reports and SLA figures read [`TicketUpdate::worked_at`], never the
//! time it was typed in ([`TicketUpdate::entered_at`]). The kernel's late-entry
//! check decides (`wl-kernel` §5.13) with 01's limits, which are the platform
//! defaults: more than 30 minutes late carries the late-entry flag and needs a
//! reason, more than 24 hours late waits for a holder of `late_entries.approve`
//! and the update exists only once it is approved.
//!
//! Property only registers the entity (`property.ticket_update`) and writes the
//! row; the ticket itself is wl-ticketing's and is reached by its subject.

use renox::db::{DateTime, Transaction};
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};
use wl_kernel::late_entries::{self, Entered, EntrySpec};
use wl_kernel::machines::BoxFuture;
use wl_kernel::tickets::TICKET_SUBJECT_TYPE;
use wl_kernel::{LateEntryDef, NewRecord, Subject};

/// The subject type of a ticket update, the late-entry entity.
pub const TICKET_UPDATE_SUBJECT_TYPE: &str = "property.ticket_update";

/// A field update of a ticket.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "property_ticket_updates", index(ticket_id))]
pub struct TicketUpdate {
    /// The primary key.
    pub id: i64,
    /// The ticket (wl-ticketing's id).
    pub ticket_id: i64,
    /// What was done.
    pub note: String,
    /// The business time: when the work happened.
    pub worked_at: DateTime,
    /// When the update was typed in.
    pub entered_at: DateTime,
    /// Whether it carries the late-entry flag.
    pub late: bool,
    /// Why it is late.
    pub late_reason: Option<String>,
    /// The late entry the kernel stored.
    pub late_entry_id: Option<i64>,
    /// The user who entered it.
    pub entered_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl TicketUpdate {
    /// The updates of `ticket_id`, by work time.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_ticket(db: &Db, ticket_id: i64) -> Result<Vec<TicketUpdate>> {
        TicketUpdate::query()
            .where_(TicketUpdate::TICKET_ID.eq(ticket_id))
            .order_by(TicketUpdate::WORKED_AT)
            .order_by(TicketUpdate::ID)
            .get(db)
            .await
    }
}

/// The kernel's handler: writes the row in its transaction.
fn create<'a>(tx: &'a mut Transaction, new: &'a NewRecord) -> BoxFuture<'a, Result<i64>> {
    Box::pin(async move {
        let ticket_id = new.payload["ticket_id"].as_i64().unwrap_or_default();
        let note = new.payload["note"].as_str().unwrap_or_default().to_owned();
        let row = TicketUpdate::create(
            &mut *tx,
            TicketUpdate {
                ticket_id,
                note,
                worked_at: new.occurred_at,
                entered_at: new.entered_at,
                late: new.late,
                late_reason: new.reason.clone(),
                late_entry_id: new.late_entry_id,
                entered_by: Some(new.user_id),
                ..TicketUpdate::default()
            },
        )
        .await?;
        Ok(row.id)
    })
}

/// Registers the entity with the kernel's late-entry check.
pub(crate) fn register() {
    LateEntryDef::of(TICKET_UPDATE_SUBJECT_TYPE, create)
        .business_time("worked_at")
        .parent("ticket")
        .register();
}

/// Enters an update of ticket `ticket_id` that happened at `worked_at`, typed
/// in now by `user_id`. `reason` is required when it is late (the kernel
/// refuses it without one).
///
/// # Errors
///
/// 422 for a late entry without a reason, or an empty note; the database's
/// errors.
pub async fn enter(
    state: &AppState,
    user_id: i64,
    ticket_id: i64,
    worked_at: DateTime,
    note: &str,
    reason: Option<&str>,
) -> Result<Entered> {
    if note.trim().is_empty() {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            crate::texts::english("property.ticket_updates.note_required", &[]),
        ));
    }
    let ticket = Subject::new(TICKET_SUBJECT_TYPE, ticket_id);
    let scope = wl_kernel::scopes::scope_of(&state.db, &ticket).await?;
    let mut spec = EntrySpec::new(TICKET_UPDATE_SUBJECT_TYPE, user_id, worked_at)
        .parent(ticket)
        .scope(scope)
        .payload(json!({ "ticket_id": ticket_id, "note": note.trim() }));
    if let Some(reason) = reason.map(str::trim).filter(|r| !r.is_empty()) {
        spec = spec.reason(reason);
    }
    late_entries::enter(state, spec).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entity_is_registered_as_module_dot_entity() {
        register();
        assert!(wl_kernel::late_entries::get(TICKET_UPDATE_SUBJECT_TYPE).is_some());
        let (module, entity) = TICKET_UPDATE_SUBJECT_TYPE.split_once('.').expect("a dot");
        assert_eq!((module, entity), ("property", "ticket_update"));
    }
}
