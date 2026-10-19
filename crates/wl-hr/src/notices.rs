//! The module's notifications (issue #216, #217), sent through the kernel
//! [`Notifier`](wl_kernel::notify::Notifier): transactional, with English
//! texts first. Without wl-notify the kernel's fallback puts them in the
//! bell.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::json;
use wl_kernel::notify::{self, NotificationTypeDef, NotifyChannel, NotifyRequest};

use crate::employee::Employee;
use crate::onboarding::{Onboarding, OnboardingStep};
use crate::texts::english;

/// The assignees are told of a step (a new onboarding, a new holder).
pub const STEP_ASSIGNED: &str = "hr.onboarding_step_assigned";
/// The employee is told, on their own channel, that a step was recorded in
/// person (`HRM.ACL.10`).
pub const IN_PERSON_RECORDED: &str = "hr.onboarding_in_person_recorded";
/// HR and the manager are told that an onboarding completed.
pub const ONBOARDING_COMPLETED: &str = "hr.onboarding_completed";
/// The assignees are told that a step they marked was reopened.
pub const STEP_REOPENED: &str = "hr.onboarding_step_reopened";

/// `date` as `9 Nov 2026`.
fn date_text(date: NaiveDate) -> String {
    date.format("%-d %b %Y").to_string()
}

/// An e-mail type of the module with English texts.
fn english_type(key: &str, texts: &str) -> NotificationTypeDef {
    let subject = english(&format!("wl_hr.notices.{texts}_subject"), &[]);
    let body = english(&format!("wl_hr.notices.{texts}_body"), &[]);
    NotificationTypeDef::new(key, "hr")
        .group("hr")
        .template(NotifyChannel::Email, "en", Some(&subject), &body)
}

/// Registers the module's notification types with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(english_type(STEP_ASSIGNED, "step_assigned"));
    notify::register_type(english_type(ONBOARDING_COMPLETED, "completed"));
    notify::register_type(english_type(STEP_REOPENED, "step_reopened"));
    // The employee's own channel: SMS to the number on record.
    let body = english("wl_hr.notices.in_person_body", &[]);
    notify::register_type(
        NotificationTypeDef::new(IN_PERSON_RECORDED, "hr")
            .group("hr")
            .channels(&[NotifyChannel::Sms, NotifyChannel::Email])
            .template(NotifyChannel::Sms, "en", None, &body)
            .template(
                NotifyChannel::Email,
                "en",
                Some(&english("wl_hr.notices.in_person_subject", &[])),
                &body,
            ),
    );
}

/// The employee's name for staff-facing text.
async fn name_of(state: &AppState, employee: &Employee) -> String {
    wl_kernel::parties::directory(state)
        .display_name(&state.db, employee.party(), "en")
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("#{}", employee.id))
}

/// Tells `users` that `step` of `employee` is assigned to them.
pub(crate) async fn step_assigned(
    state: &AppState,
    employee: &Employee,
    step: &OnboardingStep,
    users: &[i64],
) -> Result {
    if users.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(STEP_ASSIGNED, step.subject())
        .subject_label(&step.title_en)
        .data(json!({
            "employee": name_of(state, employee).await,
            "step": step.title_en,
            "due": date_text(step.due_date),
        }));
    for user in users {
        request = request.to_user(*user);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells `users` that `step` they marked was reopened by `by`, with `reason`.
pub(crate) async fn step_reopened(
    state: &AppState,
    employee: &Employee,
    step: &OnboardingStep,
    users: &[i64],
    reason: &str,
) -> Result {
    if users.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(STEP_REOPENED, step.subject())
        .subject_label(&step.title_en)
        .data(json!({
            "employee": name_of(state, employee).await,
            "step": step.title_en,
            "due": date_text(step.due_date),
            "reason": reason,
        }));
    for user in users {
        request = request.to_user(*user);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the employee, on their own channel, that `step` was recorded in
/// person with `witness` as the witness.
pub(crate) async fn in_person_recorded(
    state: &AppState,
    employee: &Employee,
    step: &OnboardingStep,
    witness: &str,
    at: &str,
) -> Result {
    let request = NotifyRequest::new(IN_PERSON_RECORDED, step.subject())
        .subject_label(&step.title_en)
        .to_party(employee.party())
        .data(json!({
            "step": step.title_en,
            "witness": witness,
            "at": at,
        }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells HR and the manager that `onboarding` of `employee` completed.
pub(crate) async fn completed(
    state: &AppState,
    employee: &Employee,
    onboarding: &Onboarding,
    users: &[i64],
) -> Result {
    if users.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(ONBOARDING_COMPLETED, onboarding.subject())
        .data(json!({ "employee": name_of(state, employee).await }));
    for user in users {
        request = request.to_user(*user);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}
