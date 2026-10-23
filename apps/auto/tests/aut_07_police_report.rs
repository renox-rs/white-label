//! BL-AUT-07 (#403, `AUT.ACL.20`, D-08): accident damage is repaired only with
//! the police accident report attached and accepted. The job card `approve`
//! and the claim's `book_survey`, `submit_estimate` and `approve` ask the
//! document guard (a stand-in here, the real wl-documents in
//! `host/tests/auto_police_real.rs`); a report on the claim counts for the job
//! card and the other way round; clearing the accident flag needs the
//! workshop manager.

mod support;

use auto::accident_flag;
use auto::claims::{self, Answer};
use auto::job_cards::{self, Intake, NewJobCard, NewLabour, Promise};
use auto::permissions as aut;
use auto::{ClaimStatus, JobCard, JobCardStatus};
use renox::prelude::*;
use support::claimfx::{Estimate, carla, registered, sent_estimate};
use support::{Garage, boot};
use wl_kernel::Subject;

const KIND: &str = "police_report";

/// A job card flagged as accident damage (with the report number typed at
/// check-in) whose estimate is sent: the customer's answer is awaited.
async fn accident_card(t: &Garage, flagged: bool) -> JobCard {
    let vehicle = t.hilux().await;
    let sami = t.advisor().await;
    let tareq_user = t.technician_user("Tareq").await;
    let mut new = NewJobCard::new(vehicle.id, "Front hit on the highway")
        .odometer(52_300)
        .condition("Front bumper broken")
        .acknowledged_by("Mona Hassan")
        .technician(tareq_user.id)
        .workshop(Subject::new("scheduling.location", 1));
    if flagged {
        new = new.accident("DXB-PR-70");
    }
    let card = job_cards::check_in(t.state(), &sami, new).await.unwrap();
    let tareq = t.technician("Tareq").await;
    job_cards::start_inspection(t.state(), &tareq, card.id)
        .await
        .unwrap();
    job_cards::add_labour(
        t.state(),
        &tareq,
        card.id,
        NewLabour::hourly("Bumper repair", 2_000, 20_000),
    )
    .await
    .unwrap();
    let card = job_cards::send_estimate(t.state(), &sami, card.id, Promise::default())
        .await
        .unwrap();
    assert_eq!(card.status, JobCardStatus::AwaitingApproval);
    card
}

fn words(error: &Error) -> String {
    error.to_string()
}

/// The sent estimate of the claim fixtures, with the police report blocks on.
async fn claim_world(t: &Garage) -> (Estimate, i64) {
    t.documents.police_blocks();
    let e = sent_estimate(t).await;
    let claim = registered(t, e.card.id).await;
    (e, claim.id)
}

/// #403 point 1: JC-70 is flagged accident damage and has no report, so the
/// customer's approval is refused with "police report required", on the
/// advisor's page and on the customer's link, and the job card stays where it
/// is; once the report is accepted the same approval passes.
#[renox::test]
async fn aut_07_an_accident_job_card_is_not_approved_without_the_police_report() {
    let t = boot().await;
    t.documents.police_blocks();
    let card = accident_card(&t, true).await;
    let sami = t.advisor().await;
    assert!(card.accident_damage);

    let refusal = job_cards::approve_recorded(t.state(), &sami, card.id, "Mona", "recorded", None)
        .await
        .unwrap_err();
    assert!(
        words(&refusal).contains("Police report required"),
        "{refusal}"
    );
    assert_eq!(
        t.reload(card.id).await.status,
        JobCardStatus::AwaitingApproval
    );

    let round = job_cards::pending_round(t.db(), card.id)
        .await
        .unwrap()
        .expect("the estimate round");
    let by_link = job_cards::approve_by_link(t.state(), &round.token, "Mona", None, true)
        .await
        .unwrap_err();
    assert!(
        words(&by_link).contains("Police report required"),
        "{by_link}"
    );
    assert_eq!(
        t.reload(card.id).await.status,
        JobCardStatus::AwaitingApproval
    );

    t.documents.set(&card.subject(), KIND, None);
    let approved = job_cards::approve_recorded(t.state(), &sami, card.id, "Mona", "recorded", None)
        .await
        .unwrap();
    assert_eq!(approved.status, JobCardStatus::InProgress);
}

/// #403 point 6: JC-71 has no accident flag, so `approve` needs no report even
/// though the guard is on.
#[renox::test]
async fn aut_07_a_job_card_without_the_flag_needs_no_report() {
    let t = boot().await;
    t.documents.police_blocks();
    let card = accident_card(&t, false).await;
    assert!(!card.accident_damage);
    let sami = t.advisor().await;
    let approved = job_cards::approve_recorded(t.state(), &sami, card.id, "Mona", "recorded", None)
        .await
        .unwrap();
    assert_eq!(approved.status, JobCardStatus::InProgress);
}

/// #403 point 2: CL-7 is `registered` without a report, so `book_survey` and
/// `submit_estimate` are refused naming the report; with the report accepted
/// both go through.
#[renox::test]
async fn aut_07_book_survey_and_submit_estimate_need_the_report() {
    let t = boot().await;
    let (_e, claim_id) = claim_world(&t).await;
    let carla = carla(&t).await;

    let survey = claims::book_survey(t.state(), &carla, claim_id)
        .await
        .unwrap_err();
    assert!(
        words(&survey).contains("Police report required"),
        "{survey}"
    );
    let submit = claims::submit_estimate(t.state(), &carla, claim_id)
        .await
        .unwrap_err();
    assert!(
        words(&submit).contains("Police report required"),
        "{submit}"
    );
    let claim = auto::InsuranceClaim::find_or_404(t.db(), claim_id)
        .await
        .unwrap();
    assert_eq!(claim.status, ClaimStatus::Registered);

    t.documents.set(&claim.subject(), KIND, None);
    let booked = claims::book_survey(t.state(), &carla, claim_id)
        .await
        .unwrap();
    assert_eq!(booked.status, ClaimStatus::AwaitingSurvey);
    let submitted = claims::submit_estimate(t.state(), &carla, claim_id)
        .await
        .unwrap();
    assert_eq!(submitted.status, ClaimStatus::EstimateSubmitted);
}

/// #403 point 3: CL-7 is `estimate_submitted` and its report is removed
/// (archived), so the claim `approve` is refused and the job card is not
/// started; with a report again the approval goes through.
#[renox::test]
async fn aut_07_a_removed_report_refuses_the_claim_approval_again() {
    let t = boot().await;
    let (e, claim_id) = claim_world(&t).await;
    let carla = carla(&t).await;
    let claim = auto::InsuranceClaim::find_or_404(t.db(), claim_id)
        .await
        .unwrap();
    t.documents.set(&claim.subject(), KIND, None);
    claims::submit_estimate(t.state(), &carla, claim_id)
        .await
        .unwrap();

    t.documents.remove(&claim.subject(), KIND);
    let answer = || Answer::new("LPO-5521", "orient-approval-1").labour(e.door.id, 40_000);
    let refusal = claims::approve(t.state(), &carla, claim_id, answer())
        .await
        .unwrap_err();
    assert!(
        words(&refusal).contains("Police report required"),
        "{refusal}"
    );
    let still = auto::InsuranceClaim::find_or_404(t.db(), claim_id)
        .await
        .unwrap();
    assert_eq!(still.status, ClaimStatus::EstimateSubmitted);
    assert_eq!(
        t.reload(e.card.id).await.status,
        JobCardStatus::AwaitingApproval
    );

    t.documents.set(&claim.subject(), KIND, None);
    let approved = claims::approve(t.state(), &carla, claim_id, answer())
        .await
        .unwrap();
    assert_eq!(approved.status, ClaimStatus::Approved);
    assert_eq!(t.reload(e.card.id).await.status, JobCardStatus::InProgress);
}

/// #403 point 4: a report accepted on CL-7 counts for JC-70's `approve` (a
/// private repair after a rejected claim), and a report on the job card counts
/// for its claim.
#[renox::test]
async fn aut_07_a_report_on_the_claim_counts_for_the_job_card_and_the_other_way_round() {
    let t = boot().await;
    let (e, claim_id) = claim_world(&t).await;
    let carla = carla(&t).await;
    let sami = t.advisor().await;
    let claim = auto::InsuranceClaim::find_or_404(t.db(), claim_id)
        .await
        .unwrap();

    // The job card's report serves the claim.
    t.documents.set(&e.card.subject(), KIND, None);
    claims::book_survey(t.state(), &carla, claim_id)
        .await
        .unwrap();
    claims::submit_estimate(t.state(), &carla, claim_id)
        .await
        .unwrap();
    t.documents.remove(&e.card.subject(), KIND);

    // The claim is rejected; the private repair still needs a report.
    t.documents.set(&claim.subject(), KIND, None);
    claims::reject(t.state(), &carla, claim_id, "Not covered")
        .await
        .unwrap();
    t.documents.remove(&claim.subject(), KIND);
    let refusal =
        job_cards::approve_recorded(t.state(), &sami, e.card.id, "Mona", "recorded", None)
            .await
            .unwrap_err();
    assert!(
        words(&refusal).contains("Police report required"),
        "{refusal}"
    );

    // A report accepted on the claim lets the job card start.
    t.documents.set(&claim.subject(), KIND, None);
    let approved =
        job_cards::approve_recorded(t.state(), &sami, e.card.id, "Mona", "recorded", None)
            .await
            .unwrap();
    assert_eq!(approved.status, JobCardStatus::InProgress);
}

/// #403 point 5 (the kind): the seeded entries are a legal block, which the
/// platform locks, with override mode never; the stand-in carries the same.
#[renox::test]
async fn aut_07_no_override_is_offered_for_the_police_report() {
    use wl_kernel::documents::{DocumentGuard, OverrideMode};
    let t = boot().await;
    t.documents.police_blocks();
    for guarded in [
        "job_card.approve",
        "insurance_claim.book_survey",
        "insurance_claim.submit_estimate",
        "insurance_claim.approve",
    ] {
        let entries = t.documents.blocks_for(t.db(), guarded).await.unwrap();
        assert_eq!(entries.len(), 1, "{guarded}");
        assert_eq!(entries[0].override_mode, OverrideMode::Never, "{guarded}");
        assert!(entries[0].locked, "{guarded}");
    }
    // A refusal never offers an override link.
    let card = accident_card(&t, true).await;
    let refusal = wl_kernel::documents::guard(t.state())
        .check_as(
            t.state(),
            "job_card.approve",
            card.id,
            wl_kernel::documents::today(),
            None,
        )
        .await
        .unwrap()
        .unwrap_err();
    assert!(refusal.override_links().is_empty());
}

/// #403 point 7: JC-70 is linked to CL-7, so clearing its accident flag is
/// refused; on JC-72 (no claim) the advisor asks with a reason, the one who
/// asked cannot decide it, the workshop manager approves and the flag is
/// cleared; the intake form cannot take the flag off either.
#[renox::test]
async fn aut_07_clearing_the_accident_flag_needs_the_workshop_manager_and_no_claim() {
    let t = boot().await;
    let (e, _claim_id) = claim_world(&t).await;
    let sami = t.advisor().await;
    let linked = t.reload(e.card.id).await;
    assert!(linked.accident_damage);
    let refusal = accident_flag::request_clear(t.state(), &sami, linked.id, "Marked by mistake")
        .await
        .unwrap_err();
    assert!(words(&refusal).contains("insurance claim"), "{refusal}");
    assert!(t.reload(linked.id).await.accident_damage);

    // JC-72: no claim.
    let jc72 = accident_card(&t, true).await;
    assert!(
        accident_flag::request_clear(t.state(), &sami, jc72.id, "  ")
            .await
            .is_err(),
        "a reason is needed"
    );
    let intake = Intake {
        complaint: "Front hit".to_owned(),
        odometer_in: 52_300,
        fuel_level: None,
        condition: "Front bumper broken".to_owned(),
        acknowledged_by: None,
        signature: None,
        accident_damage: false,
        police_report_no: None,
        inspection_fee: 0,
        storage_fee_daily: 0,
    };
    let on_intake = job_cards::record_intake(t.state(), &sami, jc72.id, intake)
        .await
        .unwrap_err();
    assert!(
        words(&on_intake).contains("workshop manager"),
        "{on_intake}"
    );

    let asked = accident_flag::request_clear(t.state(), &sami, jc72.id, "Old damage, no accident")
        .await
        .unwrap();
    assert!(asked.accident_damage && asked.accident_clear_at.is_some());
    assert!(
        accident_flag::request_clear(t.state(), &sami, jc72.id, "Again")
            .await
            .is_err()
    );

    // The advisor cannot decide; neither can someone who asked.
    assert!(matches!(
        accident_flag::decide_clear(t.state(), &sami, jc72.id, true)
            .await
            .unwrap_err(),
        Error::Forbidden
    ));
    let both = t
        .user("Both", &[aut::JOB_CARDS_ADVISE, aut::JOB_CARDS_ACCIDENT])
        .await;
    let both = t.auth(&both).await;
    let free = linked_free(&t).await;
    accident_flag::request_clear(t.state(), &both, free, "Wrong flag")
        .await
        .unwrap();
    let self_decision = accident_flag::decide_clear(t.state(), &both, free, true)
        .await
        .unwrap_err();
    assert!(words(&self_decision).contains("asked"), "{self_decision}");

    let wanda = t
        .user("Wanda", &[aut::JOB_CARDS_VIEW, aut::JOB_CARDS_ACCIDENT])
        .await;
    let wanda = t.auth(&wanda).await;
    let cleared = accident_flag::decide_clear(t.state(), &wanda, jc72.id, true)
        .await
        .unwrap();
    assert!(!cleared.accident_damage);
    assert!(cleared.accident_clear_at.is_none());
}

/// A flagged job card without a claim that the "asked it themselves" case uses.
async fn linked_free(t: &Garage) -> i64 {
    let vehicle = t.hilux().await;
    let sami = t.advisor().await;
    let card = job_cards::check_in(
        t.state(),
        &sami,
        NewJobCard::new(vehicle.id, "Side scrape")
            .odometer(52_500)
            .condition("Scrape on the right door")
            .accident("DXB-PR-72"),
    )
    .await
    .unwrap();
    card.id
}

/// The manager refuses: the flag stays and the request is gone.
#[renox::test]
async fn aut_07_a_refused_request_keeps_the_flag() {
    let t = boot().await;
    let id = linked_free(&t).await;
    let sami = t.advisor().await;
    accident_flag::request_clear(t.state(), &sami, id, "Not an accident")
        .await
        .unwrap();
    let wanda = t
        .user("Wanda", &[aut::JOB_CARDS_VIEW, aut::JOB_CARDS_ACCIDENT])
        .await;
    let wanda = t.auth(&wanda).await;
    let kept = accident_flag::decide_clear(t.state(), &wanda, id, false)
        .await
        .unwrap();
    assert!(kept.accident_damage);
    assert!(kept.accident_clear_at.is_none() && kept.accident_clear_reason.is_none());
    let none = accident_flag::decide_clear(t.state(), &wanda, id, true)
        .await
        .unwrap_err();
    assert!(words(&none).contains("No request"), "{none}");
}
