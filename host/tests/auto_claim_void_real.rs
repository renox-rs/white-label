//! BL-AUT-10 (#406) against the REAL shared modules of the host (wl-notify,
//! wl-documents, wl-scheduling, wl-parties): a claim registered in error is
//! voided while nothing was sent to the insurer, its job card is unlinked and
//! the police report attached to the claim no longer counts for the job card;
//! once a real message was queued to the insurer the void is refused. The
//! app's own tests (`apps/auto/tests/aut_10_void_claim.rs`) show the same
//! points on stand-ins.

mod support;

use auto::claims::{self, NewClaim};
use auto::documents::{GUARDED_JOB_CARD_APPROVE, PoliceVerdict, police_verdict};
use auto::job_cards::{self, NewJobCard};
use auto::vehicles::{self, NewVehicle};
use auto::{ClaimStatus, Emirate, InsuranceClaim};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use support::autofx::{Shop, World, boot, police_report, sent_of, shop};
use wl_kernel::Subject;
use wl_kernel::documents::today;

/// A registered claim for a job card in `inspecting`.
async fn registered(w: &World, s: &Shop) -> (InsuranceClaim, i64) {
    let vehicle = vehicles::create(
        w.state(),
        &s.sami,
        NewVehicle::new(Emirate::Dubai, "A", "12345", "Toyota", "Hilux", s.owner.0)
            .odometer(52_000),
    )
    .await
    .unwrap();
    let card = job_cards::check_in(
        w.state(),
        &s.sami,
        NewJobCard::new(vehicle.id, "Right side hit")
            .odometer(52_400)
            .condition("Right door dented")
            .acknowledged_by("Mona Hassan")
            .technician(s.tareq_user.id)
            .workshop(Subject::new("scheduling.location", 1)),
    )
    .await
    .unwrap();
    let claim = claims::register(
        w.state(),
        &s.carla,
        NewClaim::new(
            s.insurer.id,
            vehicle.id,
            "POL-7788",
            "OR-2026-118",
            today() - TimeDelta::days(3),
        )
        .job_card(card.id)
        .police_report("DXB-PR-5521", Some(today() - TimeDelta::days(2))),
    )
    .await
    .unwrap();
    (claim, card.id)
}

/// Nothing sent: voided, `RecordVoided` and the Void record keep the number,
/// the job card is unlinked and the claim's police report no longer counts for
/// it.
#[renox::test]
async fn aut_10_real_a_claim_with_nothing_sent_is_voided_and_its_report_stops_counting() {
    let w = boot().await;
    let s = shop(&w, None).await;
    let (claim, card_id) = registered(&w, &s).await;
    police_report(&w, &s, &claim).await;
    let before = police_verdict(w.state(), GUARDED_JOB_CARD_APPROVE, card_id, None)
        .await
        .unwrap();
    assert_eq!(before, PoliceVerdict::Valid, "the claim's report counts");

    let voided = claims::void(w.state(), &s.carla, claim.id, "duplicate")
        .await
        .unwrap();
    assert_eq!(voided.status, ClaimStatus::Voided);
    assert_eq!(voided.job_card_id, None);
    assert_eq!(voided.unlinked_job_card_id, Some(card_id));
    let record = wl_kernel::voids::VoidRecord::of_subject(w.db(), &voided.subject())
        .await
        .unwrap()
        .expect("the Void record");
    assert_eq!(record.reason, "duplicate");
    assert_eq!(record.number.as_deref(), Some(claim.number().as_str()));
    let after = police_verdict(w.state(), GUARDED_JOB_CARD_APPROVE, card_id, None)
        .await
        .unwrap();
    assert!(matches!(after, PoliceVerdict::Refused(_)), "{after:?}");
}

/// A real survey email queued: the void is refused; withdrawing is the way out.
#[renox::test]
async fn aut_10_real_a_claim_with_a_message_to_the_insurer_cannot_be_voided() {
    let w = boot().await;
    let s = shop(&w, None).await;
    let (claim, _card_id) = registered(&w, &s).await;
    claims::send_survey_request(w.state(), &s.carla, claim.id)
        .await
        .unwrap();
    let mails = sent_of(&w, "auto.insurer_request").await;
    assert_eq!(mails.len(), 1);
    assert_eq!(mails[0].address.as_deref(), Some("claims@orient.example"));

    let refused = claims::void(w.state(), &s.carla, claim.id, "duplicate")
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("already sent"), "{refused}");
    assert_eq!(
        InsuranceClaim::find_or_404(w.db(), claim.id)
            .await
            .unwrap()
            .status,
        ClaimStatus::Registered
    );
}
