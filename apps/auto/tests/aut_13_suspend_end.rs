//! BL-AUT-13 (#409): suspend and end a fleet contract with its bookings and
//! invoice (08 §7.1 *Fleet contract* `suspend`, `resume`, `end`, §7.2
//! `AUT.RUL.04`). A suspended contract refuses a new booking for its vehicles
//! (the app's `BookingGate`, asked by the real wl-scheduling), keeps the
//! bookings already made and lists them for the advisor to confirm or cancel,
//! and lets its open job cards finish; a contract ends on the first daily run
//! after its end date, never by a user: its future bookings are cancelled, the
//! last consolidated invoice is drafted and the fleet contact is told.

mod support;

use auto::fleet_contract_machine::{END, FleetInput};
use auto::fleet_contracts;
use auto::fleet_daily::run_daily_on;
use auto::fleet_end::NOTICE_TYPE;
use auto::fleet_invoice::{FleetInvoice, KIND_FINAL};
use auto::fleet_services;
use auto::{FleetContract, FleetService, FleetStatus, JobCardStatus};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use support::fleetfx::{
    CONTACT, FLEET, active_contract, bay_and_service, fatima, finish_and_deliver, job_in_progress,
    workshop,
};
use support::{at, boot, day};
use wl_kernel::PartyId;
use wl_kernel::documents::today;
use wl_kernel::notify::NotifyChannel;
use wl_scheduling::service::{self, Actor, NewBooking};
use wl_scheduling::{Booking, BookingStatus, Refusal};

async fn contract_now(t: &support::Garage, id: i64) -> FleetContract {
    FleetContract::find_or_404(t.db(), id).await.unwrap()
}

/// Given FC suspended for unpaid invoices, a new booking for its vehicle is
/// refused in words, `AUT.RUL.04` asks for none, and the job card under it
/// continues to `delivered`; once the contract is resumed the booking is made.
#[renox::test]
async fn aut_13_a_suspended_contract_refuses_a_new_booking_and_its_job_card_continues_to_delivered()
{
    let t = boot().await;
    let (bay, service_type) = bay_and_service(&t).await;
    let (contract, van, _plan) = active_contract(&t, 59_800).await;
    let fatima = fatima(&t).await;
    let sami = t.advisor().await;
    let (card, line) = job_in_progress(&t, &van, 1_000, 12_000).await;

    let suspended = fleet_contracts::suspend(t.state(), &fatima, contract.id, "Unpaid invoices")
        .await
        .unwrap();
    assert_eq!(suspended.status, FleetStatus::Suspended);

    let new_booking = || {
        NewBooking::new(
            PartyId(FLEET),
            service_type.id,
            &[bay.id],
            at(day(3), 10, 0),
        )
        .for_subject(van.subject())
        .at(workshop())
    };
    let refusal = service::book(
        t.state(),
        new_booking(),
        Actor::staff(sami.id),
        renox::db::now(),
    )
    .await
    .unwrap_err();
    let text = match refusal {
        Refusal::Invalid(text) => text,
        other => panic!("the booking is refused in words: {other:?}"),
    };
    assert!(text.contains("suspended"), "{text}");
    assert!(text.contains(&contract.number()), "{text}");
    let made = Booking::query()
        .where_(Booking::SUBJECT_ID.eq(Some(van.id)))
        .count(t.db())
        .await
        .unwrap();
    assert_eq!(made, 0, "nothing was booked");

    // `AUT.RUL.04` asks for none either, though the van is due.
    let run = run_daily_on(t.state(), today()).await.unwrap();
    assert_eq!(run.booked, 0);

    // The job card under the contract continues to delivered, and is billed
    // in the monthly invoice.
    let delivered = finish_and_deliver(&t, card.id, line).await;
    assert_eq!(delivered.status, JobCardStatus::Delivered);
    assert!(delivered.invoice_id.is_none());
    assert!(t.books.about(&delivered.subject()).is_empty());

    fleet_contracts::resume(t.state(), &fatima, contract.id)
        .await
        .unwrap();
    service::book(
        t.state(),
        new_booking(),
        Actor::staff(sami.id),
        renox::db::now(),
    )
    .await
    .expect("the booking is made once the contract is resumed");
}

/// Given FC has a booking when it is suspended, the booking is kept and listed
/// for the advisor, who confirms it; a user with no right to decide cannot.
#[renox::test]
async fn aut_13_a_booking_made_before_the_suspension_is_kept_listed_and_confirmed() {
    let t = boot().await;
    bay_and_service(&t).await;
    let (contract, _van, plan) = active_contract(&t, 59_800).await;
    let fatima = fatima(&t).await;
    let run = fleet_services::request_bookings(t.state(), &contract, today())
        .await
        .unwrap();
    let (_, booking) = run.booked[0];

    fleet_contracts::suspend(t.state(), &fatima, contract.id, "Unpaid invoices")
        .await
        .unwrap();
    let kept = Booking::find_or_404(t.db(), booking.0).await.unwrap();
    assert_eq!(kept.status, BookingStatus::Booked, "kept");
    let listed = fleet_services::bookings_to_confirm(t.db(), contract.id)
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, plan.id);
    assert!(listed[0].booking_confirmed_at.is_none());

    let nobody = t.user("Nadia", &[auto::permissions::VEHICLES_VIEW]).await;
    let nadia = t.auth(&nobody).await;
    assert!(matches!(
        fleet_services::confirm_booking(t.state(), &nadia, plan.id).await,
        Err(Error::Forbidden)
    ));
    let sami = t.advisor().await;
    let confirmed = fleet_services::confirm_booking(t.state(), &sami, plan.id)
        .await
        .unwrap();
    assert!(confirmed.booking_confirmed_at.is_some());
    assert_eq!(
        Booking::find_or_404(t.db(), booking.0)
            .await
            .unwrap()
            .status,
        BookingStatus::Booked
    );
}

/// The advisor cancels a kept booking: it is cancelled for staff with the
/// reason `fleet_suspended` and the plan is free to ask again.
#[renox::test]
async fn aut_13_the_advisor_cancels_a_booking_kept_for_a_suspended_contract() {
    let t = boot().await;
    bay_and_service(&t).await;
    let (contract, _van, plan) = active_contract(&t, 59_800).await;
    let fatima = fatima(&t).await;
    let run = fleet_services::request_bookings(t.state(), &contract, today())
        .await
        .unwrap();
    let (_, booking) = run.booked[0];
    fleet_contracts::suspend(t.state(), &fatima, contract.id, "Unpaid invoices")
        .await
        .unwrap();

    let sami = t.advisor().await;
    fleet_services::cancel_booking(t.state(), &sami, plan.id)
        .await
        .unwrap();
    let cancelled = Booking::find_or_404(t.db(), booking.0).await.unwrap();
    assert_eq!(cancelled.status, BookingStatus::Cancelled);
    assert_eq!(cancelled.cancel_code.as_deref(), Some("fleet_suspended"));
    let freed = FleetService::find_or_404(t.db(), plan.id).await.unwrap();
    assert_eq!(freed.booking_id, None);
    assert!(
        fleet_services::bookings_to_confirm(t.db(), contract.id)
            .await
            .unwrap()
            .is_empty()
    );
    let again = fleet_services::cancel_booking(t.state(), &sami, plan.id)
        .await
        .unwrap_err();
    assert!(again.to_string().contains("no booking"), "{again}");
}

/// Given FC ending yesterday, it is still `active` on its last day and on the
/// first run after it is `ended`: its future bookings are cancelled, the plans
/// stop, the last invoice is drafted, the fleet contact is told, once; no
/// termination event is emitted for an end.
#[renox::test]
async fn aut_13_a_contract_ends_on_the_first_run_after_its_end_date() {
    let t = boot().await;
    bay_and_service(&t).await;
    let (contract, van, plan) = active_contract(&t, 59_800).await;
    let run = fleet_services::request_bookings(t.state(), &contract, today())
        .await
        .unwrap();
    let (_, booking) = run.booked[0];
    let (card, line) = job_in_progress(&t, &van, 1_000, 12_000).await;
    finish_and_deliver(&t, card.id, line).await;
    FleetContract::query()
        .where_(FleetContract::ID.eq(contract.id))
        .update(t.db(), &[("end_date", &day(-1))])
        .await
        .unwrap();

    let on_the_last_day = run_daily_on(t.state(), day(-1)).await.unwrap();
    assert!(on_the_last_day.ended.is_empty());
    assert_eq!(
        contract_now(&t, contract.id).await.status,
        FleetStatus::Active
    );
    assert!(t.books.about(&contract.subject()).is_empty());

    let after = run_daily_on(t.state(), today()).await.unwrap();
    assert_eq!(after.ended, [contract.id]);
    let ended = contract_now(&t, contract.id).await;
    assert_eq!(ended.status, FleetStatus::Ended);
    assert!(ended.wound_down_at.is_some());

    // The future booking is cancelled by the system and the plan stops.
    let cancelled = Booking::find_or_404(t.db(), booking.0).await.unwrap();
    assert_eq!(cancelled.status, BookingStatus::Cancelled);
    assert_eq!(
        cancelled.cancel_code.as_deref(),
        Some("fleet_contract_ended")
    );
    assert!(
        !FleetService::find_or_404(t.db(), plan.id)
            .await
            .unwrap()
            .active
    );

    // The last invoice covers the days to the end date and the job delivered.
    let periods = FleetInvoice::of_contract(t.db(), contract.id)
        .await
        .unwrap();
    let last = periods
        .iter()
        .find(|p| p.kind == KIND_FINAL)
        .expect("a final invoice");
    assert_eq!(last.period_end_date, day(-1));
    assert!(last.period_start_date <= day(-1));
    assert_eq!((last.jobs, last.net_total), (1, 12_000));
    let invoices = t.books.about(&contract.subject());
    assert_eq!(invoices.len(), 1);
    assert_eq!(invoices[0].party().0, FLEET);
    assert_eq!(last.invoice_id, Some(invoices[0].id));
    assert_eq!(
        ended.invoiced_through_date,
        Some(day(-1)),
        "no further invoice is drafted"
    );

    // The fleet contact is told, once.
    let told = t.messages.of_type(NOTICE_TYPE);
    assert_eq!(told.len(), 1);
    assert!(support::Messages::told_party(&told[0], CONTACT));
    let text = support::Messages::text(&told[0], NotifyChannel::Email);
    assert!(text.contains(&contract.number()), "{text}");
    assert!(t.terminated.all().is_empty(), "an end is not a termination");

    let quiet = run_daily_on(t.state(), day(1)).await.unwrap();
    assert!(quiet.ended.is_empty() && quiet.wound_down.is_empty());
    assert_eq!(t.messages.of_type(NOTICE_TYPE).len(), 1);
    assert_eq!(t.books.about(&contract.subject()).len(), 1);
}

/// `end` is the daily job's alone: a user cannot run it, the machine declares
/// it system only, and the contract page offers no such button.
#[renox::test]
async fn aut_13_end_is_not_offered_to_users() {
    let t = boot().await;
    let (contract, _van, _plan) = active_contract(&t, 50_100).await;
    let fatima = fatima(&t).await;
    assert!(FleetContract::machine().find(END).unwrap().is_system());

    let input = FleetInput {
        on: Some(day(500)),
        ..FleetInput::default()
    };
    let refused = contract
        .transition(END)
        .by(&fatima)
        .input(&input)
        .run(t.state())
        .await;
    assert!(refused.is_err(), "a user cannot end a contract");
    assert_eq!(
        contract_now(&t, contract.id).await.status,
        FleetStatus::Active
    );

    let manager = t
        .user(
            "Fatima",
            &[
                auto::permissions::FLEET_CONTRACTS_MANAGE,
                auto::permissions::VEHICLES_VIEW,
                auto::permissions::JOB_CARDS_VIEW,
            ],
        )
        .await;
    t.app.acting_as(&manager);
    let page = t.app.get(&format!("/auto/fleet/{}", contract.id)).await;
    page.assert_ok().assert_see("Suspend");
    assert!(!page.text().contains("/end"), "no end button");
}

/// The advisor sees the bookings kept for a suspended contract on its page and
/// confirms one; the page of the fleet account manager says why nothing new is
/// booked.
#[renox::test]
async fn aut_13_the_page_lists_the_kept_bookings_for_the_advisor() {
    let t = boot().await;
    bay_and_service(&t).await;
    let (contract, _van, plan) = active_contract(&t, 59_800).await;
    let fatima = fatima(&t).await;
    fleet_services::request_bookings(t.state(), &contract, today())
        .await
        .unwrap();
    fleet_contracts::suspend(t.state(), &fatima, contract.id, "Unpaid invoices")
        .await
        .unwrap();

    let sami = t.advisor_user().await;
    t.app.acting_as(&sami);
    let path = format!("/auto/fleet/{}", contract.id);
    t.app
        .get(&path)
        .await
        .assert_ok()
        .assert_see("Suspended")
        .assert_see("Unpaid invoices")
        .assert_see("To confirm or cancel")
        .assert_see("no new booking is made");
    t.app
        .htmx()
        .post(&format!("/auto/fleet/plans/{}/confirm", plan.id), &[])
        .await
        .assert_ok();
    t.app.get(&path).await.assert_ok().assert_see("Confirmed");
    assert!(
        FleetService::find_or_404(t.db(), plan.id)
            .await
            .unwrap()
            .booking_confirmed_at
            .is_some()
    );
}
