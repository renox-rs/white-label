//! The module's notifications (issue #216, #217), sent through the kernel
//! [`Notifier`](wl_kernel::notify::Notifier): transactional, with English
//! texts first. Without wl-notify the kernel's fallback puts them in the
//! bell.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::json;
use wl_kernel::notify::{
    self, Category, GiveUpReaction, MessageId, NotificationTypeDef, NotifyChannel, NotifyRecipient,
    NotifyRequest,
};
use wl_kernel::workforce::AffectedWork;

use crate::employee::Employee;
use crate::onboarding::{Onboarding, OnboardingStep};
use crate::pay_account::PayAccount;
use crate::payroll::PayrollRun;
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
/// The assignees are reminded of an overdue step (`HRM.RUL.17`).
pub const STEP_REMINDER: &str = "hr.onboarding_step_reminder";
/// HR and the manager are told that a step is far overdue (`HRM.RUL.17`).
pub const STEP_ESCALATED: &str = "hr.onboarding_step_escalated";
/// The manager and HR are told that a qualification expired (`HRM.RUL.10`).
pub const QUALIFICATION_EXPIRED: &str = "hr.qualification_expired";

/// The preparer is told that the approver sent a payroll run back to draft.
pub const PAYROLL_RETURNED: &str = "hr.payroll_returned";

/// The preparer and the approver are told that lines of the salary file
/// failed (`HRM.RUL.11`).
pub const PAYROLL_FAILED: &str = "hr.payroll_failed";

/// The MD and the accountant are told that a WPS rejection is still open
/// before the due date (`HRM.RUL.11`).
pub const PAYROLL_ESCALATED: &str = "hr.payroll_escalated";

/// The payroll officers are told that held lines were released into a draft
/// off-cycle run (`HRM.RUL.12`).
pub const PAYROLL_RELEASED: &str = "hr.payroll_released";

/// The approvers are told that a new pay account waits for them (`HRM.ACL.05`).
pub const PAY_ACCOUNT_REQUESTED: &str = "hr.pay_account_requested";

/// The requester is told that a new pay account was rejected.
pub const PAY_ACCOUNT_REJECTED: &str = "hr.pay_account_rejected";

/// The MD is told that the worker or HR stopped a pay account change.
pub const PAY_ACCOUNT_STOPPED: &str = "hr.pay_account_stopped";

/// The worker is told, by SMS on the number on record before the request (and
/// by e-mail where known), that a new pay account takes effect on a date.
pub const PAY_ACCOUNT_CHANGE: &str = "hr.pay_account_change";

/// `date` as `9 Nov 2026`.
fn date_text(date: NaiveDate) -> String {
    date.format("%-d %b %Y").to_string()
}

/// An e-mail type of the module with English texts.
fn english_type(key: &str, texts: &str) -> NotificationTypeDef {
    let subject = english(&format!("wl_hr.notices.{texts}_subject"), &[]);
    let body = english(&format!("wl_hr.notices.{texts}_body"), &[]);
    NotificationTypeDef::new(key, "hr").group("hr").template(
        NotifyChannel::Email,
        "en",
        Some(&subject),
        &body,
    )
}

/// A reminder type of the module with English texts.
fn english_reminder(key: &str, texts: &str) -> NotificationTypeDef {
    english_type(key, texts).category(Category::Reminder)
}

/// Registers the module's notification types with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(english_reminder(STEP_REMINDER, "step_reminder"));
    notify::register_type(english_reminder(STEP_ESCALATED, "step_escalated"));
    notify::register_type(english_type(STEP_ASSIGNED, "step_assigned"));
    notify::register_type(english_type(ONBOARDING_COMPLETED, "completed"));
    notify::register_type(english_type(STEP_REOPENED, "step_reopened"));
    notify::register_type(english_type(QUALIFICATION_EXPIRED, "qualification_expired"));
    notify::register_type(english_type(PAYROLL_RETURNED, "payroll_returned"));
    notify::register_type(english_type(PAYROLL_FAILED, "payroll_failed"));
    notify::register_type(english_type(PAYROLL_ESCALATED, "payroll_escalated"));
    notify::register_type(english_type(PAYROLL_RELEASED, "payroll_released"));
    notify::register_type(english_type(PAY_ACCOUNT_REQUESTED, "pay_account_requested"));
    notify::register_type(english_type(PAY_ACCOUNT_REJECTED, "pay_account_rejected"));
    notify::register_type(english_type(PAY_ACCOUNT_STOPPED, "pay_account_stopped"));
    // The worker's change notice: the module itself reacts to a give-up
    // (`MessageGaveUp`), so the type has no reaction of its own.
    let sms = english("wl_hr.notices.pay_account_change_sms", &[]);
    notify::register_type(
        NotificationTypeDef::new(PAY_ACCOUNT_CHANGE, "hr")
            .group("hr")
            .category(Category::Security)
            .give_up(&[GiveUpReaction::None])
            .channels(&[NotifyChannel::Sms, NotifyChannel::Email])
            .template(NotifyChannel::Sms, "en", None, &sms)
            .template(
                NotifyChannel::Email,
                "en",
                Some(&english("wl_hr.notices.pay_account_change_subject", &[])),
                &english("wl_hr.notices.pay_account_change_body", &[]),
            ),
    );
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
pub(crate) async fn name_of(state: &AppState, employee: &Employee) -> String {
    wl_kernel::parties::directory(state)
        .display_name(&state.db, employee.party(), "en")
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("#{}", employee.id))
}

/// Tells `users` (the manager and HR) that the qualification of `kind` of
/// `employee` expired on `expires_on`, and how much work it blocks.
pub(crate) async fn qualification_expired(
    state: &AppState,
    employee: &Employee,
    kind: &str,
    expires_on: NaiveDate,
    affected: &[AffectedWork],
    users: &[i64],
) -> Result {
    if users.is_empty() {
        return Ok(());
    }
    let work: Vec<String> = affected
        .iter()
        .map(|w| format!("{} ({})", w.label, date_text(w.date)))
        .collect();
    let mut request = NotifyRequest::new(QUALIFICATION_EXPIRED, employee.subject())
        .subject_label(kind)
        .data(json!({
            "employee": name_of(state, employee).await,
            "kind": kind,
            "expires_on": date_text(expires_on),
            "count": affected.len(),
            "work": work.join(", "),
        }));
    for user in users {
        request = request.to_user(*user);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
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

/// Reminds `users` that `step` of `employee` is `days` days overdue.
pub(crate) async fn step_reminder(
    state: &AppState,
    employee: &Employee,
    step: &OnboardingStep,
    users: &[i64],
    days: i64,
) -> Result {
    overdue(state, STEP_REMINDER, employee, step, users, days).await
}

/// Tells `users` (HR and the manager) that `step` of `employee` is `days`
/// days overdue.
pub(crate) async fn step_escalated(
    state: &AppState,
    employee: &Employee,
    step: &OnboardingStep,
    users: &[i64],
    days: i64,
) -> Result {
    overdue(state, STEP_ESCALATED, employee, step, users, days).await
}

async fn overdue(
    state: &AppState,
    type_key: &str,
    employee: &Employee,
    step: &OnboardingStep,
    users: &[i64],
    days: i64,
) -> Result {
    if users.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(type_key, step.subject())
        .subject_label(&step.title_en)
        .data(json!({
            "employee": name_of(state, employee).await,
            "step": step.title_en,
            "due": date_text(step.due_date),
            "days": days,
        }));
    for user in users {
        request = request.to_user(*user);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells `users` (the preparer) that payroll run `run` was returned to draft
/// with `reason` and the approver's `comments` on the lines to fix.
pub(crate) async fn payroll_returned(
    state: &AppState,
    run: &PayrollRun,
    reason: &str,
    comments: &str,
    users: &[i64],
) -> Result {
    if users.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(PAYROLL_RETURNED, run.subject())
        .subject_label(&run.period)
        .data(json!({
            "period": run.period,
            "reason": reason,
            "comments": comments,
        }));
    for user in users {
        request = request.to_user(*user);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells `users` (the preparer and the approver) that `count` lines of the
/// salary file of `run` failed, with the `reasons`.
pub(crate) async fn payroll_failed(
    state: &AppState,
    run: &PayrollRun,
    count: usize,
    reasons: &str,
    users: &[i64],
) -> Result {
    if users.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(PAYROLL_FAILED, run.subject())
        .subject_label(&run.period)
        .data(json!({
            "period": run.period,
            "count": count,
            "reasons": reasons,
        }));
    for user in users {
        request = request.to_user(*user);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells `users` (the MD and the accountant) that payroll run `run` is still
/// `status` with `count` failed lines, and the WPS file is due on `due`.
pub(crate) async fn payroll_escalated(
    state: &AppState,
    run: &PayrollRun,
    due: NaiveDate,
    count: usize,
    users: &[i64],
) -> Result {
    if users.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(PAYROLL_ESCALATED, run.subject())
        .subject_label(&run.period)
        .data(json!({
            "period": run.period,
            "status": run.status.as_str(),
            "due": date_text(due),
            "count": count,
        }));
    for user in users {
        request = request.to_user(*user);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells `users` (the payroll officers) that `count` held lines were released
/// into the draft off-cycle run `run`.
pub(crate) async fn payroll_released(
    state: &AppState,
    run: &PayrollRun,
    count: usize,
    users: &[i64],
) -> Result {
    if users.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(PAYROLL_RELEASED, run.subject())
        .subject_label(&run.period)
        .data(json!({
            "period": run.period,
            "count": count,
        }));
    for user in users {
        request = request.to_user(*user);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells `users` (the approvers) that the new pay account `account` of
/// `employee` waits for approval. The number is never in the text.
pub(crate) async fn pay_account_requested(
    state: &AppState,
    employee: &Employee,
    account: &PayAccount,
    users: &[i64],
) -> Result {
    if users.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(PAY_ACCOUNT_REQUESTED, account.subject())
        .subject_label(&account.masked())
        .data(json!({
            "employee": name_of(state, employee).await,
            "last4": account.last4,
            "explanation": account.mismatch_explanation.clone().unwrap_or_default(),
        }));
    for user in users {
        request = request.to_user(*user);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells `users` (the requester) that the new pay account was rejected.
pub(crate) async fn pay_account_rejected(
    state: &AppState,
    employee: &Employee,
    account: &PayAccount,
    users: &[i64],
) -> Result {
    if users.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(PAY_ACCOUNT_REJECTED, account.subject())
        .subject_label(&account.masked())
        .data(json!({
            "employee": name_of(state, employee).await,
            "last4": account.last4,
            "reason": account.reject_reason.clone().unwrap_or_default(),
        }));
    for user in users {
        request = request.to_user(*user);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells `users` (the MD) that the change to `account` was stopped by `by`.
pub(crate) async fn pay_account_stopped(
    state: &AppState,
    employee: &Employee,
    account: &PayAccount,
    by: &str,
    users: &[i64],
) -> Result {
    if users.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(PAY_ACCOUNT_STOPPED, account.subject())
        .subject_label(&account.masked())
        .data(json!({
            "employee": name_of(state, employee).await,
            "last4": account.last4,
            "by": by,
        }));
    for user in users {
        request = request.to_user(*user);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the worker of the change of `account`: an SMS to `number`, the one
/// on record before the request, and an e-mail where one was on record. It
/// names the last 4 digits, the day it takes effect (`ends`) and the signed
/// `link` that stops it. Returns the messages queued.
pub(crate) async fn pay_account_change(
    state: &AppState,
    account: &PayAccount,
    number: &str,
    link: &str,
    ends: NaiveDate,
) -> Result<Vec<MessageId>> {
    let mut request = NotifyRequest::new(PAY_ACCOUNT_CHANGE, account.subject())
        .subject_label(&account.masked())
        .to(NotifyRecipient::Address {
            channel: NotifyChannel::Sms,
            address: number.to_owned(),
        })
        .data(json!({
            "last4": account.last4,
            "date": ends.to_string(),
            "link": link,
        }));
    if let Some(email) = &account.notice_email {
        request = request.to(NotifyRecipient::Address {
            channel: NotifyChannel::Email,
            address: email.clone(),
        });
    }
    notify::notifier(state).notify(state, request).await
}
