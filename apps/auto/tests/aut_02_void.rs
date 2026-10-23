//! BL-AUT-02 (#398): void a job card opened in error (08 §7.1 *Job card*
//! `void`, `PLT.ACL.09`), through the real wl-scheduling for the booking.

mod support;

use auto::job_cards::{self, NewPart};
use auto::{InsuranceClaim, JobCard, JobCardStatus, ClaimStatus};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use support::claimfx::{carla, orient};
use support::{Garage, boot, day};
use wl_kernel::voids::VoidRecord;
use wl_kernel::{PartyId, Subject};
use wl_scheduling::service::{self, Actor, NewBooking};
use wl_scheduling::{
    AvailabilityRule, Booking, BookingStatus, Resource, ResourceType, ServiceType, resource,
};

fn workshop() -> Subject {
    Subject::new("scheduling.location", 1)
}

/// A checked-in booking of the Hilux and the job card its check-in opened.
async fn checked_in(t: &Garage, name: &str) -> (JobCard, Booking) {
    let mut bay = Resource::new(name, ResourceType::Bay).at(&workshop());
    bay.save(t.db()).await.unwrap();
    for weekday in 1..=7 {
        let mut rule = AvailabilityRule::new(bay.id, weekday, 0, 1440);
        rule.save(t.db()).await.unwrap();
    }
    let code = format!("service_{}", bay.id);
    let mut service = ServiceType::new(&code, "Periodic service", 120);
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
    .expect("the booking is made");
    service::check_in(t.state(), booking.id, Actor::staff(sami.id), renox::db::now())
        .await
        .expect("checked in");
    let card = JobCard::of_booking(t.db(), booking.id).await.unwrap().unwrap();
    (card, booking)
}

/// JC-20 `checked_in` with no lines: A voids it with reason "duplicate of
/// JC-19"; it is `voided`, keeps its number, its booking is `cancelled` with
/// work done none and `RecordVoided` is emitted.
#[renox::test]
async fn aut_02_a_duplicate_check_in_is_voided_and_its_booking_cancelled_with_no_work() {
    let t = boot().await;
    let (card, booking) = checked_in(&t, "Bay 1").await;
    let sami = t.advisor().await;

    let voided = job_cards::void(t.state(), &sami, card.id, "duplicate of JOB-00019")
        .await
        .expect("the void runs");

    assert_eq!(voided.status, JobCardStatus::Voided);
    assert_eq!(voided.number(), card.number(), "the number is kept");
    assert_eq!(voided.closed_reason.as_deref(), Some("duplicate of JOB-00019"));
    let booking = Booking::find_or_404(t.db(), booking.id).await.unwrap();
    assert_eq!(booking.status, BookingStatus::Cancelled);
    assert_eq!(booking.cancel_code.as_deref(), Some("duplicate_check_in"));
    assert_eq!(booking.work_done_amount, 0, "work done none");
    let events = t.voided.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].subject, card.subject());
    assert_eq!(events[0].user_id, sami.id);
    let record = VoidRecord::of_subject(t.db(), &card.subject())
        .await
        .unwrap()
        .expect("the Void record");
    assert_eq!(record.number.as_deref(), Some(card.number().as_str()));
    assert_eq!(record.state_before, "checked_in");
    assert!(
        t.voided.all().len() == 1 && t.cancelled.all().is_empty(),
        "a void is not a cancel"
    );
}

/// JC-21 `checked_in` with a part line: the void is refused in words and the
/// job card stays; a line taken off the estimate before it was sent does not
/// count.
#[renox::test]
async fn aut_02_a_job_card_with_a_part_line_is_not_voided() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let card = t.card_in(&vehicle, JobCardStatus::CheckedIn).await;
    let sami = t.advisor().await;
    let tareq = t.technician("Tareq").await;
    let part = job_cards::add_part(t.state(), &tareq, card.id, NewPart::new("Wiper", 1_000, 2_000))
        .await
        .unwrap();

    let refused = job_cards::void(t.state(), &sami, card.id, "duplicate")
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("recorded"), "{refused}");
    assert_eq!(t.reload(card.id).await.status, JobCardStatus::CheckedIn);
    assert!(t.voided.all().is_empty());

    job_cards::remove_part(t.state(), &tareq, part.id).await.unwrap();
    let voided = job_cards::void(t.state(), &sami, card.id, "duplicate")
        .await
        .unwrap();
    assert_eq!(voided.status, JobCardStatus::Voided);
}

/// JC-22 `inspecting`: void is not offered (refused by the machine), and the
/// page offers no void nor an action to unlink a claim.
#[renox::test]
async fn aut_02_void_is_only_for_a_checked_in_job_card_and_no_unlink_is_offered() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let sami = t.advisor().await;
    let inspecting = t.card_in(&vehicle, JobCardStatus::Inspecting).await;
    let refused = job_cards::void(t.state(), &sami, inspecting.id, "duplicate")
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("state"), "{refused}");

    let fresh = t.card_in(&vehicle, JobCardStatus::CheckedIn).await;
    t.app.acting_as(&t.advisor_user().await);
    t.app
        .get(&format!("/auto/job-cards/{}", fresh.id))
        .await
        .assert_ok()
        .assert_see("Void")
        .assert_dont_see("nlink");
    t.app
        .get(&format!("/auto/job-cards/{}", inspecting.id))
        .await
        .assert_ok()
        .assert_dont_see("Void");
    assert!(
        !JobCard::machine()
            .transitions()
            .iter()
            .any(|t| t.name().contains("unlink")),
        "no action unlinks a claim"
    );
    // A user without `job_cards.advise` cannot void.
    let tareq = t.technician("Tareq").await;
    let denied = job_cards::void(t.state(), &tareq, fresh.id, "duplicate").await;
    assert!(matches!(denied, Err(Error::Forbidden)));
}

/// JC-23 `checked_in` linked to claim CL-20 `registered` with nothing sent: the
/// void is refused; after the claim is voided it succeeds.
#[renox::test]
async fn aut_02_a_job_card_linked_to_an_open_claim_is_voided_after_the_claim() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let card = t.card_in(&vehicle, JobCardStatus::CheckedIn).await;
    let sami = t.advisor().await;
    let insurer = orient(&t).await;
    let mut claim = InsuranceClaim {
        insurer_id: insurer.id,
        customer_party_id: 7,
        vehicle_id: vehicle.id,
        job_card_id: Some(card.id),
        policy_no: "POL-1".to_owned(),
        claim_no: "OR-20".to_owned(),
        accident_date: day(-3),
        ..InsuranceClaim::default()
    };
    let mut tx = t.db().begin().await.unwrap();
    claim
        .insert_in_state(&mut tx, ClaimStatus::Registered, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let refused = job_cards::void(t.state(), &sami, card.id, "duplicate")
        .await
        .unwrap_err();
    assert!(refused.to_string().contains(&claim.number()), "{refused}");
    assert_eq!(t.reload(card.id).await.status, JobCardStatus::CheckedIn);

    let carla = carla(&t).await;
    auto::claims::void(t.state(), &carla, claim.id, "registered in error")
        .await
        .unwrap();
    let voided = job_cards::void(t.state(), &sami, card.id, "duplicate")
        .await
        .expect("the claim is voided: the job card can be");
    assert_eq!(voided.status, JobCardStatus::Voided);
}
