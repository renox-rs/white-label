//! The notification types 08 registers beyond those of its rules (08 §7.1
//! *Notification types registered by 08*, wl-notify §5.4; BL-AUT-18), and the
//! SMS `AUT.RUL.02` that tells the customer the vehicle is ready.
//!
//! | Type | Group | Give-up |
//! |---|---|---|
//! | [`VEHICLE_READY`], [`CLAIM_APPROVED`] | Booking confirmations and reminders | `mark_contact_invalid`; the job card shows *message not delivered* |
//! | [`FLEET_BOOKING_REQUEST`] | Booking confirmations and reminders | the group's default (no task) |
//! | [`INSURER_STATEMENT`] | Invoices, statements, payment reminders | `mark_contact_invalid`, `alternative_recipient`, then a task for the insurance coordinator |
//! | [`SERVICE_REMINDER`] | Marketing | `mark_contact_invalid` |
//!
//! The types of the rules the app built earlier carry theirs where they are
//! sent: `auto.collect_vehicle` ([`crate::declines`]), `auto.parts_delay`
//! ([`crate::parts_delays`]), `auto.fleet_contract_ended` and
//! `auto.fleet_termination_notice` ([`crate::fleet_end`]),
//! `auto.insurer_request` and `auto.claim_withdrawal_letter`
//! ([`crate::insurer_mail`]), `auto.supplier_warranty_claim`
//! ([`crate::supplier_rules`]) and `auto.recall_notice` ([`crate::recalls`]).
//!
//! Decisions: only `auto.vehicle_ready` is sent here (`AUT.RUL.02`, when the
//! quality check passes and the job card is `ready`). `auto.claim_approved`
//! (`AUT.RUL.12`), `auto.fleet_booking_request` (`AUT.RUL.04`),
//! `auto.insurer_statement` (`AUT.RUL.13`) and `auto.service_reminder`
//! (`AUT.RUL.01`) are registered, with their reactions, for the rules that send
//! them (deferred).

use renox::db::now;
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::Transitioned;
use wl_kernel::PartyId;
use wl_kernel::notify::{
    self, Category, GROUP_BOOKINGS, GROUP_INVOICES, GROUP_MARKETING, GiveUpReaction, MessageGaveUp,
    NotificationTypeDef, NotifyChannel, NotifyRequest, notifier,
};

use crate::job_card::{JOB_CARD_SUBJECT_TYPE, JobCard};
use crate::job_card_machine::PASS_QC;
use crate::permissions;
use crate::texts::english;
use crate::vehicle::Vehicle;

/// The SMS telling the customer the vehicle is ready (`AUT.RUL.02`).
pub const VEHICLE_READY: &str = "auto.vehicle_ready";
/// The SMS telling the customer the insurer approved the claim (`AUT.RUL.12`).
pub const CLAIM_APPROVED: &str = "auto.claim_approved";
/// The booking request to the fleet contact (`AUT.RUL.04`).
pub const FLEET_BOOKING_REQUEST: &str = "auto.fleet_booking_request";
/// The weekly statement to the insurer (`AUT.RUL.13`).
pub const INSURER_STATEMENT: &str = "auto.insurer_statement";
/// The service reminder to an opted-in vehicle contact (`AUT.RUL.01`).
pub const SERVICE_REMINDER: &str = "auto.service_reminder";

/// An SMS to the customer in English and Arabic: the group *Booking
/// confirmations and reminders*, the contact marked invalid on give-up, the
/// customer's other contact tried, and no staff task (the job card shows *message not delivered*).
fn sms(key: &str, texts: &str) -> NotificationTypeDef {
    NotificationTypeDef::new(key, "auto")
        .group(GROUP_BOOKINGS)
        .category(Category::Transactional)
        .channels(&[NotifyChannel::Sms])
        // No task, as the issue says: the kernel refuses a lone
        // `MarkContactInvalid` on a transactional type, so the other contact
        // of the customer is tried too.
        .give_up(&[
            GiveUpReaction::MarkContactInvalid,
            GiveUpReaction::AlternativeRecipient,
        ])
        .template(
            NotifyChannel::Sms,
            "en",
            None,
            &english(&format!("auto.notice.{texts}_sms_en"), &[]),
        )
        .template(
            NotifyChannel::Sms,
            "ar",
            None,
            &english(&format!("auto.notice.{texts}_sms_ar"), &[]),
        )
}

/// An email whose subject and body the sender gives, in both languages.
fn email(key: &str) -> NotificationTypeDef {
    NotificationTypeDef::new(key, "auto")
        .category(Category::Transactional)
        .channels(&[NotifyChannel::Email])
        .template(
            NotifyChannel::Email,
            "en",
            Some("{{ title }}"),
            "{{ body }}",
        )
        .template(
            NotifyChannel::Email,
            "ar",
            Some("{{ title }}"),
            "{{ body }}",
        )
}

/// The types this part registers.
pub fn types() -> Vec<NotificationTypeDef> {
    vec![
        sms(VEHICLE_READY, "ready"),
        sms(CLAIM_APPROVED, "claim_approved"),
        // The group's default reaction: no task.
        email(FLEET_BOOKING_REQUEST).group(GROUP_BOOKINGS),
        email(INSURER_STATEMENT)
            .group(GROUP_INVOICES)
            .give_up(&[
                GiveUpReaction::MarkContactInvalid,
                GiveUpReaction::AlternativeRecipient,
                GiveUpReaction::StaffTask,
            ])
            .staff_role(permissions::INSURANCE_COORDINATOR_ROLE),
        email(SERVICE_REMINDER)
            .group(GROUP_MARKETING)
            .category(Category::Marketing)
            .give_up(&[GiveUpReaction::MarkContactInvalid]),
    ]
}

/// Tells the customer of `card`, which just became `ready`, by SMS. A message
/// that cannot even be queued is shown to the advisor like one that gave up.
///
/// # Errors
///
/// The database's errors.
pub async fn vehicle_ready(state: &AppState, job_card_id: i64) -> Result {
    let Some(card) = JobCard::find(&state.db, job_card_id).await? else {
        return Ok(());
    };
    let vehicle = Vehicle::find_or_404(&state.db, card.vehicle_id).await?;
    let (workshop, address, phone) = crate::declines::workshop_contact(state);
    let request = NotifyRequest::new(VEHICLE_READY, card.subject())
        .subject_label(&card.number())
        .to_party(PartyId(card.customer_party_id))
        .data(json!({
            "plate": vehicle.plate(),
            "job": card.number(),
            "workshop": workshop,
            "address": address,
            "phone": phone,
        }));
    if notifier(state).notify(state, request).await.is_err() {
        mark_failed(state, card.id, now()).await?;
    }
    Ok(())
}

/// Shows *message not delivered* on the job card.
async fn mark_failed(state: &AppState, job_card_id: i64, at: DateTime) -> Result {
    JobCard::query()
        .where_(JobCard::ID.eq(job_card_id))
        .update(
            &state.db,
            &[
                ("ready_message_failed_at", &Some(at)),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(())
}

/// The job card follows the message: `MessageGaveUp` of the *vehicle ready*
/// SMS marks it for the advisor, and no task is made.
///
/// # Errors
///
/// The database's errors.
pub async fn on_gave_up(state: &AppState, event: &MessageGaveUp) -> Result {
    if event.type_key != VEHICLE_READY || event.subject.subject_type != JOB_CARD_SUBJECT_TYPE {
        return Ok(());
    }
    mark_failed(state, event.subject.subject_id, event.gave_up_at).await
}

/// Registers the notification types and the listeners.
pub(crate) fn register(app: &mut Registry) {
    for def in types() {
        notify::register_type(def);
    }
    app.listen(|event: Transitioned<JobCard>, state: AppState| async move {
        if event.transition.as_str() == PASS_QC {
            vehicle_ready(&state, event.key).await
        } else {
            Ok(())
        }
    });
    app.listen(
        |event: MessageGaveUp, state: AppState| async move { on_gave_up(&state, &event).await },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aut_18_the_ready_sms_marks_the_contact_invalid_and_makes_no_task() {
        let def = sms(VEHICLE_READY, "ready");
        assert_eq!(def.group, GROUP_BOOKINGS);
        assert_eq!(
            def.give_up,
            [
                GiveUpReaction::MarkContactInvalid,
                GiveUpReaction::AlternativeRecipient
            ]
        );
        assert!(def.staff_role.is_none());
        assert!(def.template_for(NotifyChannel::Sms, "ar").is_some());
    }

    #[test]
    fn aut_18_the_insurer_statement_tries_the_other_contact_then_the_coordinator() {
        let found = types()
            .into_iter()
            .find(|t| t.key == INSURER_STATEMENT)
            .expect("the statement");
        assert_eq!(found.group, GROUP_INVOICES);
        assert_eq!(
            found.give_up,
            [
                GiveUpReaction::MarkContactInvalid,
                GiveUpReaction::AlternativeRecipient,
                GiveUpReaction::StaffTask
            ]
        );
        assert_eq!(found.staff_role.as_deref(), Some("insurance_coordinator"));
    }
}
