//! The base of the 08 Auto insurance repair claim (base for #404, #405, #406,
//! #413; `AUT.INS.01` .. `AUT.INS.04`, `AUT.ACL.15` .. `AUT.ACL.16`): the
//! claim is registered with the insurer, the policy and claim number and the
//! police report, the estimate sent on the job card goes to the insurer, the
//! insurer approves line by line (the job card starts with the approved lines
//! only), hidden damage goes back as a supplementary, the repair completes
//! the claim, and the bill is split between the insurer's payer bill and the
//! customer's invoice. The guard slots the negative flows fill pass for now.

mod support;

use auto::claim_machine::{APPROVE, BOOK_SURVEY, CANCEL, DECLARE_TOTAL_LOSS, INVOICE, VOID};
use auto::claims::{self, Answer, NewClaim};
use auto::insurers::{self, NewInsurer};
use auto::job_cards::{self, Delivery, NewJobCard, NewLabour, Promise};
use auto::{
    ApprovalKind, ApprovalOutcome, ClaimApproval, ClaimStatus, InsuranceClaim, JobCard,
    JobCardStatus, LabourLine, LineStatus, PartLine, WorkStatus,
};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use support::claimfx::{Estimate, aisha, carla, orient, registered, sent_estimate};
use support::{Garage, boot, day};
use wl_kernel::PartyId;
use wl_kernel::payer_bills::PayerBillStatus;

async fn claim_now(t: &Garage, id: i64) -> InsuranceClaim {
    InsuranceClaim::find_or_404(t.db(), id).await.unwrap()
}

/// The insurer approves the door at AED 400 and the bumper at AED 550.
fn approval() -> impl Fn(&Estimate) -> Answer {
    |e: &Estimate| {
        Answer::new("LPO-5521", "orient-approval-email-1")
            .labour(e.door.id, 40_000)
            .part(e.bumper.id, 55_000)
    }
}

/// The claim is registered with the insurer, the policy and claim number, the
/// accident, the police report, the repair type and the excess; the rate card
/// in force is pinned and the job card is flagged as an accident repair.
#[renox::test]
async fn aut_base_a_claim_is_registered_with_the_insurer_policy_and_police_report() {
    let t = boot().await;
    let e = sent_estimate(&t).await;
    let claim = registered(&t, e.card.id).await;

    assert_eq!(claim.status, ClaimStatus::Registered);
    assert_eq!(claim.number(), format!("CLM-{:05}", claim.id));
    assert_eq!(claim.claim_no, "OR-2026-118");
    assert_eq!(claim.policy_no, "POL-7788");
    assert_eq!(claim.police_report_no.as_deref(), Some("DXB-PR-5521"));
    assert_eq!(claim.excess, 20_000);
    assert_eq!(claim.customer_party_id, 7, "the owner of the vehicle");
    assert_eq!(claim.job_card_id, Some(e.card.id));
    assert_eq!(
        (claim.rate_card_id, claim.rate_card_version),
        (Some(31), Some(2)),
        "the card in force is pinned"
    );
    let card = JobCard::find_or_404(t.db(), e.card.id).await.unwrap();
    assert!(card.accident_damage);
    assert_eq!(card.police_report_no.as_deref(), Some("DXB-PR-5521"));
    assert!(claims::funds_job_card(t.db(), e.card.id).await.unwrap());
    let history = renox::state_machine::history(t.db(), InsuranceClaim::TABLE, claim.id)
        .await
        .unwrap();
    assert_eq!(history.len(), 1, "the registration is the first change");
}

/// A claim needs a known and active insurer, a vehicle, the numbers, a real
/// accident date, a job card of the same vehicle that no other claim funds,
/// and a coordinator.
#[renox::test]
async fn aut_base_a_claim_is_refused_without_what_it_needs() {
    let t = boot().await;
    let e = sent_estimate(&t).await;
    let first = registered(&t, e.card.id).await;
    let carla = carla(&t).await;
    let vehicle = t.hilux().await;
    let base = || NewClaim::new(first.insurer_id, vehicle.id, "POL-1", "OR-2", day(-3));

    let again = claims::register(
        t.state(),
        &carla,
        NewClaim::new(
            first.insurer_id,
            vehicle.id,
            "POL-1",
            "OR-2026-118",
            day(-3),
        ),
    )
    .await
    .unwrap_err();
    assert!(
        again.to_string().contains("claim with that number"),
        "{again}"
    );
    let future = claims::register(
        t.state(),
        &carla,
        NewClaim::new(first.insurer_id, vehicle.id, "POL-1", "OR-3", day(2)),
    )
    .await
    .unwrap_err();
    assert!(future.to_string().contains("accident"), "{future}");
    let report_before = claims::register(
        t.state(),
        &carla,
        base().police_report("DXB-1", Some(day(-9))),
    )
    .await
    .unwrap_err();
    assert!(
        report_before.to_string().contains("police report"),
        "{report_before}"
    );
    let negative = claims::register(t.state(), &carla, base().excess(-1))
        .await
        .unwrap_err();
    assert!(negative.to_string().contains("excess"), "{negative}");
    let blank = claims::register(
        t.state(),
        &carla,
        NewClaim::new(first.insurer_id, vehicle.id, " ", "OR-4", day(-3)),
    )
    .await
    .unwrap_err();
    assert!(blank.to_string().contains("policy number"), "{blank}");
    let taken = claims::register(t.state(), &carla, base().job_card(e.card.id))
        .await
        .unwrap_err();
    assert!(taken.to_string().contains("funds that job card"), "{taken}");
    let unknown = claims::register(
        t.state(),
        &carla,
        NewClaim::new(999, vehicle.id, "POL-1", "OR-5", day(-3)),
    )
    .await
    .unwrap_err();
    assert!(unknown.to_string().contains("insurer"), "{unknown}");
    let sami = t.advisor().await;
    let advisor = claims::register(t.state(), &sami, base())
        .await
        .unwrap_err();
    assert!(matches!(advisor, Error::Forbidden));

    insurers::set_active(t.state(), &carla, first.insurer_id, false)
        .await
        .unwrap();
    let stopped = claims::register(t.state(), &carla, base())
        .await
        .unwrap_err();
    assert!(stopped.to_string().contains("no new claims"), "{stopped}");
}

/// The insurer is a party recorded once, with sane terms; only the
/// coordinator records it.
#[renox::test]
async fn aut_base_an_insurer_is_recorded_once_with_its_terms() {
    let t = boot().await;
    let insurer = orient(&t).await;
    assert_eq!(insurer.payer_agreement_id, Some(3));
    assert_eq!(insurer.payment_terms_days, 30);
    assert_eq!(insurer.labour_rate, Some(12_000));
    assert_eq!(
        insurer.subject().to_string(),
        format!("auto.insurer:{}", insurer.id)
    );
    let carla = carla(&t).await;
    let twice = insurers::register(t.state(), &carla, NewInsurer::new(9))
        .await
        .unwrap_err();
    assert!(twice.to_string().contains("insurer already"), "{twice}");
    t.people.add(10, "Gulf Cover");
    let bad_email = insurers::register(t.state(), &carla, NewInsurer::new(10).email("nope"))
        .await
        .unwrap_err();
    assert!(bad_email.to_string().contains("address"), "{bad_email}");
    let sami = t.advisor().await;
    assert!(matches!(
        insurers::register(t.state(), &sami, NewInsurer::new(10)).await,
        Err(Error::Forbidden)
    ));
    let card = insurers::rate_card_of(t.state(), &insurer, day(0))
        .await
        .unwrap()
        .expect("a version is in force");
    assert_eq!((card.rate_card, card.version), (31, 2));
    assert!(
        insurers::rate_card_of(t.state(), &insurer, day(-60))
            .await
            .unwrap()
            .is_none(),
        "before its effective date there is none"
    );
}

/// The survey is booked and the estimate that was sent on the job card goes to
/// the insurer as a request waiting for its answer; nothing goes before the
/// estimate is sent.
#[renox::test]
async fn aut_base_the_survey_is_booked_and_the_estimate_submitted() {
    let t = boot().await;
    let e = sent_estimate(&t).await;
    let claim = registered(&t, e.card.id).await;
    let carla = carla(&t).await;

    let surveyed = claims::book_survey(t.state(), &carla, claim.id)
        .await
        .unwrap();
    assert_eq!(surveyed.status, ClaimStatus::AwaitingSurvey);
    assert!(surveyed.survey_booked_at.is_some());
    assert!(surveyed.sent_to_insurer_at.is_some());

    let submitted = claims::submit_estimate(t.state(), &carla, claim.id)
        .await
        .unwrap();
    assert_eq!(submitted.status, ClaimStatus::EstimateSubmitted);
    assert_eq!(submitted.estimate_amount, 110_000);
    let requests = ClaimApproval::of_claim(t.db(), claim.id).await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].kind, ApprovalKind::Estimate);
    assert_eq!(requests[0].outcome, ApprovalOutcome::Pending);
    assert_eq!(requests[0].estimate_amount, 110_000);
    assert!(requests[0].job_round_id.is_some());
}

/// A claim whose job card has no estimate waiting cannot submit one.
#[renox::test]
async fn aut_base_the_estimate_must_be_sent_on_the_job_card_first() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let sami = t.advisor().await;
    let card = job_cards::check_in(
        t.state(),
        &sami,
        NewJobCard::new(vehicle.id, "Hit")
            .odometer(52_400)
            .condition("Dented"),
    )
    .await
    .unwrap();
    let claim = registered(&t, card.id).await;
    let carla = carla(&t).await;
    let refused = claims::submit_estimate(t.state(), &carla, claim.id)
        .await
        .unwrap_err();
    assert!(
        refused
            .to_string()
            .contains("Send the estimate on the job card"),
        "{refused}"
    );
    let state = claim_now(&t, claim.id).await.status;
    assert_eq!(state, ClaimStatus::Registered);
}

/// The insurer's approval needs its reference, its written approval and an
/// amount for at least one line of the estimate, none above its price.
#[renox::test]
async fn aut_base_an_approval_needs_the_reference_the_document_and_the_lines() {
    let t = boot().await;
    let e = sent_estimate(&t).await;
    let claim = registered(&t, e.card.id).await;
    let carla = carla(&t).await;
    claims::submit_estimate(t.state(), &carla, claim.id)
        .await
        .unwrap();

    let no_reference = claims::approve(
        t.state(),
        &carla,
        claim.id,
        Answer::new(" ", "email-1").labour(e.door.id, 40_000),
    )
    .await
    .unwrap_err();
    assert!(
        no_reference.to_string().contains("reference"),
        "{no_reference}"
    );
    let no_document = claims::approve(
        t.state(),
        &carla,
        claim.id,
        Answer::new("LPO-1", "").labour(e.door.id, 40_000),
    )
    .await
    .unwrap_err();
    assert!(
        no_document.to_string().contains("written approval"),
        "{no_document}"
    );
    let no_lines = claims::approve(t.state(), &carla, claim.id, Answer::new("LPO-1", "email-1"))
        .await
        .unwrap_err();
    assert!(
        no_lines.to_string().contains("at least one line"),
        "{no_lines}"
    );
    let too_much = claims::approve(
        t.state(),
        &carla,
        claim.id,
        Answer::new("LPO-1", "email-1").labour(e.door.id, 40_001),
    )
    .await
    .unwrap_err();
    assert!(
        too_much.to_string().contains("more than its price"),
        "{too_much}"
    );
    let stranger = claims::approve(
        t.state(),
        &carla,
        claim.id,
        Answer::new("LPO-1", "email-1").labour(99_999, 100),
    )
    .await
    .unwrap_err();
    assert!(
        stranger.to_string().contains("not one the estimate sent"),
        "{stranger}"
    );
    assert_eq!(
        claim_now(&t, claim.id).await.status,
        ClaimStatus::EstimateSubmitted
    );
    let guards = InsuranceClaim::machine()
        .find(APPROVE)
        .unwrap()
        .guard_names()
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert!(guards.contains(&"police_report".to_owned()));
}

/// The insurer approves the door and the bumper: the job card starts with the
/// approved lines only, the alignment is declined, the excess is set and the
/// approved amount per line is kept.
#[renox::test]
async fn aut_base_the_job_card_starts_with_the_lines_the_insurer_approved() {
    let t = boot().await;
    let e = sent_estimate(&t).await;
    let claim = registered(&t, e.card.id).await;
    let carla = carla(&t).await;
    claims::submit_estimate(t.state(), &carla, claim.id)
        .await
        .unwrap();

    let approved = claims::approve(t.state(), &carla, claim.id, approval()(&e).excess(25_000))
        .await
        .unwrap();
    assert_eq!(approved.status, ClaimStatus::Approved);
    assert_eq!(approved.approved_amount, 95_000);
    assert_eq!(approved.excess, 25_000, "set with the approval");
    assert!(approved.approved_at.is_some());

    let request = ClaimApproval::of_claim(t.db(), claim.id)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(request.outcome, ApprovalOutcome::Approved);
    assert_eq!(request.reference.as_deref(), Some("LPO-5521"));
    assert_eq!(request.document.as_deref(), Some("orient-approval-email-1"));
    assert_eq!(request.approved_amount, 95_000);
    let described: Vec<(&str, i64)> = request
        .lines
        .0
        .iter()
        .map(|l| (l.description.as_str(), l.amount))
        .collect();
    assert_eq!(
        described,
        [("Door panel repair", 40_000), ("Bumper cover", 55_000)]
    );

    let card = JobCard::find_or_404(t.db(), e.card.id).await.unwrap();
    assert_eq!(card.status, JobCardStatus::InProgress);
    assert_eq!(
        LabourLine::find_or_404(t.db(), e.door.id)
            .await
            .unwrap()
            .approval,
        LineStatus::Approved
    );
    assert_eq!(
        PartLine::find_or_404(t.db(), e.bumper.id)
            .await
            .unwrap()
            .approval,
        LineStatus::Approved
    );
    assert_eq!(
        LabourLine::find_or_404(t.db(), e.alignment.id)
            .await
            .unwrap()
            .approval,
        LineStatus::Declined,
        "work is limited to the approved lines (AUT.ACL.15)"
    );
}

/// The insurer rejects the claim with its reason; the job card stays waiting
/// for an answer, so the customer can approve the estimate privately.
#[renox::test]
async fn aut_base_a_rejected_claim_leaves_the_job_for_a_private_estimate() {
    let t = boot().await;
    let e = sent_estimate(&t).await;
    let claim = registered(&t, e.card.id).await;
    let carla = carla(&t).await;
    claims::submit_estimate(t.state(), &carla, claim.id)
        .await
        .unwrap();

    let blank = claims::reject(t.state(), &carla, claim.id, " ")
        .await
        .unwrap_err();
    assert!(
        blank.to_string().to_lowercase().contains("reason"),
        "{blank}"
    );
    let rejected = claims::reject(t.state(), &carla, claim.id, "Policy lapsed")
        .await
        .unwrap();
    assert_eq!(rejected.status, ClaimStatus::Rejected);
    assert_eq!(rejected.closed_reason.as_deref(), Some("Policy lapsed"));
    let request = ClaimApproval::of_claim(t.db(), claim.id)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(request.outcome, ApprovalOutcome::Declined);
    assert_eq!(request.reason.as_deref(), Some("Policy lapsed"));
    assert!(
        !claims::funds_job_card(t.db(), e.card.id).await.unwrap(),
        "a rejected claim funds nothing"
    );
    let card = JobCard::find_or_404(t.db(), e.card.id).await.unwrap();
    assert_eq!(card.status, JobCardStatus::AwaitingApproval);
    let sami = t.advisor().await;
    let private =
        job_cards::approve_recorded(t.state(), &sami, card.id, "Mona Hassan", "recorded", None)
            .await
            .unwrap();
    assert_eq!(private.status, JobCardStatus::InProgress);
}

/// Hidden damage found during the repair goes back to the insurer as a
/// supplementary: approving it raises the approved amount and releases the
/// lines; declining it declines the added lines on the job card.
#[renox::test]
async fn aut_base_a_supplementary_is_approved_or_declined_by_the_insurer() {
    let t = boot().await;
    let e = sent_estimate(&t).await;
    let claim = registered(&t, e.card.id).await;
    let carla = carla(&t).await;
    claims::submit_estimate(t.state(), &carla, claim.id)
        .await
        .unwrap();
    claims::approve(t.state(), &carla, claim.id, approval()(&e))
        .await
        .unwrap();
    let sami = t.advisor().await;
    let tareq = t.technician("Tareq").await;

    // Hidden damage: the added work is sent on the job card.
    let frame = job_cards::add_labour(
        t.state(),
        &tareq,
        e.card.id,
        NewLabour::hourly("Straighten the frame rail", 3_000, 20_000),
    )
    .await
    .unwrap();
    job_cards::send_estimate(t.state(), &sami, e.card.id, Promise::default())
        .await
        .unwrap();
    let none = claims::request_supplementary(t.state(), &sami, claim.id, " ")
        .await
        .unwrap_err();
    assert!(none.to_string().contains("what was found"), "{none}");
    let pending = claims::request_supplementary(
        t.state(),
        &sami,
        claim.id,
        "Frame rail bent behind the bumper",
    )
    .await
    .unwrap();
    assert_eq!(pending.status, ClaimStatus::SupplementaryPending);
    assert!(pending.supplementary_at.is_some());
    let requests = ClaimApproval::of_claim(t.db(), claim.id).await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].kind, ApprovalKind::Supplementary);
    assert_eq!(requests[1].estimate_amount, 60_000);
    assert_eq!(
        requests[1].findings.as_deref(),
        Some("Frame rail bent behind the bumper")
    );

    let approved = claims::approve_supplementary(
        t.state(),
        &carla,
        claim.id,
        Answer::new("LPO-5521-S1", "orient-supplementary-email").labour(frame.id, 60_000),
    )
    .await
    .unwrap();
    assert_eq!(approved.status, ClaimStatus::Approved);
    assert_eq!(approved.approved_amount, 155_000, "95,000 and 60,000");
    assert!(approved.supplementary_at.is_none());
    let card = JobCard::find_or_404(t.db(), e.card.id).await.unwrap();
    assert_eq!(card.status, JobCardStatus::InProgress);
    assert_eq!(
        LabourLine::find_or_404(t.db(), frame.id)
            .await
            .unwrap()
            .approval,
        LineStatus::Approved
    );

    // A second one, which the insurer declines.
    let paint = job_cards::add_labour(
        t.state(),
        &tareq,
        e.card.id,
        NewLabour::hourly("Respray the door", 2_000, 15_000),
    )
    .await
    .unwrap();
    job_cards::send_estimate(t.state(), &sami, e.card.id, Promise::default())
        .await
        .unwrap();
    claims::request_supplementary(t.state(), &sami, claim.id, "Paint damage found")
        .await
        .unwrap();
    let no_reason = claims::decline_supplementary(t.state(), &carla, claim.id, "")
        .await
        .unwrap_err();
    assert!(
        no_reason.to_string().to_lowercase().contains("reason"),
        "{no_reason}"
    );
    let declined =
        claims::decline_supplementary(t.state(), &carla, claim.id, "Cosmetic, not covered")
            .await
            .unwrap();
    assert_eq!(declined.status, ClaimStatus::Approved);
    assert_eq!(declined.approved_amount, 155_000, "unchanged");
    assert_eq!(
        LabourLine::find_or_404(t.db(), paint.id)
            .await
            .unwrap()
            .approval,
        LineStatus::Declined
    );
    let card = JobCard::find_or_404(t.db(), e.card.id).await.unwrap();
    assert_eq!(card.status, JobCardStatus::InProgress, "the job goes on");
    let last = ClaimApproval::of_claim(t.db(), claim.id).await.unwrap();
    assert_eq!(last[2].outcome, ApprovalOutcome::Declined);
    assert_eq!(last[2].reason.as_deref(), Some("Cosmetic, not covered"));
}

/// Does the work the insurer approved, passes the QC: the claim completes by
/// itself; the job card is not billed on delivery.
async fn repaired(t: &Garage, e: &Estimate) {
    let tareq = t.technician("Tareq").await;
    let waleed = t.qc().await;
    let approved = LabourLine::query()
        .where_(LabourLine::JOB_CARD_ID.eq(e.card.id))
        .where_(LabourLine::APPROVAL.eq(LineStatus::Approved))
        .get(t.db())
        .await
        .unwrap();
    for line in approved {
        job_cards::record_work(t.state(), &tareq, line.id, WorkStatus::Done, None)
            .await
            .unwrap();
    }
    job_cards::finish_work(t.state(), &tareq, e.card.id)
        .await
        .unwrap();
    let all: Vec<String> = auto::job_card::QC_KEYS
        .iter()
        .map(|k| (*k).to_owned())
        .collect();
    job_cards::save_qc(t.state(), &waleed, e.card.id, &all)
        .await
        .unwrap();
    job_cards::pass_qc(t.state(), &waleed, e.card.id)
        .await
        .unwrap();
}

/// The repair passes QC: the claim is complete; the vehicle is collected and
/// the job card makes no invoice of its own, the claim bills it.
#[renox::test]
async fn aut_base_the_repair_completes_the_claim_and_the_job_card_is_not_billed() {
    let t = boot().await;
    let e = sent_estimate(&t).await;
    let claim = registered(&t, e.card.id).await;
    let carla = carla(&t).await;
    claims::submit_estimate(t.state(), &carla, claim.id)
        .await
        .unwrap();
    claims::approve(t.state(), &carla, claim.id, approval()(&e))
        .await
        .unwrap();

    repaired(&t, &e).await;
    let done = claim_now(&t, claim.id).await;
    assert_eq!(done.status, ClaimStatus::RepairComplete);
    assert!(done.completed_at.is_some());

    let sami = t.advisor().await;
    let delivered = job_cards::deliver(
        t.state(),
        &sami,
        e.card.id,
        Delivery {
            delivered_to: "Mona Hassan".to_owned(),
            ..Delivery::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(delivered.status, JobCardStatus::Delivered);
    assert!(delivered.invoice_id.is_none());
    assert!(
        t.books.about(&e.card.subject()).is_empty(),
        "the claim bills the job, not the job card"
    );
}

/// The split billing: the insurer gets a payer bill for the approved amount
/// less the excess; the customer is invoiced the excess and the private item
/// they approved themselves; the claim settles when both are paid.
#[renox::test]
async fn aut_base_the_bill_is_split_between_the_insurer_and_the_customer() {
    let t = boot().await;
    let e = sent_estimate(&t).await;
    let claim = registered(&t, e.card.id).await;
    let carla = carla(&t).await;
    claims::submit_estimate(t.state(), &carla, claim.id)
        .await
        .unwrap();
    claims::approve(t.state(), &carla, claim.id, approval()(&e))
        .await
        .unwrap();

    // The customer asks for the alignment after all, as a private item.
    let sami = t.advisor().await;
    let tareq = t.technician("Tareq").await;
    let private = job_cards::add_labour(
        t.state(),
        &tareq,
        e.card.id,
        NewLabour::hourly("Wheel alignment", 1_000, 10_000),
    )
    .await
    .unwrap();
    job_cards::send_estimate(t.state(), &sami, e.card.id, Promise::default())
        .await
        .unwrap();
    job_cards::approve_recorded(t.state(), &sami, e.card.id, "Mona Hassan", "recorded", None)
        .await
        .unwrap();
    assert_eq!(
        LabourLine::find_or_404(t.db(), private.id)
            .await
            .unwrap()
            .approval,
        LineStatus::Approved
    );
    repaired(&t, &e).await;

    // Only the accountant invoices.
    let early = claims::invoice(t.state(), &carla, claim.id)
        .await
        .unwrap_err();
    assert!(matches!(early, Error::Forbidden));
    let aisha = aisha(&t).await;
    let invoiced = claims::invoice(t.state(), &aisha, claim.id).await.unwrap();
    assert_eq!(invoiced.status, ClaimStatus::Invoiced);
    assert!(invoiced.invoiced_at.is_some());

    // The insurer: the approved AED 950 less the excess of AED 200.
    let bills = t.payer_books.about(&claim.subject());
    assert_eq!(bills.len(), 1);
    let bill = &bills[0];
    assert_eq!(invoiced.payer_bill_id, Some(bill.id));
    assert_eq!(bill.claimed(), 75_000);
    assert_eq!(bill.status, PayerBillStatus::Issued);
    assert_eq!(bill.request.agreement, 3);
    assert_eq!(bill.request.payer_reference.as_deref(), Some("OR-2026-118"));
    assert_eq!(bill.request.customer, PartyId(7));
    let lines: Vec<(&str, i64)> = bill
        .request
        .lines
        .iter()
        .map(|l| (l.description.as_str(), l.claimed))
        .collect();
    assert_eq!(
        lines,
        [("Door panel repair", 20_000), ("Bumper cover", 55_000)]
    );

    // The customer: the excess and the alignment.
    let invoices = t.books.about(&claim.subject());
    assert_eq!(invoices.len(), 1);
    let invoice = &invoices[0];
    assert!(invoice.issued);
    assert_eq!(invoiced.customer_invoice_id, Some(invoice.id));
    assert_eq!(invoice.party(), PartyId(7));
    assert_eq!(invoice.net(), 30_000);
    let described: Vec<&str> = invoice
        .draft
        .lines
        .iter()
        .map(|l| l.description.as_str())
        .collect();
    assert_eq!(
        described,
        ["Excess of claim OR-2026-118", "Wheel alignment (1 h)"]
    );

    // Both together are the price of the approved lines and the private item.
    assert_eq!(bill.claimed() + invoice.net(), 105_000);

    // Not settled until both are paid.
    assert!(!claims::settle_if_paid(t.state(), claim.id).await.unwrap());
    t.books.pay(invoice.id, invoice.gross());
    assert!(!claims::settle_if_paid(t.state(), claim.id).await.unwrap());
    t.payer_books.set_status(bill.id, PayerBillStatus::Paid);
    assert!(claims::settle_if_paid(t.state(), claim.id).await.unwrap());
    let settled = claim_now(&t, claim.id).await;
    assert_eq!(settled.status, ClaimStatus::Settled);
    assert!(settled.settled_at.is_some());
}

/// The insurer needs a payer agreement to be billed; a claim that is not
/// repaired yet is not invoiced.
#[renox::test]
async fn aut_base_a_claim_is_invoiced_only_when_repaired_and_under_an_agreement() {
    let t = boot().await;
    let e = sent_estimate(&t).await;
    let claim = registered(&t, e.card.id).await;
    let carla = carla(&t).await;
    let aisha = aisha(&t).await;
    claims::submit_estimate(t.state(), &carla, claim.id)
        .await
        .unwrap();
    claims::approve(t.state(), &carla, claim.id, approval()(&e))
        .await
        .unwrap();
    assert!(
        claims::invoice(t.state(), &aisha, claim.id).await.is_err(),
        "an approved claim is not repaired yet"
    );

    // Without a payer agreement on the insurer.
    insurers::update(
        t.state(),
        &carla,
        claim.insurer_id,
        NewInsurer::new(9).email("claims@orient.example"),
    )
    .await
    .unwrap();
    repaired(&t, &e).await;
    let none = claims::invoice(t.state(), &aisha, claim.id)
        .await
        .unwrap_err();
    assert!(none.to_string().contains("payer agreement"), "{none}");
    assert!(t.payer_books.all().is_empty());
    assert!(t.books.all().is_empty());
    assert_eq!(
        claim_now(&t, claim.id).await.status,
        ClaimStatus::RepairComplete
    );
}

/// `cancel`, `declare_total_loss` and `void` are declared with the guard slots
/// the issues fill; they pass for now and end the claim as the spec says.
#[renox::test]
async fn aut_base_withdraw_total_loss_and_void_are_declared_with_slots_that_pass() {
    let t = boot().await;
    let machine = InsuranceClaim::machine();
    let guards = |name: &str| {
        machine
            .find(name)
            .unwrap()
            .guard_names()
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    assert_eq!(guards(CANCEL), ["written_withdrawal"]);
    assert_eq!(guards(DECLARE_TOTAL_LOSS), ["total_loss_notice"]);
    assert_eq!(guards(VOID), ["nothing_sent"]);
    assert_eq!(guards(BOOK_SURVEY), ["claim_details", "police_report"]);
    assert_eq!(guards(INVOICE), ["charges_approved"]);

    let e = sent_estimate(&t).await;
    let claim = registered(&t, e.card.id).await;
    let carla = carla(&t).await;
    let voided = claims::void(t.state(), &carla, claim.id, "Registered twice")
        .await
        .unwrap();
    assert_eq!(voided.status, ClaimStatus::Voided);
    assert_eq!(voided.closed_reason.as_deref(), Some("Registered twice"));
    assert!(
        claims::void(t.state(), &carla, claim.id, "again")
            .await
            .is_err()
    );

    // A second claim on the same vehicle is withdrawn by the customer.
    let vehicle = t.hilux().await;
    let withdrawn = claims::register(
        t.state(),
        &carla,
        NewClaim::new(
            claim.insurer_id,
            vehicle.id,
            "POL-7788",
            "OR-2026-119",
            day(-3),
        )
        .police_report("DXB-PR-5522", None),
    )
    .await
    .unwrap();
    let blank = claims::cancel(t.state(), &carla, withdrawn.id, "", None)
        .await
        .unwrap_err();
    assert!(
        blank.to_string().to_lowercase().contains("reason"),
        "{blank}"
    );
    let cancelled = claims::cancel(
        t.state(),
        &carla,
        withdrawn.id,
        "Withdrawn by the customer",
        Some("withdrawal-letter"),
    )
    .await
    .unwrap();
    assert_eq!(cancelled.status, ClaimStatus::Cancelled);
    assert_eq!(
        cancelled.withdrawal_document.as_deref(),
        Some("withdrawal-letter")
    );

    // A third one, declared a total loss by the insurer after the survey.
    let lost = claims::register(
        t.state(),
        &carla,
        NewClaim::new(
            claim.insurer_id,
            vehicle.id,
            "POL-7788",
            "OR-2026-120",
            day(-3),
        ),
    )
    .await
    .unwrap();
    claims::book_survey(t.state(), &carla, lost.id)
        .await
        .unwrap();
    let total = claims::declare_total_loss(t.state(), &carla, lost.id, "orient-total-loss-notice")
        .await
        .unwrap();
    assert_eq!(total.status, ClaimStatus::TotalLoss);
    assert_eq!(
        total.total_loss_notice.as_deref(),
        Some("orient-total-loss-notice")
    );
    assert!(total.total_loss_at.is_some());
    let aisha = aisha(&t).await;
    let no_charges = claims::invoice(t.state(), &aisha, lost.id)
        .await
        .unwrap_err();
    assert!(
        no_charges.to_string().contains("approval of the charges"),
        "{no_charges}"
    );
    let registered_only = claims::register(
        t.state(),
        &carla,
        NewClaim::new(
            claim.insurer_id,
            vehicle.id,
            "POL-7788",
            "OR-2026-121",
            day(-3),
        ),
    )
    .await
    .unwrap();
    assert!(
        claims::declare_total_loss(t.state(), &carla, registered_only.id, "notice")
            .await
            .is_err(),
        "a total loss is not declared from registered"
    );
}

/// The police report is recorded on a claim that is not over, and gives the
/// job card its report.
#[renox::test]
async fn aut_base_the_police_report_is_recorded_on_the_claim_and_the_job_card() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let sami = t.advisor().await;
    let card = job_cards::check_in(
        t.state(),
        &sami,
        NewJobCard::new(vehicle.id, "Hit")
            .odometer(52_400)
            .condition("Dented"),
    )
    .await
    .unwrap();
    let insurer = orient(&t).await;
    let carla = carla(&t).await;
    let claim = claims::register(
        t.state(),
        &carla,
        NewClaim::new(insurer.id, vehicle.id, "POL-1", "OR-1", day(-4)).job_card(card.id),
    )
    .await
    .unwrap();
    assert!(!claim.has_police_report());
    let early = claims::record_police_report(t.state(), &carla, claim.id, "DXB-9", Some(day(-5)))
        .await
        .unwrap_err();
    assert!(early.to_string().contains("police report"), "{early}");
    let saved = claims::record_police_report(t.state(), &carla, claim.id, " DXB-9 ", Some(day(-3)))
        .await
        .unwrap();
    assert_eq!(saved.police_report_no.as_deref(), Some("DXB-9"));
    let card = JobCard::find_or_404(t.db(), card.id).await.unwrap();
    assert!(card.accident_damage);
    assert_eq!(card.police_report_no.as_deref(), Some("DXB-9"));
}
