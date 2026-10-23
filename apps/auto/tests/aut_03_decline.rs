//! BL-AUT-03 (#399): expire an unanswered estimate and decline the job card
//! (`AUT.RUL.17`), through the real wl-scheduling for the booking and the
//! stand-ins of the invoice book and the notifier. The host test with the real
//! modules is `host/tests/auto_decline_real.rs`.

mod support;

use auto::declines::{self, COLLECT_TYPE};
use auto::job_cards::{self, Intake, NewLabour, NewPart, Promise};
use auto::{ClaimStatus, InsuranceClaim, JobCard, JobCardStatus};
use renox::chrono::TimeDelta;
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use support::claimfx::orient;
use support::{Garage, Messages, at, boot, day};
use wl_kernel::notify::{
    ChannelTried, GiveUpReaction, MessageGaveUp, MessageId, NotifyChannel, NotifyRecipient,
};
use wl_kernel::{PartyId, Subject};
use wl_scheduling::service::{self, Actor, NewBooking};
use wl_scheduling::{
    AvailabilityRule, Booking, BookingStatus, Resource, ResourceType, ServiceType, resource,
};

fn workshop() -> Subject {
    Subject::new("scheduling.location", 1)
}

/// The Hilux booked, checked in, its intake recorded (with the inspection fee
/// agreed, if any), inspected and, when `lines`, an estimate sent.
async fn waiting(t: &Garage, fee: i64, estimate: bool) -> (JobCard, Booking) {
    let mut bay = Resource::new("Bay 1", ResourceType::Bay).at(&workshop());
    bay.save(t.db()).await.unwrap();
    for weekday in 1..=7 {
        let mut rule = AvailabilityRule::new(bay.id, weekday, 0, 1440);
        rule.save(t.db()).await.unwrap();
    }
    let mut service = ServiceType::new("periodic_service", "Periodic service", 120);
    service.required_types = "bay".to_owned();
    service.check_in_required = true;
    service.save(t.db()).await.unwrap();
    resource::offer(t.db(), service.id, bay.id).await.unwrap();
    let vehicle = t.hilux().await;
    let sami = t.advisor().await;
    let start = renox::db::now() - TimeDelta::hours(2);
    let booking = service::book(
        t.state(),
        NewBooking::new(PartyId(7), service.id, &[bay.id], start)
            .for_subject(vehicle.subject())
            .at(workshop()),
        Actor::staff(sami.id),
        start - TimeDelta::hours(1),
    )
    .await
    .unwrap();
    service::check_in(t.state(), booking.id, Actor::staff(sami.id), renox::db::now())
        .await
        .unwrap();
    let card = JobCard::of_booking(t.db(), booking.id).await.unwrap().unwrap();
    let tareq_user = t.technician_user("Tareq").await;
    let tareq = t.technician("Tareq").await;
    job_cards::record_intake(
        t.state(),
        &sami,
        card.id,
        Intake {
            complaint: "Brake noise".to_owned(),
            odometer_in: 52_100,
            fuel_level: Some("half".to_owned()),
            condition: "Scratch on the left door".to_owned(),
            acknowledged_by: Some("Mona Hassan".to_owned()),
            signature: None,
            accident_damage: false,
            police_report_no: None,
            inspection_fee: fee,
            storage_fee_daily: 0,
        },
    )
    .await
    .unwrap();
    job_cards::assign_technician(t.state(), &sami, card.id, tareq_user.id)
        .await
        .unwrap();
    job_cards::start_inspection(t.state(), &tareq, card.id)
        .await
        .unwrap();
    if estimate {
        job_cards::add_labour(
            t.state(),
            &tareq,
            card.id,
            NewLabour::hourly("Brake pads", 1_000, 20_000),
        )
        .await
        .unwrap();
        job_cards::send_estimate(t.state(), &sami, card.id, Promise::default())
            .await
            .unwrap();
    }
    (t.reload(card.id).await, booking)
}

/// The job card entered `awaiting_approval` at `since`.
async fn entered_at(t: &Garage, card: &JobCard, since: DateTime) {
    JobCard::query()
        .where_(JobCard::ID.eq(card.id))
        .update(t.db(), &[("awaiting_since", &Some(since))])
        .await
        .unwrap();
}

/// JC-30 entered `awaiting_approval` on 2026-11-01 10:00 with nothing approved:
/// at the hourly check seven days later it is `declined` with the reason *no
/// response*, the collect SMS naming the plate and the job card is queued to
/// the customer, and the booking is `cancelled`. Not before the seven days, and
/// only once.
#[renox::test]
async fn aut_03_an_unanswered_estimate_expires_after_seven_days() {
    let t = boot().await;
    let (card, booking) = waiting(&t, 0, true).await;
    let entered = at(day(-10), 10, 0);
    entered_at(&t, &card, entered).await;

    let early = declines::expire_estimates(t.state(), entered + TimeDelta::days(7) - TimeDelta::hours(1))
        .await
        .unwrap();
    assert!(early.declined.is_empty(), "six days and 23 hours is not enough");
    assert_eq!(t.reload(card.id).await.status, JobCardStatus::AwaitingApproval);

    let done = declines::expire_estimates(t.state(), entered + TimeDelta::days(7))
        .await
        .unwrap();
    assert_eq!(done.declined, [card.id]);
    let declined = t.reload(card.id).await;
    assert_eq!(declined.status, JobCardStatus::Declined);
    assert_eq!(declined.closed_code.as_deref(), Some("no_response"));
    assert_eq!(declined.closed_reason.as_deref(), Some("No response to the estimate"));
    assert_eq!(declined.billed_net, 0, "no fee was agreed");
    assert!(t.books.about(&card.subject()).is_empty(), "no fee, no invoice");

    let sent = t.messages.of_type(COLLECT_TYPE);
    assert_eq!(sent.len(), 1, "the customer is asked to collect");
    assert!(Messages::told_party(&sent[0], 7));
    let text = Messages::text(&sent[0], NotifyChannel::Sms);
    assert!(text.contains("Dubai A 12345"), "{text}");
    assert!(text.contains(&card.number()), "{text}");
    assert!(text.contains("estimate expired"), "{text}");
    assert!(!text.contains("inspection fee"), "{text}");

    let booking = Booking::find_or_404(t.db(), booking.id).await.unwrap();
    assert_eq!(booking.status, BookingStatus::Cancelled);
    assert_eq!(booking.cancel_code.as_deref(), Some("estimate_declined"));

    // Once: the job card is terminal.
    let again = declines::expire_estimates(t.state(), entered + TimeDelta::days(9))
        .await
        .unwrap();
    assert!(again.declined.is_empty());
    assert_eq!(t.messages.of_type(COLLECT_TYPE).len(), 1);
}

/// An inspection fee of AED 150 agreed at check-in: an invoice for AED 150 +
/// VAT is issued to the customer at the decline and the collect SMS names AED
/// 157.50.
#[renox::test]
async fn aut_03_the_inspection_fee_agreed_at_check_in_is_invoiced_at_the_decline() {
    let t = boot().await;
    let (card, _) = waiting(&t, 15_000, true).await;
    let entered = at(day(-10), 10, 0);
    entered_at(&t, &card, entered).await;

    declines::expire_estimates(t.state(), entered + TimeDelta::days(7))
        .await
        .unwrap();

    let invoices = t.books.about(&card.subject());
    assert_eq!(invoices.len(), 1);
    assert_eq!(invoices[0].net(), 15_000);
    assert_eq!(invoices[0].gross(), 15_750, "AED 150 and 5% VAT");
    assert!(invoices[0].issued, "issued, not left in draft");
    assert_eq!(invoices[0].party(), PartyId(7));
    let declined = t.reload(card.id).await;
    assert_eq!(declined.invoice_id, Some(invoices[0].id));
    let sent = t.messages.of_type(COLLECT_TYPE);
    let text = Messages::text(&sent[0], NotifyChannel::Sms);
    assert!(text.contains("AED 157.50"), "{text}");
}

/// JC-31 `awaiting_approval` for added work after an approved estimate: nothing
/// happens on day 7.
#[renox::test]
async fn aut_03_added_work_on_an_approved_job_does_not_expire() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let card = t.card_in(&vehicle, JobCardStatus::AwaitingApproval).await;
    let entered = at(day(-10), 10, 0);
    JobCard::query()
        .where_(JobCard::ID.eq(card.id))
        .update(
            t.db(),
            &[
                ("awaiting_since", &Some(entered)),
                ("first_approved_at", &Some(entered - TimeDelta::days(1))),
            ],
        )
        .await
        .unwrap();

    let done = declines::expire_estimates(t.state(), entered + TimeDelta::days(7))
        .await
        .unwrap();

    assert!(done.declined.is_empty());
    assert_eq!(t.reload(card.id).await.status, JobCardStatus::AwaitingApproval);
    assert!(t.messages.of_type(COLLECT_TYPE).is_empty());
}

/// JC-32 linked to an open claim: nothing happens on day 7; the claim's own
/// chasing goes on.
#[renox::test]
async fn aut_03_a_job_card_linked_to_an_open_claim_does_not_expire() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let card = t.card_in(&vehicle, JobCardStatus::AwaitingApproval).await;
    let insurer = orient(&t).await;
    let mut claim = InsuranceClaim {
        insurer_id: insurer.id,
        customer_party_id: 7,
        vehicle_id: vehicle.id,
        job_card_id: Some(card.id),
        policy_no: "POL-1".to_owned(),
        claim_no: "OR-32".to_owned(),
        accident_date: day(-12),
        ..InsuranceClaim::default()
    };
    let mut tx = t.db().begin().await.unwrap();
    claim
        .insert_in_state(&mut tx, ClaimStatus::Registered, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let entered = at(day(-10), 10, 0);
    entered_at(&t, &card, entered).await;

    let done = declines::expire_estimates(t.state(), entered + TimeDelta::days(7))
        .await
        .unwrap();

    assert!(done.declined.is_empty());
    assert_eq!(t.reload(card.id).await.status, JobCardStatus::AwaitingApproval);
}

/// JC-33 `inspecting` with no labour or parts: when the customer leaves and
/// advisor A runs `decline`, it is `declined` and its booking `cancelled`;
/// with a part recorded the decline is refused in words.
#[renox::test]
async fn aut_03_the_advisor_declines_an_inspecting_job_card_with_nothing_recorded() {
    let t = boot().await;
    let (card, booking) = waiting(&t, 0, false).await;
    let sami = t.advisor().await;
    let tareq = t.technician("Tareq").await;
    assert_eq!(card.status, JobCardStatus::Inspecting);
    let part = job_cards::add_part(t.state(), &tareq, card.id, NewPart::new("Wiper", 1_000, 2_000))
        .await
        .unwrap();

    let refused = job_cards::decline(t.state(), &sami, card.id, "customer left")
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("recorded"), "{refused}");
    assert_eq!(t.reload(card.id).await.status, JobCardStatus::Inspecting);

    job_cards::remove_part(t.state(), &tareq, part.id).await.unwrap();
    let declined = job_cards::decline(t.state(), &sami, card.id, "customer left")
        .await
        .unwrap();
    assert_eq!(declined.status, JobCardStatus::Declined);
    assert_eq!(declined.closed_code.as_deref(), Some("customer_left"));
    let booking = Booking::find_or_404(t.db(), booking.id).await.unwrap();
    assert_eq!(booking.status, BookingStatus::Cancelled);
    assert_eq!(booking.cancel_code.as_deref(), Some("estimate_declined"));
    assert_eq!(t.messages.of_type(COLLECT_TYPE).len(), 1);
}

/// The collect message gives up: the job card shows *message not delivered* to
/// the advisor.
#[renox::test]
async fn aut_03_a_collect_message_that_gives_up_is_shown_to_the_advisor() {
    let t = boot().await;
    let (card, _) = waiting(&t, 0, false).await;
    let sami = t.advisor().await;
    job_cards::decline(t.state(), &sami, card.id, "customer left")
        .await
        .unwrap();
    t.app.acting_as(&t.advisor_user().await);
    t.app
        .get(&format!("/auto/job-cards/{}", card.id))
        .await
        .assert_ok()
        .assert_dont_see("message asking the customer");

    let gave_up = MessageGaveUp {
        message: MessageId(1),
        type_key: COLLECT_TYPE.to_owned(),
        subject: card.subject(),
        subject_label: None,
        recipient: NotifyRecipient::Party { id: PartyId(7) },
        gave_up_at: renox::db::now(),
        contact_point: None,
        channels_tried: Vec::<ChannelTried>::new(),
        statutory: false,
        reactions: vec![GiveUpReaction::MarkContactInvalid],
        alternative_of: None,
        contact_of: None,
    };
    declines::on_gave_up(t.state(), &gave_up).await.unwrap();

    assert!(t.reload(card.id).await.collect_message_failed_at.is_some());
    t.app
        .get(&format!("/auto/job-cards/{}", card.id))
        .await
        .assert_ok()
        .assert_see("message asking the customer to collect the vehicle was not delivered");
}
