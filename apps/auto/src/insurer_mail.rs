//! What the workshop writes to an insurer, and what it tells the customer
//! about a claim (08 §4.7 *When it goes wrong*; BL-AUT-08 .. BL-AUT-10).
//!
//! | Type | To | When |
//! |---|---|---|
//! | [`SURVEY_TYPE`] | the insurer's claim email | the surveyor is asked to inspect (`book_survey`, or the coordinator's own request) |
//! | [`WITHDRAWAL_TYPE`] | the insurer's claim email | the customer withdrew the claim (`cancel`): the insurer is told in writing |
//! | [`TOTAL_LOSS_TYPE`] | the customer, by SMS | the insurer declared a total loss (`declare_total_loss`) |
//!
//! Decisions:
//!
//! - A letter to the insurer goes to the address on the insurer's row. When it
//!   cannot be queued (no address, or the notifier refuses it) the insurance
//!   coordinator gets a task to send it by hand, and when wl-notify later gives
//!   up on it the type's own give-up reaction opens the same kind of task for
//!   the coordinator role.
//! - **Anything sent to the insurer counts**: the first message queued stamps
//!   `sent_to_insurer_at` on the claim, and a claim with that stamp is no longer
//!   voidable (`nothing_sent` of `void`, BL-AUT-10).
//! - The withdrawal letter is the letter's text in the email; a rendered PDF is
//!   deferred (the platform has no PDF renderer, see `DEFERRED.md`).

use renox::db::now;
use renox::prelude::*;
use renox::serde_json::{Value, json};
use wl_kernel::PartyId;
use wl_kernel::notify::{
    self, GROUP_DOCUMENTS, GiveUpReaction, NotificationTypeDef, NotifyChannel, NotifyRecipient,
    NotifyRequest, notifier,
};
use wl_kernel::parties::directory;
use wl_kernel::role_tasks::{self, RoleTask};

use crate::claim::InsuranceClaim;
use crate::insurer::Insurer;
use crate::permissions;
use crate::texts::english;

/// The email asking the insurer's surveyor to inspect.
pub const SURVEY_TYPE: &str = "auto.claim_survey";
/// The letter telling the insurer the customer withdrew the claim.
pub const WITHDRAWAL_TYPE: &str = "auto.claim_withdrawal";
/// The SMS telling the customer the insurer declared a total loss.
pub const TOTAL_LOSS_TYPE: &str = "auto.claim_total_loss";

/// A type of email to the insurer, with its texts under `texts` in English.
fn insurer_type(key: &str, texts: &str) -> NotificationTypeDef {
    let subject = english(&format!("auto.mail.{texts}_subject"), &[]);
    let body = english(&format!("auto.mail.{texts}_body"), &[]);
    NotificationTypeDef::new(key, "auto")
        .group(GROUP_DOCUMENTS)
        .give_up(&[GiveUpReaction::StaffTask])
        .staff_role(permissions::INSURANCE_COORDINATOR_ROLE)
        .channels(&[NotifyChannel::Email])
        .template(NotifyChannel::Email, "en", Some(&subject), &body)
}

/// The notification type of the withdrawal letter.
pub fn withdrawal_type() -> NotificationTypeDef {
    insurer_type(WITHDRAWAL_TYPE, "withdrawal")
}

/// The notification type of the survey request.
pub fn survey_type() -> NotificationTypeDef {
    insurer_type(SURVEY_TYPE, "survey")
}

/// The notification type of the total-loss notice to the customer: an SMS in
/// English and Arabic; a message that gives up marks the contact invalid and
/// gives the advisor a task.
pub fn total_loss_type() -> NotificationTypeDef {
    NotificationTypeDef::new(TOTAL_LOSS_TYPE, "auto")
        .group(GROUP_DOCUMENTS)
        .give_up(&[
            GiveUpReaction::MarkContactInvalid,
            GiveUpReaction::StaffTask,
        ])
        .staff_role(permissions::SERVICE_ADVISOR_ROLE)
        .channels(&[NotifyChannel::Sms])
        .template(
            NotifyChannel::Sms,
            "en",
            None,
            &english("auto.mail.total_loss_sms_en", &[]),
        )
        .template(
            NotifyChannel::Sms,
            "ar",
            None,
            &english("auto.mail.total_loss_sms_ar", &[]),
        )
}

/// Registers the notification types with the kernel.
pub(crate) fn register() {
    notify::register_type(withdrawal_type());
    notify::register_type(survey_type());
    notify::register_type(total_loss_type());
}

/// The words every letter about `claim` shares: the claim and policy numbers,
/// the insurer's and the customer's names.
///
/// # Errors
///
/// The database's and the directory's errors.
pub async fn letter_data(state: &AppState, claim: &InsuranceClaim) -> Result<Value> {
    let insurer = Insurer::find_or_404(&state.db, claim.insurer_id).await?;
    let people = directory(state);
    let insurer_name = people
        .display_name(&state.db, PartyId(insurer.party_id), "en")
        .await?
        .unwrap_or_default();
    let customer = people
        .display_name(&state.db, PartyId(claim.customer_party_id), "en")
        .await?
        .unwrap_or_default();
    Ok(json!({
        "claim": claim.number(),
        "claim_no": claim.claim_no,
        "policy_no": claim.policy_no,
        "insurer": insurer_name,
        "customer": customer,
    }))
}

/// Emails the insurer of `claim` a message of `type_key` with `data`. Answers
/// whether it was queued; when it was not (no address on the insurer, or the
/// notifier refused it) the coordinator gets a task to send it by hand. The
/// first message queued stamps `sent_to_insurer_at` on the claim.
///
/// # Errors
///
/// The database's errors; the notifier's refusal is not one (it opens the
/// task).
pub async fn email_insurer(
    state: &AppState,
    claim: &InsuranceClaim,
    type_key: &str,
    data: Value,
) -> Result<bool> {
    let insurer = Insurer::find_or_404(&state.db, claim.insurer_id).await?;
    let address = insurer
        .claim_email
        .as_deref()
        .map(str::trim)
        .filter(|address| !address.is_empty())
        .map(str::to_owned);
    let queued = match address {
        Some(address) => {
            let request = NotifyRequest::new(type_key, claim.subject())
                .subject_label(&claim.number())
                .to(NotifyRecipient::Address {
                    channel: NotifyChannel::Email,
                    address,
                })
                .data(data);
            notifier(state).notify(state, request).await.is_ok()
        }
        None => false,
    };
    if queued {
        if claim.sent_to_insurer_at.is_none() {
            InsuranceClaim::query()
                .where_(InsuranceClaim::ID.eq(claim.id))
                .where_(InsuranceClaim::SENT_TO_INSURER_AT.is_null())
                .update(
                    &state.db,
                    &[("sent_to_insurer_at", &Some(now())), ("updated_at", &now())],
                )
                .await?;
        }
        return Ok(true);
    }
    let (title, body) = {
        let number = claim.number();
        let params: [(&str, &dyn std::fmt::Display); 1] = [("claim", &number)];
        (
            english("auto.mail.task_title", &params),
            english("auto.mail.task_body", &params),
        )
    };
    let task = RoleTask::new(
        claim.subject(),
        permissions::INSURANCE_COORDINATOR_ROLE,
        &title,
        &format!("auto.insurer_mail:{}:{type_key}", claim.id),
    )
    .description(&body);
    role_tasks::open(state, task).await?;
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_letters_are_emails_and_the_notice_to_the_customer_is_an_sms() {
        for def in [withdrawal_type(), survey_type()] {
            assert_eq!(def.channels, [NotifyChannel::Email]);
            assert!(def.template_for(NotifyChannel::Email, "en").is_some());
            assert_eq!(def.staff_role.as_deref(), Some("insurance_coordinator"));
        }
        let sms = total_loss_type();
        assert_eq!(sms.channels, [NotifyChannel::Sms]);
        assert!(sms.template_for(NotifyChannel::Sms, "ar").is_some());
    }
}
