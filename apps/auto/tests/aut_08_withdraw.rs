//! BL-AUT-08 (#404): withdraw an insurance claim, also after the insurer
//! approved it (`AUT.ACL.15`, `AUT.RUL.17`, 08 §7.1 *Insurance repair claim*
//! `cancel`). The booking is the real wl-scheduling's; the invoices, the payer
//! bills and the messages are the stand-ins'. The same points against the real
//! invoicing and notify are in `host/tests/auto_withdraw_real.rs`.

mod support;

use auto::claims::{self, Withdrawal};
use auto::declines::{self, COLLECT_TYPE};
use auto::job_cards::{self, NewLabour, Promise};
use auto::{
    ApprovalKind, ApprovalOutcome, ClaimApproval, ClaimStatus, InsuranceClaim, JobCard,
    JobCardStatus, LabourLine, LineStatus, Payer, WorkStatus,
};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use support::claimfx::{carla, registered, sent_estimate};
use support::stopfx::{checked_in, running, waiting};
use support::{Garage, Messages, at, boot, day};
use wl_kernel::PartyId;
use wl_kernel::notify::{NotifyChannel, NotifyRecipient};
use wl_scheduling::{Booking, BookingStatus};

async fn claim_now(t: &Garage, id: i64) -> InsuranceClaim {
    InsuranceClaim::find_or_404(t.db(), id).await.unwrap()
}

/// CL-8 `approved` with JC-80 `in_progress` (AED 1,200 of labour done): the
/// coordinator cancels with the customer's written withdrawal and the choice
/// *stop*. The claim is `cancelled`, the insurer is told in writing, the job
/// card is `cancelled` with the reason *claim withdrawn*, and an invoice to the
/// customer for AED 1,200 + VAT exists; the insurer is not invoiced.
#[renox::test]
async fn aut_08_a_claim_withdrawn_after_approval_with_stop_bills_the_work_done_to_the_customer() {
    let t = boot().await;
    let jc = running(&t, &[("Panel repair", 6_000, 20_000)], &[0]).await;
    let carla = carla(&t).await;

    let cancelled = claims::withdraw(
        t.state(),
        &carla,
        jc.claim.id,
        Withdrawal::new("Withdrawn by the customer")
            .document("withdrawal-letter-80")
            .stop(),
    )
    .await
    .unwrap();
    assert_eq!(cancelled.status, ClaimStatus::Cancelled);
    assert_eq!(cancelled.withdrawal_choice.as_deref(), Some("stop"));
    assert_eq!(
        cancelled.withdrawal_document.as_deref(),
        Some("withdrawal-letter-80")
    );

    // The insurer is told in writing, at its claim email.
    let letters = t.messages.of_type("auto.claim_withdrawal_letter");
    assert_eq!(letters.len(), 1, "one letter to the insurer");
    assert!(letters[0].recipients.iter().any(|r| matches!(
        r,
        NotifyRecipient::Address { address, .. } if address == "claims@orient.example"
    )));
    let text = Messages::text(&letters[0], NotifyChannel::Email);
    assert!(text.contains("OR-2026-118"), "{text}");
    assert!(text.contains("withdrawal-letter-80"), "{text}");
    assert!(text.contains("Orient Insurance"), "{text}");
    assert!(cancelled.letter_sent_at.is_some());

    // The job card is cancelled and the work done is the customer's invoice.
    let card = t.reload(jc.card.id).await;
    assert_eq!(card.status, JobCardStatus::Cancelled);
    assert_eq!(card.closed_code.as_deref(), Some("claim_withdrawn"));
    assert_eq!(
        card.closed_reason.as_deref(),
        Some("The claim was withdrawn")
    );
    assert_eq!(card.work_done_value, 120_000);
    let invoices = t.books.about(&card.subject());
    assert_eq!(invoices.len(), 1, "one invoice for the work done");
    assert_eq!(invoices[0].party(), PartyId(7), "the customer's");
    assert_eq!(invoices[0].net(), 120_000);
    assert_eq!(invoices[0].gross(), 126_000, "AED 1,200 and 5% VAT");
    assert!(invoices[0].issued);
    assert!(
        t.payer_books.all().is_empty(),
        "the insurer is not invoiced after the withdrawal"
    );
    assert_eq!(t.cancelled.all().len(), 1);
    assert_eq!(t.cancelled.all()[0].reason_code, "claim_withdrawn");
}

/// The same with the choice *continue privately*: CL-8 is `cancelled`, a private
/// estimate of every line (done and remaining) is sent, JC-80 is
/// `awaiting_approval`, every line is the customer's, nothing is invoiced yet,
/// and labour entry is refused until the customer signs.
#[renox::test]
async fn aut_08_continuing_privately_sends_a_private_estimate_and_takes_no_labour_until_it_is_signed()
 {
    let t = boot().await;
    let jc = running(
        &t,
        &[("Panel repair", 6_000, 20_000), ("Paint", 4_000, 15_000)],
        &[0],
    )
    .await;
    let carla = carla(&t).await;

    let cancelled = claims::withdraw(
        t.state(),
        &carla,
        jc.claim.id,
        Withdrawal::new("Withdrawn by the customer")
            .document("withdrawal-letter-80")
            .continue_privately(),
    )
    .await
    .unwrap();
    assert_eq!(cancelled.status, ClaimStatus::Cancelled);
    assert_eq!(
        cancelled.withdrawal_choice.as_deref(),
        Some("continue_privately")
    );

    let card = t.reload(jc.card.id).await;
    assert_eq!(card.status, JobCardStatus::AwaitingApproval);
    assert!(
        card.private_estimate,
        "the estimate waiting is the private one"
    );
    let round = job_cards::pending_round(t.db(), card.id)
        .await
        .unwrap()
        .expect("a private estimate waits for the customer");
    assert_eq!(round.net_total, 180_000, "AED 1,200 done and AED 600 to do");
    let lines = LabourLine::query()
        .where_(LabourLine::JOB_CARD_ID.eq(card.id))
        .get(t.db())
        .await
        .unwrap();
    assert_eq!(lines.len(), 2);
    for line in &lines {
        assert_eq!(line.payer, Payer::Customer, "{}", line.description);
        assert_eq!(line.approval_id, Some(round.id), "{}", line.description);
    }
    assert!(
        t.books.about(&card.subject()).is_empty(),
        "nothing is invoiced while the customer decides"
    );
    assert!(t.payer_books.all().is_empty());
    assert!(t.cancelled.all().is_empty());

    // Until the private estimate is signed the technician records nothing.
    let tareq = t.technician("Tareq").await;
    let labour = job_cards::add_labour(
        t.state(),
        &tareq,
        card.id,
        NewLabour::hourly("Polish", 1_000, 10_000),
    )
    .await
    .unwrap_err();
    assert!(labour.to_string().contains("private estimate"), "{labour}");
    let work = job_cards::record_work(t.state(), &tareq, jc.lines[1].id, WorkStatus::Done, None)
        .await
        .unwrap_err();
    assert!(work.to_string().contains("private estimate"), "{work}");

    // Signed, the job goes on and labour is taken again.
    let sami = t.advisor().await;
    let signed =
        job_cards::approve_recorded(t.state(), &sami, card.id, "Mona Hassan", "recorded", None)
            .await
            .unwrap();
    assert_eq!(signed.status, JobCardStatus::InProgress);
    assert!(!signed.private_estimate);
    job_cards::record_work(t.state(), &tareq, jc.lines[1].id, WorkStatus::Done, None)
        .await
        .expect("labour is recorded once the private estimate is signed");
}

/// CL-9 `estimate_submitted` with JC-90 `awaiting_approval` and no work: JC-90
/// is `declined`, its estimate lines are declined, the customer is asked to
/// collect the vehicle.
#[renox::test]
async fn aut_08_a_withdrawn_claim_with_no_work_declines_the_job_card_awaiting_approval() {
    let t = boot().await;
    let e = sent_estimate(&t).await;
    let claim = registered(&t, e.card.id).await;
    let carla = carla(&t).await;
    claims::submit_estimate(t.state(), &carla, claim.id)
        .await
        .unwrap();

    let cancelled = claims::cancel(
        t.state(),
        &carla,
        claim.id,
        "Withdrawn by the customer",
        None,
    )
    .await
    .unwrap();
    assert_eq!(cancelled.status, ClaimStatus::Cancelled);
    let card = t.reload(e.card.id).await;
    assert_eq!(card.status, JobCardStatus::Declined);
    assert_eq!(card.closed_code.as_deref(), Some("claim_withdrawn"));
    assert_eq!(
        LabourLine::find_or_404(t.db(), e.door.id)
            .await
            .unwrap()
            .approval,
        LineStatus::Declined
    );
    assert_eq!(
        t.messages.of_type(COLLECT_TYPE).len(),
        1,
        "the customer is asked to collect the vehicle"
    );
    assert!(t.cancelled.all().is_empty(), "a decline is not a cancel");
}

/// CL-14 `awaiting_survey` with JC-91 `checked_in`: cancelling CL-14 makes
/// JC-91 `declined` and cancels its booking.
#[renox::test]
async fn aut_08_cancelling_a_claim_awaiting_survey_declines_a_checked_in_job_card_and_its_booking()
{
    let t = boot().await;
    let (card, booking) = checked_in(&t).await;
    let claim = registered(&t, card.id).await;
    let carla = carla(&t).await;
    let surveyed = claims::book_survey(t.state(), &carla, claim.id)
        .await
        .unwrap();
    assert_eq!(surveyed.status, ClaimStatus::AwaitingSurvey);

    claims::cancel(
        t.state(),
        &carla,
        claim.id,
        "Withdrawn by the customer",
        None,
    )
    .await
    .unwrap();
    let declined = t.reload(card.id).await;
    assert_eq!(declined.status, JobCardStatus::Declined);
    assert_eq!(declined.closed_code.as_deref(), Some("claim_withdrawn"));
    let booking = Booking::find_or_404(t.db(), booking.id).await.unwrap();
    assert_eq!(booking.status, BookingStatus::Cancelled);
    assert_eq!(booking.cancel_code.as_deref(), Some("estimate_declined"));
}

/// JC-80 withdrawn with *continue privately* on 2026-11-01 10:00 and the
/// private estimate unsigned: at the check seven days later `AUT.RUL.17` makes
/// JC-80 `cancelled` with the reason *claim withdrawn* and the work done is
/// invoiced to the customer. Not an hour before, and once.
#[renox::test]
async fn aut_08_a_private_estimate_unsigned_after_seven_days_cancels_the_job_card() {
    let t = boot().await;
    let jc = running(&t, &[("Panel repair", 6_000, 20_000)], &[0]).await;
    let carla = carla(&t).await;
    claims::withdraw(
        t.state(),
        &carla,
        jc.claim.id,
        Withdrawal::new("Withdrawn by the customer")
            .document("withdrawal-letter-80")
            .continue_privately(),
    )
    .await
    .unwrap();
    let entered = at(day(-10), 10, 0);
    JobCard::query()
        .where_(JobCard::ID.eq(jc.card.id))
        .update(t.db(), &[("awaiting_since", &Some(entered))])
        .await
        .unwrap();

    let early = declines::expire_estimates(
        t.state(),
        entered + TimeDelta::days(7) - TimeDelta::hours(1),
    )
    .await
    .unwrap();
    assert!(
        early.cancelled.is_empty(),
        "six days and 23 hours is not enough"
    );
    assert_eq!(
        t.reload(jc.card.id).await.status,
        JobCardStatus::AwaitingApproval
    );

    let done = declines::expire_estimates(t.state(), entered + TimeDelta::days(7))
        .await
        .unwrap();
    assert_eq!(done.cancelled, [jc.card.id]);
    assert!(done.declined.is_empty());
    let card = t.reload(jc.card.id).await;
    assert_eq!(card.status, JobCardStatus::Cancelled);
    assert_eq!(card.closed_code.as_deref(), Some("claim_withdrawn"));
    assert_eq!(
        card.closed_reason.as_deref(),
        Some("The claim was withdrawn")
    );
    let invoices = t.books.about(&card.subject());
    assert_eq!(invoices.len(), 1, "the work done is invoiced");
    assert_eq!(invoices[0].party(), PartyId(7));
    assert_eq!(invoices[0].net(), 120_000);
    assert_eq!(t.cancelled.all().len(), 1);

    let again = declines::expire_estimates(t.state(), entered + TimeDelta::days(9))
        .await
        .unwrap();
    assert!(again.cancelled.is_empty() && again.declined.is_empty());
    assert_eq!(t.books.about(&card.subject()).len(), 1, "once");
}

/// A customer who will not sign the private estimate stops the job: the
/// advisor's decline cancels it with the work done billed to the customer.
#[renox::test]
async fn aut_08_declining_the_private_estimate_stops_the_job() {
    let t = boot().await;
    let jc = running(&t, &[("Panel repair", 6_000, 20_000)], &[0]).await;
    let carla = carla(&t).await;
    claims::withdraw(
        t.state(),
        &carla,
        jc.claim.id,
        Withdrawal::new("Withdrawn by the customer")
            .document("withdrawal-letter-80")
            .continue_privately(),
    )
    .await
    .unwrap();
    let sami = t.advisor().await;
    let stopped = job_cards::decline(t.state(), &sami, jc.card.id, "The customer will not pay")
        .await
        .unwrap();
    assert_eq!(stopped.status, JobCardStatus::Cancelled);
    assert_eq!(stopped.closed_code.as_deref(), Some("claim_withdrawn"));
    assert_eq!(t.books.about(&stopped.subject()).len(), 1);
}

/// CL-9 cancelled with an inspection fee of AED 150 agreed at check-in: the fee
/// is invoiced to the customer, not to the insurer, and the insurer is sent no
/// payer bill.
#[renox::test]
async fn aut_08_the_inspection_fee_of_a_withdrawn_claim_is_invoiced_to_the_customer() {
    let t = boot().await;
    let (card, _booking) = waiting(&t, 15_000, true).await;
    let claim = registered(&t, card.id).await;
    let carla = carla(&t).await;
    claims::submit_estimate(t.state(), &carla, claim.id)
        .await
        .unwrap();

    claims::cancel(
        t.state(),
        &carla,
        claim.id,
        "Withdrawn by the customer",
        None,
    )
    .await
    .unwrap();
    let declined = t.reload(card.id).await;
    assert_eq!(declined.status, JobCardStatus::Declined);
    assert_eq!(declined.billed_net, 15_000);
    let invoices = t.books.about(&card.subject());
    assert_eq!(invoices.len(), 1);
    assert_eq!(invoices[0].party(), PartyId(7), "addressed to the customer");
    assert_ne!(invoices[0].party(), PartyId(9), "never to the insurer");
    assert_eq!(invoices[0].net(), 15_000);
    assert_eq!(invoices[0].gross(), 15_750);
    assert!(t.payer_books.all().is_empty());
}

/// CL-8 `approved` without the customer's written withdrawal: `cancel` is
/// refused and names the guard; with it, and with work started, the choice is
/// needed too.
#[renox::test]
async fn aut_08_cancel_after_approval_needs_the_written_withdrawal_and_the_choice() {
    let t = boot().await;
    let jc = running(&t, &[("Panel repair", 6_000, 20_000)], &[0]).await;
    let carla = carla(&t).await;

    let none = claims::withdraw(
        t.state(),
        &carla,
        jc.claim.id,
        Withdrawal::new("Withdrawn by the customer").stop(),
    )
    .await
    .unwrap_err();
    assert!(none.to_string().contains("written withdrawal"), "{none}");
    let blank = claims::withdraw(
        t.state(),
        &carla,
        jc.claim.id,
        Withdrawal::new("Withdrawn by the customer")
            .document("  ")
            .stop(),
    )
    .await
    .unwrap_err();
    assert!(blank.to_string().contains("written withdrawal"), "{blank}");
    let no_choice = claims::withdraw(
        t.state(),
        &carla,
        jc.claim.id,
        Withdrawal::new("Withdrawn by the customer").document("letter"),
    )
    .await
    .unwrap_err();
    assert!(
        no_choice.to_string().contains("continues privately"),
        "work started: the customer chooses ({no_choice})"
    );
    let mut unknown = Withdrawal::new("Withdrawn by the customer").document("letter");
    unknown.choice = Some("maybe".to_owned());
    let unknown = claims::withdraw(t.state(), &carla, jc.claim.id, unknown)
        .await
        .unwrap_err();
    assert!(
        unknown.to_string().contains("continue privately"),
        "{unknown}"
    );
    let reasonless = claims::withdraw(
        t.state(),
        &carla,
        jc.claim.id,
        Withdrawal::new(" ").document("letter").stop(),
    )
    .await
    .unwrap_err();
    assert!(
        reasonless.to_string().to_lowercase().contains("reason"),
        "{reasonless}"
    );

    // Nothing changed.
    assert_eq!(
        claim_now(&t, jc.claim.id).await.status,
        ClaimStatus::Approved
    );
    assert_eq!(t.reload(jc.card.id).await.status, JobCardStatus::InProgress);
    assert!(
        t.messages
            .of_type("auto.claim_withdrawal_letter")
            .is_empty()
    );
    assert!(t.books.about(&jc.card.subject()).is_empty());
}

/// A supplementary the insurer never answered is cancelled with the claim;
/// with *continue privately* its added lines are declined and the rest goes into
/// the private estimate.
#[renox::test]
async fn aut_08_a_pending_supplementary_is_cancelled_with_the_claim() {
    let t = boot().await;
    let jc = running(&t, &[("Panel repair", 6_000, 20_000)], &[0]).await;
    let sami = t.advisor().await;
    let tareq = t.technician("Tareq").await;
    let hidden = job_cards::add_labour(
        t.state(),
        &tareq,
        jc.card.id,
        NewLabour::hourly("Hidden crack", 2_000, 20_000),
    )
    .await
    .unwrap();
    job_cards::send_estimate(t.state(), &sami, jc.card.id, Promise::default())
        .await
        .unwrap();
    let asked = claims::request_supplementary(t.state(), &sami, jc.claim.id, "A crack behind")
        .await
        .unwrap();
    assert_eq!(asked.status, ClaimStatus::SupplementaryPending);

    let carla = carla(&t).await;
    let cancelled = claims::withdraw(
        t.state(),
        &carla,
        jc.claim.id,
        Withdrawal::new("Withdrawn by the customer")
            .document("withdrawal-letter-80")
            .continue_privately(),
    )
    .await
    .unwrap();
    assert_eq!(cancelled.status, ClaimStatus::Cancelled);
    let request = ClaimApproval::of_claim(t.db(), jc.claim.id)
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.kind == ApprovalKind::Supplementary)
        .expect("the supplementary request");
    assert_eq!(
        request.outcome,
        ApprovalOutcome::Pending,
        "no answer is expected"
    );
    assert_eq!(
        LabourLine::find_or_404(t.db(), hidden.id)
            .await
            .unwrap()
            .approval,
        LineStatus::Declined
    );
    let card = t.reload(jc.card.id).await;
    assert_eq!(card.status, JobCardStatus::AwaitingApproval);
    assert!(card.private_estimate);
    let round = job_cards::pending_round(t.db(), card.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(round.net_total, 120_000, "the added lines are left out");
}

/// The claim page offers the withdrawal with the choice, and the coordinator
/// withdraws a claim from it.
#[renox::test]
async fn aut_08_the_claim_page_withdraws_with_the_written_withdrawal_and_the_choice() {
    let t = boot().await;
    let jc = running(&t, &[("Panel repair", 6_000, 20_000)], &[0]).await;
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
    let path = format!("/auto/claims/{}", jc.claim.id);
    t.app
        .get(&path)
        .await
        .assert_ok()
        .assert_see("Withdraw the claim");

    let refused = t
        .app
        .htmx()
        .post(
            &format!("{path}/cancel"),
            &[("reason", "Withdrawn by the customer"), ("choice", "stop")],
        )
        .await;
    refused.assert_status(422);
    assert!(refused.text().contains("written withdrawal"));

    t.app
        .htmx()
        .post(
            &format!("{path}/cancel"),
            &[
                ("reason", "Withdrawn by the customer"),
                ("document", "withdrawal-letter-80"),
                ("choice", "stop"),
            ],
        )
        .await
        .assert_ok();
    assert_eq!(
        claim_now(&t, jc.claim.id).await.status,
        ClaimStatus::Cancelled
    );
    assert_eq!(t.reload(jc.card.id).await.status, JobCardStatus::Cancelled);
    t.app
        .get(&path)
        .await
        .assert_ok()
        .assert_see("Withdrawn")
        .assert_see("withdrawal-letter-80");
}
