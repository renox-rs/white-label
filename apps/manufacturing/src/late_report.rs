//! A production report typed in after the shift (07 Manufacturing §7.1 *Late
//! entry*, `PLT.ACL.10`, BL-MFG-14). When the line tablets are down the
//! operators work on paper job cards and the supervisor enters them at the
//! end of the shift, with the business time of the output. The report keeps
//! that time ([`ProductionReport::reported_at`]); the kernel flags the entry
//! with a reason when it is more than M minutes late and asks the holder of
//! `late_entries.approve` of the plant (the production manager) when it is
//! more than N hours late. M and N are the entity's limits
//! ([`SUBJECT_TYPE`]), seeded by the host.
//!
//! The entity is [`LateReport`]: what the supervisor typed. The report itself
//! (and its stock movements) is posted from it once the entry is recorded or
//! approved, with [`crate::report::post`]; a rejected entry posts nothing.
//! Goods receipts keep their arrival time in wl-inventory; the decision on a
//! returned lot has the same check in [`crate::late_inspection`].

use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::late_entries::{
    self, Entered, EntrySpec, LateEntryDef, LateEntryRecorded, NewRecord,
};
use wl_kernel::machines::BoxFuture;

use crate::db::refused;
use crate::permissions;
use crate::report::{self, ReportAsk};
use crate::texts::english;
use crate::work_order::{WorkOrder, WorkOrderStatus};

/// The subject type of the late entry of a production report: the key of the
/// limits M and N.
pub const SUBJECT_TYPE: &str = "manufacturing.late_report";

/// A production report typed in later, as it was typed.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "manufacturing_late_reports",
    index(work_order_id),
    index(late_entry_id)
)]
pub struct LateReport {
    /// The primary key.
    pub id: i64,
    /// The work order.
    pub work_order_id: i64,
    /// The plant of the work order (where the entry is approved).
    pub plant_id: i64,
    /// The good quantity, in thousandths.
    pub good_milli: i64,
    /// The scrap, in thousandths.
    pub scrap_milli: i64,
    /// A note.
    pub note: Option<String>,
    /// When the output was made (the business time).
    pub occurred_at: DateTime,
    /// When it was typed in.
    pub entered_at: DateTime,
    /// Whether it carries the late-entry flag.
    pub late: bool,
    /// Why it is late.
    pub late_reason: Option<String>,
    /// The late entry of the kernel.
    pub late_entry_id: Option<i64>,
    /// The user who typed it in.
    pub entered_by: i64,
    /// The production report posted from it.
    pub report_id: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

fn create<'a>(tx: &'a mut Transaction, new: &'a NewRecord) -> BoxFuture<'a, Result<i64>> {
    Box::pin(async move {
        let payload = &new.payload;
        let row = LateReport::create(
            &mut *tx,
            LateReport {
                work_order_id: payload["work_order_id"].as_i64().unwrap_or_default(),
                plant_id: payload["plant_id"].as_i64().unwrap_or_default(),
                good_milli: payload["good_milli"].as_i64().unwrap_or_default(),
                scrap_milli: payload["scrap_milli"].as_i64().unwrap_or_default(),
                note: payload["note"].as_str().map(str::to_owned),
                occurred_at: new.occurred_at,
                entered_at: new.entered_at,
                late: new.late,
                late_reason: new.reason.clone(),
                late_entry_id: new.late_entry_id,
                entered_by: new.user_id,
                ..LateReport::default()
            },
        )
        .await?;
        Ok(row.id)
    })
}

/// Registers the entity with the kernel's late-entry check, and the posting
/// of the report once the entry is recorded or approved.
pub(crate) fn register(app: &mut Registry) {
    LateEntryDef::of(SUBJECT_TYPE, create)
        .business_time("occurred_at")
        .parent("work_order")
        .register();
    app.listen(|event: LateEntryRecorded, state: AppState| async move {
        if event.subject.subject_type == SUBJECT_TYPE {
            post_from(&state, event.subject.subject_id).await?;
        }
        Ok(())
    });
}

/// Posts the production report of late report `id`, once.
///
/// # Errors
///
/// The 422 refusals of [`report::post`] (the work order is no longer in
/// progress); the row stays unposted.
pub async fn post_from(state: &AppState, id: i64) -> Result<Option<i64>> {
    let Some(found) = LateReport::find(&state.db, id).await? else {
        return Ok(None);
    };
    if let Some(done) = found.report_id {
        return Ok(Some(done));
    }
    let posted = report::post(
        state,
        found.entered_by,
        found.work_order_id,
        ReportAsk {
            good_milli: found.good_milli,
            scrap_milli: found.scrap_milli,
            note: found.note.clone(),
        },
        Some(found.occurred_at),
    )
    .await?;
    LateReport::query()
        .where_(LateReport::ID.eq(found.id))
        .update(
            &state.db,
            &[("report_id", &Some(posted.id)), ("updated_at", &now())],
        )
        .await?;
    Ok(Some(posted.id))
}

/// `by` enters the output `ask` that line workers made at `made_at` on work
/// order `id`, from the paper job card.
///
/// Not late: the report is posted at once. Late by more than M minutes: it is
/// flagged and posted with the reason (required). Late by more than N hours:
/// an approval request goes to the holders of `late_entries.approve` of the
/// plant and the report is posted when one approves.
///
/// # Errors
///
/// `Forbidden` without `work_orders.report` for the plant or the line; a 422
/// for a time in the future, a work order that is not `in_progress`, a
/// quantity below zero or nothing reported, or a late entry without a reason.
pub async fn enter(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    ask: ReportAsk,
    made_at: DateTime,
    reason: Option<&str>,
    outage: Option<&str>,
) -> Result<Entered> {
    let order = WorkOrder::find_or_404(&state.db, id).await?;
    if !order.may(by, permissions::WORK_ORDERS_REPORT) {
        return Err(Error::Forbidden);
    }
    if made_at > now() {
        return Err(refused(english("manufacturing.late_report.future", &[])));
    }
    if order.status != WorkOrderStatus::InProgress {
        return Err(refused(english("manufacturing.report.state", &[])));
    }
    if ask.good_milli < 0 || ask.scrap_milli < 0 || ask.good_milli + ask.scrap_milli == 0 {
        return Err(refused(english("manufacturing.report.quantity", &[])));
    }
    let mut spec = EntrySpec::new(SUBJECT_TYPE, by.id, made_at)
        .parent(order.subject())
        .scope(permissions::plant_scope(order.plant_id))
        .payload(renox::serde_json::json!({
            "work_order_id": order.id,
            "plant_id": order.plant_id,
            "good_milli": ask.good_milli,
            "scrap_milli": ask.scrap_milli,
            "note": ask.note.as_deref().map(str::trim).filter(|n| !n.is_empty()),
        }));
    if let Some(reason) = reason.map(str::trim).filter(|r| !r.is_empty()) {
        spec = spec.reason(reason);
    }
    if let Some(outage) = outage.map(str::trim).filter(|r| !r.is_empty()) {
        spec = spec.outage_reference(outage);
    }
    let entered = late_entries::enter(state, spec).await?;
    // Recorded entries are posted by the listener; this is the same posting,
    // done once.
    if let Some(record) = entered.record_id() {
        post_from(state, record).await?;
    }
    Ok(entered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entity_is_registered_as_module_dot_entity() {
        assert_eq!(SUBJECT_TYPE.split('.').count(), 2);
    }
}
