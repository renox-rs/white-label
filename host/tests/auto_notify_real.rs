//! The messages and invoices of the job card negative flows with the REAL
//! wl-notify, wl-invoicing, wl-parties and wl-scheduling of the host: the
//! collect SMS of a declined job card (#399 points 1 and 2, #401 points 1 and
//! 6), the cancel invoice of part-done and flat-rate labour with its event
//! (#397 point 2) and the inspection fee of a withdrawn claim (#404 point 6).

mod support;

use auto::claims::{self, NewClaim};
use auto::declines;
use auto::job_cards::NewLabour;
use auto::job_stops::{self, StopRequest};
use auto::release::{self, Release};
use auto::{JobCard, JobCardCancelled, JobCardStatus, WorkStatus, storage};
use host::roles::{SERVICE_ADVISOR, TECHNICIAN};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use support::autofx::{Recorded, boot, boot_with, inspected_card, record, sent_of, shop};
use support::tenant_with_mobile;
use wl_kernel::PartyId;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus, invoicer};

/// JC-30 with an inspection fee of AED 150 agreed, the estimate unanswered for
/// ten days: the hourly rule declines it, the customer with a mobile gets the
/// real `auto.collect_vehicle` SMS naming the plate, the job card and AED 157.50;
/// reminders follow at 3 and 7 days (#401 point 1); the vehicle is not released
/// while the fee invoice is unpaid and is released once paid (#401 point 6).
#[renox::test]
async fn aut_03_real_the_collect_sms_names_the_plate_the_job_and_the_fee_and_is_repeated() {
    let w = boot().await;
    let (_, sami) = w.staff("Sami", SERVICE_ADVISOR).await;
    let tareq = w.staff("Tareq", TECHNICIAN).await;
    let owner = tenant_with_mobile(&w.app, "Mona Hassan", "+971501234567").await;
    let card = inspected_card(
        &w,
        &sami,
        &tareq,
        owner,
        "12345",
        15_000,
        vec![NewLabour::hourly("Brake pads", 1_000, 20_000)],
        true,
    )
    .await;
    let entered = renox::db::now() - TimeDelta::days(10);
    JobCard::query()
        .where_(JobCard::ID.eq(card.id))
        .update(w.db(), &[("awaiting_since", &Some(entered))])
        .await
        .unwrap();

    let done = declines::expire_estimates(w.state(), entered + TimeDelta::days(7))
        .await
        .unwrap();
    assert_eq!(done.declined, [card.id]);

    let sent = sent_of(&w, declines::COLLECT_TYPE).await;
    assert_eq!(sent.len(), 1, "one collect message");
    let text = &sent[0].rendered_body;
    assert!(text.contains("12345"), "{text}");
    assert!(text.contains(&card.number()), "{text}");
    assert!(text.contains("157.50"), "{text}");
    assert_eq!(
        format!("{:?}", sent[0].channel),
        "Sms",
        "{:?} {:?}",
        sent[0].status,
        sent[0].address
    );

    // The reminders of AUT.RUL.18: day 3 and day 7, once each.
    let declined = JobCard::find_or_404(w.db(), card.id).await.unwrap();
    assert_eq!(declined.status, JobCardStatus::Declined);
    let start = storage::start_of(&declined).expect("declined");
    storage::process(w.state(), start + TimeDelta::days(2))
        .await
        .unwrap();
    assert_eq!(sent_of(&w, declines::COLLECT_TYPE).await.len(), 1, "day 2");
    storage::process(w.state(), start + TimeDelta::days(3))
        .await
        .unwrap();
    storage::process(w.state(), start + TimeDelta::days(3))
        .await
        .unwrap();
    let after_three = sent_of(&w, declines::COLLECT_TYPE).await;
    assert_eq!(after_three.len(), 2, "day 3, once");
    assert!(after_three[1].rendered_body.contains(&card.number()));
    storage::process(w.state(), start + TimeDelta::days(7))
        .await
        .unwrap();
    assert_eq!(sent_of(&w, declines::COLLECT_TYPE).await.len(), 3, "day 7");

    // Cash terms: the fee invoice is unpaid, so the vehicle stays.
    let invoice_id = declined.invoice_id.expect("the fee invoice");
    let unpaid = release::release(w.state(), &sami, card.id, Release::safe("Mona Hassan"))
        .await
        .unwrap_err();
    assert!(unpaid.to_string().contains("not paid"), "{unpaid}");
    let books = invoicer(w.state());
    let invoice = books
        .find(w.db(), InvoiceId(invoice_id))
        .await
        .unwrap()
        .unwrap();
    assert_ne!(invoice.status, InvoiceStatus::Draft);
    books
        .allocate(
            w.state(),
            InvoiceId(invoice_id),
            wl_kernel::Subject::new("payments.payment", 9_001),
            invoice.gross_total,
        )
        .await
        .unwrap();
    let released = release::release(w.state(), &sami, card.id, Release::safe("Mona Hassan"))
        .await
        .expect("paid: the vehicle is released");
    assert!(released.released_at.is_some());
    let before = sent_of(&w, declines::COLLECT_TYPE).await.len();
    storage::process(w.state(), start + TimeDelta::days(30))
        .await
        .unwrap();
    assert_eq!(
        sent_of(&w, declines::COLLECT_TYPE).await.len(),
        before,
        "a released vehicle is not reminded"
    );
}

/// JC-12: labour "service" at AED 150 an hour, 3 hours estimated, 2 hours done
/// (part done), and flat-rate "alignment" AED 120 for 1 flat hour with 0.5 hours
/// done: the cancel invoice on the real books is AED 300 + 60 = AED 360 + VAT,
/// and `JobCardCancelled` is emitted with the invoice (#397 points 1 and 2).
#[renox::test]
async fn aut_01_real_part_done_and_flat_rate_labour_is_invoiced_and_the_cancel_is_announced() {
    let cancelled = Recorded::<JobCardCancelled>::new();
    let w = boot_with(|app| record(app, &cancelled)).await;
    let (_, sami) = w.staff("Sami", SERVICE_ADVISOR).await;
    let tareq = w.staff("Tareq", TECHNICIAN).await;
    let owner = tenant_with_mobile(&w.app, "Mona Hassan", "+971501234567").await;
    let card = inspected_card(
        &w,
        &sami,
        &tareq,
        owner,
        "12345",
        0,
        vec![
            NewLabour::hourly("Service", 3_000, 15_000),
            NewLabour::flat("Alignment", 1_000, 12_000),
        ],
        true,
    )
    .await;
    let round = auto::job_cards::pending_round(w.db(), card.id)
        .await
        .unwrap()
        .unwrap();
    auto::job_cards::approve_by_link(w.state(), &round.token, "Mona Hassan", None, true)
        .await
        .unwrap();
    let lines = auto::LabourLine::query()
        .where_(auto::LabourLine::JOB_CARD_ID.eq(card.id))
        .order_by(auto::LabourLine::ID)
        .get(w.db())
        .await
        .unwrap();
    for (line, hours) in lines.iter().zip([2_000, 500]) {
        auto::job_cards::record_work(
            w.state(),
            &tareq.1,
            line.id,
            WorkStatus::PartDone,
            Some(hours),
        )
        .await
        .unwrap();
    }

    job_stops::cancel(
        w.state(),
        &sami,
        card.id,
        StopRequest::new("customer_stopped").signed("Mona Hassan"),
    )
    .await
    .expect("the cancel runs");

    let card = JobCard::find_or_404(w.db(), card.id).await.unwrap();
    assert_eq!(card.status, JobCardStatus::Cancelled);
    let invoice = invoicer(w.state())
        .find(w.db(), InvoiceId(card.invoice_id.expect("an invoice")))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(invoice.gross_total, 37_800, "AED 300 + 60 and 5% VAT");
    assert_eq!(invoice.party, owner);
    assert_ne!(invoice.status, InvoiceStatus::Draft);
    let events = cancelled.all();
    assert_eq!(events.len(), 1, "announced once");
    assert_eq!(events[0].job_card, card.subject());
    assert_eq!(events[0].reason_code, "customer_stopped");
    assert_eq!(events[0].billed, 36_000);
    assert_eq!(events[0].work_done_value, 36_000);
    assert_eq!(events[0].invoice, card.invoice_id);
}

/// CL-9 `estimate_submitted` with JC-90 `awaiting_approval`, no work, an
/// inspection fee of AED 150 agreed at check-in: the customer withdraws, the
/// job card is declined and the fee is invoiced to the customer on the real
/// books, never to the insurer, who is sent no payer bill (#404 point 6).
#[renox::test]
async fn aut_08_real_the_inspection_fee_of_a_withdrawn_claim_goes_to_the_customer() {
    let w = boot().await;
    let s = shop(&w, None).await;
    let tareq = (s.tareq_user.clone(), s.tareq.clone());
    let card = inspected_card(
        &w,
        &s.sami,
        &tareq,
        s.owner,
        "12345",
        15_000,
        vec![NewLabour::hourly("Brake pads", 1_000, 20_000)],
        true,
    )
    .await;
    let claim = claims::register(
        w.state(),
        &s.carla,
        NewClaim::new(
            s.insurer.id,
            card.vehicle_id,
            "POL-7788",
            "OR-2026-118",
            today() - TimeDelta::days(3),
        )
        .job_card(card.id)
        .police_report("DXB-PR-5521", Some(today() - TimeDelta::days(2)))
        .excess(20_000),
    )
    .await
    .unwrap();
    support::autofx::police_report(&w, &s, &claim).await;
    claims::submit_estimate(w.state(), &s.carla, claim.id)
        .await
        .unwrap();

    let cancelled = claims::cancel(
        w.state(),
        &s.carla,
        claim.id,
        "Withdrawn by the customer",
        None,
    )
    .await
    .unwrap();

    let declined = JobCard::find_or_404(w.db(), card.id).await.unwrap();
    assert_eq!(declined.status, JobCardStatus::Declined);
    let invoice = invoicer(w.state())
        .find(
            w.db(),
            InvoiceId(declined.invoice_id.expect("the fee invoice")),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        invoice.party,
        PartyId(s.owner.0),
        "the customer, not Orient"
    );
    assert_ne!(invoice.party, s.orient);
    assert_eq!(invoice.gross_total, 15_750, "AED 150 and 5% VAT");
    assert_ne!(invoice.status, InvoiceStatus::Draft);
    assert!(
        wl_kernel::payer_bills::payer_bills(w.state())
            .of_source(w.db(), &cancelled.subject())
            .await
            .unwrap()
            .is_empty()
    );
}
