//! BL-AUT-12 (#408): terminate a fleet contract early (08 §4.5 step 6, §7.1
//! *Fleet contract* `terminate`). The fleet account manager asks with a reason,
//! a notice date and an effective date (the last day of service); the owner
//! approves under "Needs your action" (never the one who asked); the contract
//! keeps its state, marked *termination scheduled*, until the first daily run
//! after the effective date, which cancels the bookings after it, drafts the
//! final consolidated invoice, invoices the early termination fee, tells the
//! fleet contact and emits `FleetContractTerminated`. A date before the day of
//! the request ends the contract at the approval. The days are simulated by
//! moving the clock (`TestApp::travel`), as the daily run at 00:30 would see
//! them.

mod support;

use std::time::Duration;

use auto::fleet_billing;
use auto::fleet_contracts::{self, Termination};
use auto::fleet_daily::run_daily_on;
use auto::fleet_end::TERMINATION_TYPE;
use auto::fleet_invoice::{FleetInvoice, KIND_FINAL, KIND_MONTHLY};
use auto::fleet_services;
use auto::{FleetContract, FleetJob, FleetService, FleetStatus, JobCardStatus};
use renox::prelude::*;
use support::claimfx::aisha;
use support::fleetfx::{
    CONTACT, FLEET, active_contract, bay_and_service, book_plan, delivered_job, fatima,
    finish_and_deliver, job_in_progress, omar, second_van, workshop,
};
use support::{Garage, at, boot, day};
use wl_kernel::PartyId;
use wl_kernel::approvals;
use wl_kernel::documents::today;
use wl_scheduling::service::{self, Actor, NewBooking};
use wl_scheduling::{Booking, BookingStatus};

async fn contract_now(t: &Garage, id: i64) -> FleetContract {
    FleetContract::find_or_404(t.db(), id).await.unwrap()
}

/// Runs `work` with the clock `days` days ahead of the real one, as the daily
/// run of that day would see it.
async fn on_day<F: std::future::Future>(t: &Garage, days: u64, work: F) -> F::Output {
    t.app.travel_back();
    t.app.travel(Duration::from_secs(86_400 * days));
    t.app.at_travelled_time(work).await
}

/// The manager asks, and the owner has the request under "Needs your action".
async fn ask(
    t: &Garage,
    contract: &FleetContract,
    notice: renox::chrono::NaiveDate,
    effective: renox::chrono::NaiveDate,
    waive: bool,
) -> Termination {
    let fatima = fatima(t).await;
    fleet_contracts::terminate(
        t.state(),
        &fatima,
        contract.id,
        "Fleet sold",
        notice,
        effective,
        waive,
    )
    .await
    .expect("the request is made")
}

/// Given FC-1 `active` with monthly invoices to yesterday, when manager A
/// requests termination (fleet sold, notice today, effective in 3 days) and
/// owner O approves today, then FC-1 is still `active` on the effective day
/// and on the run after it is `ended`; bookings dated after the effective day
/// are cancelled and a booking on it stays; a final invoice for the days after
/// the last invoiced period up to the effective day is drafted, the AED 2,000
/// fee is invoiced, the fleet contact is notified and `FleetContractTerminated`
/// is emitted.
#[renox::test]
async fn aut_12_a_terminated_contract_ends_on_the_run_after_the_effective_date() {
    let t = boot().await;
    let (bay, service_type) = bay_and_service(&t).await;
    let (contract, van, plan) = active_contract(&t, 50_100).await;
    let (van2, plan2) = second_van(&t, contract.id, 40_000).await;
    let aisha = aisha(&t).await;

    // Invoiced through yesterday: a job delivered 5 days ago is in the monthly
    // invoice; a job delivered today is not.
    delivered_job(&t, &van, 5).await;
    let monthly = fleet_billing::invoice_period(
        t.state(),
        Some(&aisha),
        contract.id,
        day(-30),
        day(-1),
        KIND_MONTHLY,
    )
    .await
    .unwrap();
    assert_eq!(monthly.net_total, 12_000);
    let sami = t.advisor().await;
    let delivered_today = delivered_job(&t, &van, 0).await;

    // One booking after the effective day, one on it.
    let after = service::book(
        t.state(),
        NewBooking::new(
            PartyId(FLEET),
            service_type.id,
            &[bay.id],
            at(day(5), 10, 0),
        )
        .for_subject(van.subject())
        .at(workshop()),
        Actor::staff(sami.id),
        renox::db::now(),
    )
    .await
    .unwrap();
    let on_the_day = service::book(
        t.state(),
        NewBooking::new(
            PartyId(FLEET),
            service_type.id,
            &[bay.id],
            at(day(3), 10, 0),
        )
        .for_subject(van2.subject())
        .at(workshop()),
        Actor::staff(sami.id),
        renox::db::now(),
    )
    .await
    .unwrap();
    book_plan(&t, plan.id, after.id, day(5)).await;
    book_plan(&t, plan2.id, on_the_day.id, day(3)).await;

    let asked = ask(&t, &contract, day(0), day(3), false).await;
    let request = asked.approval_request.expect("the owner decides");
    assert_eq!(asked.contract.status, FleetStatus::Active);
    assert!(
        !asked.contract.termination_scheduled,
        "nothing before the approval"
    );
    assert_eq!(
        t.books.about(&contract.subject()).len(),
        1,
        "only the monthly invoice so far"
    );

    let omar = omar(&t).await;
    approvals::approve(t.state(), request, &omar, "")
        .await
        .expect("the owner approves");
    let approved = contract_now(&t, contract.id).await;
    assert_eq!(approved.status, FleetStatus::Active, "it keeps its state");
    assert!(approved.termination_scheduled);
    assert_eq!(approved.termination_reason.as_deref(), Some("Fleet sold"));
    assert_eq!(
        (approved.notice_date, approved.effective_date),
        (Some(day(0)), Some(day(3)))
    );
    assert_eq!(approved.termination_approved_by, Some(omar.id));
    assert!(approved.termination_approved_at.is_some());
    assert!(t.terminated.all().is_empty());

    // The effective day is still served; the run after it ends the contract.
    assert!(
        fleet_contracts::due_transition(&approved, day(3)).is_none(),
        "the effective day is still served at contract rates"
    );
    let run = on_day(&t, 4, async { run_daily_on(t.state(), today()).await })
        .await
        .unwrap();
    assert_eq!(run.terminated, [contract.id], "{run:?}");
    let ended = contract_now(&t, contract.id).await;
    assert_eq!(ended.status, FleetStatus::Ended);
    assert!(!ended.termination_scheduled);

    // The booking after the effective day is cancelled; the one on it stays.
    let cancelled = Booking::find_or_404(t.db(), after.id).await.unwrap();
    assert_eq!(cancelled.status, BookingStatus::Cancelled);
    assert_eq!(
        cancelled.cancel_code.as_deref(),
        Some("fleet_contract_ended")
    );
    let stays = Booking::find_or_404(t.db(), on_the_day.id).await.unwrap();
    assert_eq!(stays.status, BookingStatus::Booked);
    assert!(
        !FleetService::find_or_404(t.db(), plan.id)
            .await
            .unwrap()
            .active
    );

    // The final invoice: from the day after the last invoiced period to the
    // effective day, with the job delivered in between.
    let periods = FleetInvoice::of_contract(t.db(), contract.id)
        .await
        .unwrap();
    let last = periods
        .iter()
        .find(|p| p.kind == KIND_FINAL)
        .expect("a final invoice");
    assert_eq!(last.period_start_date, day(0));
    assert_eq!(last.period_end_date, day(3));
    assert_eq!((last.jobs, last.net_total), (1, 12_000));
    let job = FleetJob::of_job_card(t.db(), delivered_today.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.invoice_id, last.invoice_id);

    // The fee of AED 2,000 is invoiced to the fleet.
    let invoices = t.books.about(&contract.subject());
    assert_eq!(invoices.len(), 3, "monthly, final and the fee");
    let fee = invoices
        .iter()
        .find(|i| {
            i.draft
                .lines
                .iter()
                .any(|l| l.description.contains("Early termination fee"))
        })
        .expect("the fee is invoiced");
    assert_eq!((fee.party().0, fee.net()), (FLEET, 200_000));
    assert_eq!(ended.termination_fee_invoice_id, Some(fee.id));

    // The fleet contact is told, and the event is emitted, once.
    let told = t.messages.of_type(TERMINATION_TYPE);
    assert_eq!(told.len(), 1);
    assert!(support::Messages::told_party(&told[0], CONTACT));
    let events = t.terminated.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].reason.as_deref(), Some("Fleet sold"));
    assert_eq!(events[0].effective_date, Some(day(3)));
    assert_eq!(events[0].fee_invoice, Some(fee.id));
    assert_eq!(events[0].final_invoice, last.invoice_id);
    let again = on_day(&t, 5, async { run_daily_on(t.state(), today()).await })
        .await
        .unwrap();
    assert!(again.terminated.is_empty() && again.wound_down.is_empty());
    assert_eq!(t.terminated.all().len(), 1);
    assert_eq!(t.books.about(&contract.subject()).len(), 3);
}

/// JC-110 open under FC-1 on the effective day finishes and is billed at the
/// contract rates (the prices of its lines) on its own invoice; the contract
/// is ended meanwhile and the final invoice did not take it.
#[renox::test]
async fn aut_12_an_open_job_card_finishes_and_is_billed_at_contract_rates() {
    let t = boot().await;
    let (contract, van, _plan) = active_contract(&t, 50_100).await;
    let (card, line) = job_in_progress(&t, &van, 2_000, 12_000).await;
    assert!(
        FleetJob::of_job_card(t.db(), card.id)
            .await
            .unwrap()
            .is_some()
    );

    let asked = ask(&t, &contract, day(0), day(1), true).await;
    approvals::approve(
        t.state(),
        asked.approval_request.unwrap(),
        &omar(&t).await,
        "",
    )
    .await
    .unwrap();
    on_day(&t, 2, async { run_daily_on(t.state(), today()).await })
        .await
        .unwrap();
    assert_eq!(
        contract_now(&t, contract.id).await.status,
        FleetStatus::Ended
    );
    assert_eq!(
        t.reload(card.id).await.status,
        JobCardStatus::InProgress,
        "the job card is not stopped"
    );

    let delivered = on_day(&t, 2, finish_and_deliver(&t, card.id, line)).await;
    assert_eq!(delivered.status, JobCardStatus::Delivered);
    let invoices = t.books.about(&delivered.subject());
    assert_eq!(invoices.len(), 1, "billed on delivery");
    assert_eq!(invoices[0].net(), 24_000, "2 h at AED 120");
    assert_eq!(invoices[0].party().0, FLEET);
    assert_eq!(delivered.invoice_id, Some(invoices[0].id));
    let job = FleetJob::of_job_card(t.db(), card.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.invoice_id, Some(invoices[0].id));
    // The final invoice listed no job; the fee was waived.
    let periods = FleetInvoice::of_contract(t.db(), contract.id)
        .await
        .unwrap();
    assert!(periods.iter().all(|p| p.jobs == 0));
}

/// The requester cannot approve their own request; the owner can; the request
/// is refused for an effective date before the notice date or inside a period
/// invoiced already.
#[renox::test]
async fn aut_12_the_requester_cannot_approve_and_the_dates_are_checked() {
    let t = boot().await;
    let (contract, van, _plan) = active_contract(&t, 50_100).await;
    let fatima_user = t
        .user(
            "Ann",
            &[
                auto::permissions::FLEET_CONTRACTS_MANAGE,
                auto::permissions::FLEET_CONTRACTS_APPROVE,
                auto::permissions::VEHICLES_VIEW,
                auto::permissions::JOB_CARDS_VIEW,
            ],
        )
        .await;
    let fatima = t.auth(&fatima_user).await;
    let aisha = aisha(&t).await;
    delivered_job(&t, &van, 5).await;
    fleet_billing::invoice_period(
        t.state(),
        Some(&aisha),
        contract.id,
        day(-30),
        day(-1),
        KIND_MONTHLY,
    )
    .await
    .unwrap();

    let before_notice = fleet_contracts::terminate(
        t.state(),
        &fatima,
        contract.id,
        "Fleet sold",
        day(5),
        day(2),
        false,
    )
    .await
    .unwrap_err();
    assert!(
        before_notice.to_string().contains("before the notice date"),
        "{before_notice}"
    );
    let inside = fleet_contracts::terminate(
        t.state(),
        &fatima,
        contract.id,
        "Fleet sold",
        day(-20),
        day(-15),
        false,
    )
    .await
    .unwrap_err();
    assert!(inside.to_string().contains("invoiced already"), "{inside}");
    assert_eq!(
        contract_now(&t, contract.id).await.status,
        FleetStatus::Active
    );

    let asked = fleet_contracts::terminate(
        t.state(),
        &fatima,
        contract.id,
        "Fleet sold",
        day(0),
        day(10),
        false,
    )
    .await
    .unwrap();
    let request = asked.approval_request.expect("waits for the owner");
    let own = approvals::approve(t.state(), request, &fatima_user, "").await;
    assert!(own.is_err(), "the requester does not approve");
    assert!(!contract_now(&t, contract.id).await.termination_scheduled);
    let second = fleet_contracts::terminate(
        t.state(),
        &fatima,
        contract.id,
        "Fleet sold",
        day(0),
        day(12),
        false,
    )
    .await;
    assert!(second.is_err(), "one request waits already");
    approvals::approve(t.state(), request, &omar(&t).await, "")
        .await
        .expect("the owner approves");
    assert!(contract_now(&t, contract.id).await.termination_scheduled);
    let scheduled = fleet_contracts::terminate(
        t.state(),
        &fatima,
        contract.id,
        "Fleet sold",
        day(0),
        day(12),
        false,
    )
    .await
    .unwrap_err();
    assert!(
        scheduled.to_string().contains("scheduled already"),
        "{scheduled}"
    );
}

/// Between the approval and the run after the effective day the contract
/// stays `active` with "termination scheduled" shown, and `AUT.RUL.04`
/// requests no booking dated after the effective day (one on it is made).
#[renox::test]
async fn aut_12_a_scheduled_termination_is_shown_and_asks_for_no_booking_after_it() {
    let t = boot().await;
    bay_and_service(&t).await;
    let (contract, _van, plan) = active_contract(&t, 50_100).await;
    let asked = ask(&t, &contract, day(0), day(2), false).await;
    approvals::approve(
        t.state(),
        asked.approval_request.unwrap(),
        &omar(&t).await,
        "",
    )
    .await
    .unwrap();

    let scheduled = contract_now(&t, contract.id).await;
    assert_eq!(scheduled.status, FleetStatus::Active);
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
    t.app
        .get(&format!("/auto/fleet/{}", contract.id))
        .await
        .assert_ok()
        .assert_see("Active")
        .assert_see("Termination scheduled")
        .assert_see(&day(2).to_string());

    // Due after the effective day: no booking is asked for.
    FleetService::query()
        .where_(FleetService::ID.eq(plan.id))
        .update(t.db(), &[("next_due_date", &Some(day(5)))])
        .await
        .unwrap();
    let none = fleet_services::request_bookings(t.state(), &scheduled, today())
        .await
        .unwrap();
    assert!(
        none.booked.is_empty(),
        "nothing is booked after the effective day"
    );
    // Due on the effective day itself: it is booked.
    FleetService::query()
        .where_(FleetService::ID.eq(plan.id))
        .update(t.db(), &[("next_due_date", &Some(day(1)))])
        .await
        .unwrap();
    let booked = fleet_services::request_bookings(t.state(), &scheduled, today())
        .await
        .unwrap();
    assert_eq!(booked.booked.len(), 1, "{:?}", booked.failed);
}

/// A date before the day of the request ends the contract at the approval; a
/// date of today waits for the run after it; a request approved after its
/// effective date passed ends at once.
#[renox::test]
async fn aut_12_the_effective_date_decides_whether_the_contract_ends_at_the_approval() {
    let t = boot().await;
    let (past, van, _plan) = active_contract(&t, 50_100).await;
    let aisha = aisha(&t).await;
    delivered_job(&t, &van, 10).await;
    // Invoiced through 10 days ago.
    fleet_billing::invoice_period(
        t.state(),
        Some(&aisha),
        past.id,
        day(-40),
        day(-10),
        KIND_MONTHLY,
    )
    .await
    .unwrap();

    // Notice 8 days ago, effective 6 days ago, approved today: ended at once.
    let asked = ask(&t, &past, day(-8), day(-6), false).await;
    assert_eq!(asked.contract.status, FleetStatus::Active);
    approvals::approve(
        t.state(),
        asked.approval_request.unwrap(),
        &omar(&t).await,
        "",
    )
    .await
    .unwrap();
    let ended = contract_now(&t, past.id).await;
    assert_eq!(ended.status, FleetStatus::Ended, "ended at the approval");
    let periods = FleetInvoice::of_contract(t.db(), past.id).await.unwrap();
    let last = periods.iter().find(|p| p.kind == KIND_FINAL).unwrap();
    assert_eq!(
        (last.period_start_date, last.period_end_date),
        (day(-9), day(-6))
    );
    assert_eq!(t.terminated.all().len(), 1);
}

/// An effective date of today waits for the next run.
#[renox::test]
async fn aut_12_an_effective_date_of_today_ends_on_the_next_days_run() {
    let t = boot().await;
    let (contract, _van, _plan) = active_contract(&t, 50_100).await;
    let asked = ask(&t, &contract, day(0), day(0), false).await;
    approvals::approve(
        t.state(),
        asked.approval_request.unwrap(),
        &omar(&t).await,
        "",
    )
    .await
    .unwrap();
    let waiting = contract_now(&t, contract.id).await;
    assert_eq!(waiting.status, FleetStatus::Active);
    assert!(waiting.termination_scheduled);
    let same_day = run_daily_on(t.state(), today()).await.unwrap();
    assert!(same_day.terminated.is_empty());
    let next = on_day(&t, 1, async { run_daily_on(t.state(), today()).await })
        .await
        .unwrap();
    assert_eq!(next.terminated, [contract.id]);
    assert_eq!(
        contract_now(&t, contract.id).await.status,
        FleetStatus::Ended
    );
}

/// A request approved after its effective date passed ends the contract at
/// the approval, whichever date it was asked for.
#[renox::test]
async fn aut_12_a_request_approved_after_its_effective_date_ends_the_contract_at_once() {
    let t = boot().await;
    let (contract, _van, _plan) = active_contract(&t, 50_100).await;
    let asked = ask(&t, &contract, day(0), day(1), false).await;
    let request = asked.approval_request.unwrap();
    let omar = omar(&t).await;
    on_day(&t, 2, async {
        approvals::approve(t.state(), request, &omar, "").await
    })
    .await
    .unwrap();
    assert_eq!(
        contract_now(&t, contract.id).await.status,
        FleetStatus::Ended
    );
    assert_eq!(t.terminated.all().len(), 1);
}

/// A rejected request leaves the contract as it was; a fee the owner's approval
/// waives is not invoiced.
#[renox::test]
async fn aut_12_a_rejected_request_changes_nothing_and_a_waived_fee_is_not_invoiced() {
    let t = boot().await;
    let (contract, _van, _plan) = active_contract(&t, 50_100).await;
    let omar = omar(&t).await;

    let first = ask(&t, &contract, day(0), day(2), false).await;
    approvals::reject(t.state(), first.approval_request.unwrap(), &omar, "Not yet")
        .await
        .unwrap();
    let same = contract_now(&t, contract.id).await;
    assert_eq!(same.status, FleetStatus::Active);
    assert!(!same.termination_scheduled);
    assert!(same.termination_reason.is_none());

    let second = ask(&t, &contract, day(0), day(1), true).await;
    approvals::approve(t.state(), second.approval_request.unwrap(), &omar, "")
        .await
        .unwrap();
    assert!(contract_now(&t, contract.id).await.termination_fee_waived);
    on_day(&t, 2, async { run_daily_on(t.state(), today()).await })
        .await
        .unwrap();
    let ended = contract_now(&t, contract.id).await;
    assert_eq!(ended.status, FleetStatus::Ended);
    assert_eq!(ended.termination_fee_invoice_id, None);
    assert!(
        t.books.about(&contract.subject()).iter().all(|i| !i
            .draft
            .lines
            .iter()
            .any(|l| l.description.contains("fee"))),
        "no fee invoice"
    );
}

/// The termination is requested on the contract page; the request waits for
/// the owner, who has it under the contract; a bad reason is refused in words.
#[renox::test]
async fn aut_12_the_contract_page_asks_for_the_termination_and_the_owner_sees_it() {
    let t = boot().await;
    let (contract, _van, _plan) = active_contract(&t, 50_100).await;
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
    let path = format!("/auto/fleet/{}", contract.id);
    t.app
        .get(&path)
        .await
        .assert_ok()
        .assert_see("Terminate early");

    let bad = t
        .app
        .htmx()
        .post(
            &format!("{path}/terminate"),
            &[
                ("reason", "boredom"),
                ("notice_date", &day(0).to_string()),
                ("effective_date", &day(5).to_string()),
            ],
        )
        .await;
    bad.assert_status(422);
    assert!(bad.text().contains("Pick the reason"), "{}", bad.text());
    t.app
        .htmx()
        .post(
            &format!("{path}/terminate"),
            &[
                ("reason", "fleet_sold"),
                ("note", "Sold to Gulf Movers"),
                ("notice_date", &day(0).to_string()),
                ("effective_date", &day(5).to_string()),
                ("waive_fee", "on"),
            ],
        )
        .await
        .assert_ok();
    t.app
        .get(&path)
        .await
        .assert_ok()
        .assert_see("A termination waits for");

    let owner = omar(&t).await;
    t.app.acting_as(&owner);
    t.app
        .get(&path)
        .await
        .assert_ok()
        .assert_see("A termination waits for");
}
