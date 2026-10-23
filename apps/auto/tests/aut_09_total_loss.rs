//! BL-AUT-09 (#405): declare a total loss and bill strip-down, labour and
//! storage (`AUT.INS.04`, 08 §7.1 *Insurance repair claim* `declare_total_loss`
//! and `invoice`, *Release vehicle*), with the stand-ins of the invoice book,
//! the payer bills and the notifier and the real wl-scheduling. The same points
//! against the real invoicing are in `host/tests/auto_total_loss_real.rs`.

mod support;

use auto::claims::{self, Answer};
use auto::insurers::{self, NewInsurer};
use auto::release::{self, Release};
use auto::{
    ApprovalKind, ApprovalOutcome, ClaimApproval, ClaimStatus, InsuranceClaim, JobCard,
    JobCardStatus, settings,
};
use renox::prelude::*;
use support::claimfx::{aisha, carla, registered};
use support::stopfx::{checked_in, running};
use support::{Garage, Messages, boot};
use wl_kernel::PartyId;
use wl_kernel::notify::NotifyChannel;
use wl_kernel::payer_bills::PayerBillStatus;
use wl_scheduling::{Booking, BookingStatus};

const NOTICE: &str = "orient-total-loss-notice-100";

async fn claim_now(t: &Garage, id: i64) -> InsuranceClaim {
    InsuranceClaim::find_or_404(t.db(), id).await.unwrap()
}

/// JC-100 `in_progress` under the approved CL-10: strip-down AED 800 and labour
/// AED 600, both done.
async fn jc100(t: &Garage) -> support::stopfx::Running {
    running(
        t,
        &[
            ("Strip-down", 4_000, 20_000),
            ("Body labour", 3_000, 20_000),
        ],
        &[0, 1],
    )
    .await
}

/// CL-10 `approved` with JC-100 `in_progress`: coordinator A declares the total
/// loss with the notice. CL-10 is `total_loss`, JC-100 is `cancelled` with the
/// reason *total loss* and no invoice of its own, the charges are listed on the
/// claim for the insurer's approval and the customer is notified.
#[renox::test]
async fn aut_09_a_total_loss_cancels_the_job_card_with_no_invoice_of_its_own() {
    let t = boot().await;
    let jc = jc100(&t).await;
    let carla = carla(&t).await;

    let lost = claims::declare_total_loss(t.state(), &carla, jc.claim.id, NOTICE)
        .await
        .unwrap();
    assert_eq!(lost.status, ClaimStatus::TotalLoss);
    assert_eq!(lost.total_loss_notice.as_deref(), Some(NOTICE));
    let card = t.reload(jc.card.id).await;
    assert_eq!(card.status, JobCardStatus::Cancelled);
    assert_eq!(card.closed_code.as_deref(), Some("total_loss"));
    assert_eq!(
        card.closed_reason.as_deref(),
        Some("The vehicle is a total loss")
    );
    assert_eq!(card.work_done_value, 140_000);
    assert!(card.invoice_id.is_none());
    assert!(
        t.books.about(&card.subject()).is_empty(),
        "no invoice of its own"
    );
    assert_eq!(t.cancelled.all()[0].reason_code, "total_loss");

    // The charges wait for the insurer's approval.
    let requests = ClaimApproval::of_claim(t.db(), jc.claim.id).await.unwrap();
    let charges = requests
        .iter()
        .find(|row| row.kind == ApprovalKind::Charges)
        .expect("the charges are listed");
    assert_eq!(charges.outcome, ApprovalOutcome::Pending);
    assert_eq!(charges.estimate_amount, 140_000);
    let lines = claims::request_lines(t.db(), jc.claim.id, ApprovalKind::Charges)
        .await
        .unwrap();
    let described: Vec<(&str, i64)> = lines
        .iter()
        .map(|l| (l.description.as_str(), l.price))
        .collect();
    assert_eq!(described, [("Strip-down", 80_000), ("Body labour", 60_000)]);

    // The customer is notified.
    let notices = t.messages.of_type("auto.claim_total_loss");
    assert_eq!(notices.len(), 1);
    assert!(Messages::told_party(&notices[0], 7));
    let text = Messages::text(&notices[0], NotifyChannel::Sms);
    assert!(text.contains("Dubai A 12345"), "{text}");
    assert!(text.contains("total loss"), "{text}");
}

/// The insurer approves AED 800 of strip-down and AED 400 of the labour:
/// `invoice` makes the insurer's payer bill of AED 1,200 and the customer's
/// invoice of AED 200 + VAT; it is refused while no approval of the charges is
/// attached.
#[renox::test]
async fn aut_09_the_charges_are_billed_to_the_insurer_as_approved_and_the_rest_to_the_customer() {
    let t = boot().await;
    let jc = jc100(&t).await;
    let carla = carla(&t).await;
    let aisha = aisha(&t).await;
    claims::declare_total_loss(t.state(), &carla, jc.claim.id, NOTICE)
        .await
        .unwrap();

    let early = claims::invoice(t.state(), &aisha, jc.claim.id)
        .await
        .unwrap_err();
    assert!(
        early.to_string().contains("approval of the charges"),
        "{early}"
    );
    assert!(t.payer_books.all().is_empty());

    let unknown = claims::approve_charges(
        t.state(),
        &carla,
        jc.claim.id,
        Answer::new("TL-1", "orient-charges-1").labour(9_999, 1_000),
    )
    .await
    .unwrap_err();
    assert!(unknown.to_string().contains("not one"), "{unknown}");
    let too_much = claims::approve_charges(
        t.state(),
        &carla,
        jc.claim.id,
        Answer::new("TL-1", "orient-charges-1").labour(jc.lines[0].id, 90_000),
    )
    .await
    .unwrap_err();
    assert!(too_much.to_string().contains("more than"), "{too_much}");

    claims::approve_charges(
        t.state(),
        &carla,
        jc.claim.id,
        Answer::new("TL-1", "orient-charges-1")
            .labour(jc.lines[0].id, 80_000)
            .labour(jc.lines[1].id, 40_000),
    )
    .await
    .unwrap();
    let invoiced = claims::invoice(t.state(), &aisha, jc.claim.id)
        .await
        .unwrap();
    assert_eq!(invoiced.status, ClaimStatus::Invoiced);

    let bills = t.payer_books.about(&invoiced.subject());
    assert_eq!(bills.len(), 1, "one payer bill for the insurer");
    assert_eq!(bills[0].claimed(), 120_000, "AED 1,200 + VAT is issued");
    let invoices = t.books.about(&invoiced.subject());
    assert_eq!(invoices.len(), 1, "one invoice for the rest");
    assert_eq!(invoices[0].party(), PartyId(7));
    assert_eq!(invoices[0].net(), 20_000);
    assert_eq!(invoices[0].gross(), 21_000);
    assert!(
        t.books.about(&jc.card.subject()).is_empty(),
        "the job card billed nothing"
    );
}

/// The insurer approves every charge: only its payer bill is issued, and the
/// claim settles when it is paid.
#[renox::test]
async fn aut_09_when_the_insurer_approves_every_charge_only_the_insurer_is_invoiced() {
    let t = boot().await;
    let jc = jc100(&t).await;
    let carla = carla(&t).await;
    let aisha = aisha(&t).await;
    claims::declare_total_loss(t.state(), &carla, jc.claim.id, NOTICE)
        .await
        .unwrap();
    claims::approve_charges(
        t.state(),
        &carla,
        jc.claim.id,
        Answer::new("TL-1", "orient-charges-1")
            .labour(jc.lines[0].id, 80_000)
            .labour(jc.lines[1].id, 60_000),
    )
    .await
    .unwrap();
    let invoiced = claims::invoice(t.state(), &aisha, jc.claim.id)
        .await
        .unwrap();
    assert!(invoiced.customer_invoice_id.is_none());
    assert!(t.books.about(&invoiced.subject()).is_empty());
    let bills = t.payer_books.about(&invoiced.subject());
    assert_eq!(bills.len(), 1);
    assert_eq!(bills[0].claimed(), 140_000);

    assert!(
        !claims::settle_if_paid(t.state(), invoiced.id)
            .await
            .unwrap()
    );
    t.payer_books.set_status(bills[0].id, PayerBillStatus::Paid);
    assert!(
        claims::settle_if_paid(t.state(), invoiced.id)
            .await
            .unwrap()
    );
    assert_eq!(
        claim_now(&t, invoiced.id).await.status,
        ClaimStatus::Settled
    );
}

/// Vehicle release after a total loss needs the insurer's written instruction
/// and a signature, and is allowed unpaid.
#[renox::test]
async fn aut_09_the_vehicle_leaves_only_on_the_insurers_written_instruction_with_a_signature() {
    let t = boot().await;
    let jc = jc100(&t).await;
    let carla = carla(&t).await;
    claims::declare_total_loss(t.state(), &carla, jc.claim.id, NOTICE)
        .await
        .unwrap();
    let sami = t.advisor().await;
    let signed = || Release {
        released_to: "Orient Insurance".to_owned(),
        condition: "as_is".to_owned(),
        signature: Some("Samir, Orient".to_owned()),
    };

    let plain = release::release(t.state(), &sami, jc.card.id, signed())
        .await
        .unwrap_err();
    assert!(plain.to_string().contains("written instruction"), "{plain}");
    let blank =
        release::release_with_instruction(t.state(), &sami, jc.card.id, signed(), Some(" "))
            .await
            .unwrap_err();
    assert!(blank.to_string().contains("written instruction"), "{blank}");
    let unsigned = release::release_with_instruction(
        t.state(),
        &sami,
        jc.card.id,
        Release::safe("Orient Insurance"),
        Some("orient-release-letter-1"),
    )
    .await
    .unwrap_err();
    assert!(unsigned.to_string().contains("signs for it"), "{unsigned}");
    assert!(t.reload(jc.card.id).await.released_at.is_none());

    let released = release::release_with_instruction(
        t.state(),
        &sami,
        jc.card.id,
        signed(),
        Some("orient-release-letter-1"),
    )
    .await
    .expect("allowed before anything is paid");
    assert!(released.released_at.is_some());
    assert_eq!(
        released.release_instruction.as_deref(),
        Some("orient-release-letter-1")
    );
    assert_eq!(released.released_to.as_deref(), Some("Orient Insurance"));
}

/// CL-11 `registered`: `declare_total_loss` is refused and not offered; without
/// the insurer's notice it is refused too.
#[renox::test]
async fn aut_09_a_total_loss_is_not_declared_from_registered_or_without_the_notice() {
    let t = boot().await;
    let (card, _booking) = checked_in(&t).await;
    let claim = registered(&t, card.id).await;
    let carla = carla(&t).await;
    let refused = claims::declare_total_loss(t.state(), &carla, claim.id, NOTICE)
        .await
        .unwrap_err();
    assert!(!refused.to_string().is_empty());
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
    let page = t.app.get(&format!("/auto/claims/{}", claim.id)).await;
    page.assert_ok();
    assert!(!page.text().contains("Declare a total loss"));

    claims::book_survey(t.state(), &carla, claim.id)
        .await
        .unwrap();
    let blank = claims::declare_total_loss(t.state(), &carla, claim.id, "  ")
        .await
        .unwrap_err();
    assert!(blank.to_string().contains("total-loss notice"), "{blank}");
    assert_eq!(
        claim_now(&t, claim.id).await.status,
        ClaimStatus::AwaitingSurvey
    );
    t.app
        .get(&format!("/auto/claims/{}", claim.id))
        .await
        .assert_ok()
        .assert_see("Declare a total loss");
}

/// CL-12 `awaiting_survey` with JC-102 `checked_in`: the total loss declines the
/// job card (no invoice, no collect message) and cancels its booking.
#[renox::test]
async fn aut_09_a_total_loss_declines_a_job_card_with_no_work_started() {
    let t = boot().await;
    let (card, booking) = checked_in(&t).await;
    let claim = registered(&t, card.id).await;
    let carla = carla(&t).await;
    claims::book_survey(t.state(), &carla, claim.id)
        .await
        .unwrap();
    claims::declare_total_loss(t.state(), &carla, claim.id, NOTICE)
        .await
        .unwrap();

    let declined = t.reload(card.id).await;
    assert_eq!(declined.status, JobCardStatus::Declined);
    assert_eq!(declined.closed_code.as_deref(), Some("total_loss"));
    assert_eq!(declined.billed_net, 0);
    assert!(t.books.about(&card.subject()).is_empty());
    assert!(
        t.messages.of_type(auto::declines::COLLECT_TYPE).is_empty(),
        "the customer is told through the claim"
    );
    assert_eq!(t.messages.of_type("auto.claim_total_loss").len(), 1);
    let booking = Booking::find_or_404(t.db(), booking.id).await.unwrap();
    assert_eq!(booking.status, BookingStatus::Cancelled);
}

/// The client's `AUT.RUL.18` storage fee on and agreed at check-in: the release
/// after the total loss makes no storage invoice; storage is one of the claim's
/// charges, at the insurer's rate, billed with the claim.
#[renox::test]
async fn aut_09_storage_after_a_total_loss_is_billed_through_the_claim_only() {
    let t = boot().await;
    let jc = jc100(&t).await;
    let carla = carla(&t).await;
    let aisha = aisha(&t).await;
    settings::set(t.db(), settings::STORAGE_FEE_ENABLED, "1", None)
        .await
        .unwrap();
    JobCard::query()
        .where_(JobCard::ID.eq(jc.card.id))
        .update(t.db(), &[("storage_fee_daily", &3_000_i64)])
        .await
        .unwrap();
    insurers::update(
        t.state(),
        &carla,
        jc.claim.insurer_id,
        NewInsurer::new(9)
            .agreement(3)
            .email("claims@orient.example")
            .labour_rate(12_000)
            .storage_rate(5_000),
    )
    .await
    .unwrap();

    claims::declare_total_loss(t.state(), &carla, jc.claim.id, NOTICE)
        .await
        .unwrap();
    let lines = claims::request_lines(t.db(), jc.claim.id, ApprovalKind::Charges)
        .await
        .unwrap();
    let storage = lines.iter().find(|l| l.kind == "charge").expect("storage");
    assert_eq!(storage.price, 5_000, "one day at the insurer's rate");

    let sami = t.advisor().await;
    release::release_with_instruction(
        t.state(),
        &sami,
        jc.card.id,
        Release::as_is("Orient Insurance", "Samir"),
        Some("orient-release-letter-1"),
    )
    .await
    .unwrap();
    let card = t.reload(jc.card.id).await;
    assert!(card.storage_invoice_id.is_none(), "no AUT.RUL.18 invoice");
    assert!(t.books.about(&card.subject()).is_empty());

    claims::approve_charges(
        t.state(),
        &carla,
        jc.claim.id,
        Answer::new("TL-1", "orient-charges-1")
            .labour(jc.lines[0].id, 80_000)
            .labour(jc.lines[1].id, 60_000)
            .charge(5_000),
    )
    .await
    .unwrap();
    let invoiced = claims::invoice(t.state(), &aisha, jc.claim.id)
        .await
        .unwrap();
    let bills = t.payer_books.about(&invoiced.subject());
    assert_eq!(bills[0].claimed(), 145_000, "the storage is on the claim");
}

/// The storage of a total loss is priced at the day the insurer decided: the
/// invoice, run days later, does not charge the customer for the days that
/// passed in between.
#[renox::test]
async fn aut_09_the_storage_price_is_fixed_at_the_insurers_decision() {
    let t = boot().await;
    let jc = jc100(&t).await;
    let carla = carla(&t).await;
    let aisha = aisha(&t).await;
    insurers::update(
        t.state(),
        &carla,
        jc.claim.insurer_id,
        NewInsurer::new(9)
            .agreement(3)
            .email("claims@orient.example")
            .labour_rate(12_000)
            .storage_rate(5_000),
    )
    .await
    .unwrap();
    let ago = |days: i64| renox::db::now() - renox::chrono::Duration::days(days);
    JobCard::query()
        .where_(JobCard::ID.eq(jc.card.id))
        .update(t.db(), &[("created_at", &Some(ago(4)))])
        .await
        .unwrap();
    claims::declare_total_loss(t.state(), &carla, jc.claim.id, NOTICE)
        .await
        .unwrap();
    let lines = claims::request_lines(t.db(), jc.claim.id, ApprovalKind::Charges)
        .await
        .unwrap();
    let storage = lines.iter().find(|l| l.kind == "charge").expect("storage");
    assert_eq!(storage.price, 20_000, "4 days at AED 50");

    claims::approve_charges(
        t.state(),
        &carla,
        jc.claim.id,
        Answer::new("TL-1", "orient-charges-1")
            .labour(jc.lines[0].id, 80_000)
            .labour(jc.lines[1].id, 60_000)
            .charge(15_000),
    )
    .await
    .unwrap();

    // Five more days pass before the accountant invoices: the decision is five
    // days old and the vehicle has been in nine days.
    ClaimApproval::query()
        .where_(ClaimApproval::CLAIM_ID.eq(jc.claim.id))
        .update(t.db(), &[("decided_at", &Some(ago(5)))])
        .await
        .unwrap();
    JobCard::query()
        .where_(JobCard::ID.eq(jc.card.id))
        .update(t.db(), &[("created_at", &Some(ago(9)))])
        .await
        .unwrap();
    let invoiced = claims::invoice(t.state(), &aisha, jc.claim.id)
        .await
        .unwrap();
    let mine = t.books.about(&invoiced.subject());
    assert_eq!(mine.len(), 1, "the customer's invoice");
    assert_eq!(
        mine[0].net(),
        5_000,
        "only the 20,000 priced at the decision less the 15,000 approved"
    );
}
