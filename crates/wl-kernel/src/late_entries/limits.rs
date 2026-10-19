//! M and N per entity (wl-kernel §7: "Late-entry flag after M minutes /
//! approval after N hours: 30 minutes / 24 hours, editable per entity").
//!
//! An entity without a row uses the defaults of `PLT.ACL.10`.

use renox::db::Executor;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use super::assess::Limits;

/// The limits an admin set for one entity (table `wl_late_entry_limits`,
/// one row per subject type).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_late_entry_limits", unique(subject_type))]
pub struct LateEntryLimits {
    /// The primary key.
    pub id: i64,
    /// The entity, e.g. `bookings.check_in`.
    pub subject_type: String,
    /// M: minutes of delay above which an entry is flagged.
    pub flag_after_minutes: i64,
    /// N: hours of delay above which an entry needs approval.
    pub approval_after_hours: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The limits of `subject_type`: its row, else the defaults.
pub async fn limits_for<'c>(db: impl Executor<'c>, subject_type: &str) -> Result<Limits> {
    let row = LateEntryLimits::query()
        .where_(LateEntryLimits::SUBJECT_TYPE.eq(subject_type))
        .first(db)
        .await?;
    Ok(row.map_or_else(Limits::default, |row| Limits {
        flag_after_minutes: row.flag_after_minutes,
        approval_after_hours: row.approval_after_hours,
    }))
}

/// Sets M and N of `subject_type`.
///
/// # Errors
///
/// 422 when M is not above 0 or N is below M (the approval limit cannot be
/// reached before the flag limit).
pub async fn set_limits(db: &Db, subject_type: &str, limits: Limits) -> Result<()> {
    let invalid = limits.flag_after_minutes <= 0
        || limits.approval_after_hours * 60 < limits.flag_after_minutes;
    if invalid {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            crate::texts::en("wl_kernel.late_entries.limits_invalid"),
        ));
    }
    let existing = LateEntryLimits::query()
        .where_(LateEntryLimits::SUBJECT_TYPE.eq(subject_type))
        .first(db)
        .await?;
    match existing {
        Some(mut row) => {
            row.flag_after_minutes = limits.flag_after_minutes;
            row.approval_after_hours = limits.approval_after_hours;
            row.save(db).await?;
        }
        None => {
            LateEntryLimits::create(
                db,
                LateEntryLimits {
                    subject_type: subject_type.to_owned(),
                    flag_after_minutes: limits.flag_after_minutes,
                    approval_after_hours: limits.approval_after_hours,
                    ..Default::default()
                },
            )
            .await?;
        }
    }
    Ok(())
}
