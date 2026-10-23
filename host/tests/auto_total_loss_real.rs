//! BL-AUT-09 (#405) against the REAL shared modules of the host (wl-invoicing
//! payer bills and tax invoices, wl-notify, wl-documents, wl-scheduling): the
//! job card cancelled for a total loss makes no invoice of its own; the claim's
//! `invoice` bills the insurer on a real payer bill and tax invoice for what it
//! approved and the customer on a real invoice for the rest; the claim settles
//! when the insurer's remittance arrives; the vehicle is released on the
//! insurer's instruction unpaid. The app's own tests
//! (`apps/auto/tests/aut_09_total_loss.rs`) show the same points on stand-ins.

mod support;

use auto::claims::{self, Answer};
use auto::job_cards::{self, NewJobCard};
use auto::release::{self, Release};
use auto::vehicles::{self, NewVehicle};
use auto::{ApprovalKind, Emirate, settings};
use auto::{ClaimStatus, InsuranceClaim, JobCard, JobCardStatus};
use renox::prelude::*;
use support::autofx::{approved_job, boot, police_report, sent_of, shop};
use wl_invoicing::PayerBill;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{InvoiceId, invoicer};
use wl_kernel::payer_bills::{PayerBillId, Remittance, RemittanceLine, payer_bills};

const NOTICE: &str = "orient-total-loss-notice-100";

/// The insurer approves AED 800 of the strip-down and AED 400 of the labour:
/// a real payer bill of AED 1,200 (tax invoice AED 1,260) and a real customer
/// invoice of AED 200 + VAT; the job card billed nothing; the customer was
/// told; the vehicle leaves unpaid on the insurer's instruction.
#[renox::test]
async fn aut_09_real_a_total_loss_is_billed_to_the_insurer_as_approved_and_the_rest_to_the_customer()
 {
    let w = boot().await;
    let s = shop(&w, None).await;
    let job = approved_job(
        &w,
        &s,
        &[
            ("Strip-down", 4_000, 20_000),
            ("Body labour", 3_000, 20_000),
        ],
        &[0, 1],
    )
    .await;

    let lost = claims::declare_total_loss(w.state(), &s.carla, job.claim.id, NOTICE)
        .await
        .unwrap();
    assert_eq!(lost.status, ClaimStatus::TotalLoss);
    let card = JobCard::find_or_404(w.db(), job.card.id).await.unwrap();
    assert_eq!(card.status, JobCardStatus::Cancelled);
    assert_eq!(card.closed_code.as_deref(), Some("total_loss"));
    assert!(card.invoice_id.is_none(), "no invoice of its own");
    assert!(
        invoicer(w.state())
            .billing(w.db(), &card.subject())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(sent_of(&w, "auto.claim_total_loss").await.len(), 1);

    let early = claims::invoice(w.state(), &s.aisha, job.claim.id)
        .await
        .unwrap_err();
    assert!(early.to_string().contains("approval of the charges"));

    claims::approve_charges(
        w.state(),
        &s.carla,
        job.claim.id,
        Answer::new("TL-1", "orient-charges-1")
            .labour(job.lines[0].id, 80_000)
            .labour(job.lines[1].id, 40_000),
    )
    .await
    .unwrap();
    let invoiced = claims::invoice(w.state(), &s.aisha, job.claim.id)
        .await
        .unwrap();
    assert_eq!(invoiced.status, ClaimStatus::Invoiced);

    let bill_id = invoiced.payer_bill_id.expect("a payer bill");
    let bills = payer_bills(w.state());
    let summary = bills
        .find(w.db(), PayerBillId(bill_id))
        .await
        .unwrap()
        .expect("the payer bill");
    assert_eq!(summary.claimed, 120_000, "AED 800 and AED 400");
    assert_eq!(summary.payer, s.orient);
    let payer_bill = PayerBill::find_or_404(w.db(), bill_id).await.unwrap();
    let tax_invoice = invoicer(w.state())
        .find(
            w.db(),
            InvoiceId(payer_bill.payer_invoice_id.expect("the tax invoice")),
        )
        .await
        .unwrap()
        .expect("the tax invoice exists");
    assert_eq!(tax_invoice.party, s.orient);
    assert_eq!(tax_invoice.gross_total, 126_000, "AED 1,200 and 5% VAT");
    let customer = invoicer(w.state())
        .find(
            w.db(),
            InvoiceId(
                invoiced
                    .customer_invoice_id
                    .expect("the customer's invoice"),
            ),
        )
        .await
        .unwrap()
        .expect("the invoice exists");
    assert_eq!(customer.party, s.owner);
    assert_eq!(customer.gross_total, 21_000, "AED 200 and 5% VAT");

    // Unpaid, the vehicle leaves on the insurer's instruction.
    let released = release::release_with_instruction(
        w.state(),
        &s.sami,
        card.id,
        Release::as_is("Orient Insurance", "Samir, Orient"),
        Some("orient-release-letter-1"),
    )
    .await
    .unwrap();
    assert!(released.released_at.is_some());
    assert!(released.storage_invoice_id.is_none());

    // The insurer's advice settles its part; the customer's AED 210 is still
    // open, so the claim stays invoiced.
    let lines = bills.lines(w.db(), PayerBillId(bill_id)).await.unwrap();
    let mut advice = Remittance::new("ORIENT-ADV-TL", today());
    for line in &lines {
        advice = advice.line(RemittanceLine::paid(line.id, line.claimed));
    }
    bills
        .record_remittance(w.state(), PayerBillId(bill_id), advice, &s.aisha)
        .await
        .unwrap();
    assert!(
        !claims::settle_if_paid(w.state(), job.claim.id)
            .await
            .unwrap()
    );
    let still = InsuranceClaim::find_or_404(w.db(), job.claim.id)
        .await
        .unwrap();
    assert_eq!(still.status, ClaimStatus::Invoiced);
}

/// The insurer approves every charge: only its payer bill exists and the claim
/// settles when the remittance arrives.
#[renox::test]
async fn aut_09_real_a_total_loss_approved_in_full_has_no_customer_invoice() {
    let w = boot().await;
    let s = shop(&w, None).await;
    let job = approved_job(&w, &s, &[("Strip-down", 4_000, 20_000)], &[0]).await;
    claims::declare_total_loss(w.state(), &s.carla, job.claim.id, NOTICE)
        .await
        .unwrap();
    claims::approve_charges(
        w.state(),
        &s.carla,
        job.claim.id,
        Answer::new("TL-1", "orient-charges-1").labour(job.lines[0].id, 80_000),
    )
    .await
    .unwrap();
    let invoiced = claims::invoice(w.state(), &s.aisha, job.claim.id)
        .await
        .unwrap();
    assert!(invoiced.customer_invoice_id.is_none());
    let bill_id = invoiced.payer_bill_id.expect("a payer bill");
    let bills = payer_bills(w.state());
    let lines = bills.lines(w.db(), PayerBillId(bill_id)).await.unwrap();
    let mut advice = Remittance::new("ORIENT-ADV-TL2", today());
    for line in &lines {
        advice = advice.line(RemittanceLine::paid(line.id, line.claimed));
    }
    bills
        .record_remittance(w.state(), PayerBillId(bill_id), advice, &s.aisha)
        .await
        .unwrap();
    // The remittance alone settles it: nothing else has to run.
    assert_eq!(
        InsuranceClaim::find_or_404(w.db(), job.claim.id)
            .await
            .unwrap()
            .status,
        ClaimStatus::Settled
    );
}

/// The customer's AED 200 share and the insurer's AED 1,200 are both paid:
/// only then is the claim settled (the settle runs on the last payment).
#[renox::test]
async fn aut_09_real_the_claim_settles_only_when_the_customers_share_is_paid_too() {
    let w = boot().await;
    let s = shop(&w, None).await;
    let job = approved_job(
        &w,
        &s,
        &[
            ("Strip-down", 4_000, 20_000),
            ("Body labour", 3_000, 20_000),
        ],
        &[0, 1],
    )
    .await;
    claims::declare_total_loss(w.state(), &s.carla, job.claim.id, NOTICE)
        .await
        .unwrap();
    claims::approve_charges(
        w.state(),
        &s.carla,
        job.claim.id,
        Answer::new("TL-1", "orient-charges-1")
            .labour(job.lines[0].id, 80_000)
            .labour(job.lines[1].id, 40_000),
    )
    .await
    .unwrap();
    let invoiced = claims::invoice(w.state(), &s.aisha, job.claim.id)
        .await
        .unwrap();
    let bill_id = invoiced.payer_bill_id.unwrap();
    let bills = payer_bills(w.state());
    let lines = bills.lines(w.db(), PayerBillId(bill_id)).await.unwrap();
    let mut advice = Remittance::new("ORIENT-ADV-TL3", today());
    for line in &lines {
        advice = advice.line(RemittanceLine::paid(line.id, line.claimed));
    }
    bills
        .record_remittance(w.state(), PayerBillId(bill_id), advice, &s.aisha)
        .await
        .unwrap();
    assert_eq!(
        InsuranceClaim::find_or_404(w.db(), job.claim.id)
            .await
            .unwrap()
            .status,
        ClaimStatus::Invoiced,
        "the insurer paid, the customer has not"
    );
    let customer_id = invoiced
        .customer_invoice_id
        .expect("the customer's invoice");
    let customer = invoicer(w.state())
        .find(w.db(), InvoiceId(customer_id))
        .await
        .unwrap()
        .unwrap();
    assert_ne!(
        customer.status,
        wl_kernel::invoicing::InvoiceStatus::Draft,
        "the accountant who invoiced the claim issued the customer's part"
    );
    invoicer(w.state())
        .allocate(
            w.state(),
            InvoiceId(customer_id),
            wl_kernel::Subject::new("payments.payment", 9_100),
            21_000,
        )
        .await
        .unwrap();
    // Settled by the last payment or by the settle check, once.
    claims::settle_if_paid(w.state(), job.claim.id)
        .await
        .unwrap();
    assert_eq!(
        InsuranceClaim::find_or_404(w.db(), job.claim.id)
            .await
            .unwrap()
            .status,
        ClaimStatus::Settled
    );
}

/// CL-12 `awaiting_survey` with JC-102 `checked_in`: the total loss declines the
/// job card on the real books and the real notifier: no invoice, no collect
/// message, the customer is told through the claim (#405 point 7).
#[renox::test]
async fn aut_09_real_a_total_loss_declines_a_job_card_with_no_work_started() {
    let w = boot().await;
    let s = shop(&w, None).await;
    let vehicle = vehicles::create(
        w.state(),
        &s.sami,
        NewVehicle::new(Emirate::Dubai, "A", "12345", "Toyota", "Hilux", s.owner.0)
            .vin("JTHBK262582012345")
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
            .workshop(support::autofx::workshop()),
    )
    .await
    .unwrap();
    let claim = claims::register(
        w.state(),
        &s.carla,
        claims::NewClaim::new(
            s.insurer.id,
            vehicle.id,
            "POL-7788",
            "OR-2026-118",
            today() - renox::chrono::TimeDelta::days(3),
        )
        .job_card(card.id)
        .police_report(
            "DXB-PR-5521",
            Some(today() - renox::chrono::TimeDelta::days(2)),
        )
        .excess(20_000),
    )
    .await
    .unwrap();
    police_report(&w, &s, &claim).await;
    claims::book_survey(w.state(), &s.carla, claim.id)
        .await
        .unwrap();

    claims::declare_total_loss(w.state(), &s.carla, claim.id, NOTICE)
        .await
        .unwrap();

    let declined = JobCard::find_or_404(w.db(), card.id).await.unwrap();
    assert_eq!(declined.status, JobCardStatus::Declined);
    assert_eq!(declined.closed_code.as_deref(), Some("total_loss"));
    assert!(declined.invoice_id.is_none());
    assert!(
        invoicer(w.state())
            .billing(w.db(), &declined.subject())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(sent_of(&w, auto::declines::COLLECT_TYPE).await.is_empty());
    assert_eq!(sent_of(&w, "auto.claim_total_loss").await.len(), 1);
}

/// The storage fee on and AED 30 a day agreed at check-in, the insurer's rate
/// AED 50: the release after the total loss makes no storage invoice on the
/// real books; the storage is a charge of the claim at the insurer's rate and
/// goes on the real payer bill (#405 point 8).
#[renox::test]
async fn aut_09_real_storage_after_a_total_loss_goes_on_the_claim_only() {
    let w = boot().await;
    let s = shop(&w, Some(5_000)).await;
    settings::set(w.db(), settings::STORAGE_FEE_ENABLED, "1", None)
        .await
        .unwrap();
    let job = approved_job(&w, &s, &[("Strip-down", 4_000, 20_000)], &[0]).await;
    JobCard::query()
        .where_(JobCard::ID.eq(job.card.id))
        .update(w.db(), &[("storage_fee_daily", &3_000_i64)])
        .await
        .unwrap();
    claims::declare_total_loss(w.state(), &s.carla, job.claim.id, NOTICE)
        .await
        .unwrap();
    let lines = claims::request_lines(w.db(), job.claim.id, ApprovalKind::Charges)
        .await
        .unwrap();
    let storage = lines.iter().find(|l| l.kind == "charge").expect("storage");
    assert_eq!(storage.price, 5_000, "one day at the insurer's rate");

    let released = release::release_with_instruction(
        w.state(),
        &s.sami,
        job.card.id,
        Release::as_is("Orient Insurance", "Samir"),
        Some("orient-release-letter-1"),
    )
    .await
    .unwrap();
    assert!(
        released.storage_invoice_id.is_none(),
        "no AUT.RUL.18 invoice"
    );
    assert!(
        invoicer(w.state())
            .billing(w.db(), &released.subject())
            .await
            .unwrap()
            .is_empty()
    );

    claims::approve_charges(
        w.state(),
        &s.carla,
        job.claim.id,
        Answer::new("TL-1", "orient-charges-1")
            .labour(job.lines[0].id, 80_000)
            .charge(5_000),
    )
    .await
    .unwrap();
    let invoiced = claims::invoice(w.state(), &s.aisha, job.claim.id)
        .await
        .unwrap();
    let summary = payer_bills(w.state())
        .find(w.db(), PayerBillId(invoiced.payer_bill_id.unwrap()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summary.claimed, 85_000, "labour and the storage");
}
