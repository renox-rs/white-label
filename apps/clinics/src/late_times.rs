//! Late entry after an outage: the business time of a check-in, a check-out
//! or a consumable entered later from the paper sheet (05 §7.2 *Settings for
//! the shared negative flows*, wl-kernel §5.13, `PLT.ACL.10`; BL-CLN-15).
//!
//! The kernel decides: a time up to M minutes behind the entry is on time;
//! later, the entry is flagged and needs a reason; beyond N hours it waits
//! for a holder of `late_entries.approve`. This module registers the entity
//! `clinics.visit_time` with the kernel, whose handler writes a
//! [`VisitTime`] row and, for a check-in or a check-out, the visit's own
//! `checked_in_at` or `ended_at`: reports use the business time, never the
//! time of the entry.
//!
//! Decisions:
//!
//! - The front desk (`charges.desk`) enters the times; the kernel's limits M
//!   and N per entity are the kernel's own (30 minutes, 24 hours) until the
//!   company administrator changes them.
//! - A business time in the future is refused.
//! - A check-out time is entered for a visit that has been completed; the
//!   consumable's time for a consumable line of the visit.
//! - The time of a cash payment is wl-payments' record, not the clinic's:
//!   that entity opts in to the kernel's late entry in wl-payments (listed
//!   in `DEFERRED-b6.md`).

use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::late_entries::{self, Entered, EntrySpec, LateEntryDef, NewRecord};
use wl_kernel::machines::BoxFuture;
use wl_kernel::scopes::scope_of;

use crate::charge::{ChargeKind, ChargeLine};
use crate::db::{refused, set_visit};
use crate::permissions;
use crate::texts::english;
use crate::visit::{Visit, VisitStatus};

/// The kernel entity of a time entered late (`<module>.<entity>`).
pub const SUBJECT_TYPE: &str = "clinics.visit_time";

/// The time of the check-in.
pub const CHECK_IN: &str = "check_in";
/// The time of the check-out.
pub const CHECK_OUT: &str = "check_out";
/// The time a consumable was used.
pub const CONSUMABLE: &str = "consumable";

/// A time of a visit, as entered.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "clinics_visit_times", index(visit_id))]
pub struct VisitTime {
    /// The primary key.
    pub id: i64,
    /// The visit.
    #[model(references = "clinics_visits")]
    pub visit_id: i64,
    /// `check_in`, `check_out` or `consumable`.
    pub kind: String,
    /// The consumable line, for `consumable`.
    pub charge_line_id: Option<i64>,
    /// The business time: when it really happened.
    pub occurred_at: DateTime,
    /// The entry time: when it was typed in.
    pub entered_at: DateTime,
    /// Whether it carries the late-entry flag.
    pub late: bool,
    /// Why it is late.
    pub late_reason: Option<String>,
    /// The kernel's late entry.
    pub late_entry_id: Option<i64>,
    /// The user who entered it.
    pub entered_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl VisitTime {
    /// The times entered for visit `visit`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_visit(db: &Db, visit: i64) -> Result<Vec<VisitTime>> {
        VisitTime::query()
            .where_(VisitTime::VISIT_ID.eq(visit))
            .order_by(VisitTime::ID)
            .get(db)
            .await
    }
}

/// The kernel's handler: writes the row, and the visit's own time, in the
/// kernel's transaction.
fn create<'a>(tx: &'a mut Transaction, new: &'a NewRecord) -> BoxFuture<'a, Result<i64>> {
    Box::pin(async move {
        let visit_id = new.payload["visit_id"].as_i64().unwrap_or_default();
        let kind = new.payload["kind"].as_str().unwrap_or_default().to_owned();
        let line = new.payload["line_id"].as_i64();
        let row = VisitTime::create(
            &mut *tx,
            VisitTime {
                visit_id,
                kind: kind.clone(),
                charge_line_id: line,
                occurred_at: new.occurred_at,
                entered_at: new.entered_at,
                late: new.late,
                late_reason: new.reason.clone(),
                late_entry_id: new.late_entry_id,
                entered_by: Some(new.user_id),
                ..VisitTime::default()
            },
        )
        .await?;
        match kind.as_str() {
            CHECK_IN => set_visit(tx, visit_id, &[("checked_in_at", &new.occurred_at)]).await?,
            CHECK_OUT => {
                set_visit(tx, visit_id, &[("ended_at", &Some(new.occurred_at))]).await?;
            }
            _ => {}
        }
        Ok(row.id)
    })
}

/// Registers the entity with the kernel's late-entry check.
pub(crate) fn register() {
    LateEntryDef::of(SUBJECT_TYPE, create)
        .business_time("occurred_at")
        .parent("visit")
        .register();
}

/// Enters the time `kind` of visit `visit`, which happened at `occurred_at`,
/// typed in now by `by`. `reason` is required when it is late (the kernel
/// refuses it without one); `line` names the consumable line for
/// [`CONSUMABLE`].
///
/// # Errors
///
/// `Forbidden` without `charges.desk`; 422 for an unknown kind, a time in the
/// future, a check-out of a visit that is not completed, a consumable that is
/// not a line of the visit, or a late time without a reason; the database's
/// errors.
pub async fn enter(
    state: &AppState,
    by: &AuthUser,
    visit: i64,
    kind: &str,
    occurred_at: DateTime,
    reason: Option<&str>,
    line: Option<i64>,
) -> Result<Entered> {
    if !by.has_permission(permissions::CHARGES_DESK) {
        return Err(Error::Forbidden);
    }
    let visit = Visit::find_or_404(&state.db, visit).await?;
    if ![CHECK_IN, CHECK_OUT, CONSUMABLE].contains(&kind) {
        return Err(refused(english("clinics.late.problem.kind", &[])));
    }
    if occurred_at > now() {
        return Err(refused(english("clinics.late.problem.future", &[])));
    }
    if kind == CHECK_OUT && visit.status != VisitStatus::Completed {
        return Err(refused(english("clinics.late.problem.not_completed", &[])));
    }
    if kind == CONSUMABLE {
        let ours = match line {
            Some(id) => ChargeLine::find(&state.db, id).await?.is_some_and(|l| {
                l.visit_id == visit.id && l.kind == ChargeKind::Consumable && l.removed_on.is_none()
            }),
            None => false,
        };
        if !ours {
            return Err(refused(english("clinics.late.problem.line", &[])));
        }
    }
    let mut spec = EntrySpec::new(SUBJECT_TYPE, by.id, occurred_at)
        .parent(visit.subject())
        .scope(scope_of(&state.db, &visit.subject()).await?)
        .payload(renox::serde_json::json!({
            "visit_id": visit.id,
            "kind": kind,
            "line_id": line,
        }));
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
        assert!(wl_kernel::late_entries::get(SUBJECT_TYPE).is_some());
        let (module, entity) = SUBJECT_TYPE.split_once('.').expect("a dot");
        assert_eq!((module, entity), ("clinics", "visit_time"));
    }
}
