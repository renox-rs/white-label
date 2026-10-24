//! A test drive entered late (08 §7.1 settings *Late entry*, wl-kernel §5.13,
//! `PLT.ACL.10`; BL-AUT-18): the advisor or technician writes up a drive from
//! the paper sheet after an outage, with the time it really left.
//!
//! The kernel decides: a time up to M minutes behind the entry is on time;
//! later, the entry is flagged and needs a reason; beyond 24 hours it waits
//! for a holder of `late_entries.approve` (the workshop manager and the branch
//! manager in an 08 deployment) and nothing is written until it is approved,
//! so a rejected entry creates nothing. This module registers the entity
//! `auto.test_drive_time` whose handler writes the [`TestDrive`] with the
//! business time as its `out_at`: reports use the business time, never the time
//! of the entry.
//!
//! Decisions: the job card times and the deliveries 08 names keep their own
//! business times already (the job card history is written by the transitions,
//! the delivery is the invoice on `deliver`); only the test drive is typed in
//! after the fact, so it is the one entity entered late here.

use renox::db::Transaction;
use renox::prelude::*;
use wl_kernel::late_entries::{self, Entered, EntrySpec, LateEntryDef, NewRecord};
use wl_kernel::machines::BoxFuture;
use wl_kernel::scopes::scope_of;

use crate::test_drive::TestDrive;
use crate::test_drives::{NewDrive, check_new};

/// The kernel entity of a test drive entered late (`<module>.<entity>`).
pub const SUBJECT_TYPE: &str = "auto.test_drive_time";

/// The kernel's handler: writes the drive in the kernel's transaction.
fn create<'a>(tx: &'a mut Transaction, new: &'a NewRecord) -> BoxFuture<'a, Result<i64>> {
    Box::pin(async move {
        let job_card_id = new.payload["job_card_id"].as_i64().unwrap_or_default();
        let odometer_out = new.payload["odometer_out"].as_i64().unwrap_or_default();
        let in_at = new.payload["in_at"]
            .as_i64()
            .and_then(|secs| DateTime::from_timestamp(secs, 0));
        let odometer_in = new.payload["odometer_in"].as_i64();
        let drive = TestDrive::create(
            &mut *tx,
            TestDrive {
                job_card_id,
                driver_user_id: new.user_id,
                out_at: new.occurred_at,
                odometer_out,
                in_at,
                odometer_in,
                ..TestDrive::default()
            },
        )
        .await?;
        Ok(drive.id)
    })
}

/// Registers the entity with the kernel's late-entry check.
pub(crate) fn register() {
    LateEntryDef::of(SUBJECT_TYPE, create)
        .business_time("out_at")
        .parent("job_card")
        .register();
}

/// Enters the test drive `new` of job card `job_card_id`, which left at
/// `new.out_at`, typed in now by `by`. `reason` is required when it is late
/// (the kernel refuses it without one).
///
/// # Errors
///
/// 403 for someone who may not work the job card; 422 for a drive without a
/// time out, a time in the future, the errors of
/// [`record`](crate::test_drives::record) and a late time without a reason;
/// the database's errors.
pub async fn enter(
    state: &AppState,
    by: &AuthUser,
    job_card_id: i64,
    new: NewDrive,
    reason: Option<&str>,
) -> Result<Entered> {
    let Some(left) = new.out_at else {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            crate::texts::english("auto.late.problem.no_time", &[]),
        ));
    };
    if left > renox::db::now() {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            crate::texts::english("auto.late.problem.future", &[]),
        ));
    }
    let (card, _) = check_new(state, by, job_card_id, &new).await?;
    let mut spec = EntrySpec::new(SUBJECT_TYPE, by.id, left)
        .parent(card.subject())
        .scope(scope_of(&state.db, &card.subject()).await?)
        .payload(renox::serde_json::json!({
            "job_card_id": card.id,
            "odometer_out": new.odometer_out,
            "in_at": new.in_at.map(|at| at.timestamp()),
            "odometer_in": new.odometer_in,
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
        assert_eq!((module, entity), ("auto", "test_drive_time"));
    }
}
