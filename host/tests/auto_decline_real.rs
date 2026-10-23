//! BL-AUT-03 (#399) against the REAL shared modules of the host (wl-scheduling,
//! wl-invoicing, wl-parties): an estimate nobody answers in seven days is
//! declined by the hourly rule, the inspection fee agreed at check-in is
//! invoiced and issued on the real books, and the real booking is cancelled.
//! The collect SMS is checked against the notifier stand-in in
//! `apps/auto/tests/aut_03_decline.rs` (the demo customer has no mobile).

mod support;

use std::sync::{Arc, Mutex};

use auto::declines;
use auto::job_cards::{self, Intake, NewLabour, Promise};
use auto::vehicles::{self, NewVehicle};
use auto::{Emirate, JobCard, JobCardStatus};
use host::roles::{SERVICE_ADVISOR, TECHNICIAN};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use renox::testing::TestApp;
use support::{person, tenant};
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus, invoicer};
use wl_kernel::{PartyId, Subject};
use wl_scheduling::service::{self, Actor, NewBooking};
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
        "auto-decline-real-who"
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

/// JC-30 with an inspection fee of AED 150 agreed at check-in: an estimate sent
/// ten days ago and not answered.
#[renox::test]
async fn aut_03_real_an_unanswered_estimate_is_declined_with_the_fee_invoiced_and_the_booking_cancelled()
 {
    let w = boot().await;
    let (_, sami) = w.staff("Sami", SERVICE_ADVISOR).await;
    let (tareq_user, tareq) = w.staff("Tareq", TECHNICIAN).await;
    let owner = tenant(&w.app, "Mona Hassan").await;
    let vehicle = vehicles::create(
        w.state(),
        &sami,
        NewVehicle::new(Emirate::Dubai, "A", "12345", "Toyota", "Hilux", owner.0).odometer(52_000),
    )
    .await
    .unwrap();
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
    let start = renox::db::now() - TimeDelta::hours(2);
    let booking = service::book(
        w.state(),
        NewBooking::new(PartyId(owner.0), periodic.id, &[bay.id], start)
            .for_subject(vehicle.subject())
            .at(workshop()),
        Actor::staff(sami.id),
        start - TimeDelta::hours(1),
    )
    .await
    .unwrap();
    service::check_in(w.state(), booking.id, Actor::staff(sami.id), renox::db::now())
        .await
        .unwrap();
    let card = JobCard::of_booking(w.db(), booking.id).await.unwrap().unwrap();
    job_cards::record_intake(
        w.state(),
        &sami,
        card.id,
        Intake {
            complaint: "Brake noise".to_owned(),
            odometer_in: 52_100,
            fuel_level: Some("half".to_owned()),
            condition: "Clean".to_owned(),
            acknowledged_by: Some("Mona Hassan".to_owned()),
            signature: None,
            accident_damage: false,
            police_report_no: None,
            inspection_fee: 15_000,
            storage_fee_daily: 0,
        },
    )
    .await
    .unwrap();
    job_cards::assign_technician(w.state(), &sami, card.id, tareq_user.id)
        .await
        .unwrap();
    job_cards::start_inspection(w.state(), &tareq, card.id)
        .await
        .unwrap();
    job_cards::add_labour(
        w.state(),
        &tareq,
        card.id,
        NewLabour::hourly("Brake pads", 1_000, 20_000),
    )
    .await
    .unwrap();
    job_cards::send_estimate(w.state(), &sami, card.id, Promise::default())
        .await
        .unwrap();
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
    let declined = JobCard::find_or_404(w.db(), card.id).await.unwrap();
    assert_eq!(declined.status, JobCardStatus::Declined);
    let invoice = invoicer(w.state())
        .find(w.db(), InvoiceId(declined.invoice_id.expect("the fee invoice")))
        .await
        .unwrap()
        .expect("the invoice exists");
    assert_eq!(invoice.gross_total, 15_750, "AED 150 and 5% VAT");
    assert_eq!(invoice.party, PartyId(owner.0));
    assert_ne!(invoice.status, InvoiceStatus::Draft, "issued");
    let booking = Booking::find_or_404(w.db(), booking.id).await.unwrap();
    assert_eq!(booking.status, BookingStatus::Cancelled);
    assert_eq!(booking.cancel_code.as_deref(), Some("estimate_declined"));
}
