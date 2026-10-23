//! BL-AUT-13 and BL-AUT-12 against the REAL shared modules of the host
//! (wl-scheduling, wl-invoicing, wl-parties, wl-notify, the kernel's
//! approvals): a suspended contract refuses a booking in the real scheduling,
//! a contract that ended or was terminated cancels its future real bookings,
//! drafts its last real invoice and the fee, tells the contact through the real
//! notifier, and the owner of the host's roles approves a termination, never
//! the fleet account manager who asked. The app's own tests
//! (`apps/auto/tests`) show the same against stand-ins.

mod support;

use std::time::Duration;

use auto::fleet_billing;
use auto::fleet_contract::FleetContractTerminated;
use auto::fleet_contracts;
use auto::fleet_daily::run_daily_on;
use auto::fleet_end::{NOTICE_TYPE, TERMINATION_TYPE};
use auto::fleet_invoice::{FleetInvoice, KIND_FINAL, KIND_MONTHLY};
use auto::{FleetContract, FleetService, FleetStatus, JobCard, JobCardStatus};
use renox::prelude::*;
use support::autofx::{
    FleetShop, Recorded, World, boot, boot_with, fleet_deliver, fleet_job, fleet_shop, record,
    sent_of, workshop,
};
use wl_kernel::PartyId;
use wl_kernel::approvals;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus, invoicer};
use wl_scheduling::service::{self, Actor, NewBooking};
use wl_scheduling::{Booking, BookingStatus, Refusal};

/// The day `n` days from today.
fn day(n: i64) -> renox::chrono::NaiveDate {
    today() + renox::chrono::TimeDelta::days(n)
}

/// Asia/Dubai `date` at `hour:00`, as an instant.
fn at(date: renox::chrono::NaiveDate, hour: u32) -> renox::db::DateTime {
    wl_kernel::calendar::BusinessCalendar::default()
        .from_local(date.and_hms_opt(hour, 0, 0).expect("a time"))
}

/// Runs `work` with the clock `days` days ahead of the real one.
async fn on_day<F: std::future::Future>(w: &World, days: u64, work: F) -> F::Output {
    w.app.travel_back();
    w.app.travel(Duration::from_secs(86_400 * days));
    w.app.at_travelled_time(work).await
}

/// A booking of the van on the real scheduling at `date` 10:00, given to the
/// plan as the daily run would have.
async fn plan_booking(w: &World, shop: &FleetShop, date: renox::chrono::NaiveDate) -> Booking {
    let booking = service::book(
        w.state(),
        NewBooking::new(shop.fleet, shop.periodic.id, &[shop.bay.id], at(date, 10))
            .for_subject(shop_van(shop))
            .at(workshop()),
        Actor::staff(shop.sami.id),
        renox::db::now(),
    )
    .await
    .expect("the booking is made");
    FleetService::query()
        .where_(FleetService::ID.eq(shop.plan.id))
        .update(
            w.db(),
            &[
                ("booking_id", &Some(booking.id)),
                ("requested_for_date", &Some(date)),
            ],
        )
        .await
        .unwrap();
    booking
}

/// The van as a booking subject.
fn shop_van(shop: &FleetShop) -> wl_kernel::Subject {
    shop.van.subject()
}

async fn contract_now(w: &World, id: i64) -> FleetContract {
    FleetContract::find_or_404(w.db(), id).await.unwrap()
}

/// FC suspended: the real scheduling refuses a new booking for its van in
/// words, the job card under it goes on to delivered and is not invoiced on
/// its own; resumed, the booking is made.
#[renox::test]
async fn aut_13_real_a_suspended_contract_refuses_a_booking_and_its_job_card_goes_on() {
    let w = boot().await;
    let shop = fleet_shop(&w, 50_100).await;
    let (card, line) = fleet_job(&w, &shop, 1_000).await;
    fleet_contracts::suspend(w.state(), &shop.fatima, shop.contract.id, "Unpaid invoices")
        .await
        .unwrap();

    let refused = service::book(
        w.state(),
        NewBooking::new(shop.fleet, shop.periodic.id, &[shop.bay.id], at(day(3), 10))
            .for_subject(shop_van(&shop))
            .at(workshop()),
        Actor::staff(shop.sami.id),
        renox::db::now(),
    )
    .await
    .unwrap_err();
    let Refusal::Invalid(text) = refused else {
        panic!("refused in words");
    };
    assert!(text.contains("suspended"), "{text}");
    assert_eq!(
        Booking::query()
            .where_(Booking::SUBJECT_ID.eq(Some(shop.van.id)))
            .count(w.db())
            .await
            .unwrap(),
        0
    );

    let delivered = fleet_deliver(&w, &shop, &card, &line).await;
    assert_eq!(delivered.status, JobCardStatus::Delivered);
    assert!(delivered.invoice_id.is_none());

    fleet_contracts::resume(w.state(), &shop.fatima, shop.contract.id)
        .await
        .unwrap();
    plan_booking(&w, &shop, day(3)).await;
}

/// FC ending yesterday: on the first run after it the real bookings after the
/// last day are cancelled, the last real invoice is drafted for the job
/// delivered, and the fleet contact is told through the real notifier, once.
#[renox::test]
async fn aut_13_real_a_contract_that_ended_cancels_its_bookings_drafts_its_invoice_and_tells_the_contact()
 {
    let w = boot().await;
    let shop = fleet_shop(&w, 50_100).await;
    let booking = plan_booking(&w, &shop, day(4)).await;
    let (card, line) = fleet_job(&w, &shop, 1_000).await;
    fleet_deliver(&w, &shop, &card, &line).await;
    FleetContract::query()
        .where_(FleetContract::ID.eq(shop.contract.id))
        .update(w.db(), &[("end_date", &day(-1))])
        .await
        .unwrap();

    // On its last day the contract is still in force: the run of that day
    // ends nothing (#409 point 2).
    FleetContract::query()
        .where_(FleetContract::ID.eq(shop.contract.id))
        .update(w.db(), &[("end_date", &today())])
        .await
        .unwrap();
    let early = run_daily_on(w.state(), today()).await.unwrap();
    assert!(early.ended.is_empty(), "{early:?}");
    assert_eq!(
        contract_now(&w, shop.contract.id).await.status,
        FleetStatus::RenewalDue,
        "in force on its last day (the renewal reminder only)"
    );
    assert_eq!(
        Booking::find_or_404(w.db(), booking.id)
            .await
            .unwrap()
            .status,
        BookingStatus::Booked
    );
    FleetContract::query()
        .where_(FleetContract::ID.eq(shop.contract.id))
        .update(w.db(), &[("end_date", &day(-1))])
        .await
        .unwrap();

    let run = run_daily_on(w.state(), today()).await.unwrap();
    assert_eq!(run.ended, [shop.contract.id], "{run:?}");

    let cancelled = Booking::find_or_404(w.db(), booking.id).await.unwrap();
    assert_eq!(cancelled.status, BookingStatus::Cancelled);
    assert_eq!(
        cancelled.cancel_code.as_deref(),
        Some("fleet_contract_ended")
    );
    let periods = FleetInvoice::of_contract(w.db(), shop.contract.id)
        .await
        .unwrap();
    let last = periods
        .iter()
        .find(|p| p.kind == KIND_FINAL)
        .expect("the last invoice");
    assert_eq!((last.jobs, last.net_total), (1, 12_000));
    let invoice = invoicer(w.state())
        .find(w.db(), InvoiceId(last.invoice_id.expect("an invoice")))
        .await
        .unwrap()
        .expect("the invoice exists");
    assert_eq!(invoice.party, PartyId(shop.fleet.0));
    assert_eq!(invoice.gross_total, 12_600, "AED 120 and 5% VAT");
    assert_eq!(invoice.status, InvoiceStatus::Draft, "drafted by the run");

    let mails = sent_of(&w, NOTICE_TYPE).await;
    assert_eq!(mails.len(), 1);
    assert_eq!(mails[0].party_id, Some(shop.contact.0));
    let again = run_daily_on(w.state(), today()).await.unwrap();
    assert!(again.ended.is_empty() && again.wound_down.is_empty());
    assert_eq!(sent_of(&w, NOTICE_TYPE).await.len(), 1);
}

/// The fleet account manager asks to terminate; she cannot approve it; the
/// owner of the host's roles does. The contract stays active until the run
/// after the effective day, which cancels the real booking after it, drafts the
/// final invoice (AED 120 and VAT) and invoices the AED 2,000 fee (AED 2,100
/// with VAT) to the fleet; the contact is told.
#[renox::test]
async fn aut_12_real_a_terminated_contract_cancels_bookings_and_invoices_the_period_and_the_fee() {
    let terminated = Recorded::<FleetContractTerminated>::new();
    let w = boot_with(|app| record(app, &terminated)).await;
    let shop = fleet_shop(&w, 50_100).await;
    let before = delivered(&w, &shop, 5).await;
    let monthly = fleet_billing::invoice_period(
        w.state(),
        Some(&shop.aisha),
        shop.contract.id,
        day(-30),
        day(-1),
        KIND_MONTHLY,
    )
    .await
    .unwrap();
    assert_eq!(monthly.net_total, 12_000);
    assert_eq!(before.status, JobCardStatus::Delivered);
    let after_card = delivered(&w, &shop, 0).await;
    let booking = plan_booking(&w, &shop, day(5)).await;
    // A booking on the effective day itself stays.
    let on_the_day = service::book(
        w.state(),
        NewBooking::new(shop.fleet, shop.periodic.id, &[shop.bay.id], at(day(3), 14))
            .for_subject(shop_van(&shop))
            .at(workshop()),
        Actor::staff(shop.sami.id),
        renox::db::now(),
    )
    .await
    .expect("the booking is made");

    let asked = fleet_contracts::terminate(
        w.state(),
        &shop.fatima,
        shop.contract.id,
        "Fleet sold",
        day(0),
        day(3),
        false,
    )
    .await
    .unwrap();
    let request = asked.approval_request.expect("the owner decides");
    assert_eq!(asked.contract.status, FleetStatus::Active);
    let own = approvals::approve(w.state(), request, &shop.fatima_user, "").await;
    assert!(own.is_err(), "the fleet account manager cannot approve");
    approvals::approve(w.state(), request, &shop.omar, "")
        .await
        .expect("the owner approves");
    let scheduled = contract_now(&w, shop.contract.id).await;
    assert_eq!(scheduled.status, FleetStatus::Active);
    assert!(scheduled.termination_scheduled);
    assert_eq!(scheduled.termination_approved_by, Some(shop.omar.id));

    let run = on_day(&w, 4, async { run_daily_on(w.state(), today()).await })
        .await
        .unwrap();
    assert_eq!(run.terminated, [shop.contract.id], "{run:?}");
    let ended = contract_now(&w, shop.contract.id).await;
    assert_eq!(ended.status, FleetStatus::Ended);

    let cancelled = Booking::find_or_404(w.db(), booking.id).await.unwrap();
    assert_eq!(cancelled.status, BookingStatus::Cancelled);
    assert_ne!(
        Booking::find_or_404(w.db(), on_the_day.id)
            .await
            .unwrap()
            .status,
        BookingStatus::Cancelled,
        "the booking on the effective day stays"
    );
    let events = terminated.all();
    assert_eq!(events.len(), 1, "FleetContractTerminated, once");

    let periods = FleetInvoice::of_contract(w.db(), shop.contract.id)
        .await
        .unwrap();
    let last = periods.iter().find(|p| p.kind == KIND_FINAL).unwrap();
    assert_eq!(
        (last.period_start_date, last.period_end_date),
        (day(0), day(3))
    );
    assert_eq!((last.jobs, last.net_total), (1, 12_000));
    assert_eq!(
        auto::FleetJob::of_job_card(w.db(), after_card.id)
            .await
            .unwrap()
            .unwrap()
            .invoice_id,
        last.invoice_id
    );
    let fee = invoicer(w.state())
        .find(
            w.db(),
            InvoiceId(
                ended
                    .termination_fee_invoice_id
                    .expect("the fee is invoiced"),
            ),
        )
        .await
        .unwrap()
        .expect("the fee invoice exists");
    assert_eq!(fee.party, PartyId(shop.fleet.0));
    assert_eq!(fee.gross_total, 210_000, "AED 2,000 and 5% VAT");
    assert_eq!(
        sent_of(&w, TERMINATION_TYPE).await.len(),
        1,
        "a terminated contract gets the statutory termination notice"
    );
    assert!(sent_of(&w, NOTICE_TYPE).await.is_empty());
}

/// JC-110 open when the contract ended finishes and is billed at the rates of
/// its lines on its own real invoice (2 h at AED 120 and VAT), not in the final
/// invoice; a fee the owner's approval waived is not invoiced.
#[renox::test]
async fn aut_12_real_an_open_job_card_finishes_and_is_billed_at_contract_rates() {
    let w = boot().await;
    let shop = fleet_shop(&w, 50_100).await;
    let (card, line) = fleet_job(&w, &shop, 2_000).await;
    let asked = fleet_contracts::terminate(
        w.state(),
        &shop.fatima,
        shop.contract.id,
        "Agreement",
        day(0),
        day(1),
        true,
    )
    .await
    .unwrap();
    approvals::approve(w.state(), asked.approval_request.unwrap(), &shop.omar, "")
        .await
        .unwrap();
    on_day(&w, 2, async { run_daily_on(w.state(), today()).await })
        .await
        .unwrap();
    let ended = contract_now(&w, shop.contract.id).await;
    assert_eq!(ended.status, FleetStatus::Ended);
    assert!(ended.termination_fee_waived);
    assert_eq!(ended.termination_fee_invoice_id, None);
    assert_eq!(
        JobCard::find_or_404(w.db(), card.id).await.unwrap().status,
        JobCardStatus::InProgress,
        "not stopped"
    );

    let delivered = on_day(&w, 2, fleet_deliver(&w, &shop, &card, &line)).await;
    assert_eq!(delivered.status, JobCardStatus::Delivered);
    let invoice = invoicer(w.state())
        .find(
            w.db(),
            InvoiceId(delivered.invoice_id.expect("billed on delivery")),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(invoice.party, PartyId(shop.fleet.0));
    assert_eq!(invoice.gross_total, 25_200, "2 h at AED 120 and 5% VAT");
}

/// A job card of the van delivered `days_ago` days ago.
async fn delivered(w: &World, shop: &FleetShop, days_ago: i64) -> JobCard {
    let (card, line) = fleet_job(w, shop, 1_000).await;
    let done = fleet_deliver(w, shop, &card, &line).await;
    let at = renox::db::now() - renox::chrono::TimeDelta::days(days_ago);
    JobCard::query()
        .where_(JobCard::ID.eq(done.id))
        .update(w.db(), &[("delivered_at", &Some(at))])
        .await
        .unwrap();
    JobCard::find_or_404(w.db(), done.id).await.unwrap()
}
