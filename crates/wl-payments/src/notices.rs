//! The notices the module sends through the kernel's `Notifier`
//! (wl-payments A9, A10).
//!
//! - `payments.payment_returned`: the party is told its payment was
//!   returned by the bank (transactional, e-mail, EN). Nothing is sent for
//!   `entered_in_error`.
//! - `payments.followup_task`: the holders of the accountant role get a
//!   notice for a new follow-up task.
//!
//! - `payments.refund_failed`: the requester and the accountants are told a
//!   refund failed (transactional, bell and e-mail).
//! - `payments.refund_delayed`: the party is told its refund is delayed
//!   (transactional, e-mail, EN); skipped while the party is blocked.
//!
//! - `payments.refund_method_changed`: the party is told how its failed
//!   refund will be paid now (transactional, e-mail, EN); skipped while the
//!   party is blocked.
//! - `payments.refund_cancelled`: the party is told the refund is cancelled
//!   and the amount stays on its credit (transactional, e-mail, EN); skipped
//!   while the party is blocked.
//! - `payments.refund_method_task`: the holders of the accountant role get a
//!   notice for the task to pay a failed refund another way (`FIN.RUL.21`).
//!
//! - `payments.cheque_bounced`: the party is told its cheque was returned
//!   (`FIN.RUL.04`, transactional, e-mail, EN); skipped while the party is
//!   blocked.
//! - `payments.cheque_task`: the staff are told of a cheque task (a bounced
//!   cheque to follow up or replace, a bounce fee to raise, a legal case, a
//!   cheque to hand back): the holders of the accountant role, the user who
//!   registered the cheque and the task's assignee.
//! - `payments.cheque_presented`: the party is told the new presentation
//!   date of its re-presented cheque (transactional, e-mail, EN); skipped
//!   while the party is blocked.
//! - `payments.cheque_notice`: the formal notice of a legal case to the party
//!   (transactional, e-mail, EN; the default template, content pending
//!   `⚠ open: lawyer L-21`).
//!
//! While `notify` is off the kernel's `NoNotifier` sends nothing to a party
//! and a bell notice to each user.

use renox::auth::permissions::{Scope, users_with_role_in};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::json;
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};
use wl_kernel::parties::PartyStatus;
use wl_kernel::payments::{PaymentMethod, RefundFailureKind};

use crate::cheque::{Cheque, ChequeReturn};
use crate::deposit::Deposit;
use crate::deposit_deductions::DepositDeduction;
use crate::followup::{ACCOUNTANT_ROLE, FollowupTask, money_text};
use crate::payment::Payment;
use crate::refund::Refund;
use crate::texts::english;

/// The type of the party's bank-return notice.
pub const PAYMENT_RETURNED: &str = "payments.payment_returned";

/// The type of the accountants' follow-up notice.
pub const FOLLOWUP_TASK: &str = "payments.followup_task";

/// The type of the staff's notice that a refund failed.
pub const REFUND_FAILED: &str = "payments.refund_failed";

/// The type of the party's notice that its refund is delayed.
pub const REFUND_DELAYED: &str = "payments.refund_delayed";

/// The type of the party's notice of the new method of its failed refund.
pub const REFUND_METHOD_CHANGED: &str = "payments.refund_method_changed";

/// The type of the party's notice that its failed refund is cancelled.
pub const REFUND_CANCELLED: &str = "payments.refund_cancelled";

/// The type of the accountants' notice of a task to change a refund's method.
pub const REFUND_METHOD_TASK: &str = "payments.refund_method_task";

/// The type of the party's notice that its cheque bounced.
pub const CHEQUE_BOUNCED: &str = "payments.cheque_bounced";

/// The type of the staff's notice of a cheque task.
pub const CHEQUE_TASK: &str = "payments.cheque_task";

/// The type of the party's notice of the new presentation date.
pub const CHEQUE_PRESENTED: &str = "payments.cheque_presented";

/// The type of the party's formal notice of a legal case.
pub const CHEQUE_NOTICE: &str = "payments.cheque_notice";

/// The type of the manager's notice that a cheque was referred to legal.
pub const CHEQUE_REFERRED: &str = "payments.cheque_referred";

/// The type of the party's reminder of its bounced cheque (`FIN.RUL.18`).
pub const CHEQUE_REMINDER: &str = "payments.cheque_reminder";

/// The type of the staff's reminder of a bounced cheque (`FIN.RUL.18`).
pub const CHEQUE_REMINDER_STAFF: &str = "payments.cheque_reminder_staff";

/// The type of the party's notice that the deposit deductions are published.
pub const DEPOSIT_DEDUCTIONS: &str = "payments.deposit_deductions";

/// The type of the staff's notice that the party disputed deductions.
pub const DEPOSIT_DISPUTED: &str = "payments.deposit_disputed";

/// The type of the party's notice that its dispute was answered.
pub const DEPOSIT_DISPUTE_ANSWERED: &str = "payments.deposit_dispute_answered";

/// The type of the party's notice that its deposit was handed to another holder.
pub const DEPOSIT_TRANSFERRED: &str = "payments.deposit_transferred";

/// The type of the accountants' notice of a deposit task.
pub const DEPOSIT_TASK: &str = "payments.deposit_task";

const MODULE: &str = "payments";
const GROUP: &str = "payments";

/// `date` as `18 Oct 2026`.
fn date_text(date: NaiveDate) -> String {
    date.format("%-d %b %Y").to_string()
}

/// The template variables of the bank-return notice.
fn returned_data(payment: &Payment) -> renox::serde_json::Value {
    json!({
        "number": payment.number.clone().unwrap_or_default(),
        "amount": money_text(&payment.currency, payment.amount),
        "payment_date": date_text(payment.payment_date),
    })
}

/// The template variables of the follow-up notice.
fn followup_data(payment: &Payment, task: &FollowupTask) -> renox::serde_json::Value {
    json!({
        "text": task.text,
        "number": payment.number.clone().unwrap_or_default(),
        "amount": money_text(&task.currency, task.amount),
        "link": "/payments/tasks",
    })
}

/// The notification type [`PAYMENT_RETURNED`].
pub fn returned_type() -> NotificationTypeDef {
    NotificationTypeDef::new(PAYMENT_RETURNED, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_payments.returned.subject")),
            english("wl_payments.returned.body"),
        )
}

/// The notification type [`FOLLOWUP_TASK`].
pub fn followup_type() -> NotificationTypeDef {
    NotificationTypeDef::new(FOLLOWUP_TASK, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_payments.followup.subject")),
            english("wl_payments.followup.body"),
        )
}

/// The notification type [`REFUND_FAILED`].
pub fn refund_failed_type() -> NotificationTypeDef {
    NotificationTypeDef::new(REFUND_FAILED, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_payments.refund_failed.subject")),
            english("wl_payments.refund_failed.body"),
        )
}

/// The notification type [`REFUND_DELAYED`].
pub fn refund_delayed_type() -> NotificationTypeDef {
    NotificationTypeDef::new(REFUND_DELAYED, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_payments.refund_delayed.subject")),
            english("wl_payments.refund_delayed.body"),
        )
}

/// The notification type [`REFUND_METHOD_CHANGED`].
pub fn refund_method_changed_type() -> NotificationTypeDef {
    NotificationTypeDef::new(REFUND_METHOD_CHANGED, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_payments.refund_method_changed.subject")),
            english("wl_payments.refund_method_changed.body"),
        )
}

/// The notification type [`REFUND_CANCELLED`].
pub fn refund_cancelled_type() -> NotificationTypeDef {
    NotificationTypeDef::new(REFUND_CANCELLED, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_payments.refund_cancelled.subject")),
            english("wl_payments.refund_cancelled.body"),
        )
}

/// The notification type [`REFUND_METHOD_TASK`].
pub fn refund_method_task_type() -> NotificationTypeDef {
    NotificationTypeDef::new(REFUND_METHOD_TASK, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_payments.refund_method_task.subject")),
            english("wl_payments.refund_method_task.body"),
        )
}

/// The notification type [`CHEQUE_BOUNCED`].
pub fn cheque_bounced_type() -> NotificationTypeDef {
    NotificationTypeDef::new(CHEQUE_BOUNCED, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_payments.cheque_bounced.subject")),
            english("wl_payments.cheque_bounced.body"),
        )
}

/// The notification type [`CHEQUE_TASK`].
pub fn cheque_task_type() -> NotificationTypeDef {
    NotificationTypeDef::new(CHEQUE_TASK, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_payments.cheque_task.subject")),
            english("wl_payments.cheque_task.body"),
        )
}

/// The notification type [`CHEQUE_PRESENTED`].
pub fn cheque_presented_type() -> NotificationTypeDef {
    NotificationTypeDef::new(CHEQUE_PRESENTED, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_payments.cheque_presented.subject")),
            english("wl_payments.cheque_presented.body"),
        )
}

/// The notification type [`CHEQUE_NOTICE`].
pub fn cheque_notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(CHEQUE_NOTICE, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_payments.cheque_notice.subject")),
            english("wl_payments.cheque_notice.body"),
        )
}

/// The notification type [`CHEQUE_REFERRED`].
pub fn cheque_referred_type() -> NotificationTypeDef {
    NotificationTypeDef::new(CHEQUE_REFERRED, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_payments.cheque_referred.subject")),
            english("wl_payments.cheque_referred.body"),
        )
}

/// The notification type [`CHEQUE_REMINDER`].
pub fn cheque_reminder_type() -> NotificationTypeDef {
    NotificationTypeDef::new(CHEQUE_REMINDER, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_payments.cheque_reminder.subject")),
            english("wl_payments.cheque_reminder.body"),
        )
}

/// The notification type [`CHEQUE_REMINDER_STAFF`].
pub fn cheque_reminder_staff_type() -> NotificationTypeDef {
    NotificationTypeDef::new(CHEQUE_REMINDER_STAFF, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_payments.cheque_reminder_staff.subject")),
            english("wl_payments.cheque_reminder_staff.body"),
        )
}

/// The notification type [`DEPOSIT_DEDUCTIONS`].
pub fn deposit_deductions_type() -> NotificationTypeDef {
    NotificationTypeDef::new(DEPOSIT_DEDUCTIONS, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_payments.deposit_deductions.subject")),
            english("wl_payments.deposit_deductions.body"),
        )
}

/// The notification type [`DEPOSIT_DISPUTED`].
pub fn deposit_disputed_type() -> NotificationTypeDef {
    NotificationTypeDef::new(DEPOSIT_DISPUTED, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_payments.deposit_disputed.subject")),
            english("wl_payments.deposit_disputed.body"),
        )
}

/// The notification type [`DEPOSIT_DISPUTE_ANSWERED`].
pub fn deposit_dispute_answered_type() -> NotificationTypeDef {
    NotificationTypeDef::new(DEPOSIT_DISPUTE_ANSWERED, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_payments.deposit_dispute_answered.subject")),
            english("wl_payments.deposit_dispute_answered.body"),
        )
}

/// The notification type [`DEPOSIT_TRANSFERRED`].
pub fn deposit_transferred_type() -> NotificationTypeDef {
    NotificationTypeDef::new(DEPOSIT_TRANSFERRED, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_payments.deposit_transferred.subject")),
            english("wl_payments.deposit_transferred.body"),
        )
}

/// The notification type [`DEPOSIT_TASK`].
pub fn deposit_task_type() -> NotificationTypeDef {
    NotificationTypeDef::new(DEPOSIT_TASK, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_payments.deposit_task.subject")),
            english("wl_payments.deposit_task.body"),
        )
}

/// Registers the types with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(deposit_task_type());
    notify::register_type(deposit_transferred_type());
    notify::register_type(deposit_deductions_type());
    notify::register_type(deposit_disputed_type());
    notify::register_type(deposit_dispute_answered_type());
    notify::register_type(cheque_reminder_type());
    notify::register_type(cheque_reminder_staff_type());
    notify::register_type(cheque_bounced_type());
    notify::register_type(cheque_task_type());
    notify::register_type(cheque_presented_type());
    notify::register_type(cheque_notice_type());
    notify::register_type(cheque_referred_type());
    notify::register_type(refund_method_changed_type());
    notify::register_type(refund_cancelled_type());
    notify::register_type(refund_method_task_type());
    notify::register_type(returned_type());
    notify::register_type(followup_type());
    notify::register_type(refund_failed_type());
    notify::register_type(refund_delayed_type());
}

/// The staff-facing text of a failure kind.
fn kind_text(kind: RefundFailureKind) -> &'static str {
    match kind {
        RefundFailureKind::GatewayDeclined => "Declined by the gateway",
        RefundFailureKind::CardUnavailable => "Card unavailable",
        RefundFailureKind::BankReturn => "Returned by the bank",
    }
}

/// The template variables of the staff's refund-failed notice.
fn refund_failed_data(
    refund: &Refund,
    party_name: &str,
    kind: RefundFailureKind,
    reason: &str,
) -> renox::serde_json::Value {
    json!({
        "refund": format!("#{}", refund.id),
        "party": party_name,
        "amount": money_text(&refund.currency, refund.amount),
        "kind": kind_text(kind),
        "reason": reason,
        "link": format!("/refunds/{}", refund.id),
    })
}

/// The template variables of the party's refund-delayed notice.
fn refund_delayed_data(refund: &Refund) -> renox::serde_json::Value {
    json!({ "amount": money_text(&refund.currency, refund.amount) })
}

/// The users told that a refund failed: the one who requested it and the
/// holders of the accountant role, each once.
pub(crate) fn staff_ids(requested_by: Option<i64>, accountants: &[i64]) -> Vec<i64> {
    let mut ids: Vec<i64> = requested_by
        .into_iter()
        .chain(accountants.iter().copied())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Whether the party hears that its refund is delayed: not while it is
/// blocked (wl-parties suppresses automatic messages to a blocked party).
pub(crate) fn tells_party(status: Option<PartyStatus>) -> bool {
    status != Some(PartyStatus::Blocked)
}

/// After a committed `fail` of `refund`: the requester and the accountants
/// get [`REFUND_FAILED`], and the party gets [`REFUND_DELAYED`] unless it is
/// blocked. Once per failure.
pub(crate) async fn refund_failed(
    state: &AppState,
    refund: &Refund,
    kind: RefundFailureKind,
    reason: &str,
) -> Result {
    let directory = wl_kernel::parties::directory(state);
    let party = refund.party();
    let name = directory
        .find(&state.db, party)
        .await?
        .map_or_else(|| party.to_string(), |found| found.display_name);
    let accountants: Vec<i64> = users_with_role_in(&state.db, ACCOUNTANT_ROLE, &Scope::global())
        .await?
        .iter()
        .map(|user| user.id)
        .collect();
    let ids = staff_ids(refund.requested_by, &accountants);
    if !ids.is_empty() {
        let mut request = NotifyRequest::new(REFUND_FAILED, refund.refund_id().subject())
            .data(refund_failed_data(refund, &name, kind, reason));
        for id in ids {
            request = request.to_user(id);
        }
        notify::notifier(state).notify(state, request).await?;
    }
    let status = directory.status(&state.db, party).await?;
    if tells_party(status) {
        let request = NotifyRequest::new(REFUND_DELAYED, refund.refund_id().subject())
            .to_party(party)
            .data(refund_delayed_data(refund));
        notify::notifier(state).notify(state, request).await?;
    }
    Ok(())
}

/// The template variables of the party's method-changed notice.
fn method_changed_data(refund: &Refund, method: PaymentMethod) -> renox::serde_json::Value {
    json!({
        "amount": money_text(&refund.currency, refund.amount),
        "method": english(&format!("wl_payments.method.{}", method.as_str())),
    })
}

/// The template variables of the party's refund-cancelled notice.
fn cancelled_data(refund: &Refund) -> renox::serde_json::Value {
    json!({ "amount": money_text(&refund.currency, refund.amount) })
}

/// The template variables of the accountants' change-method task notice.
fn method_task_data(refund: &Refund, task: &FollowupTask) -> renox::serde_json::Value {
    json!({
        "text": task.text,
        "reason": refund.failure_reason.clone().unwrap_or_default(),
        "link": format!("/refunds/{}", refund.id),
    })
}

/// After a `change_method` of `refund` to `method`: the party is told how it
/// will be paid, unless it is blocked.
pub(crate) async fn method_changed(
    state: &AppState,
    refund: &Refund,
    method: PaymentMethod,
) -> Result {
    let status = wl_kernel::parties::directory(state)
        .status(&state.db, refund.party())
        .await?;
    if tells_party(status) {
        let request = NotifyRequest::new(REFUND_METHOD_CHANGED, refund.refund_id().subject())
            .to_party(refund.party())
            .data(method_changed_data(refund, method));
        notify::notifier(state).notify(state, request).await?;
    }
    Ok(())
}

/// After a `cancel` of `refund`: the party is told, unless it is blocked.
pub(crate) async fn cancelled(state: &AppState, refund: &Refund) -> Result {
    let status = wl_kernel::parties::directory(state)
        .status(&state.db, refund.party())
        .await?;
    if tells_party(status) {
        let request = NotifyRequest::new(REFUND_CANCELLED, refund.refund_id().subject())
            .to_party(refund.party())
            .data(cancelled_data(refund));
        notify::notifier(state).notify(state, request).await?;
    }
    Ok(())
}

/// Tells each holder of the accountant role about the `change_method`
/// `task` of the failed `refund`, one message per user.
pub(crate) async fn change_method_task(
    state: &AppState,
    refund: &Refund,
    task: &FollowupTask,
) -> Result {
    let mut ids: Vec<i64> = users_with_role_in(&state.db, ACCOUNTANT_ROLE, &Scope::global())
        .await?
        .iter()
        .map(|user| user.id)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(REFUND_METHOD_TASK, refund.refund_id().subject())
        .data(method_task_data(refund, task));
    for id in ids {
        request = request.to_user(id);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the party of `payment` that the bank returned it.
pub(crate) async fn payment_returned(state: &AppState, payment: &Payment) -> Result {
    let request = NotifyRequest::new(PAYMENT_RETURNED, payment.subject())
        .to_party(payment.party())
        .data(returned_data(payment));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells each holder of the accountant role about `task`, one message per
/// user.
pub(crate) async fn followup_created(
    state: &AppState,
    payment: &Payment,
    task: &FollowupTask,
) -> Result {
    let mut ids: Vec<i64> = users_with_role_in(&state.db, ACCOUNTANT_ROLE, &Scope::global())
        .await?
        .iter()
        .map(|user| user.id)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Ok(());
    }
    let mut request =
        NotifyRequest::new(FOLLOWUP_TASK, payment.subject()).data(followup_data(payment, task));
    for id in ids {
        request = request.to_user(id);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// The template variables of the party's cheque-bounced notice.
fn cheque_bounced_data(cheque: &Cheque, bounce: &ChequeReturn) -> renox::serde_json::Value {
    json!({
        "number": cheque.number,
        "bank": cheque.bank,
        "amount": money_text(&cheque.currency, cheque.amount),
        "reason": bounce.reason,
        "open": money_text(&cheque.currency, cheque.open_amount),
    })
}

/// The template variables of a cheque task notice.
fn cheque_task_data(
    cheque: &Cheque,
    party_name: &str,
    task: &FollowupTask,
) -> renox::serde_json::Value {
    json!({
        "text": task.text,
        "number": cheque.number,
        "party": party_name,
        "amount": money_text(&cheque.currency, cheque.open_amount),
        "link": format!("/cheques/{}", cheque.id),
    })
}

/// After a committed `bounce` of the received `cheque`: the drawer is told
/// the cheque was returned (`FIN.RUL.04`), unless it is blocked.
pub(crate) async fn cheque_bounced(
    state: &AppState,
    cheque: &Cheque,
    bounce: &ChequeReturn,
) -> Result {
    let status = wl_kernel::parties::directory(state)
        .status(&state.db, cheque.party())
        .await?;
    if tells_party(status) {
        let request = NotifyRequest::new(CHEQUE_BOUNCED, cheque.subject())
            .to_party(cheque.party())
            .data(cheque_bounced_data(cheque, bounce));
        notify::notifier(state).notify(state, request).await?;
    }
    Ok(())
}

/// Tells the staff about the cheque `task`: the holders of the accountant
/// role, the user who registered `cheque` and the task's assignee, each
/// once.
pub(crate) async fn cheque_task(state: &AppState, cheque: &Cheque, task: &FollowupTask) -> Result {
    let accountants: Vec<i64> = users_with_role_in(&state.db, ACCOUNTANT_ROLE, &Scope::global())
        .await?
        .iter()
        .map(|user| user.id)
        .collect();
    let mut ids = staff_ids(cheque.recorded_by, &accountants);
    if let Some(owner) = task.assignee_id {
        ids.push(owner);
        ids.sort_unstable();
        ids.dedup();
    }
    cheque_task_to(state, cheque, task, &ids).await
}

/// Tells the users `ids` about the cheque `task`.
pub(crate) async fn cheque_task_to(
    state: &AppState,
    cheque: &Cheque,
    task: &FollowupTask,
    ids: &[i64],
) -> Result {
    if ids.is_empty() {
        return Ok(());
    }
    let party = wl_kernel::parties::directory(state)
        .display_name(&state.db, cheque.party(), "en")
        .await?
        .unwrap_or_else(|| cheque.party().to_string());
    let mut request = NotifyRequest::new(CHEQUE_TASK, cheque.subject())
        .data(cheque_task_data(cheque, &party, task));
    for id in ids {
        request = request.to_user(*id);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// The template variables of the party's re-presentation notice.
fn cheque_presented_data(cheque: &Cheque) -> renox::serde_json::Value {
    json!({
        "number": cheque.number,
        "amount": money_text(&cheque.currency, cheque.open_amount),
        "date": cheque.deposit_date.map(date_text).unwrap_or_default(),
    })
}

/// After a committed `represent` of the received `cheque`: the drawer is told
/// the new presentation date, unless it is blocked.
pub(crate) async fn cheque_presented(state: &AppState, cheque: &Cheque) -> Result {
    let status = wl_kernel::parties::directory(state)
        .status(&state.db, cheque.party())
        .await?;
    if tells_party(status) {
        let request = NotifyRequest::new(CHEQUE_PRESENTED, cheque.subject())
            .to_party(cheque.party())
            .data(cheque_presented_data(cheque));
        notify::notifier(state).notify(state, request).await?;
    }
    Ok(())
}

/// The template variables of the formal notice of a legal case.
fn cheque_notice_data(
    cheque: &Cheque,
    drawer: &str,
    contact: &str,
    bounce: &ChequeReturn,
) -> renox::serde_json::Value {
    let fee = if cheque.bounce_fee_total > 0 {
        format!(
            ", plus a bounce fee of {}",
            money_text(&cheque.currency, cheque.bounce_fee_total)
        )
    } else {
        String::new()
    };
    let case_type = match cheque.case_type {
        Some(crate::cheque::CaseType::CriminalComplaint) => "criminal complaint",
        _ => "civil execution",
    };
    json!({
        "drawer": drawer,
        "number": cheque.number,
        "bank": cheque.bank,
        "cheque_date": date_text(cheque.cheque_date),
        "amount": money_text(&cheque.currency, cheque.amount),
        "returned_on": date_text(bounce.returned_on),
        "reason": bounce.reason,
        "honoured": money_text(&cheque.currency, cheque.honoured_amount),
        "open": money_text(&cheque.currency, cheque.open_amount),
        "fee": fee,
        "days": crate::cheque_legal::NOTICE_DAYS,
        "case_type": case_type,
        "contact": contact,
    })
}

/// After a committed `refer_to_legal` of the received `cheque`: the formal
/// notice to the drawer, the default template (`⚠ open: lawyer L-21`). It is
/// sent even to a blocked party: it is a legal step.
pub(crate) async fn cheque_formal_notice(state: &AppState, cheque: &Cheque) -> Result {
    let Some(bounce) = ChequeReturn::latest(&state.db, cheque.id).await? else {
        return Ok(());
    };
    let drawer = wl_kernel::parties::directory(state)
        .display_name(&state.db, cheque.party(), "en")
        .await?
        .unwrap_or_else(|| cheque.party().to_string());
    let contact = match wl_kernel::LegalEntity::find(&state.db, cheque.legal_entity_id).await? {
        Some(entity) => entity.name_en,
        None => String::new(),
    };
    let request = NotifyRequest::new(CHEQUE_NOTICE, cheque.subject())
        .to_party(cheque.party())
        .data(cheque_notice_data(cheque, &drawer, &contact, &bounce));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// After a committed `refer_to_legal`: the holders of the manager role
/// (`managers`) are told, and the case owner `owner` is named.
pub(crate) async fn cheque_referred(
    state: &AppState,
    cheque: &Cheque,
    owner: i64,
    managers: &[i64],
) -> Result {
    if managers.is_empty() {
        return Ok(());
    }
    let owner_name = renox::auth::User::find(&state.db, owner)
        .await?
        .map_or_else(|| format!("#{owner}"), |user| user.name);
    let data = json!({
        "number": cheque.number,
        "text": format!(
            "Cheque {} ({} open) was referred to a legal case",
            cheque.number,
            money_text(&cheque.currency, cheque.open_amount)
        ),
        "owner": owner_name,
        "link": format!("/cheques/{}", cheque.id),
    });
    let mut request = NotifyRequest::new(CHEQUE_REFERRED, cheque.subject()).data(data);
    for id in managers {
        request = request.to_user(*id);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// The template variables of the reminders of a bounced cheque: the open
/// amount and the bounce fee.
fn cheque_reminder_data(cheque: &Cheque, party: &str, days: i64) -> renox::serde_json::Value {
    let fee = if cheque.bounce_fee_total > 0 {
        format!(
            ", plus a bounce fee of {}",
            money_text(&cheque.currency, cheque.bounce_fee_total)
        )
    } else {
        String::new()
    };
    json!({
        "number": cheque.number,
        "bank": cheque.bank,
        "party": party,
        "amount": money_text(&cheque.currency, cheque.amount),
        "open": money_text(&cheque.currency, cheque.open_amount),
        "fee": fee,
        "returned_on": cheque.bounced_on.map(date_text).unwrap_or_default(),
        "days": days,
        "link": format!("/cheques/{}", cheque.id),
    })
}

/// The users `FIN.RUL.18` reminds for `cheque`: the responsible staff, who
/// collected the cheque or else registered it; the holders of the accountant
/// role when neither is known.
pub(crate) async fn chase_staff(state: &AppState, cheque: &Cheque) -> Result<Vec<i64>> {
    let responsible: Vec<i64> = cheque
        .collected_by
        .into_iter()
        .chain(cheque.recorded_by)
        .collect();
    let mut ids = staff_ids(None, &responsible);
    if ids.is_empty() {
        ids = users_with_role_in(&state.db, ACCOUNTANT_ROLE, &Scope::global())
            .await?
            .iter()
            .map(|user| user.id)
            .collect();
    }
    Ok(ids)
}

/// `FIN.RUL.18` at N days: the drawer of the bounced received `cheque` is
/// reminded of the open amount and the bounce fee (not while it is blocked)
/// and the responsible staff are told. `days` is how long ago it bounced.
/// Returns whether the party was told.
pub(crate) async fn cheque_reminder(state: &AppState, cheque: &Cheque, days: i64) -> Result<bool> {
    let party = wl_kernel::parties::directory(state)
        .display_name(&state.db, cheque.party(), "en")
        .await?
        .unwrap_or_else(|| cheque.party().to_string());
    let data = cheque_reminder_data(cheque, &party, days);
    let status = wl_kernel::parties::directory(state)
        .status(&state.db, cheque.party())
        .await?;
    let told = tells_party(status);
    if told {
        let request = NotifyRequest::new(CHEQUE_REMINDER, cheque.subject())
            .to_party(cheque.party())
            .data(data.clone());
        notify::notifier(state).notify(state, request).await?;
    }
    let staff = chase_staff(state, cheque).await?;
    if !staff.is_empty() {
        let mut request = NotifyRequest::new(CHEQUE_REMINDER_STAFF, cheque.subject()).data(data);
        for id in staff {
            request = request.to_user(id);
        }
        notify::notifier(state).notify(state, request).await?;
    }
    Ok(told)
}

/// After the deductions of `deposit` were published as `deduction`: the party
/// is told what they come to and until when it may dispute (not while it is
/// blocked).
pub(crate) async fn deposit_deductions_published(
    state: &AppState,
    deposit: &Deposit,
    deduction: &DepositDeduction,
) -> Result {
    let status = wl_kernel::parties::directory(state)
        .status(&state.db, deposit.party())
        .await?;
    if !tells_party(status) {
        return Ok(());
    }
    let until = match deduction.window_ends_on {
        Some(end) => format!(
            " You may dispute them in the portal until {}.",
            date_text(end)
        ),
        None => String::new(),
    };
    let request = NotifyRequest::new(DEPOSIT_DEDUCTIONS, deposit.subject())
        .to_party(deposit.party())
        .data(json!({
            "deposit": deposit.id,
            "version": deduction.version,
            "total": money_text(&deposit.currency, deduction.total),
            "held": money_text(&deposit.currency, deposit.held_amount),
            "until": until,
            "kind": match deduction.kind {
                wl_kernel::deposits::DeductionKind::Forfeiture => "forfeiture",
                wl_kernel::deposits::DeductionKind::Deductions => "deductions",
            },
        }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// After the party disputed `lines` lines of `deduction`: the holders of the
/// accountant role are told and given the response window.
pub(crate) async fn deposit_disputed(
    state: &AppState,
    deposit: &Deposit,
    deduction: &DepositDeduction,
    lines: usize,
) -> Result {
    let ids: Vec<i64> = users_with_role_in(&state.db, ACCOUNTANT_ROLE, &Scope::global())
        .await?
        .iter()
        .map(|user| user.id)
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    let party = wl_kernel::parties::directory(state)
        .display_name(&state.db, deposit.party(), "en")
        .await?
        .unwrap_or_else(|| deposit.party().to_string());
    let days = crate::config_of(state).dispute_response_days;
    let mut request = NotifyRequest::new(DEPOSIT_DISPUTED, deposit.subject()).data(json!({
        "deposit": deposit.id,
        "party": party,
        "version": deduction.version,
        "lines": lines,
        "days": days,
        "link": format!("/deposits/{}", deposit.id),
    }));
    for id in ids {
        request = request.to_user(id);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// After staff answered the dispute of `deposit` (`answer`, with `response`):
/// the party is told (not while it is blocked).
pub(crate) async fn deposit_dispute_answered(
    state: &AppState,
    deposit: &Deposit,
    answer: wl_kernel::deposits::DisputeAnswer,
    response: &str,
) -> Result {
    let status = wl_kernel::parties::directory(state)
        .status(&state.db, deposit.party())
        .await?;
    if !tells_party(status) {
        return Ok(());
    }
    let outcome = match answer {
        wl_kernel::deposits::DisputeAnswer::Revised => {
            "The deductions were revised; the new version is in the portal and the dispute window starts again."
        }
        wl_kernel::deposits::DisputeAnswer::Upheld => "The deductions stand.",
    };
    let request = NotifyRequest::new(DEPOSIT_DISPUTE_ANSWERED, deposit.subject())
        .to_party(deposit.party())
        .data(json!({
            "deposit": deposit.id,
            "outcome": outcome,
            "response": response,
        }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// After `deposit` was handed to another holder: the party is told who now
/// holds it (not while it is blocked).
pub(crate) async fn deposit_transferred(state: &AppState, deposit: &Deposit) -> Result {
    let Some(holder) = deposit.transferred_to else {
        return Ok(());
    };
    let status = wl_kernel::parties::directory(state)
        .status(&state.db, deposit.party())
        .await?;
    if !tells_party(status) {
        return Ok(());
    }
    let holder_name = wl_kernel::parties::directory(state)
        .display_name(&state.db, wl_kernel::PartyId(holder), "en")
        .await?
        .unwrap_or_else(|| format!("#{holder}"));
    let request = NotifyRequest::new(DEPOSIT_TRANSFERRED, deposit.subject())
        .to_party(deposit.party())
        .data(json!({
            "deposit": deposit.id,
            "amount": money_text(&deposit.currency, deposit.held_amount),
            "holder": holder_name,
        }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the holders of the accountant role about the `task` about `deposit`.
pub(crate) async fn deposit_task(
    state: &AppState,
    deposit: &Deposit,
    task: &FollowupTask,
) -> Result {
    let ids: Vec<i64> = users_with_role_in(&state.db, ACCOUNTANT_ROLE, &Scope::global())
        .await?
        .iter()
        .map(|user| user.id)
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(DEPOSIT_TASK, deposit.subject()).data(json!({
        "text": task.text,
        "link": format!("/deposits/{}", deposit.id),
    }));
    for id in ids {
        request = request.to_user(id);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payment() -> Payment {
        Payment {
            id: 4,
            party_id: 7,
            number: Some("RCT-PGP-2026-0001".into()),
            amount: 300_000,
            currency: "AED".into(),
            payment_date: NaiveDate::from_ymd_opt(2026, 10, 18).expect("date"),
            ..Default::default()
        }
    }

    #[test]
    fn the_returned_type_is_a_transactional_email_with_english_text() {
        let def = returned_type();
        assert_eq!(def.key, "payments.payment_returned");
        assert_eq!(def.category, Category::Transactional);
        assert_eq!(def.channels, [NotifyChannel::Email]);
        let template = def.template_for(NotifyChannel::Email, "en").expect("en");
        assert!(template.body.contains("returned by the bank"));
        for variable in ["{{ number }}", "{{ amount }}", "{{ payment_date }}"] {
            assert!(template.body.contains(variable), "{variable}");
        }
    }

    #[test]
    fn the_returned_data_names_the_receipt_the_amount_and_the_date() {
        let def = returned_type();
        let body = &def
            .template_for(NotifyChannel::Email, "en")
            .expect("en")
            .body;
        let text = notify::fill(body, &returned_data(&payment()));
        assert!(text.contains("RCT-PGP-2026-0001"));
        assert!(text.contains("AED 3,000.00"));
        assert!(text.contains("18 Oct 2026"));
        assert!(!text.contains("{{"));
    }

    fn refund() -> Refund {
        Refund {
            id: 5,
            party_id: 7,
            amount: 100_000,
            currency: "AED".into(),
            ..Default::default()
        }
    }

    #[test]
    fn the_refund_types_are_transactional_emails() {
        for (def, key) in [
            (refund_failed_type(), "payments.refund_failed"),
            (refund_delayed_type(), "payments.refund_delayed"),
        ] {
            assert_eq!(def.key, key);
            assert_eq!(def.category, Category::Transactional);
            assert!(def.template_for(NotifyChannel::Email, "en").is_some());
        }
    }

    #[test]
    fn the_failed_notice_names_the_refund_the_amount_the_kind_and_the_reason() {
        let def = refund_failed_type();
        let template = def.template_for(NotifyChannel::Email, "en").expect("en");
        let data = refund_failed_data(
            &refund(),
            "Acme LLC",
            RefundFailureKind::BankReturn,
            "account closed",
        );
        let text = notify::fill(&template.body, &data);
        for part in [
            "#5",
            "Acme LLC",
            "AED 1,000.00",
            "Returned by the bank",
            "account closed",
            "/refunds/5",
        ] {
            assert!(text.contains(part), "{part}");
        }
        assert!(!text.contains("{{"));
    }

    #[test]
    fn the_delayed_notice_tells_the_party_the_amount() {
        let def = refund_delayed_type();
        let template = def.template_for(NotifyChannel::Email, "en").expect("en");
        let text = notify::fill(&template.body, &refund_delayed_data(&refund()));
        assert!(text.contains("your refund of AED 1,000.00 is delayed"));
        assert!(text.contains("we will contact you"));
    }

    #[test]
    fn staff_are_told_once_each() {
        assert_eq!(staff_ids(Some(3), &[5, 3, 4, 5]), [3, 4, 5]);
        assert_eq!(staff_ids(None, &[]), Vec::<i64>::new());
        assert_eq!(staff_ids(None, &[2]), [2]);
    }

    #[test]
    fn a_blocked_party_is_not_told() {
        assert!(!tells_party(Some(PartyStatus::Blocked)));
        assert!(tells_party(Some(PartyStatus::Active)));
        assert!(tells_party(None));
    }

    #[test]
    fn the_method_and_cancel_notices_tell_the_party_the_amount() {
        let data = method_changed_data(&refund(), PaymentMethod::Cheque);
        let def = refund_method_changed_type();
        let body = &def
            .template_for(NotifyChannel::Email, "en")
            .expect("en")
            .body;
        let text = notify::fill(body, &data);
        assert!(text.contains("AED 1,000.00"));
        assert!(text.contains("Cheque"));
        assert!(!text.contains("{{"));

        let def = refund_cancelled_type();
        let body = &def
            .template_for(NotifyChannel::Email, "en")
            .expect("en")
            .body;
        let text = notify::fill(body, &cancelled_data(&refund()));
        assert!(text.contains("AED 1,000.00"));
        assert!(text.contains("credit"));
    }

    #[test]
    fn the_method_task_notice_names_the_task_and_links_the_refund() {
        let failed = Refund {
            failure_reason: Some("card expired".into()),
            ..refund()
        };
        let task = FollowupTask::change_method(&failed);
        assert_eq!(task.text, "Change the method of refund #5 (AED 1,000.00)");
        let data = method_task_data(&failed, &task);
        let def = refund_method_task_type();
        let template = def.template_for(NotifyChannel::Email, "en").expect("en");
        let text = notify::fill(&template.body, &data);
        assert!(text.contains("card expired"));
        assert!(text.contains("/refunds/5"));
        assert!(!text.contains("{{"));
    }

    #[test]
    fn the_followup_notice_carries_the_task_text() {
        let task = FollowupTask::raise_charge(&payment(), 50_000, vec![]);
        let data = followup_data(&payment(), &task);
        assert_eq!(data["text"], "Raise a charge for AED 500.00");
        assert_eq!(data["link"], "/payments/tasks");
        let def = followup_type();
        let body = &def
            .template_for(NotifyChannel::Email, "en")
            .expect("en")
            .body;
        assert!(notify::fill(body, &data).contains("Raise a charge for AED 500.00"));
    }
}
