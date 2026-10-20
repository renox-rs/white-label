//! The module's notifications, sent through the kernel
//! [`Notifier`](wl_kernel::notify::Notifier): transactional, with English
//! texts first. Without wl-notify the kernel's fallback puts them in the
//! bell.
//!
//! Every flag tells the reviewers (the holders of `attendance.review`, the
//! effect of the `flag` transition). Besides them (wl-attendance §6
//! *Automations*):
//!
//! - `overlap` and `overtime_cap` (`HRM.RUL.13`): the supervisor;
//! - `midday_break` (`HRM.RUL.01`, `HRM.RUL.13`): the supervisor and the HSE
//!   officer;
//! - `permit_invalid` (`HRM.RUL.14`): HR and the supervisor.
//!
//! The supervisor is the user who marked the record; for a self check-in it
//! is the employee's manager.

use std::collections::BTreeSet;

use renox::auth::permissions::{Scope, users_with_permission_in, users_with_role};
use renox::prelude::*;
use renox::serde_json::json;
use wl_kernel::documents::dubai_text;
use wl_kernel::notify::{self, NotificationTypeDef, NotifyChannel, NotifyRequest};
use wl_kernel::workforce::{StaffProfile, workforce};

use crate::config::config_of;
use crate::flags::{Added, FlagType};
use crate::location::WorkLocation;
use crate::permissions;
use crate::record::{AttendanceMethod, AttendanceRecord};
use crate::texts::english;

/// The people are told that a record was flagged.
pub const RECORD_FLAGGED: &str = "attendance.record_flagged";

/// Registers the module's notification types with the kernel registry.
pub(crate) fn register_types() {
    let subject = english("wl_attendance.notices.flagged_subject", &[]);
    let body = english("wl_attendance.notices.flagged_body", &[]);
    notify::register_type(
        NotificationTypeDef::new(RECORD_FLAGGED, "attendance")
            .group("attendance")
            .template(NotifyChannel::Email, "en", Some(&subject), &body),
    );
}

/// The English name of a flag type for staff-facing text.
pub fn flag_text(flag: FlagType) -> String {
    english(&format!("wl_attendance.flags.{}", flag.as_str()), &[])
}

/// The users who hold `attendance.review`.
pub async fn reviewers(db: &Db) -> Result<Vec<i64>> {
    Ok(
        users_with_permission_in(db, permissions::REVIEW, &Scope::global())
            .await?
            .iter()
            .map(|u| u.id)
            .collect(),
    )
}

/// The users who hold `role` (globally).
async fn holders(db: &Db, role: &str) -> Result<Vec<i64>> {
    Ok(users_with_role(db, role)
        .await?
        .iter()
        .map(|u| u.id)
        .collect())
}

/// The supervisor of `record`: who marked it, else (a self check-in) the
/// employee's managers.
pub fn supervisors(record: &AttendanceRecord, profile: Option<&StaffProfile>) -> Vec<i64> {
    match (record.method, record.recorded_by) {
        (AttendanceMethod::SelfCheckIn, _) | (_, None) => {
            profile.map(|p| p.managers.clone()).unwrap_or_default()
        }
        (_, Some(user)) => vec![user],
    }
}

/// Tells the people about the flag that was added.
pub(crate) async fn flag_added(state: &AppState, added: &Added) -> Result {
    let config = config_of(state);
    let record = &added.record;
    let profile = workforce(state)
        .staff(state, record.employee())
        .await?;
    let mut users: BTreeSet<i64> = reviewers(&state.db).await?.into_iter().collect();
    users.extend(supervisors(record, profile.as_ref()));
    match added.flag {
        FlagType::MiddayBreak => users.extend(holders(&state.db, &config.hse_role).await?),
        FlagType::PermitInvalid => users.extend(holders(&state.db, &config.hr_role).await?),
        _ => {}
    }
    if users.is_empty() {
        return Ok(());
    }
    let employee = match &profile {
        Some(profile) => wl_kernel::parties::directory(state)
            .display_name(&state.db, profile.party, "en")
            .await
            .ok()
            .flatten(),
        None => None,
    }
    .unwrap_or_else(|| format!("#{}", record.employee_id));
    let location = WorkLocation::find(&state.db, record.location_id)
        .await?
        .map_or_else(|| format!("#{}", record.location_id), |l| l.name);
    let mut request = NotifyRequest::new(RECORD_FLAGGED, record.record_id().subject())
        .subject_label(&format!("{employee} {}", record.date))
        .data(json!({
            "employee": employee,
            "location": location,
            "date": record.date.to_string(),
            "from": dubai_text(record.check_in),
            "flag": added.flag.as_str(),
            "flag_text": flag_text(added.flag),
            "note": added.note,
        }));
    for user in users {
        request = request.to_user(user);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}
