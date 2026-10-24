//! Bookings and resources in the search index and the *Related* panel
//! (BL-AGT-11, `PLT.SRC.01`, `PLT.REL.01`).
//!
//! A booking is found by its number and by its party's name (the title of
//! the party's search document, which the parties module writes); a
//! resource by its name. Both carry the scope of their location (a branch).
//! A booking's *Related* panel shows its party and resources; a resource's
//! its bookings and the employee it is.
//!
//! The documents are written by the booking's creation, every transition of
//! the booking machine ([`reindex_booking`]); resources the apps and seeds
//! insert come in with the rebuild the demo seeds end with. The invoice of
//! a booking is declared by the module that issues it (it holds the link).

use renox::auth::permissions::Scope;
use renox::db::{DbValue, Transaction, sql};
use renox::prelude::*;
use renox::state_machine::Transition;
use wl_kernel::Subject;
use wl_kernel::parties::PARTY_SUBJECT_TYPE;
use wl_kernel::related::Relation;
use wl_kernel::scheduling::BOOKING_SUBJECT_TYPE;
use wl_kernel::search::{SearchDocument, SearchDocumentRow, Searchable, index_in, kinds};
use wl_kernel::workforce::EMPLOYEE_SUBJECT_TYPE;

use crate::booking::Booking;
use crate::permissions;
use crate::resource::{RESOURCE_SUBJECT_TYPE, Resource};

/// Registers the two searchable types and their relations.
pub(crate) fn register() {
    Searchable::of::<Booking>(BOOKING_SUBJECT_TYPE)
        .permission(permissions::VIEW)
        .label("Bookings")
        .link("/scheduling/bookings/{id}")
        .document_in(|tx, booking| Box::pin(booking_document(tx, booking)))
        .register();
    Searchable::of::<Resource>(RESOURCE_SUBJECT_TYPE)
        .permission(permissions::VIEW)
        .label("Resources")
        .document(resource_document)
        .register();

    Relation::belongs_to(
        BOOKING_SUBJECT_TYPE,
        "party",
        PARTY_SUBJECT_TYPE,
        "wl_scheduling_bookings",
        "party_id",
    )
    .label("Party")
    .permission("parties.view")
    .link("/parties/{id}")
    .register();
    Relation::new(BOOKING_SUBJECT_TYPE, "resources", RESOURCE_SUBJECT_TYPE)
        .label("Resources")
        .load(|db, ids| {
            Box::pin(async move {
                pairs(
                    db,
                    "SELECT DISTINCT booking_id AS from_id, resource_id AS to_id \
                     FROM wl_scheduling_booking_resources WHERE booking_id IN ",
                    ids,
                    (BOOKING_SUBJECT_TYPE, RESOURCE_SUBJECT_TYPE),
                )
                .await
            })
        })
        .register();
    Relation::new(RESOURCE_SUBJECT_TYPE, "bookings", BOOKING_SUBJECT_TYPE)
        .label("Bookings")
        .load(|db, ids| {
            Box::pin(async move {
                pairs(
                    db,
                    "SELECT DISTINCT resource_id AS from_id, booking_id AS to_id \
                     FROM wl_scheduling_booking_resources WHERE resource_id IN ",
                    ids,
                    (RESOURCE_SUBJECT_TYPE, BOOKING_SUBJECT_TYPE),
                )
                .await
            })
        })
        .register();
    Relation::belongs_to(
        RESOURCE_SUBJECT_TYPE,
        "employee",
        EMPLOYEE_SUBJECT_TYPE,
        "wl_scheduling_resources",
        "employee_id",
    )
    .label("Employee")
    .register();
}

/// Runs `statement` (ending in `IN `) for `ids`, reading `from_id` and
/// `to_id` as records of the two `types`.
async fn pairs(
    db: &Db,
    statement: &str,
    ids: &[i64],
    types: (&str, &str),
) -> Result<Vec<(Subject, Subject)>> {
    let placeholders = vec!["?"; ids.len()].join(", ");
    let rows = sql(format!("{statement}({placeholders})"))
        .bind_all(ids.iter().map(|id| DbValue::Integer(*id)))
        .fetch_all(db)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let from: i64 = row.try_get("from_id")?;
        let to: i64 = row.try_get("to_id")?;
        out.push((Subject::new(types.0, from), Subject::new(types.1, to)));
    }
    Ok(out)
}

/// The scope of a location, global without one.
fn location_scope(location_type: Option<&str>, location_id: Option<i64>) -> Scope {
    match (location_type, location_id) {
        (Some(kind), Some(id)) if !kind.is_empty() => Scope::new(kind, id),
        _ => Scope::global(),
    }
}

/// How a booking reads in a search result: its number, its party, when.
async fn booking_document(tx: &mut Transaction, booking: &Booking) -> Result<SearchDocument> {
    let party = SearchDocumentRow::query()
        .where_eq("subject_type", PARTY_SUBJECT_TYPE)
        .where_eq("subject_id", booking.party_id)
        .first(&mut *tx)
        .await?
        .map(|row| row.title)
        .unwrap_or_default();
    let when = wl_kernel::documents::dubai_text(booking.starts_at);
    let secondary = if party.is_empty() {
        when
    } else {
        format!("{party} · {when}")
    };
    Ok(SearchDocument::new(&booking.number)
        .secondary(secondary)
        .identifier(kinds::NUMBER, &booking.number)
        .status(booking.status.as_str())
        .scope(location_scope(
            booking.location_type.as_deref(),
            booking.location_id,
        )))
}

/// How a resource reads in a search result.
fn resource_document(resource: &Resource) -> SearchDocument {
    let mut document = SearchDocument::new(&resource.name)
        .secondary(resource.resource_type.as_str().replace('_', " "))
        .status(if resource.active {
            "active"
        } else {
            "inactive"
        })
        .scope(location_scope(
            resource.location_type.as_deref(),
            resource.location_id,
        ));
    if let Some(role) = &resource.role {
        document = document.keyword(role);
    }
    document
}

/// Writes the document of booking `id` in `tx`.
pub(crate) async fn index_booking_in(tx: &mut Transaction, id: i64) -> Result {
    index_in(tx, &Subject::new(BOOKING_SUBJECT_TYPE, id)).await?;
    Ok(())
}

/// A booking transition that keeps the booking's document current.
pub(crate) fn reindex_booking(transition: Transition<Booking>) -> Transition<Booking> {
    transition.effect(|booking: &Booking, _ctx, tx| {
        let id = booking.id;
        Box::pin(async move { index_booking_in(tx, id).await })
    })
}
