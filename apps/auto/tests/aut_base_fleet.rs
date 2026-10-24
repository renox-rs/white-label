//! The base of the 08 Auto fleet service contract (base for #408, #409, #413;
//! `AUT.FLT.01`, `AUT.RUL.04`, `AUT.RUL.05`, `FIN.USG.01`): a contract with a
//! fleet, vehicles under it, a service plan that books the services that fall
//! due through wl-scheduling, the job cards of the fleet billed in one
//! consolidated invoice a month with the vehicle and the PO reference of each,
//! and the daily run that flags, ends and invoices. The issues fill the
//! effects of `suspend`, `end` and `terminate`.

mod support;

use auto::fleet_billing::{self, bills_monthly};
use auto::fleet_contract_machine::{
    ACTIVATE, END, FLAG_RENEWAL, RENEW, RESUME, SUSPEND, TERMINATE, TERMINATE_SYSTEM,
};
use auto::fleet_contracts::{self, NewContract};
use auto::fleet_daily::run_daily_on;
use auto::fleet_invoice::KIND_MONTHLY;
use auto::fleet_services::{self, NewPlan};
use auto::job_cards::{self, NewJobCard};
use auto::{FleetContract, FleetJob, FleetService, FleetStatus, JobCard, JobCardStatus, Vehicle};
use renox::chrono::Datelike;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use support::claimfx::aisha;
use support::fleetfx::{FLEET, active_contract, bay_and_service, draft_contract, fatima, van};
use support::{boot, day};
use wl_kernel::documents::today;
use wl_scheduling::{Booking, BookingStatus};

async fn contract_now(t: &support::Garage, id: i64) -> FleetContract {
    FleetContract::find_or_404(t.db(), id).await.unwrap()
}

/// A contract is agreed with the fleet, its contact, the term, the rates and
/// the inclusions; it starts as a draft with a number; only the fleet account
/// manager agrees one, with a real fleet and a sane term.
#[renox::test]
async fn aut_base_a_fleet_contract_is_agreed_as_a_draft() {
    let t = boot().await;
    let contract = draft_contract(&t, 0, 365).await;
    assert_eq!(contract.status, FleetStatus::Draft);
    assert_eq!(contract.number(), format!("FC-{:04}", contract.id));
    assert_eq!(contract.fleet_party_id, FLEET);
    assert_eq!(contract.labour_rate, 12_000);
    assert_eq!(contract.early_termination_fee, 200_000);
    assert_eq!(contract.price_tier.as_deref(), Some("fleet"));
    assert_eq!(contract.notice_period_days, 30);
    let history = renox::state_machine::history(t.db(), FleetContract::TABLE, contract.id)
        .await
        .unwrap();
    assert_eq!(history.len(), 1);

    let fatima = fatima(&t).await;
    let backwards = fleet_contracts::create(
        t.state(),
        &fatima,
        NewContract::new(FLEET, "Backwards", day(10), day(5)),
    )
    .await
    .unwrap_err();
    assert!(
        backwards.to_string().contains("after the first day"),
        "{backwards}"
    );
    let nobody = fleet_contracts::create(
        t.state(),
        &fatima,
        NewContract::new(999, "Nobody", day(0), day(5)),
    )
    .await
    .unwrap_err();
    assert!(nobody.to_string().contains("parties"), "{nobody}");
    let blank = fleet_contracts::create(
        t.state(),
        &fatima,
        NewContract::new(FLEET, " ", day(0), day(5)),
    )
    .await
    .unwrap_err();
    assert!(blank.to_string().contains("name"), "{blank}");
    let sami = t.advisor().await;
    assert!(matches!(
        fleet_contracts::create(
            t.state(),
            &sami,
            NewContract::new(FLEET, "X", day(0), day(5))
        )
        .await,
        Err(Error::Forbidden)
    ));
}

/// The terms change while the contract is a draft; once in force the fleet,
/// the term and the rate are locked (a new rate is a renewal).
#[renox::test]
async fn aut_base_the_terms_are_locked_once_the_contract_is_in_force() {
    let t = boot().await;
    let fatima = fatima(&t).await;
    let (contract, _van, _plan) = active_contract(&t, 59_800).await;

    let same_terms = NewContract::new(
        FLEET,
        "Fleet service 2026 (renamed)",
        contract.start_date,
        contract.end_date,
    )
    .labour_rate(12_000)
    .inclusions("Everything");
    let renamed = fleet_contracts::update(t.state(), &fatima, contract.id, same_terms)
        .await
        .unwrap();
    assert_eq!(renamed.title, "Fleet service 2026 (renamed)");
    assert_eq!(renamed.inclusions.as_deref(), Some("Everything"));
    let cheaper =
        NewContract::new(FLEET, "Fleet", contract.start_date, contract.end_date).labour_rate(9_000);
    let locked = fleet_contracts::update(t.state(), &fatima, contract.id, cheaper)
        .await
        .unwrap_err();
    assert!(locked.to_string().contains("renewing"), "{locked}");

    let draft = draft_contract(&t, 0, 100).await;
    let changed = fleet_contracts::update(
        t.state(),
        &fatima,
        draft.id,
        NewContract::new(FLEET, "Draft", day(1), day(200)).labour_rate(9_000),
    )
    .await
    .unwrap();
    assert_eq!((changed.labour_rate, changed.end_date), (9_000, day(200)));
}

/// A vehicle is under one contract at a time; it leaves only when no job card
/// is open under the contract, and its plans stop.
#[renox::test]
async fn aut_base_vehicles_go_under_and_off_a_contract() {
    let t = boot().await;
    let fatima = fatima(&t).await;
    let (contract, van, plan) = active_contract(&t, 59_800).await;
    assert_eq!(van.fleet_contract_id, Some(contract.id));
    assert_eq!(
        fleet_contracts::vehicles_of(t.db(), contract.id)
            .await
            .unwrap()
            .len(),
        1
    );

    let other = draft_contract(&t, 0, 100).await;
    let taken = fleet_contracts::add_vehicle(t.state(), &fatima, other.id, van.id)
        .await
        .unwrap_err();
    assert!(
        taken.to_string().contains("another fleet contract"),
        "{taken}"
    );
    let stranger = fleet_contracts::remove_vehicle(t.state(), &fatima, other.id, van.id)
        .await
        .unwrap_err();
    assert!(
        stranger.to_string().contains("not under this contract"),
        "{stranger}"
    );

    // An open job card under the contract keeps the vehicle on it.
    let sami = t.advisor().await;
    let card = job_cards::check_in(
        t.state(),
        &sami,
        NewJobCard::new(van.id, "Oil leak")
            .odometer(59_900)
            .condition("Clean"),
    )
    .await
    .unwrap();
    let busy = fleet_contracts::remove_vehicle(t.state(), &fatima, contract.id, van.id)
        .await
        .unwrap_err();
    assert!(busy.to_string().contains("job card"), "{busy}");
    job_cards::void(t.state(), &sami, card.id, "Opened in error")
        .await
        .unwrap();

    let off = fleet_contracts::remove_vehicle(t.state(), &fatima, contract.id, van.id)
        .await
        .unwrap();
    assert_eq!(off.fleet_contract_id, None);
    assert!(
        !FleetService::find_or_404(t.db(), plan.id)
            .await
            .unwrap()
            .active
    );
}

/// Activation needs what 08 asks, one thing at a time: a vehicle, a rate, a
/// service plan and the signed contract; it sets the first due reading of the
/// plan.
#[renox::test]
async fn aut_base_a_contract_is_activated_with_vehicles_rates_a_plan_and_the_signed_contract() {
    let t = boot().await;
    let fatima = fatima(&t).await;
    t.people.add(FLEET, "Al Noor Logistics");
    let no_rate = fleet_contracts::create(
        t.state(),
        &fatima,
        NewContract::new(FLEET, "No rate", day(0), day(300)),
    )
    .await
    .unwrap();
    let nothing = fleet_contracts::activate(t.state(), &fatima, no_rate.id)
        .await
        .unwrap_err();
    assert!(
        nothing.to_string().contains("at least one vehicle"),
        "{nothing}"
    );

    let contract = draft_contract(&t, 0, 300).await;
    let van = van(&t, 59_800).await;
    fleet_contracts::add_vehicle(t.state(), &fatima, contract.id, van.id)
        .await
        .unwrap();
    let no_plan = fleet_contracts::activate(t.state(), &fatima, contract.id)
        .await
        .unwrap_err();
    assert!(no_plan.to_string().contains("service plan"), "{no_plan}");

    let no_interval = fleet_services::add_plan(
        t.state(),
        &fatima,
        contract.id,
        NewPlan::new(van.id, "periodic_service"),
    )
    .await
    .unwrap_err();
    assert!(
        no_interval.to_string().contains("above zero"),
        "{no_interval}"
    );
    let plan = fleet_services::add_plan(
        t.state(),
        &fatima,
        contract.id,
        NewPlan::new(van.id, "Periodic_Service").every_km(10_000),
    )
    .await
    .unwrap();
    assert_eq!(plan.service_code, "periodic_service");
    assert_eq!(
        plan.next_due_km,
        Some(10_000),
        "from zero when there was no service"
    );

    let unsigned = fleet_contracts::activate(t.state(), &fatima, contract.id)
        .await
        .unwrap_err();
    assert!(
        unsigned.to_string().contains("signed contract"),
        "{unsigned}"
    );
    fleet_contracts::attach_document(t.state(), &fatima, contract.id, " signed-1 ")
        .await
        .unwrap();
    let active = fleet_contracts::activate(t.state(), &fatima, contract.id)
        .await
        .unwrap();
    assert_eq!(active.status, FleetStatus::Active);
    assert!(active.activated_at.is_some());
    assert_eq!(active.signed_document.as_deref(), Some("signed-1"));
    assert!(
        fleet_contracts::activate(t.state(), &fatima, contract.id)
            .await
            .is_err()
    );

    let machine = FleetContract::machine();
    let guards = machine
        .find(ACTIVATE)
        .unwrap()
        .guard_names()
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert_eq!(
        guards,
        [
            "vehicles_set",
            "rates_set",
            "schedule_set",
            "signed_contract"
        ]
    );
}

/// A plan with an interval in months and a last service gets its due day; an
/// interval of zero or a last service in the future is refused; a vehicle
/// outside the contract has no plan.
#[renox::test]
async fn aut_base_a_plan_is_due_by_months_or_kilometres() {
    let t = boot().await;
    let fatima = fatima(&t).await;
    let contract = draft_contract(&t, 0, 300).await;
    let van = van(&t, 40_000).await;
    fleet_contracts::add_vehicle(t.state(), &fatima, contract.id, van.id)
        .await
        .unwrap();

    let by_months = fleet_services::add_plan(
        t.state(),
        &fatima,
        contract.id,
        NewPlan::new(van.id, "periodic_service")
            .every_months(6)
            .every_km(15_000)
            .last_service(day(-30), 38_000),
    )
    .await
    .unwrap();
    assert_eq!(
        by_months.next_due_date,
        Some(auto::fleet_service::add_months(day(-30), 6))
    );
    assert_eq!(by_months.next_due_km, Some(53_000));
    for bad in [
        NewPlan::new(van.id, "periodic_service").every_km(0),
        NewPlan::new(van.id, "periodic_service").every_months(-1),
        NewPlan::new(van.id, "periodic_service")
            .every_km(5_000)
            .last_service(day(3), 0),
        NewPlan::new(van.id, "").every_km(5_000),
    ] {
        assert!(
            fleet_services::add_plan(t.state(), &fatima, contract.id, bad)
                .await
                .is_err()
        );
    }
    let other = draft_contract(&t, 0, 100).await;
    let off = fleet_services::add_plan(
        t.state(),
        &fatima,
        other.id,
        NewPlan::new(van.id, "periodic_service").every_km(5_000),
    )
    .await
    .unwrap_err();
    assert!(off.to_string().contains("not under this contract"), "{off}");
}

/// A service that falls due is booked through wl-scheduling for the fleet,
/// with the vehicle as the booking's subject; one that is not due yet is not;
/// the booking is asked once.
#[renox::test]
async fn aut_base_a_service_that_falls_due_is_booked_through_scheduling() {
    let t = boot().await;
    let (bay, service) = bay_and_service(&t).await;
    let (contract, van, plan) = active_contract(&t, 59_800).await;
    assert_eq!(plan.next_due_km, Some(60_000));

    let run = fleet_services::request_bookings(t.state(), &contract, today())
        .await
        .unwrap();
    assert!(run.failed.is_empty(), "{:?}", run.failed);
    assert_eq!(run.booked.len(), 1);
    let (plan_id, booking_id) = run.booked[0];
    assert_eq!(plan_id, plan.id);
    let booking = Booking::find_or_404(t.db(), booking_id.0).await.unwrap();
    assert_eq!(booking.status, BookingStatus::Booked);
    assert_eq!(booking.party_id, FLEET);
    assert_eq!(booking.service_type_id, service.id);
    assert_eq!(booking.subject_type.as_deref(), Some("auto.vehicle"));
    assert_eq!(booking.subject_id, Some(van.id));
    assert!(
        booking
            .notes
            .as_deref()
            .unwrap_or("")
            .contains(&contract.number())
    );
    assert!(
        booking
            .resource_ids(t.db())
            .await
            .unwrap()
            .contains(&bay.id)
    );
    let kept = FleetService::find_or_404(t.db(), plan.id).await.unwrap();
    assert_eq!(kept.booking_id, Some(booking.id));
    assert!(kept.requested_at.is_some());

    // Asked once.
    let again = fleet_services::request_bookings(t.state(), &contract, today())
        .await
        .unwrap();
    assert!(again.booked.is_empty());
    assert_eq!(
        fleet_services::bookings_to_confirm(t.db(), contract.id)
            .await
            .unwrap()
            .len(),
        1
    );
}

/// A vehicle far from its next service is not booked yet; a contract that is
/// suspended or in its renewal window books nothing, and nothing is booked for
/// a day after the last day of service.
#[renox::test]
async fn aut_base_nothing_is_booked_early_or_for_a_contract_not_active() {
    let t = boot().await;
    bay_and_service(&t).await;
    let (contract, van, _plan) = active_contract(&t, 50_100).await;
    let fatima = fatima(&t).await;
    let none = fleet_services::request_bookings(t.state(), &contract, today())
        .await
        .unwrap();
    assert!(none.booked.is_empty(), "49,900 km to go");

    // The van reads 59,800 now, but the contract is suspended.
    Vehicle::query()
        .where_(Vehicle::ID.eq(van.id))
        .update(t.db(), &[("odometer_km", &59_800_i64)])
        .await
        .unwrap();
    let suspended = fleet_contracts::suspend(t.state(), &fatima, contract.id, "Unpaid invoices")
        .await
        .unwrap();
    assert_eq!(suspended.status, FleetStatus::Suspended);
    assert_eq!(
        suspended.suspended_reason.as_deref(),
        Some("Unpaid invoices")
    );
    let blocked = fleet_services::request_bookings(t.state(), &suspended, today())
        .await
        .unwrap();
    assert!(
        blocked.booked.is_empty(),
        "a suspended contract books nothing"
    );
    let resumed = fleet_contracts::resume(t.state(), &fatima, contract.id)
        .await
        .unwrap();
    assert_eq!(resumed.status, FleetStatus::Active);
    assert!(resumed.suspended_reason.is_none());

    // A termination scheduled before the due day: nothing after its last day.
    let mut ending = resumed.clone();
    ending.termination_scheduled = true;
    ending.effective_date = Some(day(-1));
    let none_after = fleet_services::request_bookings(t.state(), &ending, today())
        .await
        .unwrap();
    assert!(none_after.booked.is_empty());
    let booked = fleet_services::request_bookings(t.state(), &resumed, today())
        .await
        .unwrap();
    assert_eq!(booked.booked.len(), 1);
}

/// A booking that is cancelled frees the plan, which asks again; the bookings
/// of a contract are cancelled as the system with a reason code.
#[renox::test]
async fn aut_base_cancelling_the_bookings_of_a_contract_frees_its_plans() {
    let t = boot().await;
    bay_and_service(&t).await;
    let (contract, _van, plan) = active_contract(&t, 59_800).await;
    let run = fleet_services::request_bookings(t.state(), &contract, today())
        .await
        .unwrap();
    let (_, booking) = run.booked[0];

    let cancelled =
        fleet_services::cancel_open_bookings(t.state(), contract.id, "resource_unavailable")
            .await
            .unwrap();
    assert_eq!(cancelled, 1);
    assert_eq!(
        Booking::find_or_404(t.db(), booking.0)
            .await
            .unwrap()
            .status,
        BookingStatus::Cancelled
    );
    let freed = FleetService::find_or_404(t.db(), plan.id).await.unwrap();
    assert_eq!(freed.booking_id, None, "the plan is free to ask again");
    assert!(
        fleet_services::bookings_to_confirm(t.db(), contract.id)
            .await
            .unwrap()
            .is_empty()
    );
}

/// The visit that was booked is delivered: the plan moves on from the
/// odometer reading of the delivery.
#[renox::test]
async fn aut_base_a_delivered_visit_moves_the_plan_on() {
    let t = boot().await;
    bay_and_service(&t).await;
    let (contract, van, plan) = active_contract(&t, 59_800).await;
    let run = fleet_services::request_bookings(t.state(), &contract, today())
        .await
        .unwrap();
    let (_, booking) = run.booked[0];

    let mut card = t.card_in(&van, JobCardStatus::Delivered).await;
    JobCard::query()
        .where_(JobCard::ID.eq(card.id))
        .update(
            t.db(),
            &[
                ("booking_id", &Some(booking.0)),
                ("odometer_out", &Some(60_050_i64)),
            ],
        )
        .await
        .unwrap();
    card = JobCard::find_or_404(t.db(), card.id).await.unwrap();
    fleet_services::on_delivered(t.state(), &card)
        .await
        .unwrap();

    let moved = FleetService::find_or_404(t.db(), plan.id).await.unwrap();
    assert_eq!(moved.last_service_date, Some(today()));
    assert_eq!(moved.last_service_km, Some(60_050));
    assert_eq!(moved.next_due_km, Some(70_050));
    assert_eq!(moved.booking_id, None);

    // A job card that no plan booked moves no plan.
    let walk_in = t.card_in(&van, JobCardStatus::Delivered).await;
    fleet_services::on_delivered(t.state(), &walk_in)
        .await
        .unwrap();
    assert_eq!(
        FleetService::find_or_404(t.db(), plan.id)
            .await
            .unwrap()
            .next_due_km,
        Some(70_050)
    );
}

/// A job card of a vehicle under a contract in force is a fleet job: it gets
/// the PO reference and is not billed on delivery; a vehicle of nobody's fleet
/// has none.
#[renox::test]
async fn aut_base_a_job_card_of_a_fleet_vehicle_is_a_fleet_job_billed_monthly() {
    let t = boot().await;
    let (contract, van, _plan) = active_contract(&t, 59_800).await;
    let sami = t.advisor().await;
    let card = job_cards::check_in(
        t.state(),
        &sami,
        NewJobCard::new(van.id, "Brake noise")
            .odometer(59_900)
            .condition("Clean"),
    )
    .await
    .unwrap();
    let job = FleetJob::of_job_card(t.db(), card.id)
        .await
        .unwrap()
        .expect("a fleet job");
    assert_eq!((job.contract_id, job.vehicle_id), (contract.id, van.id));
    assert!(bills_monthly(t.db(), card.id).await.unwrap());

    let hilux = t.hilux().await;
    let private = job_cards::check_in(
        t.state(),
        &sami,
        NewJobCard::new(hilux.id, "Noise")
            .odometer(52_100)
            .condition("Clean"),
    )
    .await
    .unwrap();
    assert!(!bills_monthly(t.db(), private.id).await.unwrap());

    let po = fleet_billing::set_po(t.state(), &sami, card.id, " PO-4471 ")
        .await
        .unwrap();
    assert_eq!(po.po_reference.as_deref(), Some("PO-4471"));
    let nobody = t.user("Nadia", &[auto::permissions::VEHICLES_VIEW]).await;
    let nadia = t.auth(&nobody).await;
    assert!(matches!(
        fleet_billing::set_po(t.state(), &nadia, card.id, "x").await,
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        fleet_billing::set_po(t.state(), &sami, private.id, "x").await,
        Err(Error::NotFound)
    ));
}

/// A delivered fleet job: the invoice is drafted for the contract's period,
/// one line per approved line with the vehicle, the job card and the PO; the
/// job card makes no invoice of its own.
async fn delivered_job(
    t: &support::Garage,
    van: &Vehicle,
    po: &str,
    delivered_days_ago: i64,
) -> JobCard {
    let sami = t.advisor().await;
    let card = job_cards::check_in(
        t.state(),
        &sami,
        NewJobCard::new(van.id, "Service")
            .odometer(van.odometer_km + 10)
            .condition("Clean"),
    )
    .await
    .unwrap();
    t.approved_labour(&card, "Oil and filter change", 1_000, 12_000)
        .await;
    t.approved_part(&card, "Oil filter", None, 1_000, 3_500)
        .await;
    fleet_billing::set_po(t.state(), &sami, card.id, po)
        .await
        .unwrap();
    let delivered_at = renox::db::now() - renox::chrono::TimeDelta::days(delivered_days_ago);
    JobCard::query()
        .where_(JobCard::ID.eq(card.id))
        .update(
            t.db(),
            &[
                ("status", &JobCardStatus::Delivered),
                ("delivered_at", &Some(delivered_at)),
            ],
        )
        .await
        .unwrap();
    JobCard::find_or_404(t.db(), card.id).await.unwrap()
}

#[renox::test]
async fn aut_base_the_month_is_billed_in_one_invoice_with_a_line_per_vehicle_job_and_po() {
    let t = boot().await;
    let (contract, van, _plan) = active_contract(&t, 59_800).await;
    let first = delivered_job(&t, &van, "PO-4471", 5).await;
    let second = delivered_job(&t, &van, "PO-4480", 4).await;
    let aisha = aisha(&t).await;

    // A delivery does not invoice the job card.
    assert!(t.books.about(&first.subject()).is_empty());

    let early = fleet_billing::invoice_period(
        t.state(),
        Some(&aisha),
        contract.id,
        day(-30),
        today(),
        KIND_MONTHLY,
    )
    .await
    .unwrap_err();
    assert!(early.to_string().contains("period that is over"), "{early}");

    let invoiced = fleet_billing::invoice_period(
        t.state(),
        Some(&aisha),
        contract.id,
        day(-30),
        day(-1),
        KIND_MONTHLY,
    )
    .await
    .unwrap();
    assert_eq!(invoiced.jobs, 2);
    assert_eq!(
        invoiced.net_total, 31_000,
        "two jobs of AED 120 and AED 35 plus nothing else"
    );
    let books = t.books.about(&contract.subject());
    assert_eq!(books.len(), 1);
    let invoice = &books[0];
    assert!(invoice.issued, "the accountant issues it");
    assert_eq!(invoiced.invoice_id, Some(invoice.id));
    assert_eq!(invoice.party().0, FLEET);
    assert_eq!(invoice.net(), 31_000);
    let lines: Vec<&str> = invoice
        .draft
        .lines
        .iter()
        .map(|l| l.description.as_str())
        .collect();
    assert_eq!(lines.len(), 4);
    assert_eq!(
        lines[0],
        format!(
            "Dubai B 555 · {} · PO PO-4471 · Oil and filter change (1 h)",
            first.number()
        )
    );
    assert_eq!(
        lines[3],
        format!(
            "Dubai B 555 · {} · PO PO-4480 · Oil filter (new)",
            second.number()
        )
    );
    for card in [&first, &second] {
        let job = FleetJob::of_job_card(t.db(), card.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(job.invoice_id, Some(invoice.id));
        assert!(job.invoiced_at.is_some());
    }
    let through = contract_now(&t, contract.id).await.invoiced_through_date;
    assert_eq!(through, Some(day(-1)));

    // A period is billed once, none is skipped.
    let again = fleet_billing::invoice_period(
        t.state(),
        Some(&aisha),
        contract.id,
        day(-10),
        day(-1),
        KIND_MONTHLY,
    )
    .await
    .unwrap_err();
    assert!(again.to_string().contains("invoiced already"), "{again}");
    assert!(
        t.books.all().len() == 1,
        "nothing was drafted for the refused period"
    );
}

/// A period with nothing to bill is recorded without an invoice; a contract
/// that is a draft cannot be invoiced; a user who bills must hold a right to.
#[renox::test]
async fn aut_base_an_empty_period_is_recorded_and_a_draft_contract_is_not_invoiced() {
    let t = boot().await;
    let (contract, _van, _plan) = active_contract(&t, 59_800).await;
    let fatima = fatima(&t).await;
    let empty = fleet_billing::invoice_period(
        t.state(),
        Some(&fatima),
        contract.id,
        day(-30),
        day(-1),
        KIND_MONTHLY,
    )
    .await
    .unwrap();
    assert_eq!((empty.jobs, empty.invoice_id), (0, None));
    assert!(t.books.all().is_empty());
    assert_eq!(
        contract_now(&t, contract.id).await.invoiced_through_date,
        Some(day(-1))
    );

    let draft = draft_contract(&t, -10, 100).await;
    let refused = fleet_billing::invoice_period(
        t.state(),
        Some(&fatima),
        draft.id,
        day(-9),
        day(-1),
        KIND_MONTHLY,
    )
    .await
    .unwrap_err();
    assert!(refused.to_string().contains("still a draft"), "{refused}");
    let nadia = t.user("Nadia", &[auto::permissions::VEHICLES_VIEW]).await;
    let nadia = t.auth(&nadia).await;
    assert!(matches!(
        fleet_billing::invoice_period(
            t.state(),
            Some(&nadia),
            contract.id,
            day(0),
            day(0),
            KIND_MONTHLY
        )
        .await,
        Err(Error::Forbidden)
    ));
}

/// On day 1 the daily run drafts the invoice of the month before for each
/// contract in force (`AUT.RUL.05`): a draft, not issued; the next month
/// follows the one before; another day drafts nothing.
#[renox::test]
async fn aut_base_the_daily_run_drafts_the_invoice_of_the_month_before_on_day_1() {
    let t = boot().await;
    bay_and_service(&t).await;
    let (contract, van, _plan) = active_contract(&t, 52_000).await;
    let first_of_month = today().with_day(1).unwrap();
    let (_, last_of_last) = fleet_billing::previous_month(first_of_month);
    let days_ago = (today() - last_of_last).num_days() + 3;
    delivered_job(&t, &van, "PO-1", days_ago).await;

    let not_day_one = run_daily_on(t.state(), first_of_month.succ_opt().unwrap())
        .await
        .unwrap();
    assert!(not_day_one.invoiced.is_empty());

    let run = run_daily_on(t.state(), first_of_month).await.unwrap();
    assert!(run.failed.is_empty(), "{:?}", run.failed);
    assert_eq!(run.invoiced, [contract.id]);
    let books = t.books.about(&contract.subject());
    assert_eq!(books.len(), 1);
    assert!(!books[0].issued, "drafted for the accountant");
    assert_eq!(books[0].net(), 15_500);
    assert_eq!(
        contract_now(&t, contract.id).await.invoiced_through_date,
        Some(last_of_last)
    );
    let again = run_daily_on(t.state(), first_of_month).await.unwrap();
    assert!(again.invoiced.is_empty(), "the month is invoiced already");
}

/// The daily run flags a contract in its renewal window, ends one after its
/// last day of service (not on it), and leaves a suspended or draft contract
/// as it is; the machine declares the transitions and who runs them.
#[renox::test]
async fn aut_base_the_daily_run_flags_the_renewal_and_ends_the_contract_after_its_last_day() {
    let t = boot().await;
    bay_and_service(&t).await;
    let (contract, _van, _plan) = active_contract(&t, 50_100).await;
    let fatima = fatima(&t).await;
    FleetContract::query()
        .where_(FleetContract::ID.eq(contract.id))
        .update(t.db(), &[("end_date", &day(30))])
        .await
        .unwrap();

    let flagged = run_daily_on(t.state(), today()).await.unwrap();
    assert_eq!(flagged.flagged, [contract.id]);
    assert_eq!(
        contract_now(&t, contract.id).await.status,
        FleetStatus::RenewalDue
    );
    let quiet = run_daily_on(t.state(), today()).await.unwrap();
    assert!(quiet.flagged.is_empty() && quiet.ended.is_empty());

    // Renewed with a new last day and a new rate.
    let too_early = fleet_contracts::renew(t.state(), &fatima, contract.id, day(30), None)
        .await
        .unwrap_err();
    assert!(
        too_early.to_string().contains("after the current one"),
        "{too_early}"
    );
    let renewed = fleet_contracts::renew(t.state(), &fatima, contract.id, day(400), Some(13_000))
        .await
        .unwrap();
    assert_eq!(renewed.status, FleetStatus::Active);
    assert_eq!((renewed.end_date, renewed.labour_rate), (day(400), 13_000));

    // The last day is still served; the first day after it ends the contract.
    FleetContract::query()
        .where_(FleetContract::ID.eq(contract.id))
        .update(t.db(), &[("end_date", &day(-1))])
        .await
        .unwrap();
    let on_the_last_day = run_daily_on(t.state(), day(-1)).await.unwrap();
    assert!(on_the_last_day.ended.is_empty());
    let after = run_daily_on(t.state(), today()).await.unwrap();
    assert_eq!(after.ended, [contract.id]);
    let ended = contract_now(&t, contract.id).await;
    assert_eq!(ended.status, FleetStatus::Ended);
    assert!(ended.ended_at.is_some());
    assert!(
        fleet_contracts::resume(t.state(), &fatima, contract.id)
            .await
            .is_err()
    );

    let machine = FleetContract::machine();
    for name in [END, FLAG_RENEWAL, TERMINATE_SYSTEM] {
        assert!(machine.find(name).unwrap().is_system(), "{name}");
    }
    for name in [ACTIVATE, SUSPEND, RESUME, RENEW, TERMINATE] {
        assert!(!machine.find(name).unwrap().is_system(), "{name}");
    }
}

/// A termination is declared: the notice and effective dates are required, the
/// effective date is not before the notice nor inside an invoiced period; the
/// request waits for the owner (BL-AUT-12 tests the rest).
#[renox::test]
async fn aut_base_terminate_is_declared_with_its_dates_and_the_owner_approval() {
    let t = boot().await;
    let (contract, _van, _plan) = active_contract(&t, 50_100).await;
    let fatima = fatima(&t).await;
    let machine = FleetContract::machine();
    let guards: Vec<String> = machine
        .find(TERMINATE)
        .unwrap()
        .guard_names()
        .into_iter()
        .map(str::to_owned)
        .collect();
    assert_eq!(
        guards,
        ["termination_dates", "not_scheduled", "owner_approval"]
    );

    let backwards = fleet_contracts::terminate(
        t.state(),
        &fatima,
        contract.id,
        "Fleet sold",
        day(10),
        day(5),
        false,
    )
    .await
    .unwrap_err();
    assert!(
        backwards.to_string().contains("before the notice date"),
        "{backwards}"
    );
    let no_reason =
        fleet_contracts::terminate(t.state(), &fatima, contract.id, " ", day(1), day(20), false)
            .await
            .unwrap_err();
    assert!(
        no_reason.to_string().to_lowercase().contains("reason"),
        "{no_reason}"
    );

    let aisha = aisha(&t).await;
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
    let invoiced = fleet_contracts::terminate(
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
    assert!(
        invoiced.to_string().contains("invoiced already"),
        "{invoiced}"
    );

    let asked = fleet_contracts::terminate(
        t.state(),
        &fatima,
        contract.id,
        "Fleet sold",
        day(1),
        day(20),
        false,
    )
    .await
    .unwrap();
    assert!(asked.approval_request.is_some(), "the owner decides");
    assert_eq!(asked.contract.status, FleetStatus::Active);
    assert!(
        !asked.contract.termination_scheduled,
        "nothing before the approval"
    );
    assert!(asked.contract.termination_reason.is_none());
}
