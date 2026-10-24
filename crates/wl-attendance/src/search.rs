//! Timesheets and work locations in the search index and the *Related*
//! panel (BL-AGT-11, `PLT.SRC.01`, `PLT.REL.01`).
//!
//! A timesheet is found by its worker's name (the title of the employee's
//! search document, which wl-hr writes) and its period; a work location by
//! its name. Attendance records are never indexed one by one: they are
//! reached through a work location's *Related* panel.
//!
//! Decisions: the timesheet has no number of its own in this module (the
//! spec's "timesheet number" has no column), so no identifier is invented;
//! the invoice that billed a timesheet and the client of its context are
//! declared by the modules that hold those links (invoicing, the vertical).
//! The documents are written by every transition of the timesheet machine
//! ([`reindex_timesheet`]) and by the adjustment's creation; timesheets and
//! work locations the apps insert come in with the rebuild the demo seeds
//! end with.

use renox::auth::permissions::Scope;
use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::Transition;
use wl_kernel::Subject;
use wl_kernel::attendance::{ATTENDANCE_SUBJECT_TYPE, WORK_LOCATION_SUBJECT_TYPE};
use wl_kernel::related::Relation;
use wl_kernel::search::{SearchDocument, SearchDocumentRow, Searchable, index_in};
use wl_kernel::timesheets::TIMESHEET_SUBJECT_TYPE;
use wl_kernel::workforce::EMPLOYEE_SUBJECT_TYPE;

use crate::location::WorkLocation;
use crate::permissions;
use crate::timesheet::Timesheet;

/// Registers the two searchable types and their relations.
pub(crate) fn register() {
    Searchable::of::<Timesheet>(TIMESHEET_SUBJECT_TYPE)
        .permission(permissions::VIEW)
        .label("Timesheets")
        .link("/timesheets/{id}")
        .document_in(|tx, sheet| Box::pin(timesheet_document(tx, sheet)))
        .register();
    Searchable::of::<WorkLocation>(WORK_LOCATION_SUBJECT_TYPE)
        .permission(permissions::VIEW)
        .label("Work locations")
        .document(location_document)
        .register();

    Relation::belongs_to(
        TIMESHEET_SUBJECT_TYPE,
        "employee",
        EMPLOYEE_SUBJECT_TYPE,
        "wl_attendance_timesheets",
        "owner_employee_id",
    )
    .label("Employee")
    .register();
    Relation::has_many(
        TIMESHEET_SUBJECT_TYPE,
        "adjustments",
        TIMESHEET_SUBJECT_TYPE,
        "wl_attendance_timesheets",
        "adjusts_id",
    )
    .label("Adjustments")
    .register();
    Relation::belongs_to(
        TIMESHEET_SUBJECT_TYPE,
        "adjusts",
        TIMESHEET_SUBJECT_TYPE,
        "wl_attendance_timesheets",
        "adjusts_id",
    )
    .label("Adjusts")
    .register();
    // The employee's timesheets, from this module's table.
    Relation::has_many(
        EMPLOYEE_SUBJECT_TYPE,
        "timesheets",
        TIMESHEET_SUBJECT_TYPE,
        "wl_attendance_timesheets",
        "owner_employee_id",
    )
    .label("Timesheets")
    .register();
    Relation::has_many(
        WORK_LOCATION_SUBJECT_TYPE,
        "records",
        ATTENDANCE_SUBJECT_TYPE,
        "wl_attendance_records",
        "location_id",
    )
    .label("Attendance records")
    .permission(permissions::VIEW)
    .register();
}

/// The worker's name: the employee's search document's title, if any.
async fn worker_of(tx: &mut Transaction, employee_id: i64) -> Result<String> {
    Ok(SearchDocumentRow::query()
        .where_eq("subject_type", EMPLOYEE_SUBJECT_TYPE)
        .where_eq("subject_id", employee_id)
        .first(&mut *tx)
        .await?
        .map(|row| row.title)
        .unwrap_or_default())
}

/// How a timesheet reads in a search result.
async fn timesheet_document(tx: &mut Transaction, sheet: &Timesheet) -> Result<SearchDocument> {
    let worker = worker_of(tx, sheet.owner_employee_id).await?;
    let title = match sheet.kind.as_str() {
        "adjustment" => format!(
            "Timesheet adjustment {} to {}",
            sheet.period_from, sheet.period_to
        ),
        _ => format!("Timesheet {} to {}", sheet.period_from, sheet.period_to),
    };
    let mut document = SearchDocument::new(title)
        .secondary(worker)
        .status(sheet.status.as_str());
    if let Some(reason) = &sheet.reason {
        document = document.keyword(reason);
    }
    Ok(document)
}

/// How a work location reads in a search result.
fn location_document(location: &WorkLocation) -> SearchDocument {
    SearchDocument::new(&location.name)
        .secondary(if location.outdoor {
            "Outdoor"
        } else {
            "Indoor"
        })
        .scope(
            location
                .scope()
                .map(|scope| Scope::new(&scope.subject_type, scope.subject_id))
                .unwrap_or_else(Scope::global),
        )
}

/// Writes the document of timesheet `id` in `tx`.
pub(crate) async fn index_timesheet_in(tx: &mut Transaction, id: i64) -> Result {
    index_in(tx, &Subject::new(TIMESHEET_SUBJECT_TYPE, id)).await?;
    Ok(())
}

/// A timesheet transition that keeps the timesheet's document current.
pub(crate) fn reindex_timesheet(transition: Transition<Timesheet>) -> Transition<Timesheet> {
    transition.effect(|sheet: &Timesheet, _ctx, tx| {
        let id = sheet.id;
        Box::pin(async move { index_timesheet_in(tx, id).await })
    })
}
