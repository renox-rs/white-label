//! The notices of the vertical's negative flows: a message to staff users and
//! a message to the family of an enrolment (the student, or the guardian of a
//! student under 18, and the payer when that is another party).
//!
//! Two notification types carry them, `edtech.notice` for staff and
//! `edtech.family_notice` for students and guardians. Each has a title and a
//! body the caller words, in English; the types name no group and take `none`
//! when a message gives up, so staff keep the item on their list.

use renox::auth::permissions::users_with_role;
use renox::prelude::*;
use wl_kernel::notify::{
    self, Category, GiveUpReaction, NotificationTypeDef, NotifyChannel, NotifyRequest,
};
use wl_kernel::{PartyId, Subject};

use crate::enrolment::Enrolment;
use crate::texts::english;

/// The notification type of a notice to staff.
pub const NOTICE: &str = "edtech.notice";

/// The notification type of a notice to the family of an enrolment.
pub const FAMILY_NOTICE: &str = "edtech.family_notice";

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
            .group("")
            .category(Category::Transactional)
            .give_up(&[GiveUpReaction::None])
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
    let mut request = NotifyRequest::new(FAMILY_NOTICE, enrolment.subject())
        .subject_label(&enrolment.number())
        .data(renox::serde_json::json!({ "title": title, "body": body }));
    for party in family_of(enrolment) {
        request = request.to_party(party);
    }
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
            assert!(text.subject.as_deref().is_some_and(|s| s.contains("{{ title }}")));
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
