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
use wl_kernel::workforce::{EmployeeId, StaffProfile, workforce};

use crate::config::config_of;
use crate::flags::{Added, FlagType};
use crate::location::WorkLocation;
use crate::permissions;
use crate::record::{AttendanceMethod, AttendanceRecord};
use crate::texts::english;

/// The people are told that a record was flagged.
pub const RECORD_FLAGGED: &str = "attendance.record_flagged";

/// The supervisor is told that a day is over the overtime cap although every
/// record of it is `locked`, so that nothing could be flagged (`HRM.RUL.13`).
pub const OVER_CAP: &str = "attendance.over_cap";

/// The user who submitted a timesheet is told it was sent back.
pub const TIMESHEET_REJECTED: &str = "attendance.timesheet_rejected";

/// The creator of an adjustment is told that the invoice dispute it was
/// opened for ended `upheld` (BL-ATT-09).
pub const ADJUSTMENT_DISPUTE_UPHELD: &str = "attendance.adjustment_dispute_upheld";

/// The client's escalation contact and the account manager's manager are told
/// that a timesheet has had no answer (`HRM.RUL.15` at E days).
pub const TIMESHEET_ESCALATED: &str = "attendance.timesheet_escalated";

/// Registers the module's notification types with the kernel registry.
pub(crate) fn register_types() {
    let subject = english("wl_attendance.notices.escalated_subject", &[]);
    let body = english("wl_attendance.notices.escalated_body", &[]);
    notify::register_type(
        NotificationTypeDef::new(TIMESHEET_ESCALATED, "attendance")
            .group("attendance")
            .template(NotifyChannel::Email, "en", Some(&subject), &body),
    );
    let subject = english("wl_attendance.notices.dispute_upheld_subject", &[]);
    let body = english("wl_attendance.notices.dispute_upheld_body", &[]);
    notify::register_type(
        NotificationTypeDef::new(ADJUSTMENT_DISPUTE_UPHELD, "attendance")
            .group("attendance")
            .template(NotifyChannel::Email, "en", Some(&subject), &body),
    );
    let subject = english("wl_attendance.notices.rejected_subject", &[]);
    let body = english("wl_attendance.notices.rejected_body", &[]);
    notify::register_type(
        NotificationTypeDef::new(TIMESHEET_REJECTED, "attendance")
            .group("attendance")
            .template(NotifyChannel::Email, "en", Some(&subject), &body),
    );
    let subject = english("wl_attendance.notices.flagged_subject", &[]);
    let body = english("wl_attendance.notices.flagged_body", &[]);
    notify::register_type(
        NotificationTypeDef::new(RECORD_FLAGGED, "attendance")
            .group("attendance")
            .template(NotifyChannel::Email, "en", Some(&subject), &body),
    );
    let subject = english("wl_attendance.notices.over_cap_subject", &[]);
    let body = english("wl_attendance.notices.over_cap_body", &[]);
    notify::register_type(
        NotificationTypeDef::new(OVER_CAP, "attendance")
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
    let profile = workforce(state).staff(state, record.employee()).await?;
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

/// Tells the supervisors of the `locked` records of a day that the day is over
/// the overtime cap and that nothing could be flagged. Nobody is told when no
/// supervisor is known.
pub(crate) async fn over_cap_unflagged(
    state: &AppState,
    locked: &[AttendanceRecord],
    note: &str,
) -> Result {
    let Some(first) = locked.first() else {
        return Ok(());
    };
    let profile = workforce(state).staff(state, first.employee()).await?;
    let mut users = BTreeSet::new();
    for record in locked {
        users.extend(supervisors(record, profile.as_ref()));
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
    .unwrap_or_else(|| format!("#{}", first.employee_id));
    let mut request = NotifyRequest::new(OVER_CAP, first.record_id().subject())
        .subject_label(&format!("{employee} {}", first.date))
        .data(json!({
            "employee": employee,
            "date": first.date.to_string(),
            "note": note,
        }));
    for user in users {
        request = request.to_user(user);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells whoever submitted `sheet` that it was sent back with `reason`: the
/// user who submitted it, else the one who created it. Nobody is told when
/// neither is known.
pub(crate) async fn timesheet_rejected(
    state: &AppState,
    sheet: &crate::timesheet::Timesheet,
    reason: &str,
) -> Result {
    let Some(user) = sheet.submitted_by.or(sheet.created_by) else {
        return Ok(());
    };
    let profile = workforce(state).staff(state, sheet.owner()).await?;
    let employee = match &profile {
        Some(profile) => wl_kernel::parties::directory(state)
            .display_name(&state.db, profile.party, "en")
            .await
            .ok()
            .flatten(),
        None => None,
    }
    .unwrap_or_else(|| format!("#{}", sheet.owner_employee_id));
    let request = NotifyRequest::new(TIMESHEET_REJECTED, sheet.subject())
        .subject_label(&format!("{employee} {}", sheet.period_from))
        .data(json!({
            "employee": employee,
            "from": sheet.period_from.to_string(),
            "to": sheet.period_to.to_string(),
            "reason": reason,
        }))
        .to_user(user);
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// The name of `employee` for staff-facing text: the party's display name,
/// else `#7`.
pub(crate) async fn employee_name(state: &AppState, employee: EmployeeId) -> Result<String> {
    let profile = workforce(state).staff(state, employee).await?;
    let name = match &profile {
        Some(profile) => wl_kernel::parties::directory(state)
            .display_name(&state.db, profile.party, "en")
            .await
            .ok()
            .flatten(),
        None => None,
    };
    Ok(name.unwrap_or_else(|| format!("#{}", employee.0)))
}

/// Tells the creator of the adjustment `sheet` that its dispute ended
/// `upheld`. Nobody is told when the creator is not known.
pub(crate) async fn adjustment_dispute_upheld(
    state: &AppState,
    sheet: &crate::timesheet::Timesheet,
) -> Result {
    let Some(user) = sheet.created_by else {
        return Ok(());
    };
    let employee = employee_name(state, sheet.owner()).await?;
    let request = NotifyRequest::new(ADJUSTMENT_DISPUTE_UPHELD, sheet.subject())
        .subject_label(&format!("{employee} {}", sheet.period_from))
        .data(json!({
            "employee": employee,
            "from": sheet.period_from.to_string(),
            "to": sheet.period_to.to_string(),
        }))
        .to_user(user);
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the audience of an escalation (`HRM.RUL.15` at E days) that the
/// timesheet of `employee` has had no answer for `days` days.
pub(crate) async fn timesheet_escalated(
    state: &AppState,
    sheet: &crate::timesheet::Timesheet,
    employee: &str,
    submitted: renox::chrono::NaiveDate,
    days: i64,
    audience: &crate::escalation::Audience,
) -> Result {
    let mut request = NotifyRequest::new(TIMESHEET_ESCALATED, sheet.subject())
        .subject_label(&format!("{employee} {}", sheet.period_from))
        .data(json!({
            "employee": employee,
            "from": sheet.period_from.to_string(),
            "to": sheet.period_to.to_string(),
            "submitted": submitted.to_string(),
            "days": days,
        }));
    for contact in &audience.contacts {
        request = request.to(contact.clone());
    }
    for user in &audience.staff {
        request = request.to_user(*user);
    }
    if request.recipients.is_empty() {
        return Ok(());
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}
