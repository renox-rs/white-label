//! The notices of the vertical's negative flows: a message to staff users and
//! a message to the family of an enrolment (the student, or the guardian of a
//! student under 18, and the payer when that is another party).
//!
//! Three notification types carry them (10 §7.2 *Notification types*,
//! BL-EDU-17; the values are [`crate::defaults`]):
//!
//! - `edtech.family_notice` for students and guardians (session reminders and
//!   changes, absence alerts, tutor changes, seat hold and lapse notices,
//!   package reminders, certificate notices): the group *Guardian messages*,
//!   WhatsApp then e-mail, and when it gives up the other guardian is tried,
//!   then the counsellor gets a task (an adult student without a guardian goes
//!   straight to the task);
//! - `edtech.terms_request` for the terms, the refund policy and the guardian
//!   consent request: the group *Portal invitations, document requests,
//!   signature requests*, the contact marked invalid and a task to the
//!   counsellor. Nothing sends it yet (`DOC.SIG.01` is deferred); the type is
//!   registered so that the sender finds it ready;
//! - `edtech.notice` for staff: no group and no reaction, since staff always
//!   also get the in-app notification and keep the item on their list.
//!
//! Each has a title and a body the caller words, in English. None is
//! statutory. The invoices, membership charges, payment notices and reminders
//! are the shared modules' types; the host sets their reactions
//! (`host/src/app/edtech_settings.rs`), and the trial and one-to-one booking
//! messages keep the wl-scheduling group.

use renox::auth::permissions::users_with_role;
use renox::prelude::*;
use wl_kernel::notify::{
    self, Category, GiveUpReaction, NotificationTypeDef, NotifyChannel, NotifyRequest,
};
use wl_kernel::{PartyId, Subject};

use crate::defaults;
use crate::enrolment::Enrolment;
use crate::texts::english;

/// The notification type of a notice to staff.
pub const NOTICE: &str = "edtech.notice";

/// The notification type of a notice to the family of an enrolment.
pub const FAMILY_NOTICE: &str = "edtech.family_notice";

/// The notification type of the terms, refund policy and guardian consent
/// requests.
pub const TERMS_REQUEST: &str = "edtech.terms_request";

/// The notification type of a payment link sent to the payer: the group
/// *Invoices, statements, payment reminders*, a task to the accountant when it
/// gives up.
pub const PAYMENT_LINK: &str = "edtech.payment_link";

/// The notification type of a reminder about a package (credits running out,
/// the end date near): the group *Guardian messages*.
pub const PACKAGE_REMINDER: &str = "edtech.package_reminder";

/// The types of this module.
pub fn notification_types() -> Vec<NotificationTypeDef> {
    let subject = english("edtech.notice.subject", &[]);
    let body = english("edtech.notice.body", &[]);
    vec![
        NotificationTypeDef::new(NOTICE, "edtech")
            .group("")
            .category(Category::Reminder)
            .give_up(&[GiveUpReaction::None])
            .template(NotifyChannel::Email, "en", Some(&subject), &body),
        NotificationTypeDef::new(FAMILY_NOTICE, "edtech")
            .group(defaults::GUARDIAN_GROUP)
            .category(Category::Transactional)
            .give_up(&defaults::GUARDIAN_GIVE_UP)
            .staff_role(defaults::GUARDIAN_TASK_ROLE)
            .channels(&defaults::FAMILY_CHANNELS)
            .template(NotifyChannel::Email, "en", Some(&subject), &body),
        NotificationTypeDef::new(TERMS_REQUEST, "edtech")
            .group(defaults::REQUEST_GROUP)
            .category(Category::Transactional)
            .give_up(&defaults::REQUEST_GIVE_UP)
            .staff_role(defaults::REQUEST_TASK_ROLE)
            .template(NotifyChannel::Email, "en", Some(&subject), &body),
        NotificationTypeDef::new(PAYMENT_LINK, "edtech")
            .group(defaults::INVOICE_GROUP)
            .category(Category::Transactional)
            .give_up(&defaults::INVOICE_GIVE_UP)
            .staff_role(defaults::INVOICE_TASK_ROLE)
            .template(NotifyChannel::Email, "en", Some(&subject), &body),
        NotificationTypeDef::new(PACKAGE_REMINDER, "edtech")
            .group(defaults::GUARDIAN_GROUP)
            .category(Category::Reminder)
            .give_up(&defaults::GUARDIAN_GIVE_UP)
            .staff_role(defaults::GUARDIAN_TASK_ROLE)
            .channels(&defaults::FAMILY_CHANNELS)
            .template(NotifyChannel::Email, "en", Some(&subject), &body),
    ]
}

/// Registers the types.
pub(crate) fn register_types() {
    for def in notification_types() {
        notify::register_type(def);
    }
}

/// Tells `users` about `subject`; no message when there are none. Returns
/// whether a message was asked for.
///
/// # Errors
///
/// The notifier's errors.
pub async fn tell_users(
    state: &AppState,
    subject: Subject,
    label: &str,
    users: &[i64],
    title: &str,
    body: &str,
) -> Result<bool> {
    if users.is_empty() {
        return Ok(false);
    }
    let mut request = NotifyRequest::new(NOTICE, subject)
        .subject_label(label)
        .data(renox::serde_json::json!({ "title": title, "body": body }));
    let mut told: Vec<i64> = Vec::new();
    for user in users {
        if !told.contains(user) {
            told.push(*user);
            request = request.to_user(*user);
        }
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(true)
}

/// The parties told about an enrolment: the guardian of a student under 18,
/// else the student, and the payer when that is another party.
pub fn family_of(enrolment: &Enrolment) -> Vec<PartyId> {
    let mut parties = vec![PartyId(
        enrolment
            .guardian_party_id
            .unwrap_or(enrolment.student_party_id),
    )];
    let payer = PartyId(enrolment.payer_party_id);
    if !parties.contains(&payer) {
        parties.push(payer);
    }
    parties
}

/// Tells the family of `enrolment` something.
///
/// # Errors
///
/// The notifier's errors.
pub async fn tell_family(
    state: &AppState,
    enrolment: &Enrolment,
    title: &str,
    body: &str,
) -> Result {
    tell_family_as(state, FAMILY_NOTICE, enrolment, title, body).await
}

/// Tells the family of `enrolment` something under the notification type
/// `type_key` ([`FAMILY_NOTICE`], [`PAYMENT_LINK`] or [`PACKAGE_REMINDER`]).
///
/// # Errors
///
/// The notifier's errors.
pub async fn tell_family_as(
    state: &AppState,
    type_key: &str,
    enrolment: &Enrolment,
    title: &str,
    body: &str,
) -> Result {
    let request = NotifyRequest::new(type_key, enrolment.subject())
        .subject_label(&enrolment.number())
        .data(renox::serde_json::json!({ "title": title, "body": body }));
    let request = address(state, request, enrolment, &family_of(enrolment)).await?;
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Addresses `request` to `parties` of `enrolment`'s family. The guardian goes
/// as a rule ("a guardian of the student") when the student has several and the
/// enrolment's guardian is the first of them, so that a message that cannot be
/// delivered to guardian 1 is sent to guardian 2 before the counsellor gets a
/// task (`alternative_recipient`, 10 §4.1); every other party goes as itself.
async fn address(
    state: &AppState,
    mut request: NotifyRequest,
    enrolment: &Enrolment,
    parties: &[PartyId],
) -> Result<NotifyRequest> {
    let student = PartyId(enrolment.student_party_id);
    let guardians = wl_kernel::parties::directory(state)
        .guardians(&state.db, student)
        .await?;
    let by_rule = guardians.len() > 1
        && enrolment.guardian_party_id.is_some()
        && guardians.first().map(|g| g.0) == enrolment.guardian_party_id;
    for party in parties {
        request = if by_rule && Some(party.0) == enrolment.guardian_party_id {
            request.to_contact(student, notify::ContactPurpose::guardian_of())
        } else {
            request.to_party(*party)
        };
    }
    Ok(request)
}

/// Tells the family of every enrolment in `enrolments` something, each party
/// once: a guardian who pays for two students of the same batch gets one
/// message, not two (BL-EDU-09: "each student or guardian gets one message").
/// An enrolment whose parties were all told already is skipped.
///
/// # Errors
///
/// The notifier's errors.
pub async fn tell_families(
    state: &AppState,
    enrolments: &[Enrolment],
    title: &str,
    body: &str,
) -> Result {
    let mut told: Vec<PartyId> = Vec::new();
    for enrolment in enrolments {
        let fresh: Vec<PartyId> = family_of(enrolment)
            .into_iter()
            .filter(|party| !told.contains(party))
            .collect();
        if fresh.is_empty() {
            continue;
        }
        let request = NotifyRequest::new(FAMILY_NOTICE, enrolment.subject())
            .subject_label(&enrolment.number())
            .data(renox::serde_json::json!({ "title": title, "body": body }));
        let request = address(state, request, enrolment, &fresh).await?;
        notify::notifier(state).notify(state, request).await?;
        told.extend(fresh);
    }
    Ok(())
}

/// Tells the student of `enrolment` (the guardian of a student under 18, else
/// the student) something, and nobody else.
///
/// # Errors
///
/// The notifier's errors.
pub async fn tell_student(
    state: &AppState,
    enrolment: &Enrolment,
    title: &str,
    body: &str,
) -> Result {
    let party = family_of(enrolment)
        .into_iter()
        .next()
        .unwrap_or_else(|| enrolment.student());
    let request = NotifyRequest::new(FAMILY_NOTICE, enrolment.subject())
        .subject_label(&enrolment.number())
        .data(renox::serde_json::json!({ "title": title, "body": body }))
        .to_party(party);
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// The users who hold the global role `role`.
///
/// # Errors
///
/// The database's errors.
pub async fn holders(state: &AppState, role: &str) -> Result<Vec<i64>> {
    Ok(users_with_role(&state.db, role)
        .await?
        .iter()
        .map(|user| user.id)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_types_have_an_english_text_that_fills_in_the_title_and_the_body() {
        for def in notification_types() {
            let text = def
                .template_for(NotifyChannel::Email, "en")
                .expect("an English e-mail text");
            assert!(text.body.contains("{{ body }}"));
            assert!(
                text.subject
                    .as_deref()
                    .is_some_and(|s| s.contains("{{ title }}"))
            );
        }
    }

    #[test]
    fn the_family_is_the_guardian_else_the_student_and_the_payer_once() {
        let e = Enrolment {
            student_party_id: 1,
            guardian_party_id: Some(2),
            payer_party_id: 2,
            ..Enrolment::default()
        };
        assert_eq!(family_of(&e), [PartyId(2)]);
        let adult = Enrolment {
            student_party_id: 1,
            payer_party_id: 3,
            ..Enrolment::default()
        };
        assert_eq!(family_of(&adult), [PartyId(1), PartyId(3)]);
    }
}
