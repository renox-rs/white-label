//! BL-AUT-10 (#406): void an insurance claim registered in error (`PLT.ACL.09`,
//! 08 §7.1 *Insurance repair claim* `void`), through the kernel's void. The
//! host test with the real notify and documents is
//! `host/tests/auto_claim_void_real.rs`.

mod support;

use auto::claims;
use auto::documents::{GUARDED_JOB_CARD_APPROVE, PoliceVerdict, police_verdict};
use auto::job_cards::{self, NewLabour};
use auto::{ClaimStatus, InsuranceClaim, JobCardStatus, LabourLine, LineStatus, Payer};
use renox::prelude::*;
use support::claimfx::{carla, registered};
use support::stopfx::waiting;
use support::{Garage, boot};
use wl_kernel::Subject;

async fn claim_now(t: &Garage, id: i64) -> InsuranceClaim {
    InsuranceClaim::find_or_404(t.db(), id).await.unwrap()
}

/// CL-12 `registered`, JC-121 `inspecting` with two estimate lines, nothing
/// sent: A voids with the reason "duplicate". CL-12 is `voided`, JC-121 is
/// unlinked, `RecordVoided` is emitted, and both lines have payer = customer
/// and none is approved.
#[renox::test]
async fn aut_10_a_claim_registered_in_error_is_voided_and_its_job_card_unlinked() {
    let t = boot().await;
    let (card, _booking) = waiting(&t, 0, false).await;
    let tareq = t.technician("Tareq").await;
    let door = job_cards::add_labour(
        t.state(),
        &tareq,
        card.id,
        NewLabour::hourly("Door", 2_000, 20_000),
    )
    .await
    .unwrap();
    let paint = job_cards::add_labour(
        t.state(),
        &tareq,
        card.id,
        NewLabour::hourly("Paint", 1_000, 15_000),
    )
    .await
    .unwrap();
    let claim = registered(&t, card.id).await;
    // The lines were meant for the insurer.
    LabourLine::query()
        .where_(LabourLine::JOB_CARD_ID.eq(card.id))
        .update(t.db(), &[("payer", &Payer::Insurer)])
        .await
        .unwrap();
    let carla = carla(&t).await;

    let voided = claims::void(t.state(), &carla, claim.id, "duplicate")
        .await
        .unwrap();
    assert_eq!(voided.status, ClaimStatus::Voided);
    assert_eq!(voided.closed_reason.as_deref(), Some("duplicate"));
    assert_eq!(voided.job_card_id, None, "unlinked");
    assert_eq!(voided.unlinked_job_card_id, Some(card.id));
    assert!(!claims::funds_job_card(t.db(), card.id).await.unwrap());
    let events = t.voided.all();
    assert_eq!(events.len(), 1, "RecordVoided is emitted");
    assert_eq!(
        events[0].subject,
        Subject::new("auto.insurance_claim", claim.id)
    );
    assert_eq!(events[0].reason, "duplicate");

    for id in [door.id, paint.id] {
        let line = LabourLine::find_or_404(t.db(), id).await.unwrap();
        assert_eq!(line.payer, Payer::Customer, "{}", line.description);
        assert_ne!(line.approval, LineStatus::Approved);
    }
    // The job card carries on, with its accident flag.
    let card = t.reload(card.id).await;
    assert_eq!(card.status, JobCardStatus::Inspecting);
    assert!(card.accident_damage);
    // Voided, it is final.
    assert!(
        claims::void(t.state(), &carla, claim.id, "again")
            .await
            .is_err()
    );
}

/// CL-13 `registered` with a survey email sent: void is refused and names why;
/// once the survey is booked it is refused too.
#[renox::test]
async fn aut_10_a_claim_with_something_sent_to_the_insurer_cannot_be_voided() {
    let t = boot().await;
    let (card, _booking) = waiting(&t, 0, false).await;
    let claim = registered(&t, card.id).await;
    let carla = carla(&t).await;
    assert!(claim.sent_to_insurer_at.is_none());

    let sent = claims::send_survey_request(t.state(), &carla, claim.id)
        .await
        .unwrap();
    assert_eq!(sent.status, ClaimStatus::Registered);
    assert!(sent.sent_to_insurer_at.is_some());
    assert_eq!(t.messages.of_type("auto.claim_survey").len(), 1);

    let refused = claims::void(t.state(), &carla, claim.id, "duplicate")
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("already sent to the insurer"),
        "{refused}"
    );
    assert_eq!(
        claim_now(&t, claim.id).await.status,
        ClaimStatus::Registered
    );
    assert!(t.voided.all().is_empty());
    assert_eq!(claim_now(&t, claim.id).await.job_card_id, Some(card.id));

    // Withdrawing it is the way out.
    claims::cancel(t.state(), &carla, claim.id, "Registered twice", None)
        .await
        .unwrap();
    assert_eq!(claim_now(&t, claim.id).await.status, ClaimStatus::Cancelled);
}

/// A claim whose survey was booked or whose email could not be sent is not
/// voidable either; a void needs a reason and `insurance.manage`.
#[renox::test]
async fn aut_10_a_void_needs_a_reason_the_permission_and_a_registered_claim() {
    let t = boot().await;
    let (card, _booking) = waiting(&t, 0, false).await;
    let claim = registered(&t, card.id).await;
    let carla = carla(&t).await;

    let blank = claims::void(t.state(), &carla, claim.id, " ")
        .await
        .unwrap_err();
    assert!(
        blank.to_string().to_lowercase().contains("reason"),
        "{blank}"
    );
    let nadia = t.user("Nadia", &[auto::permissions::JOB_CARDS_VIEW]).await;
    let nadia = t.auth(&nadia).await;
    assert!(
        claims::void(t.state(), &nadia, claim.id, "duplicate")
            .await
            .is_err()
    );

    claims::book_survey(t.state(), &carla, claim.id)
        .await
        .unwrap();
    let late = claims::void(t.state(), &carla, claim.id, "duplicate")
        .await
        .unwrap_err();
    assert!(!late.to_string().is_empty());
    assert_eq!(
        claim_now(&t, claim.id).await.status,
        ClaimStatus::AwaitingSurvey
    );
}

/// The police report attached to the claim counted for its job card; after the
/// void it no longer does.
#[renox::test]
async fn aut_10_the_police_report_of_a_voided_claim_no_longer_counts_for_the_job_card() {
    let t = boot().await;
    t.documents.police_blocks();
    let (card, _booking) = waiting(&t, 0, false).await;
    let claim = registered(&t, card.id).await;
    t.documents.set(&claim.subject(), "police_report", None);
    let before = police_verdict(t.state(), GUARDED_JOB_CARD_APPROVE, card.id, None)
        .await
        .unwrap();
    assert_eq!(before, PoliceVerdict::Valid);

    let carla = carla(&t).await;
    claims::void(t.state(), &carla, claim.id, "duplicate")
        .await
        .unwrap();
    let after = police_verdict(t.state(), GUARDED_JOB_CARD_APPROVE, card.id, None)
        .await
        .unwrap();
    assert!(matches!(after, PoliceVerdict::Refused(_)), "{after:?}");
}

/// The claim page voids a registered claim and hides the action once anything
/// was sent.
#[renox::test]
async fn aut_10_the_claim_page_voids_only_before_anything_was_sent() {
    let t = boot().await;
    let (card, _booking) = waiting(&t, 0, false).await;
    let claim = registered(&t, card.id).await;
    let user = t
        .user(
            "Carla",
            &[
                auto::permissions::INSURANCE_MANAGE,
                auto::permissions::JOB_CARDS_VIEW,
                auto::permissions::VEHICLES_VIEW,
            ],
        )
        .await;
    t.app.acting_as(&user);
    let path = format!("/auto/claims/{}", claim.id);
    t.app.get(&path).await.assert_ok().assert_see("Void");
    t.app
        .htmx()
        .post(&format!("{path}/survey-request"), &[])
        .await
        .assert_ok();
    let refused = t
        .app
        .htmx()
        .post(&format!("{path}/void"), &[("reason", "duplicate")])
        .await;
    refused.assert_status(422);
    assert!(refused.text().contains("already sent"));
    assert_eq!(
        claim_now(&t, claim.id).await.status,
        ClaimStatus::Registered
    );
}
