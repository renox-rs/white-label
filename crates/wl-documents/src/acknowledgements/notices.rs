//! The messages of the SOP acknowledgements (wl-documents §4.3, §10), sent
//! through the kernel [`Notifier`](wl_kernel::notify::Notifier). They are
//! service messages to staff, not marketing (`NTF.ACL.01` consent does not
//! apply). Without wl-notify the kernel's fallback puts them in the bell.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::{Value, json};
use wl_kernel::Subject;
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};
use wl_kernel::workforce::StaffProfile;

use super::model::SopAcknowledgement;
use crate::sops::Sop;
use crate::texts::english;

/// The subject type of an acknowledgement, for notifications and links.
pub const SUBJECT_TYPE: &str = "documents.sop_acknowledgement";

/// The reader with an account is asked (staff inbox or employee portal).
pub const REQUESTED: &str = "documents.sop_ack_requested";
/// The reader without an account is asked by SMS to see their manager.
pub const REQUESTED_IN_PERSON: &str = "documents.sop_ack_requested_in_person";
/// An overdue reader (or, without an account, their manager) is reminded.
pub const REMINDER: &str = "documents.sop_ack_reminder";
/// The manager and the owner role are told, once, that a reader is far
/// overdue.
pub const ESCALATED: &str = "documents.sop_ack_escalated";
/// The reader is told, on their own channel, of an in-person record.
pub const IN_PERSON_RECORDED: &str = "documents.sop_ack_in_person_recorded";

/// `date` as `9 Nov 2026`.
fn date_text(date: NaiveDate) -> String {
    date.format("%-d %b %Y").to_string()
}

/// An e-mail type with English texts.
fn english_type(key: &str, texts: &str, category: Category) -> NotificationTypeDef {
    let subject = english(&format!("wl_documents.acks.notice.{texts}_subject"), &[]);
    let body = english(&format!("wl_documents.acks.notice.{texts}_body"), &[]);
    NotificationTypeDef::new(key, "documents")
        .group("documents")
        .category(category)
        .template(NotifyChannel::Email, "en", Some(&subject), &body)
}

/// A type that reaches the reader by SMS where a number is on record, else
/// by e-mail.
fn sms_type(key: &str, texts: &str) -> NotificationTypeDef {
    let subject = english(&format!("wl_documents.acks.notice.{texts}_subject"), &[]);
    let body = english(&format!("wl_documents.acks.notice.{texts}_body"), &[]);
    NotificationTypeDef::new(key, "documents")
        .group("documents")
        .channels(&[NotifyChannel::Sms, NotifyChannel::Email])
        .template(NotifyChannel::Sms, "en", None, &body)
        .template(NotifyChannel::Email, "en", Some(&subject), &body)
}

/// Registers the notification types with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(english_type(
        REQUESTED,
        "requested",
        Category::Transactional,
    ));
    notify::register_type(sms_type(REQUESTED_IN_PERSON, "requested_in_person"));
    notify::register_type(english_type(REMINDER, "reminder", Category::Reminder));
    notify::register_type(english_type(ESCALATED, "escalated", Category::Reminder));
    notify::register_type(sms_type(IN_PERSON_RECORDED, "in_person_recorded"));
}

/// The subject of `ack`.
fn subject_of(ack: &SopAcknowledgement) -> Subject {
    Subject::new(SUBJECT_TYPE, ack.id)
}

/// The reader's name for staff-facing text.
async fn name_of(state: &AppState, profile: &StaffProfile) -> String {
    wl_kernel::parties::directory(state)
        .display_name(&state.db, profile.party, "en")
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("#{}", profile.employee))
}

/// The template variables.
async fn data(
    state: &AppState,
    ack: &SopAcknowledgement,
    sop: &Sop,
    profile: &StaffProfile,
) -> Value {
    json!({
        "reader": name_of(state, profile).await,
        "sop": sop.code,
        "title": sop.title_en,
        "version": ack.version_no,
        "due": date_text(ack.due_date),
    })
}

/// Asks the reader to acknowledge: a reader with an account in the staff
/// inbox or the employee portal; one without is told by SMS where a number
/// is on record to see their manager.
pub(crate) async fn requested(
    state: &AppState,
    ack: &SopAcknowledgement,
    sop: &Sop,
    profile: &StaffProfile,
) -> Result {
    let request = match profile.user_id {
        Some(user) => NotifyRequest::new(REQUESTED, subject_of(ack)).to_user(user),
        None => NotifyRequest::new(REQUESTED_IN_PERSON, subject_of(ack)).to_party(profile.party),
    };
    let request = request
        .subject_label(&sop.code)
        .data(data(state, ack, sop, profile).await);
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Reminds `users` that `ack` is `days` days overdue.
pub(crate) async fn reminder(
    state: &AppState,
    ack: &SopAcknowledgement,
    sop: &Sop,
    profile: &StaffProfile,
    users: &[i64],
    days: i64,
) -> Result {
    overdue(state, REMINDER, ack, sop, profile, users, days).await
}

/// Tells `users` (the manager and the owner role) that `ack` is `days` days
/// overdue.
pub(crate) async fn escalated(
    state: &AppState,
    ack: &SopAcknowledgement,
    sop: &Sop,
    profile: &StaffProfile,
    users: &[i64],
    days: i64,
) -> Result {
    overdue(state, ESCALATED, ack, sop, profile, users, days).await
}

async fn overdue(
    state: &AppState,
    type_key: &str,
    ack: &SopAcknowledgement,
    sop: &Sop,
    profile: &StaffProfile,
    users: &[i64],
    days: i64,
) -> Result {
    if users.is_empty() {
        return Ok(());
    }
    let mut payload = data(state, ack, sop, profile).await;
    payload["days"] = json!(days);
    let mut request = NotifyRequest::new(type_key, subject_of(ack))
        .subject_label(&sop.code)
        .data(payload);
    for user in users {
        request = request.to_user(*user);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the reader, on their own channel, what was recorded in person and
/// by which witness.
pub(crate) async fn in_person_recorded(
    state: &AppState,
    ack: &SopAcknowledgement,
    sop: &Sop,
    profile: &StaffProfile,
    witness: i64,
) -> Result {
    let witness = User::find(&state.db, witness)
        .await?
        .map_or_else(|| format!("#{witness}"), |u| u.name);
    let mut payload = data(state, ack, sop, profile).await;
    payload["witness"] = json!(witness);
    payload["at"] = json!(date_text(wl_kernel::documents::today()));
    let request = NotifyRequest::new(IN_PERSON_RECORDED, subject_of(ack))
        .subject_label(&sop.code)
        .to_party(profile.party)
        .data(payload);
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}
