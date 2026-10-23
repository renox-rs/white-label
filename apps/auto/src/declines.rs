//! A job card that ends before any work was approved: declined by the
//! customer, by the advisor when the customer leaves, or by the system when the
//! estimate was not answered in 7 days (08 §4.4 row 1, §7.1 *Job card*
//! `decline`, §7.2 `AUT.RUL.17`; BL-AUT-03).
//!
//! After the decline committed ([`finish`], once): the inspection fee agreed at
//! check-in is invoiced and issued (an invoice never for a fee that was not
//! agreed), the booking is cancelled with the reason `estimate_declined`
//! (`SCH.ACL.06`, for the advisor of the job card), and the customer is asked
//! to collect the vehicle by SMS (`auto.collect_vehicle`: the plate, the job
//! card, why, the fee with VAT and the workshop's address and phone). A message
//! that gives up marks the contact invalid (wl-notify) and the job card shows
//! *message not delivered* to the advisor.
//!
//! **The rule** ([`expire_estimates`], hourly, Asia/Dubai): 7 × 24 hours after
//! the job card last entered `awaiting_approval`, with nothing approved yet and
//! no open insurance claim linked, the system declines it with the reason *no
//! response*. Added work on a job that was approved is not touched (the
//! reminders go on), a job card funded by an open claim is chased through the
//! claim, and a job card is declined once (it is terminal). The second branch
//! of the rule is the private estimate sent after a claim was withdrawn
//! (*continue privately*, BL-AUT-08): unsigned after the same 7 days, the system
//! cancels the job card with the reason *claim withdrawn* and the work done is
//! invoiced to the customer ([`crate::claim_withdrawal::cancel_unsigned`]).
//!
//! Decisions:
//!
//! - The clock is `awaiting_since`, set at each `send_estimate`; revising the
//!   estimate while it waits is not possible, so nothing restarts it.
//! - A booking is cancelled for the advisor who checked the vehicle in: after
//!   the check-in the booking machine accepts staff only. A job card without an
//!   advisor leaves its booking to the desk.

use std::sync::Arc;

use renox::chrono::Duration;
use renox::db::{DateTime, now};
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::Transitioned;
use wl_kernel::notify::{
    self, Category, GROUP_BOOKINGS, GiveUpReaction, MessageGaveUp, NotificationTypeDef,
    NotifyChannel, NotifyRequest, notifier,
};
use wl_kernel::{ClientConfig, PartyId};

use crate::approval::EstimateApproval;
use crate::claim::InsuranceClaim;
use crate::job_card::{JOB_CARD_SUBJECT_TYPE, JobCard, JobCardStatus};
use crate::job_card_machine::{DECLINE, DECLINE_SYSTEM, JobInput};
use crate::job_stops::close_booking;
use crate::permissions;
use crate::reasons::ESTIMATE_DECLINED;
use crate::stop::{self, NO_RESPONSE, TOTAL_LOSS};
use crate::texts::english;
use crate::vehicle::Vehicle;

/// The rule.
pub const RULE: &str = "AUT.RUL.17";
/// N: the days an estimate waits for an answer.
pub const EXPIRY_DAYS: i64 = 7;
/// The notification type of the message asking the customer to collect.
pub const COLLECT_TYPE: &str = "auto.collect_vehicle";
/// The scheduled job and command of the hourly check.
pub const JOB: &str = "auto:estimate-expiry";
/// How often [`JOB`] runs, in minutes.
pub const JOB_EVERY_MINUTES: u64 = 60;

/// The notification type: an SMS, in English and Arabic.
pub fn collect_type() -> NotificationTypeDef {
    NotificationTypeDef::new(COLLECT_TYPE, "auto")
        .group(GROUP_BOOKINGS)
        .category(Category::Transactional)
        .channels(&[NotifyChannel::Sms])
        .give_up(&[
            GiveUpReaction::MarkContactInvalid,
            GiveUpReaction::StaffTask,
        ])
        .staff_role(permissions::SERVICE_ADVISOR_ROLE)
        .template(
            NotifyChannel::Sms,
            "en",
            None,
            &english("auto.decline.sms_en", &[]),
        )
        .template(
            NotifyChannel::Sms,
            "ar",
            None,
            &english("auto.decline.sms_ar", &[]),
        )
}

/// The workshop's name, address and phone for a message.
pub(crate) fn workshop_contact(state: &AppState) -> (String, String, String) {
    let Some(config) = state.provided::<Arc<ClientConfig>>() else {
        return (String::new(), String::new(), String::new());
    };
    let address = config
        .legal_entities
        .iter()
        .find_map(|entity| entity.address_en.clone())
        .unwrap_or_default();
    let phone = config
        .section("auto")
        .and_then(|section| section.get("phone"))
        .and_then(|phone| phone.as_str())
        .map(str::trim)
        .unwrap_or_default()
        .to_owned();
    (config.client.name.clone(), address, phone)
}

/// Asks the customer of the declined job card `card` to collect the vehicle.
async fn ask_to_collect(state: &AppState, card: &JobCard) -> Result {
    let vehicle = Vehicle::find_or_404(&state.db, card.vehicle_id).await?;
    let (workshop, address, phone) = workshop_contact(state);
    let reason = match card.closed_code.as_deref() {
        Some(NO_RESPONSE) => english("auto.decline.reason_expired", &[]),
        _ => english("auto.decline.reason_declined", &[]),
    };
    let fee = if card.billed_net > 0 {
        let amount = crate::money_text(stop::gross_of_net(card.billed_net));
        english("auto.decline.fee", &[("amount", &amount)])
    } else {
        String::new()
    };
    let request = NotifyRequest::new(COLLECT_TYPE, card.subject())
        .subject_label(&card.number())
        .to_party(PartyId(card.customer_party_id))
        .data(json!({
            "plate": vehicle.plate(),
            "job": card.number(),
            "reason": reason,
            "fee": fee,
            "workshop": workshop,
            "address": address,
            "phone": phone,
        }));
    // A message that cannot even be queued is shown to the advisor like one
    // that gave up; the decline stands.
    if notifier(state).notify(state, request).await.is_err() {
        JobCard::query()
            .where_(JobCard::ID.eq(card.id))
            .update(
                &state.db,
                &[
                    ("collect_message_failed_at", &Some(now())),
                    ("updated_at", &now()),
                ],
            )
            .await?;
    }
    Ok(())
}

/// What follows a decline that committed: the invoice of the inspection fee,
/// the booking and the message asking the customer to collect the vehicle,
/// each once. A step that failed is done by the next call.
///
/// # Errors
///
/// The database's, the invoicer's, the scheduling contract's and the
/// notifier's errors.
pub async fn finish(state: &AppState, job_card_id: i64) -> Result<JobCard> {
    let card = JobCard::find_or_404(&state.db, job_card_id).await?;
    if card.status != JobCardStatus::Declined || card.closing_done_at.is_some() {
        return Ok(card);
    }
    crate::stop_billing::bill_closed(state, card.id).await?;
    let card = JobCard::find_or_404(&state.db, job_card_id).await?;
    close_booking(state, &card, ESTIMATE_DECLINED).await?;
    let claimed = JobCard::query()
        .where_(JobCard::ID.eq(card.id))
        .where_(JobCard::CLOSING_DONE_AT.is_null())
        .update(
            &state.db,
            &[("closing_done_at", &Some(now())), ("updated_at", &now())],
        )
        .await?;
    let card = JobCard::find_or_404(&state.db, job_card_id).await?;
    // After a total loss the customer is told through the claim (BL-AUT-09).
    if claimed > 0 && card.closed_code.as_deref() != Some(TOTAL_LOSS) {
        ask_to_collect(state, &card).await?;
    }
    Ok(card)
}

/// What the hourly check did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Expired {
    /// The job cards the system declined.
    pub declined: Vec<i64>,
    /// The job cards the system cancelled because the private estimate sent
    /// after a claim was withdrawn was not signed in time (the second branch of
    /// the rule, BL-AUT-08).
    pub cancelled: Vec<i64>,
    /// The job cards a decline was refused for, with why.
    pub failed: Vec<(i64, String)>,
}

/// `AUT.RUL.17` at `at`: declines the job cards that entered
/// `awaiting_approval` [`EXPIRY_DAYS`] days or more before, with nothing
/// approved and no open insurance claim.
///
/// # Errors
///
/// The database's errors; a job card that cannot be declined is reported in
/// [`Expired::failed`] and tried again the next hour.
pub async fn expire_estimates(state: &AppState, at: DateTime) -> Result<Expired> {
    let mut done = Expired::default();
    let waiting = JobCard::query()
        .where_(JobCard::STATUS.eq(JobCardStatus::AwaitingApproval))
        .order_by(JobCard::ID)
        .get(&state.db)
        .await?;
    for card in waiting {
        let Some(since) = card.awaiting_since else {
            continue;
        };
        // The second branch: the private estimate sent after a claim was
        // withdrawn is not signed, which counts as *stop*.
        if card.private_estimate {
            if at >= since + Duration::days(EXPIRY_DAYS) {
                match crate::claim_withdrawal::cancel_unsigned(state, &card).await {
                    Ok(_) => done.cancelled.push(card.id),
                    Err(error) => done.failed.push((card.id, error.to_string())),
                }
            }
            continue;
        }
        if card.work_approved() || at < since + Duration::days(EXPIRY_DAYS) {
            continue;
        }
        let open_claim = InsuranceClaim::of_job_card(&state.db, card.id)
            .await?
            .into_iter()
            .any(|claim| !claim.status.is_closed());
        if open_claim {
            continue;
        }
        let round = EstimateApproval::pending_of(&state.db, card.id).await?;
        let input = JobInput {
            on: Some(wl_kernel::documents::dubai_date(at)),
            approval_id: round.map(|r| r.id),
            code: Some(NO_RESPONSE.to_owned()),
            ..JobInput::default()
        };
        let reason = english("auto.stop.code.no_response", &[]);
        match crate::job_cards::run_system(
            state,
            &card,
            DECLINE_SYSTEM,
            Some(reason.as_str()),
            input,
        )
        .await
        {
            Ok(_) => done.declined.push(card.id),
            Err(error) => done.failed.push((card.id, error.to_string())),
        }
    }
    Ok(done)
}

/// The job card follows the message: `MessageGaveUp` of the collect message
/// marks it for the advisor.
///
/// # Errors
///
/// The database's errors.
pub async fn on_gave_up(state: &AppState, event: &MessageGaveUp) -> Result {
    if event.type_key != COLLECT_TYPE || event.subject.subject_type != JOB_CARD_SUBJECT_TYPE {
        return Ok(());
    }
    JobCard::query()
        .where_(JobCard::ID.eq(event.subject.subject_id))
        .update(
            &state.db,
            &[
                ("collect_message_failed_at", &Some(event.gave_up_at)),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(())
}

/// Registers the notification type, the listeners, the hourly check and its
/// command.
pub(crate) fn register(app: &mut Registry) {
    notify::register_type(collect_type());
    app.listen(|event: Transitioned<JobCard>, state: AppState| async move {
        match event.transition.as_str() {
            DECLINE | DECLINE_SYSTEM => finish(&state, event.key).await.map(|_| ()),
            _ => Ok(()),
        }
    });
    app.listen(
        |event: MessageGaveUp, state: AppState| async move { on_gave_up(&state, &event).await },
    );
    app.schedule()
        .every_minutes(JOB_EVERY_MINUTES, JOB, |state: AppState| async move {
            expire_estimates(&state, now()).await.map(|_| ())
        });
    app.command(
        JOB,
        "Decline the job cards whose estimate nobody answered in 7 days and ask the customer to collect the vehicle",
        |_args, state: AppState| async move {
            let done = expire_estimates(&state, now()).await?;
            println!(
                "{} job cards declined, {} cancelled.",
                done.declined.len(),
                done.cancelled.len()
            );
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_collect_message_is_a_sms_in_two_languages() {
        let def = collect_type();
        assert_eq!(def.channels, [NotifyChannel::Sms]);
        assert!(def.template_for(NotifyChannel::Sms, "ar").is_some());
        assert_eq!(def.group, GROUP_BOOKINGS);
    }
}
