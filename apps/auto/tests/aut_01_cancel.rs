//! BL-AUT-01 (#397): cancel a job card after the work started and bill the work
//! done (`AUT.ACL.21`, 08 §7.1 *Job card* `cancel`), through the real
//! wl-scheduling for the booking and the stand-ins of the invoice book and the
//! stock. The same points against the real wl-invoicing and wl-inventory are in
//! `host/tests/auto_cancel_real.rs`.

mod support;

use auto::job_cards::{self, Intake, NewLabour, NewPart, Promise};
use auto::job_stops::{self, StopRequest};
use auto::release::{self, Release};
use auto::{JobCard, JobCardStatus, LabourLine, LabourPricing, LineStatus, PartLine, WorkStatus};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use support::{Garage, boot};
use wl_kernel::approvals::{self, ApprovalPolicy};
use wl_kernel::{PartyId, Subject};
use wl_scheduling::service::{self, Actor, NewBooking};
use wl_scheduling::{
    AvailabilityRule, Booking, BookingStatus, Resource, ResourceType, ServiceType, resource,
};

/// The workshop of the bookings.
fn workshop() -> Subject {
    Subject::new("scheduling.location", 1)
}

/// A bay that is open every day and a periodic service that needs check-in.
async fn bay_and_service(t: &Garage) -> (Resource, ServiceType) {
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
    (bay, service)
}

/// What JC-10 is: a job card `in_progress` of the Hilux, opened by the check-in
/// of its booking, with labour "Brake pads" done (AED 200), labour "Discs" not
/// done (AED 250), the brake pads fitted (AED 300) and an oil filter issued and
/// not fitted (AED 40).
struct Jc10 {
    card: JobCard,
    booking: Booking,
    pads_labour: LabourLine,
    discs_labour: LabourLine,
    pads: PartLine,
    filter: PartLine,
}

async fn jc10(t: &Garage) -> Jc10 {
    t.stock.with_store();
    t.stock.stock(11, "BRK-PAD", 5_000);
    t.stock.stock(12, "OIL-F", 3_000);
    let (bay, service) = bay_and_service(t).await;
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
    service::check_in(
        t.state(),
        booking.id,
        Actor::staff(sami.id),
        renox::db::now(),
    )
    .await
    .expect("the vehicle is checked in");
    let card = JobCard::of_booking(t.db(), booking.id)
        .await
        .unwrap()
        .expect("the check-in opened the job card");
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
            inspection_fee: 0,
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
    let pads_labour = job_cards::add_labour(
        t.state(),
        &tareq,
        card.id,
        NewLabour::hourly("Brake pads", 1_000, 20_000),
    )
    .await
    .unwrap();
    let discs_labour = job_cards::add_labour(
        t.state(),
        &tareq,
        card.id,
        NewLabour::hourly("Discs", 1_000, 25_000),
    )
    .await
    .unwrap();
    let pads = job_cards::add_part(
        t.state(),
        &tareq,
        card.id,
        NewPart::new("Brake pad set", 1_000, 30_000).item("BRK-PAD"),
    )
    .await
    .unwrap();
    let filter = job_cards::add_part(
        t.state(),
        &tareq,
        card.id,
        NewPart::new("Oil filter", 1_000, 4_000).item("OIL-F"),
    )
    .await
    .unwrap();
    job_cards::send_estimate(t.state(), &sami, card.id, Promise::default())
        .await
        .unwrap();
    let round = job_cards::pending_round(t.db(), card.id)
        .await
        .unwrap()
        .unwrap();
    job_cards::approve_by_link(t.state(), &round.token, "Mona Hassan", None, true)
        .await
        .unwrap();
    job_cards::record_work(t.state(), &tareq, pads_labour.id, WorkStatus::Done, None)
        .await
        .unwrap();
    auto::parts::issue(t.state(), &tareq, pads.id, None)
        .await
        .unwrap();
    auto::parts::fit(t.state(), &tareq, pads.id, None)
        .await
        .unwrap();
    auto::parts::issue(t.state(), &tareq, filter.id, None)
        .await
        .unwrap();
    let card = t.reload(card.id).await;
    assert_eq!(card.status, JobCardStatus::InProgress);
    let booking = Booking::find_or_404(t.db(), booking.id).await.unwrap();
    assert_eq!(booking.status, BookingStatus::CheckedIn);
    Jc10 {
        card,
        booking,
        pads_labour,
        discs_labour,
        pads,
        filter,
    }
}

/// The branch manager Basma: approves what the advisor may not decide.
async fn branch_manager(t: &Garage) -> User {
    t.user(
        "Basma",
        &[
            auto::permissions::JOB_CARDS_VIEW,
            auto::permissions::JOB_CARDS_APPROVE,
        ],
    )
    .await
}

/// A stopped job: the customer signs.
fn customer_stopped() -> StopRequest {
    StopRequest::new("customer_stopped").signed("Mona Hassan")
}

/// JC-10 is cancelled by advisor A with reason "customer stopped" and the
/// customer signs: `cancelled`, an invoice for AED 500 + VAT (the pads fitted,
/// the labour of the pads done, nothing for the discs), the filter back in
/// stock, the booking `cancelled` with the work done billed, and
/// `JobCardCancelled` emitted.
#[renox::test]
async fn aut_01_a_cancel_bills_the_work_done_and_closes_the_stock_and_the_booking() {
    let t = boot().await;
    let jc = jc10(&t).await;
    let sami = t.advisor().await;

    let stopped = job_stops::cancel(t.state(), &sami, jc.card.id, customer_stopped())
        .await
        .expect("the cancel runs");

    assert!(stopped.approval_request.is_none());
    let card = t.reload(jc.card.id).await;
    assert_eq!(card.status, JobCardStatus::Cancelled);
    assert_eq!(card.closed_code.as_deref(), Some("customer_stopped"));
    assert_eq!(card.ack_method.as_deref(), Some("signed"));
    assert_eq!(card.ack_name.as_deref(), Some("Mona Hassan"));
    assert_eq!((card.work_done_value, card.billed_net), (50_000, 50_000));
    assert!(!card.bill_lowered);
    assert!(card.closing_done_at.is_some());

    let invoices = t.books.about(&card.subject());
    assert_eq!(invoices.len(), 1, "one invoice for the work done");
    assert_eq!(invoices[0].net(), 50_000, "AED 500");
    assert_eq!(invoices[0].gross(), 52_500, "AED 500 and 5% VAT");
    assert!(invoices[0].issued, "issued, not left in draft");
    assert_eq!(invoices[0].party(), PartyId(7));
    assert_eq!(card.invoice_id, Some(invoices[0].id));
    assert!(
        invoices[0]
            .draft
            .lines
            .iter()
            .all(|line| !line.description.contains("Discs")),
        "the labour not done is not billed"
    );

    assert_eq!(t.stock.balance("OIL-F"), 3_000, "the filter is back");
    assert_eq!(
        t.stock.balance("BRK-PAD"),
        4_000,
        "the fitted pads stay out"
    );
    let returned = t.stock.returned();
    assert_eq!(returned.len(), 1);
    assert_eq!(returned[0].quantity_milli, 1_000);
    assert_eq!(returned[0].source, card.subject());
    assert_eq!(t.part_now(jc.filter.id).await.returned_milli, 1_000);
    assert_eq!(t.part_now(jc.pads.id).await.returned_milli, 0);
    assert!(t.stock.cleared().contains(&card.subject()));

    let booking = Booking::find_or_404(t.db(), jc.booking.id).await.unwrap();
    assert_eq!(booking.status, BookingStatus::Cancelled);
    assert_eq!(booking.cancel_code.as_deref(), Some("job_stopped"));
    assert_eq!(booking.work_done_amount, 52_500);
    assert!(card.booking_cancelled_at.is_some());

    let events = t.cancelled.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].job_card, card.subject());
    assert_eq!(events[0].billed, 50_000);
    assert_eq!(events[0].invoice, Some(invoices[0].id));
    assert_eq!(events[0].booking, Some(jc.booking.id));
}

/// JC-12: labour "service" at AED 150 an hour, 3 hours estimated, 2 done, and
/// flat-rate labour "alignment" AED 120 for 1 flat hour with 0.5 hours done:
/// the invoice is AED 300 + 60 = AED 360 + VAT.
#[renox::test]
async fn aut_01_labour_part_done_is_valued_at_the_hours_done_and_the_flat_price_pro_rata() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let card = t.card_in(&vehicle, JobCardStatus::InProgress).await;
    let sami = t.advisor().await;
    let service = t.approved_labour(&card, "Service", 3_000, 15_000).await;
    let mut alignment = LabourLine {
        job_card_id: card.id,
        description: "Alignment".to_owned(),
        pricing: LabourPricing::FlatRate,
        hours_milli: 1_000,
        rate: 12_000,
        approval: LineStatus::Approved,
        ..LabourLine::default()
    };
    alignment.save(t.db()).await.unwrap();

    let request = customer_stopped()
        .labour(service.id, WorkStatus::PartDone, 2_000)
        .labour(alignment.id, WorkStatus::PartDone, 500);
    job_stops::cancel(t.state(), &sami, card.id, request)
        .await
        .unwrap();

    let invoices = t.books.about(&card.subject());
    assert_eq!(invoices.len(), 1);
    assert_eq!(invoices[0].net(), 36_000, "AED 300 + AED 60");
    assert_eq!(invoices[0].gross(), 37_800, "AED 360 + VAT");
    let line = t.labour_now(alignment.id).await;
    assert_eq!(
        (line.work, line.hours_done_milli),
        (WorkStatus::PartDone, 500)
    );
    assert_eq!(line.done_value(), 6_000);
    // A flat-rate line is never valued above its flat price.
    assert_eq!(t.reload(card.id).await.work_done_value, 36_000);
}

/// A line "part done" needs hours between none and all: all the hours is
/// "done", more than the estimate or none at all is refused in words.
#[renox::test]
async fn aut_01_hours_part_done_must_be_between_none_and_all() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let card = t.card_in(&vehicle, JobCardStatus::InProgress).await;
    let sami = t.advisor().await;
    let line = t.approved_labour(&card, "Service", 3_000, 15_000).await;

    for hours in [0, 3_000, 9_000] {
        let refused = job_stops::cancel(
            t.state(),
            &sami,
            card.id,
            customer_stopped().labour(line.id, WorkStatus::PartDone, hours),
        )
        .await
        .unwrap_err();
        assert!(refused.to_string().contains("part done"), "{refused}");
    }
    assert_eq!(t.reload(card.id).await.status, JobCardStatus::InProgress);
}

/// A lowered bill: A bills AED 300 instead of AED 500, an approval request goes
/// to branch manager B and JC-10 stays `in_progress` (nothing invoiced, nothing
/// returned, the booking open); on B's approval the cancel completes as above
/// with AED 300. A cannot approve the request.
#[renox::test]
async fn aut_01_a_lowered_bill_waits_for_the_branch_manager_who_is_not_the_advisor() {
    let t = boot().await;
    let jc = jc10(&t).await;
    let sami_user = t.advisor_user().await;
    let sami = t.auth(&sami_user).await;
    let basma = branch_manager(&t).await;

    let asked = job_stops::cancel(
        t.state(),
        &sami,
        jc.card.id,
        customer_stopped().bill(30_000),
    )
    .await
    .expect("the cancel is asked for");

    let request = asked.approval_request.expect("an approval request waits");
    assert_eq!(asked.card.status, JobCardStatus::InProgress);
    assert!(t.books.all().is_empty(), "nothing is invoiced yet");
    assert!(t.stock.returned().is_empty(), "nothing is returned yet");
    let booking = Booking::find_or_404(t.db(), jc.booking.id).await.unwrap();
    assert_eq!(booking.status, BookingStatus::CheckedIn);
    assert!(t.cancelled.all().is_empty());
    let waiting = approvals::awaiting(t.db(), &basma).await.unwrap();
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].action, "auto.job_card_cancel");
    assert_eq!(waiting[0].subject_id, jc.card.id);

    // A cannot approve it, and neither can a user without the right.
    let own = approvals::approve(t.state(), request, &sami_user, "").await;
    assert!(
        matches!(own, Err(Error::Forbidden)),
        "the advisor never approves"
    );
    let tareq_user = t.technician_user("Tareq").await;
    let other = approvals::approve(t.state(), request, &tareq_user, "").await;
    assert!(matches!(other, Err(Error::Forbidden)));
    assert_eq!(t.reload(jc.card.id).await.status, JobCardStatus::InProgress);

    approvals::approve(t.state(), request, &basma, "")
        .await
        .expect("the branch manager approves");

    let card = t.reload(jc.card.id).await;
    assert_eq!(card.status, JobCardStatus::Cancelled);
    assert!(card.bill_lowered);
    assert_eq!((card.work_done_value, card.billed_net), (50_000, 30_000));
    let invoices = t.books.about(&card.subject());
    assert_eq!(invoices.len(), 1);
    assert_eq!(invoices[0].net(), 30_000, "AED 300");
    assert_eq!(invoices[0].gross(), 31_500, "AED 300 and 5% VAT");
    assert_eq!(t.stock.balance("OIL-F"), 3_000, "the filter is back");
    let booking = Booking::find_or_404(t.db(), jc.booking.id).await.unwrap();
    assert_eq!(booking.status, BookingStatus::Cancelled);
    assert_eq!(t.cancelled.all().len(), 1);
    assert_eq!(t.cancelled.all()[0].billed, 30_000);
}

/// Billing even AED 0.01 less than the work done needs the approval; billing
/// more than it is refused in words, and so is a second cancel while one
/// waits.
#[renox::test]
async fn aut_01_billing_even_a_fil_less_needs_approval_and_more_is_refused() {
    let t = boot().await;
    let jc = jc10(&t).await;
    let sami = t.advisor().await;
    let basma = branch_manager(&t).await;

    let over = job_stops::cancel(
        t.state(),
        &sami,
        jc.card.id,
        customer_stopped().bill(50_001),
    )
    .await
    .unwrap_err();
    assert!(over.to_string().contains("work done"), "{over}");
    let negative = job_stops::cancel(t.state(), &sami, jc.card.id, customer_stopped().bill(-1))
        .await
        .unwrap_err();
    assert!(negative.to_string().contains("below 0"), "{negative}");

    let waits = job_stops::cancel(
        t.state(),
        &sami,
        jc.card.id,
        customer_stopped().bill(49_999),
    )
    .await
    .unwrap();
    assert!(waits.approval_request.is_some(), "a fil less is lowered");
    let again = job_stops::cancel(t.state(), &sami, jc.card.id, customer_stopped().bill(0))
        .await
        .unwrap_err();
    assert!(again.to_string().contains("already waits"), "{again}");

    // The branch manager approves AED 499.99.
    approvals::approve(t.state(), waits.approval_request.unwrap(), &basma, "")
        .await
        .unwrap();
    let card = t.reload(jc.card.id).await;
    assert_eq!(card.status, JobCardStatus::Cancelled);
    assert_eq!(card.billed_net, 49_999);
    let invoices = t.books.about(&card.subject());
    assert_eq!(invoices[0].net(), 49_999);
}

/// A waived bill (nothing billed) is approved too; no invoice is made, the
/// parts go back and the booking is cancelled.
#[renox::test]
async fn aut_01_a_waived_bill_is_approved_and_invoices_nothing() {
    let t = boot().await;
    let jc = jc10(&t).await;
    let sami = t.advisor().await;
    let basma = branch_manager(&t).await;

    let asked = job_stops::cancel(t.state(), &sami, jc.card.id, customer_stopped().bill(0))
        .await
        .unwrap();
    approvals::approve(t.state(), asked.approval_request.unwrap(), &basma, "")
        .await
        .unwrap();

    let card = t.reload(jc.card.id).await;
    assert_eq!(card.status, JobCardStatus::Cancelled);
    assert_eq!(card.billed_net, 0);
    assert!(t.books.all().is_empty(), "a waived bill has no invoice");
    assert_eq!(card.invoice_id, None);
    assert_eq!(t.stock.balance("OIL-F"), 3_000);
    let booking = Booking::find_or_404(t.db(), jc.booking.id).await.unwrap();
    assert_eq!(booking.status, BookingStatus::Cancelled);
    assert_eq!(booking.work_done_amount, 0);
}

/// While the cancel waits for the branch manager the technician's job card
/// refuses new labour, new parts and new work; when the request is rejected the
/// job card goes on as it was and takes labour again.
#[renox::test]
async fn aut_01_a_waiting_cancel_stops_new_labour_and_parts_until_it_is_decided() {
    let t = boot().await;
    let jc = jc10(&t).await;
    let sami = t.advisor().await;
    let tareq = t.technician("Tareq").await;
    let basma = branch_manager(&t).await;

    let asked = job_stops::cancel(
        t.state(),
        &sami,
        jc.card.id,
        customer_stopped().bill(10_000),
    )
    .await
    .unwrap();
    assert!(job_stops::cancel_pending(t.db(), jc.card.id).await.unwrap());

    let labour = job_cards::add_labour(
        t.state(),
        &tareq,
        jc.card.id,
        NewLabour::hourly("Wheel alignment", 1_000, 10_000),
    )
    .await
    .unwrap_err();
    assert!(
        labour.to_string().contains("waits for the branch manager"),
        "{labour}"
    );
    let part = job_cards::add_part(
        t.state(),
        &tareq,
        jc.card.id,
        NewPart::new("Wiper", 1_000, 2_000),
    )
    .await
    .unwrap_err();
    assert!(
        part.to_string().contains("waits for the branch manager"),
        "{part}"
    );
    let work = job_cards::record_work(
        t.state(),
        &tareq,
        jc.discs_labour.id,
        WorkStatus::Done,
        None,
    )
    .await
    .unwrap_err();
    assert!(
        work.to_string().contains("waits for the branch manager"),
        "{work}"
    );
    let issue = auto::parts::issue(t.state(), &tareq, jc.pads.id, Some(0))
        .await
        .unwrap_err();
    assert!(
        issue.to_string().contains("waits for the branch manager"),
        "{issue}"
    );

    // The page tells the technician and offers no second cancel.
    let sami_user = t.advisor_user().await;
    t.app.acting_as(&sami_user);
    t.app
        .get(&format!("/auto/job-cards/{}", jc.card.id))
        .await
        .assert_ok()
        .assert_see("waits for the branch manager")
        .assert_dont_see("Cancel the job");

    approvals::reject(
        t.state(),
        asked.approval_request.unwrap(),
        &basma,
        "Bill the work done",
    )
    .await
    .unwrap();
    assert!(!job_stops::cancel_pending(t.db(), jc.card.id).await.unwrap());
    let card = t.reload(jc.card.id).await;
    assert_eq!(card.status, JobCardStatus::InProgress, "it stays as it was");
    assert!(t.books.all().is_empty(), "nothing is invoiced");
    assert!(t.stock.returned().is_empty(), "nothing is returned");
    assert_eq!(
        t.labour_now(jc.discs_labour.id).await.work,
        WorkStatus::NotDone,
        "the work done is not written until the cancel runs"
    );
    job_cards::add_labour(
        t.state(),
        &tareq,
        jc.card.id,
        NewLabour::hourly("Wheel alignment", 1_000, 10_000),
    )
    .await
    .expect("the technician can add labour again");
}

/// The customer on cash terms with the invoice unpaid: the vehicle is not
/// released; after the payment it is. Released as is needs the signature.
#[renox::test]
async fn aut_01_the_vehicle_is_released_when_paid_or_on_account_and_as_is_with_a_signature() {
    let t = boot().await;
    let jc = jc10(&t).await;
    let sami = t.advisor().await;
    job_stops::cancel(t.state(), &sami, jc.card.id, customer_stopped())
        .await
        .unwrap();
    let invoice = t.books.about(&t.reload(jc.card.id).await.subject())[0].clone();

    let unpaid = release::release(t.state(), &sami, jc.card.id, Release::safe("Mona Hassan"))
        .await
        .unwrap_err();
    assert!(unpaid.to_string().contains("not paid"), "{unpaid}");
    assert!(unpaid.to_string().contains("AED 525.00"), "{unpaid}");
    assert!(t.reload(jc.card.id).await.released_at.is_none());

    // As is without the customer's signature: refused, paid or not.
    let unsigned = release::release(
        t.state(),
        &sami,
        jc.card.id,
        Release {
            released_to: "Mona Hassan".to_owned(),
            condition: "as_is".to_owned(),
            signature: None,
        },
    )
    .await
    .unwrap_err();
    assert!(unsigned.to_string().contains("signature"), "{unsigned}");

    t.books.pay(invoice.id, invoice.gross());
    let blank = release::release(t.state(), &sami, jc.card.id, Release::safe(" "))
        .await
        .unwrap_err();
    assert!(blank.to_string().contains("who takes"), "{blank}");
    let released = release::release(
        t.state(),
        &sami,
        jc.card.id,
        Release::as_is("Mona Hassan", "M. Hassan"),
    )
    .await
    .expect("paid: released as is with the signature");
    assert!(released.released_at.is_some());
    assert_eq!(released.release_condition.as_deref(), Some("as_is"));
    assert_eq!(released.release_signature.as_deref(), Some("M. Hassan"));
    assert_eq!(released.released_by, Some(sami.id));
    let twice = release::release(t.state(), &sami, jc.card.id, Release::safe("Mona Hassan"))
        .await
        .unwrap_err();
    assert!(twice.to_string().contains("already"), "{twice}");
}

/// A customer on account takes the vehicle unpaid; only a holder of
/// `job_cards.approve` says so.
#[renox::test]
async fn aut_01_a_customer_on_account_takes_the_vehicle_unpaid() {
    let t = boot().await;
    let jc = jc10(&t).await;
    let sami = t.advisor().await;
    let basma_user = branch_manager(&t).await;
    let basma = t.auth(&basma_user).await;
    job_stops::cancel(t.state(), &sami, jc.card.id, customer_stopped())
        .await
        .unwrap();

    let denied = release::set_on_account(t.state(), &sami, jc.card.id, true)
        .await
        .unwrap_err();
    assert!(matches!(denied, Error::Forbidden));
    release::set_on_account(t.state(), &basma, jc.card.id, true)
        .await
        .unwrap();
    let sami = t.advisor().await;
    let released = release::release(t.state(), &sami, jc.card.id, Release::safe("Mona Hassan"))
        .await
        .expect("on account: released unpaid");
    assert!(released.released_at.is_some());
}

/// A declined or cancelled job card only: a job card in progress is not
/// released.
#[renox::test]
async fn aut_01_only_a_declined_or_cancelled_job_card_is_released() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let sami = t.advisor().await;
    let open = t.card_in(&vehicle, JobCardStatus::InProgress).await;
    let refused = release::release(t.state(), &sami, open.id, Release::safe("Mona Hassan"))
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("declined or cancelled"),
        "{refused}"
    );
}

/// JC-11 `checked_in`: `cancel` is refused in words and the page does not
/// offer it; `ready` is delivered, not cancelled; a closed job card takes no
/// cancel.
#[renox::test]
async fn aut_01_a_job_card_that_was_just_checked_in_ready_or_closed_is_not_cancelled() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let sami = t.advisor().await;
    let checked_in = t.card_in(&vehicle, JobCardStatus::CheckedIn).await;
    let refused = job_stops::cancel(t.state(), &sami, checked_in.id, customer_stopped())
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("void"), "{refused}");
    assert!(refused.to_string().contains("decline"), "{refused}");
    t.app.acting_as(&t.advisor_user().await);
    t.app
        .get(&format!("/auto/job-cards/{}", checked_in.id))
        .await
        .assert_ok()
        .assert_dont_see("Cancel the job");
    let ready = t.card_in(&vehicle, JobCardStatus::Ready).await;
    let refused = job_stops::cancel(t.state(), &sami, ready.id, customer_stopped())
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("deliver"), "{refused}");
    let declined = t.card_in(&vehicle, JobCardStatus::Declined).await;
    let refused = job_stops::cancel(t.state(), &sami, declined.id, customer_stopped())
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("over"), "{refused}");
    assert_eq!(
        t.reload(checked_in.id).await.status,
        JobCardStatus::CheckedIn
    );
}

/// A customer who refuses to sign: A records "refused to sign" with witness W;
/// the cancel waits for branch manager B's approval, and runs when B approves.
/// A refusal without a witness is refused in words.
#[renox::test]
async fn aut_01_a_customer_who_refuses_to_sign_needs_a_witness_and_the_branch_manager() {
    let t = boot().await;
    let jc = jc10(&t).await;
    let sami = t.advisor().await;
    let basma = branch_manager(&t).await;

    let no_witness = job_stops::cancel(
        t.state(),
        &sami,
        jc.card.id,
        StopRequest {
            ack_method: "refused_to_sign".to_owned(),
            ..StopRequest::new("customer_stopped")
        },
    )
    .await
    .unwrap_err();
    assert!(no_witness.to_string().contains("witnessed"), "{no_witness}");

    let asked = job_stops::cancel(
        t.state(),
        &sami,
        jc.card.id,
        StopRequest::new("customer_stopped").refused("Waleed"),
    )
    .await
    .unwrap();
    let request = asked.approval_request.expect("it waits for approval");
    assert_eq!(asked.card.status, JobCardStatus::InProgress);
    assert!(t.books.all().is_empty());

    approvals::approve(t.state(), request, &basma, "")
        .await
        .unwrap();
    let card = t.reload(jc.card.id).await;
    assert_eq!(card.status, JobCardStatus::Cancelled);
    assert_eq!(card.ack_method.as_deref(), Some("refused_to_sign"));
    assert_eq!(card.ack_witness.as_deref(), Some("Waleed"));
    assert_eq!(
        t.books.about(&card.subject())[0].net(),
        50_000,
        "billed in full"
    );
}

/// The insurer's notice or the customer's written withdrawal replaces the
/// signature, and needs its reference.
#[renox::test]
async fn aut_01_a_written_withdrawal_replaces_the_signature() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let sami = t.advisor().await;
    let card = t.card_in(&vehicle, JobCardStatus::InProgress).await;
    let none = job_stops::cancel(
        t.state(),
        &sami,
        card.id,
        StopRequest {
            ack_method: "written_withdrawal".to_owned(),
            ..StopRequest::new("customer_stopped")
        },
    )
    .await
    .unwrap_err();
    assert!(none.to_string().contains("reference"), "{none}");
    let nothing = job_stops::cancel(
        t.state(),
        &sami,
        card.id,
        StopRequest::new("customer_stopped"),
    )
    .await
    .unwrap_err();
    assert!(nothing.to_string().contains("acknowledged"), "{nothing}");
    let stopped = job_stops::cancel(
        t.state(),
        &sami,
        card.id,
        StopRequest::new("customer_stopped").withdrawal("E-mail of 12 November"),
    )
    .await
    .unwrap();
    assert_eq!(
        stopped.card.ack_document.as_deref(),
        Some("E-mail of 12 November")
    );
}

/// Only the service advisor cancels, with a reason from the list; a job funded
/// by an open claim is stopped through the claim.
#[renox::test]
async fn aut_01_only_the_advisor_cancels_with_a_reason_from_the_list() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let card = t.card_in(&vehicle, JobCardStatus::InProgress).await;
    let sami = t.advisor().await;
    let tareq = t.technician("Tareq").await;
    let denied = job_stops::cancel(t.state(), &tareq, card.id, customer_stopped())
        .await
        .unwrap_err();
    assert!(matches!(denied, Error::Forbidden));
    let unknown = job_stops::cancel(
        t.state(),
        &sami,
        card.id,
        StopRequest::new("bored").signed("Mona Hassan"),
    )
    .await
    .unwrap_err();
    assert!(unknown.to_string().contains("Pick why"), "{unknown}");
    let total_loss = job_stops::cancel(
        t.state(),
        &sami,
        card.id,
        StopRequest::new("total_loss").signed("Mona Hassan"),
    )
    .await
    .unwrap_err();
    assert!(
        total_loss.to_string().contains("Pick why"),
        "the claim ends a total loss"
    );
    assert_eq!(t.reload(card.id).await.status, JobCardStatus::InProgress);
}

/// Without the approval policy a lowered bill is not let through unapproved.
#[renox::test]
async fn aut_01_a_policy_taken_away_cannot_let_a_lowered_bill_through() {
    let t = boot().await;
    let jc = jc10(&t).await;
    let sami = t.advisor().await;
    ApprovalPolicy::query()
        .where_(ApprovalPolicy::ACTION.eq("auto.job_card_cancel"))
        .delete(t.db())
        .await
        .unwrap();

    let refused = job_stops::cancel(
        t.state(),
        &sami,
        jc.card.id,
        customer_stopped().bill(30_000),
    )
    .await
    .unwrap_err();
    assert!(refused.to_string().contains("approval"), "{refused}");
    assert_eq!(t.reload(jc.card.id).await.status, JobCardStatus::InProgress);
    // Billing the work done in full needs none.
    job_stops::cancel(t.state(), &sami, jc.card.id, customer_stopped())
        .await
        .expect("no approval is needed to bill the work done");
}

/// Nothing done yet (`inspecting`): the cancel is allowed, the work done is
/// none, and the inspection fee agreed at check-in is invoiced; without a fee
/// nothing is invoiced.
#[renox::test]
async fn aut_01_nothing_done_yet_bills_the_inspection_fee_only_when_it_was_agreed() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let sami = t.advisor().await;
    let mut with_fee = t.card_in(&vehicle, JobCardStatus::Inspecting).await;
    JobCard::query()
        .where_(JobCard::ID.eq(with_fee.id))
        .update(t.db(), &[("inspection_fee", &15_000_i64)])
        .await
        .unwrap();
    with_fee = t.reload(with_fee.id).await;
    job_stops::cancel(t.state(), &sami, with_fee.id, customer_stopped())
        .await
        .unwrap();
    let invoices = t.books.about(&with_fee.subject());
    assert_eq!(invoices.len(), 1);
    assert_eq!(invoices[0].net(), 15_000, "AED 150");
    assert_eq!(invoices[0].gross(), 15_750);

    let free = t.card_in(&vehicle, JobCardStatus::Inspecting).await;
    job_stops::cancel(t.state(), &sami, free.id, customer_stopped())
        .await
        .unwrap();
    assert!(
        t.books.about(&free.subject()).is_empty(),
        "no fee, no invoice"
    );
    assert_eq!(t.reload(free.id).await.status, JobCardStatus::Cancelled);
}

/// An open supplier PO for a special-order part on JC-10: the part the
/// customer agreed to is billed and handed over, not given back, and the
/// purchaser has a task to revise the purchase order.
#[renox::test]
async fn aut_01_a_special_order_part_is_billed_handed_over_and_the_purchaser_gets_a_task() {
    let t = boot().await;
    let jc = jc10(&t).await;
    let sami = t.advisor().await;
    let mut gearbox = PartLine {
        job_card_id: jc.card.id,
        description: "Gearbox".to_owned(),
        quantity_milli: 1_000,
        unit_price: 90_000,
        special_order: true,
        special_order_agreed: true,
        approval: LineStatus::Approved,
        ..PartLine::default()
    };
    gearbox.save(t.db()).await.unwrap();
    let linked = job_stops::link_purchase_order(t.state(), &sami, gearbox.id, Some(900))
        .await
        .unwrap();
    assert_eq!(linked.purchase_order_id, Some(900));
    let not_special = job_stops::link_purchase_order(t.state(), &sami, jc.pads.id, Some(1))
        .await
        .unwrap_err();
    assert!(
        not_special.to_string().contains("special-order"),
        "{not_special}"
    );

    job_stops::cancel(t.state(), &sami, jc.card.id, customer_stopped())
        .await
        .unwrap();

    let card = t.reload(jc.card.id).await;
    assert_eq!(
        card.work_done_value, 140_000,
        "AED 500 and the gearbox AED 900"
    );
    assert_eq!(t.books.about(&card.subject())[0].net(), 140_000);
    let gearbox = t.part_now(gearbox.id).await;
    assert!(
        gearbox.handed_over_at.is_some(),
        "handed over with the vehicle"
    );
    assert_eq!(gearbox.returned_milli, 0);
    let order = Subject::new("inventory.purchase_order", 900);
    let tasks = t.tickets.about(&order);
    assert_eq!(
        tasks.len(),
        1,
        "the purchaser has a task to revise the order"
    );
    assert_eq!(
        tasks[0].ticket.source_key.as_deref(),
        Some(format!("auto.po_revise:{}:{}", card.id, gearbox.id).as_str())
    );
    assert!(tasks[0].ticket.title.contains("Gearbox"));
}

/// A claim withdrawn or a total loss cancels as the system with the insurer's
/// notice or the written withdrawal; for a total loss the job card issues no
/// invoice of its own (the claim invoices the work done).
#[renox::test]
async fn aut_01_the_system_cancels_for_a_claim_without_a_signature() {
    let t = boot().await;
    let jc = jc10(&t).await;
    let withdrawn =
        job_stops::cancel_by_system(t.state(), jc.card.id, "claim_withdrawn", "Letter of 3 Nov")
            .await
            .unwrap();
    assert_eq!(withdrawn.status, JobCardStatus::Cancelled);
    assert_eq!(withdrawn.ack_method.as_deref(), Some("written_withdrawal"));
    assert_eq!(t.books.about(&withdrawn.subject())[0].net(), 50_000);

    let vehicle = t.hilux().await;
    let total = t.card_in(&vehicle, JobCardStatus::InProgress).await;
    t.approved_labour(&total, "Panel beating", 2_000, 20_000)
        .await;
    let lost = job_stops::cancel_by_system(t.state(), total.id, "total_loss", "Insurer notice 88")
        .await
        .unwrap();
    assert_eq!(lost.status, JobCardStatus::Cancelled);
    assert_eq!(lost.ack_method.as_deref(), Some("insurer_notice"));
    assert!(
        t.books.about(&lost.subject()).is_empty(),
        "the claim invoices"
    );
    let bad = job_stops::cancel_by_system(t.state(), total.id, "customer_stopped", "x")
        .await
        .unwrap_err();
    assert!(bad.to_string().contains("over") || bad.to_string().contains("Pick why"));
}

/// The cancelled job card page shows how the job stopped: the reason, the
/// acknowledgement, the work done and what was billed; the cancel is offered
/// to the advisor with a field per approved line.
#[renox::test]
async fn aut_01_the_page_offers_the_cancel_and_shows_how_the_job_stopped() {
    let t = boot().await;
    let jc = jc10(&t).await;
    let sami_user = t.advisor_user().await;
    t.app.acting_as(&sami_user);
    let base = format!("/auto/job-cards/{}", jc.card.id);

    t.app
        .get(&base)
        .await
        .assert_ok()
        .assert_see("Cancel the job");

    // A refusal comes back in words inside the form.
    let refused = t
        .app
        .htmx()
        .post(&format!("{base}/cancel"), &[("code", "customer_stopped")])
        .await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("acknowledged"),
        "{}",
        refused.text()
    );

    let pads_work = format!("work_{}", jc.pads_labour.id);
    let discs_work = format!("work_{}", jc.discs_labour.id);
    t.app
        .htmx()
        .post(
            &format!("{base}/cancel"),
            &[
                ("code", "customer_stopped"),
                ("ack", "signed"),
                ("ack_name", "Mona Hassan"),
                (pads_work.as_str(), "done"),
                (discs_work.as_str(), "not_done"),
            ],
        )
        .await
        .assert_ok();
    assert_eq!(t.reload(jc.card.id).await.status, JobCardStatus::Cancelled);

    t.app
        .get(&base)
        .await
        .assert_ok()
        .assert_see("How the job stopped")
        .assert_see("The customer stopped the job")
        .assert_see("Signed by the customer")
        .assert_see("AED 500.00")
        .assert_see("Release the vehicle");

    // The release form refuses an unpaid invoice in place.
    let unpaid = t
        .app
        .htmx()
        .post(
            &format!("{base}/release"),
            &[("released_to", "Mona Hassan"), ("condition", "safe")],
        )
        .await;
    unpaid.assert_status(422);
    assert!(unpaid.text().contains("not paid"), "{}", unpaid.text());
}
