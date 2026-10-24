//! 08 Auto fleet contracts against the REAL shared modules of the host
//! (wl-scheduling, wl-invoicing, wl-parties): a service that falls due is
//! booked in the real scheduling for the fleet, the fleet's job card is not
//! invoiced on delivery, and the month is billed in one real invoice with a
//! line for each approved line naming the vehicle, the job card and the PO
//! reference. The app's own tests (`apps/auto/tests`) show the same flow
//! against stand-ins.

mod support;

use std::sync::{Arc, Mutex};

use auto::fleet_billing;
use auto::fleet_contracts::{self, NewContract};
use auto::fleet_daily::run_daily_on;
use auto::fleet_invoice::KIND_MONTHLY;
use auto::fleet_services::{self, NewPlan};
use auto::job_cards::{self, Delivery, NewJobCard, NewLabour, NewPart, Promise};
use auto::vehicles::{self, NewVehicle};
use auto::{Emirate, FleetContract, FleetJob, FleetService, FleetStatus, JobCard, WorkStatus};
use host::roles::{
    ACCOUNTANT, FLEET_ACCOUNT_MANAGER, SERVICE_ADVISOR, TECHNICIAN, WORKSHOP_MANAGER,
};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use renox::testing::TestApp;
use support::{company, person};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus, invoicer};
use wl_kernel::{PartyId, Subject};
use wl_scheduling::{
    AvailabilityRule, Booking, BookingStatus, Resource, ResourceType, ServiceType, resource,
};

/// The demo auto parts trader with a workshop.
const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-auto/client.toml"
);

/// A test-only module with the route that hands a test the signed-in user the
/// services take.
struct Who(Arc<Mutex<Option<AuthUser>>>);

impl Module for Who {
    fn name(&self) -> &'static str {
        "auto-fleet-real-who"
    }

    fn routes(&self) -> Routes {
        let slot = self.0.clone();
        Routes::new().get("/t/whoami", move |user: AuthUser| {
            let slot = slot.clone();
            async move {
                *slot.lock().unwrap() = Some(user);
                "ok"
            }
        })
    }
}

/// The booted workshop.
struct World {
    app: TestApp,
    who: Arc<Mutex<Option<AuthUser>>>,
}

/// Boots the demo client with `wl:install` and the seeds.
async fn boot() -> World {
    let who = Arc::new(Mutex::new(None));
    let config = wl_kernel::ClientConfig::load(CLIENT).expect("the auto client");
    let app = TestApp::new(
        host::app_for(config)
            .expect("the app")
            .module(Who(who.clone())),
    )
    .await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    World { app, who }
}

impl World {
    fn state(&self) -> &AppState {
        self.app.state()
    }

    fn db(&self) -> &Db {
        self.app.db()
    }

    /// A user with the host role `role`, as the user the services take.
    async fn staff(&self, name: &str, role: &str) -> (User, AuthUser) {
        let user = person(&self.app, name, role).await;
        self.app.acting_as(&user);
        self.app.get("/t/whoami").await.assert_ok();
        let auth = self.who.lock().unwrap().take().expect("the probe saw them");
        (user, auth)
    }
}

/// The workshop of the bookings.
fn workshop() -> Subject {
    Subject::new("scheduling.location", 1)
}

/// The fleet company with a van under an active contract, a plan that books
/// the periodic service every 10,000 km, and the workshop bay.
#[renox::test]
async fn aut_base_real_a_service_that_falls_due_is_booked_and_the_month_is_billed_in_one_invoice() {
    let w = boot().await;
    let (_, sami) = w.staff("Sami", SERVICE_ADVISOR).await;
    let (tareq_user, tareq) = w.staff("Tareq", TECHNICIAN).await;
    let (_, waleed) = w.staff("Waleed", WORKSHOP_MANAGER).await;
    let (_, fatima) = w.staff("Fatima", FLEET_ACCOUNT_MANAGER).await;
    let (_, aisha) = w.staff("Aisha", ACCOUNTANT).await;
    let fleet = company(&w.app, "Al Noor Logistics").await;

    let mut bay = Resource::new("Bay 1", ResourceType::Bay).at(&workshop());
    bay.save(w.db()).await.unwrap();
    for weekday in 1..=7 {
        AvailabilityRule::new(bay.id, weekday, 0, 1440)
            .save(w.db())
            .await
            .unwrap();
    }
    let periodic = ServiceType::query()
        .where_(ServiceType::CODE.eq("periodic_service"))
        .first(w.db())
        .await
        .unwrap()
        .expect("the seed stored the periodic service");
    resource::offer(w.db(), periodic.id, bay.id).await.unwrap();

    let van = vehicles::create(
        w.state(),
        &fatima,
        NewVehicle::new(Emirate::Dubai, "B", "555", "Nissan", "Urvan", fleet.0).odometer(59_800),
    )
    .await
    .unwrap();
    let contract = fleet_contracts::create(
        w.state(),
        &fatima,
        NewContract::new(
            fleet.0,
            "Fleet service 2026",
            today() - TimeDelta::days(40),
            today() + TimeDelta::days(300),
        )
        .labour_rate(12_000)
        .fee(200_000),
    )
    .await
    .unwrap();
    fleet_contracts::add_vehicle(w.state(), &fatima, contract.id, van.id)
        .await
        .unwrap();
    let plan = fleet_services::add_plan(
        w.state(),
        &fatima,
        contract.id,
        NewPlan::new(van.id, "periodic_service")
            .every_km(10_000)
            .last_service(today() - TimeDelta::days(100), 50_000),
    )
    .await
    .unwrap();
    fleet_contracts::attach_document(w.state(), &fatima, contract.id, "signed-fleet-2026")
        .await
        .unwrap();
    let contract = fleet_contracts::activate(w.state(), &fatima, contract.id)
        .await
        .unwrap();
    assert_eq!(contract.status, FleetStatus::Active);

    // The service falls due: the real scheduling books it for the fleet.
    let run = run_daily_on(w.state(), today()).await.unwrap();
    assert!(run.failed.is_empty(), "{:?}", run.failed);
    assert_eq!(run.booked, 1);
    let plan = FleetService::find_or_404(w.db(), plan.id).await.unwrap();
    let booking = Booking::find_or_404(w.db(), plan.booking_id.expect("a booking"))
        .await
        .unwrap();
    assert_eq!(booking.status, BookingStatus::Booked);
    assert_eq!(booking.party_id, fleet.0);
    assert_eq!(booking.subject_id, Some(van.id));
    assert_eq!(booking.service_type_id, periodic.id);
    let again = run_daily_on(w.state(), today()).await.unwrap();
    assert_eq!(again.booked, 0, "asked once");

    // The van comes in: a job card under the contract, with the fleet's PO.
    let card = job_cards::check_in(
        w.state(),
        &sami,
        NewJobCard::new(van.id, "Periodic service")
            .odometer(59_900)
            .condition("Clean")
            .acknowledged_by("Hamdan Saeed")
            .technician(tareq_user.id)
            .workshop(workshop()),
    )
    .await
    .unwrap();
    assert!(
        FleetJob::of_job_card(w.db(), card.id)
            .await
            .unwrap()
            .is_some()
    );
    fleet_billing::set_po(w.state(), &sami, card.id, "PO-4471")
        .await
        .unwrap();
    job_cards::start_inspection(w.state(), &tareq, card.id)
        .await
        .unwrap();
    let labour = job_cards::add_labour(
        w.state(),
        &tareq,
        card.id,
        NewLabour::hourly("Periodic service", 2_000, 12_000),
    )
    .await
    .unwrap();
    job_cards::add_part(
        w.state(),
        &tareq,
        card.id,
        NewPart::new("Oil filter", 1_000, 3_500),
    )
    .await
    .unwrap();
    job_cards::send_estimate(w.state(), &sami, card.id, Promise::default())
        .await
        .unwrap();
    let round = job_cards::pending_round(w.db(), card.id)
        .await
        .unwrap()
        .unwrap();
    job_cards::approve_by_link(w.state(), &round.token, "Hamdan Saeed", None, true)
        .await
        .unwrap();
    job_cards::record_work(w.state(), &tareq, labour.id, WorkStatus::Done, None)
        .await
        .unwrap();
    job_cards::finish_work(w.state(), &tareq, card.id)
        .await
        .unwrap();
    let all: Vec<String> = auto::job_card::QC_KEYS
        .iter()
        .map(|k| (*k).to_owned())
        .collect();
    job_cards::save_qc(w.state(), &waleed, card.id, &all)
        .await
        .unwrap();
    job_cards::pass_qc(w.state(), &waleed, card.id)
        .await
        .unwrap();
    let delivered = job_cards::deliver(
        w.state(),
        &sami,
        card.id,
        Delivery {
            delivered_to: "Hamdan Saeed".to_owned(),
            odometer_out: Some(59_950),
            ..Delivery::default()
        },
    )
    .await
    .unwrap();
    assert!(
        delivered.invoice_id.is_none(),
        "billed in the monthly invoice"
    );
    assert!(
        invoicer(w.state())
            .billing(w.db(), &delivered.subject())
            .await
            .unwrap()
            .is_empty()
    );

    // The consolidated invoice of the days gone by: AED 240 of labour and
    // AED 35 of parts, 5% VAT.
    JobCard::query()
        .where_(JobCard::ID.eq(card.id))
        .update(
            w.db(),
            &[("delivered_at", &Some(renox::db::now() - TimeDelta::days(3)))],
        )
        .await
        .unwrap();
    let invoiced = fleet_billing::invoice_period(
        w.state(),
        Some(&aisha),
        contract.id,
        today() - TimeDelta::days(30),
        today() - TimeDelta::days(1),
        KIND_MONTHLY,
    )
    .await
    .unwrap();
    assert_eq!((invoiced.jobs, invoiced.net_total), (1, 27_500));
    let invoice = invoicer(w.state())
        .find(w.db(), InvoiceId(invoiced.invoice_id.expect("an invoice")))
        .await
        .unwrap()
        .expect("the invoice exists");
    assert_eq!(invoice.party, PartyId(fleet.0));
    assert_eq!(invoice.gross_total, 28_875, "AED 275 and 5% VAT");
    assert_ne!(
        invoice.status,
        InvoiceStatus::Draft,
        "the accountant issues it"
    );
    let job = FleetJob::of_job_card(w.db(), card.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.invoice_id, Some(invoice.id.0));

    // The visit was not the one the plan booked: the plan keeps waiting for it.
    let kept = FleetService::find_or_404(w.db(), plan.id).await.unwrap();
    assert_eq!(kept.booking_id, plan.booking_id);
    assert_eq!(kept.last_service_km, Some(50_000));
    assert_eq!(
        FleetContract::find_or_404(w.db(), contract.id)
            .await
            .unwrap()
            .invoiced_through_date,
        Some(today() - TimeDelta::days(1))
    );
}
