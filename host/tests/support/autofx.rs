//! Shared by the host tests of the 08 Auto claims that end early (BL-AUT-08 ..
//! BL-AUT-11): the demo workshop booted with the real modules, its staff, the
//! insurer under a payer agreement with an accepted rate card, and a job card
//! under an approved claim with the police report attached and accepted.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use auto::claims::{self, Answer, NewClaim};
use auto::fleet_contracts::{self, NewContract};
use auto::fleet_services::{self, NewPlan};
use auto::insurers::{self, NewInsurer};
use auto::job_cards::{self, Delivery, NewJobCard, NewLabour, Promise};
use auto::vehicles::{self, NewVehicle};
use auto::{
    Emirate, FleetContract, FleetService, InsuranceClaim, JobCard, LabourLine, Vehicle, WorkStatus,
};
use host::roles::{
    ACCOUNTANT, BRANCH_MANAGER, FLEET_ACCOUNT_MANAGER, INSURANCE_COORDINATOR, OWNER, PROCUREMENT,
    SERVICE_ADVISOR, TECHNICIAN, WORKSHOP_MANAGER,
};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use renox::testing::TestApp;
use renox::upload::Upload;
use wl_documents::{Documents, NewDocument};
use wl_invoicing::rate_card::Unit;
use wl_invoicing::rate_cards::{self, LineInput};
use wl_invoicing::{PayerAgreement, VatRecipient};
use wl_kernel::documents::today;
use wl_kernel::{LegalEntity, PartyId, Subject};
use wl_scheduling::{AvailabilityRule, Resource, ResourceType, ServiceType, resource};

use super::{company, person, tenant};

/// The demo auto parts trader with a workshop.
pub const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-auto/client.toml"
);

/// A test-only module with the route that hands a test the signed-in user the
/// services take.
pub struct Who(Arc<Mutex<Option<AuthUser>>>);

impl Module for Who {
    fn name(&self) -> &'static str {
        "auto-claims-fx-who"
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
pub struct World {
    /// The app.
    pub app: TestApp,
    who: Arc<Mutex<Option<AuthUser>>>,
}

/// Boots the demo client with `wl:install` and the seeds.
pub async fn boot() -> World {
    boot_with(|app| app).await
}

/// [`boot`] with `more` applied to the app first (an event recorder).
pub async fn boot_with(more: impl FnOnce(App) -> App) -> World {
    let who = Arc::new(Mutex::new(None));
    let config = wl_kernel::ClientConfig::load(CLIENT).expect("the auto client");
    let app = TestApp::new(more(
        host::app_for(config)
            .expect("the app")
            .module(Who(who.clone())),
    ))
    .await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    World { app, who }
}

impl World {
    /// The app state.
    pub fn state(&self) -> &AppState {
        self.app.state()
    }

    /// The database.
    pub fn db(&self) -> &Db {
        self.app.db()
    }

    /// A user with the host role `role`, as the user the services take.
    pub async fn staff(&self, name: &str, role: &str) -> (User, AuthUser) {
        let user = person(&self.app, name, role).await;
        self.app.acting_as(&user);
        self.app.get("/t/whoami").await.assert_ok();
        let auth = self.who.lock().unwrap().take().expect("the probe saw them");
        (user, auth)
    }
}

/// The workshop's staff, the owner of the vehicle and the insurer.
pub struct Shop {
    /// The service advisor.
    pub sami: AuthUser,
    /// The technician.
    pub tareq: AuthUser,
    /// The technician's user.
    pub tareq_user: User,
    /// The workshop manager.
    pub waleed: AuthUser,
    /// The insurance coordinator.
    pub carla: AuthUser,
    /// The accountant.
    pub aisha: AuthUser,
    /// The branch manager.
    pub basma: AuthUser,
    /// The branch manager's user.
    pub basma_user: User,
    /// The purchaser.
    pub pia: AuthUser,
    /// The owner of the vehicle.
    pub owner: PartyId,
    /// Orient Insurance, as a party.
    pub orient: PartyId,
    /// Orient Insurance, as an insurer of the workshop.
    pub insurer: auto::Insurer,
}

/// The staff, and Orient Insurance under a payer agreement (the payer gets the
/// tax invoice) with an accepted rate card version in force since 30 days and,
/// when `storage_rate` is given, the storage rate it agreed, in fils a day.
pub async fn shop(w: &World, storage_rate: Option<i64>) -> Shop {
    let (_, sami) = w.staff("Sami", SERVICE_ADVISOR).await;
    let (tareq_user, tareq) = w.staff("Tareq", TECHNICIAN).await;
    let (_, waleed) = w.staff("Waleed", WORKSHOP_MANAGER).await;
    let (_, carla) = w.staff("Carla", INSURANCE_COORDINATOR).await;
    let (_, aisha) = w.staff("Aisha", ACCOUNTANT).await;
    let (basma_user, basma) = w.staff("Basma", BRANCH_MANAGER).await;
    let (_, pia) = w.staff("Pia", PROCUREMENT).await;
    let owner = tenant(&w.app, "Mona Hassan").await;
    let orient = company(&w.app, "Orient Insurance").await;
    let entity = LegalEntity::default_entity(w.db())
        .await
        .expect("the legal entity exists");
    let mut agreement = PayerAgreement {
        legal_entity_id: entity.id,
        payer_party_id: orient.0,
        name: "Orient motor".to_owned(),
        active: true,
        vat_recipient: VatRecipient::Payer,
        first_submission_days: 30,
        resubmission_days: 30,
        resubmission_limit: 2,
        write_off_limit: 100_000,
        ..PayerAgreement::default()
    };
    agreement.insert(w.db()).await.unwrap();
    let mut new = NewInsurer::new(orient.0)
        .agreement(agreement.id)
        .email("claims@orient.example")
        .labour_rate(12_000);
    if let Some(rate) = storage_rate {
        new = new.storage_rate(rate);
    }
    let insurer = insurers::register(w.state(), &carla, new).await.unwrap();
    let card = rate_cards::create_card(w.db(), None, orient, &insurer.subject())
        .await
        .unwrap();
    let version =
        rate_cards::new_version(w.db(), None, card.id, today() - TimeDelta::days(30), false)
            .await
            .unwrap();
    rate_cards::add_line(
        w.db(),
        version.id,
        LineInput::new("body_repair", "standard", Unit::Hour, 12_000),
    )
    .await
    .unwrap();
    renox::auth::permissions::define_role(w.db(), "rate_manager", &["rate_cards.manage"])
        .await
        .unwrap();
    let (_, rita) = w.staff("Rita", "rate_manager").await;
    rate_cards::accept(w.state(), &rita, version.id, Some("signed-rate-card"))
        .await
        .unwrap();
    Shop {
        sami,
        tareq,
        tareq_user,
        waleed,
        carla,
        aisha,
        basma,
        basma_user,
        pia,
        owner,
        orient,
        insurer,
    }
}

/// Attaches the police report to the claim and has the workshop manager accept
/// it: the legal block of `AUT.ACL.20`.
pub async fn police_report(w: &World, shop: &Shop, claim: &InsuranceClaim) {
    let report = Documents::attach(
        w.state(),
        Some(&shop.carla),
        NewDocument::new(
            claim.subject(),
            "police_report",
            Upload::new(
                "police-report.pdf",
                "application/pdf",
                b"%PDF-1.7 police report".to_vec(),
            ),
        )
        .number("DXB-PR-5521"),
    )
    .await
    .unwrap()
    .expect("the report is attached");
    Documents::accept(w.state(), &shop.waleed, &report)
        .await
        .unwrap();
}

/// A job card under an approved claim, in progress.
pub struct Job {
    /// The job card, `in_progress`.
    pub card: JobCard,
    /// The claim, `approved`.
    pub claim: InsuranceClaim,
    /// The labour lines, approved by the insurer at their price.
    pub lines: Vec<LabourLine>,
}

/// The Hilux of the owner on a job card with the labour lines `(description,
/// hours in thousandths, hourly rate in fils)`, the claim registered, its
/// police report accepted, the estimate submitted and approved by the insurer
/// line by line at their price, and the lines at the positions `done` marked
/// done.
pub async fn approved_job(
    w: &World,
    shop: &Shop,
    labour: &[(&str, i64, i64)],
    done: &[usize],
) -> Job {
    let vehicle = vehicles::create(
        w.state(),
        &shop.sami,
        NewVehicle::new(
            Emirate::Dubai,
            "A",
            "12345",
            "Toyota",
            "Hilux",
            shop.owner.0,
        )
        .odometer(52_000),
    )
    .await
    .unwrap();
    let card = job_cards::check_in(
        w.state(),
        &shop.sami,
        NewJobCard::new(vehicle.id, "Right side hit in a parking lot")
            .odometer(52_400)
            .condition("Right door dented, bumper cracked")
            .acknowledged_by("Mona Hassan")
            .technician(shop.tareq_user.id)
            .workshop(Subject::new("scheduling.location", 1)),
    )
    .await
    .unwrap();
    job_cards::start_inspection(w.state(), &shop.tareq, card.id)
        .await
        .unwrap();
    let mut lines = Vec::new();
    for (description, hours, rate) in labour {
        lines.push(
            job_cards::add_labour(
                w.state(),
                &shop.tareq,
                card.id,
                NewLabour::hourly(description, *hours, *rate),
            )
            .await
            .unwrap(),
        );
    }
    job_cards::send_estimate(w.state(), &shop.sami, card.id, Promise::default())
        .await
        .unwrap();
    let claim = claims::register(
        w.state(),
        &shop.carla,
        NewClaim::new(
            shop.insurer.id,
            vehicle.id,
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
    police_report(w, shop, &claim).await;
    claims::submit_estimate(w.state(), &shop.carla, claim.id)
        .await
        .unwrap();
    let mut answer = Answer::new("LPO-80", "orient-approval-email-80");
    for line in &lines {
        answer = answer.labour(line.id, line.price());
    }
    claims::approve(w.state(), &shop.carla, claim.id, answer)
        .await
        .unwrap();
    for index in done {
        job_cards::record_work(
            w.state(),
            &shop.tareq,
            lines[*index].id,
            WorkStatus::Done,
            None,
        )
        .await
        .unwrap();
    }
    Job {
        card: JobCard::find_or_404(w.db(), card.id).await.unwrap(),
        claim: InsuranceClaim::find_or_404(w.db(), claim.id).await.unwrap(),
        lines,
    }
}

/// The messages of `type_key`, oldest first.
pub async fn sent_of(w: &World, type_key: &str) -> Vec<wl_notify::Message> {
    wl_notify::Message::query()
        .where_(wl_notify::Message::TYPE_KEY.eq(type_key))
        .order_by(wl_notify::Message::ID)
        .get(w.db())
        .await
        .unwrap()
}

/// The workshop's staff, a fleet with a van under an active contract and the
/// service plan that books it, and the workshop bay.
pub struct FleetShop {
    /// The service advisor.
    pub sami: AuthUser,
    /// The technician.
    pub tareq: AuthUser,
    /// The technician's user.
    pub tareq_user: User,
    /// The workshop manager.
    pub waleed: AuthUser,
    /// The fleet account manager.
    pub fatima: AuthUser,
    /// The fleet account manager's user.
    pub fatima_user: User,
    /// The accountant.
    pub aisha: AuthUser,
    /// The owner (approves a termination).
    pub omar: User,
    /// The fleet.
    pub fleet: PartyId,
    /// The fleet contact.
    pub contact: PartyId,
    /// The bay the services are booked on.
    pub bay: Resource,
    /// The periodic service.
    pub periodic: ServiceType,
    /// The contract, `active`: from 40 days ago to 300 days ahead, AED 120 an
    /// hour, an early termination fee of AED 2,000.
    pub contract: FleetContract,
    /// The van (B 555) under it.
    pub van: Vehicle,
    /// The plan that books the periodic service of the van.
    pub plan: FleetService,
}

/// The workshop of the bookings.
pub fn workshop() -> Subject {
    Subject::new("scheduling.location", 1)
}

/// The staff, the fleet with its contact, the bay and the van under an active
/// contract with a plan that is not due (the van reads `odometer` km, the last
/// service was at 50,000 km, every 10,000 km).
pub async fn fleet_shop(w: &World, odometer: i64) -> FleetShop {
    let (_, sami) = w.staff("Sami", SERVICE_ADVISOR).await;
    let (tareq_user, tareq) = w.staff("Tareq", TECHNICIAN).await;
    let (_, waleed) = w.staff("Waleed", WORKSHOP_MANAGER).await;
    let (fatima_user, fatima) = w.staff("Fatima", FLEET_ACCOUNT_MANAGER).await;
    let (_, aisha) = w.staff("Aisha", ACCOUNTANT).await;
    let omar = person(&w.app, "Omar", OWNER).await;
    let fleet = company(&w.app, "Al Noor Logistics").await;
    let contact = tenant(&w.app, "Hamdan Saeed").await;

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
        NewVehicle::new(Emirate::Dubai, "B", "555", "Nissan", "Urvan", fleet.0).odometer(odometer),
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
        .contact(contact.0)
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
    FleetShop {
        sami,
        tareq,
        tareq_user,
        waleed,
        fatima,
        fatima_user,
        aisha,
        omar,
        fleet,
        contact,
        bay,
        periodic,
        contract,
        van,
        plan,
    }
}

/// A job card of `van` under the fleet contract, checked in, with one hourly
/// labour line of `hours_milli` at AED 120 the customer approved: in progress.
/// Answers the job card and the labour line.
pub async fn fleet_job(w: &World, shop: &FleetShop, hours_milli: i64) -> (JobCard, LabourLine) {
    let van = Vehicle::find_or_404(w.db(), shop.van.id).await.unwrap();
    let card = job_cards::check_in(
        w.state(),
        &shop.sami,
        NewJobCard::new(van.id, "Periodic service")
            .odometer(van.odometer_km + 10)
            .condition("Clean")
            .acknowledged_by("Hamdan Saeed")
            .technician(shop.tareq_user.id)
            .workshop(workshop()),
    )
    .await
    .unwrap();
    job_cards::start_inspection(w.state(), &shop.tareq, card.id)
        .await
        .unwrap();
    let line = job_cards::add_labour(
        w.state(),
        &shop.tareq,
        card.id,
        NewLabour::hourly("Periodic service", hours_milli, 12_000),
    )
    .await
    .unwrap();
    job_cards::send_estimate(w.state(), &shop.sami, card.id, Promise::default())
        .await
        .unwrap();
    let round = job_cards::pending_round(w.db(), card.id)
        .await
        .unwrap()
        .unwrap();
    job_cards::approve_by_link(w.state(), &round.token, "Hamdan Saeed", None, true)
        .await
        .unwrap();
    (JobCard::find_or_404(w.db(), card.id).await.unwrap(), line)
}

/// The work of `line` is done, the job card passes quality control and is
/// delivered: delivered.
pub async fn fleet_deliver(
    w: &World,
    shop: &FleetShop,
    card: &JobCard,
    line: &LabourLine,
) -> JobCard {
    job_cards::record_work(w.state(), &shop.tareq, line.id, WorkStatus::Done, None)
        .await
        .unwrap();
    job_cards::finish_work(w.state(), &shop.tareq, card.id)
        .await
        .unwrap();
    let all: Vec<String> = auto::job_card::QC_KEYS
        .iter()
        .map(|k| (*k).to_owned())
        .collect();
    job_cards::save_qc(w.state(), &shop.waleed, card.id, &all)
        .await
        .unwrap();
    job_cards::pass_qc(w.state(), &shop.waleed, card.id)
        .await
        .unwrap();
    job_cards::deliver(
        w.state(),
        &shop.sami,
        card.id,
        Delivery {
            delivered_to: "Hamdan Saeed".to_owned(),
            ..Delivery::default()
        },
    )
    .await
    .unwrap()
}

/// Everything of one type `E` emitted so far.
#[derive(Clone)]
pub struct Recorded<E>(Arc<Mutex<Vec<E>>>);

impl<E: Clone> Recorded<E> {
    /// Nothing recorded yet.
    pub fn new() -> Self {
        Recorded(Arc::new(Mutex::new(Vec::new())))
    }

    /// The events, oldest first.
    pub fn all(&self) -> Vec<E> {
        self.0.lock().unwrap().clone()
    }
}

/// `app` listening for `E`, recorded into `into`.
pub fn record<E: Event + Clone>(app: App, into: &Recorded<E>) -> App {
    let into = into.clone();
    app.listen(move |event: E, _state: AppState| {
        let into = into.clone();
        async move {
            into.0.lock().unwrap().push(event);
            Ok(())
        }
    })
}

/// The Hilux with the plate number `number` of `owner`, checked in at the
/// workshop (no booking) with the inspection fee `fee` agreed, inspected, the
/// `labour` added, and the estimate sent when `send`.
#[allow(clippy::too_many_arguments)]
pub async fn inspected_card(
    w: &World,
    sami: &AuthUser,
    tareq: &(User, AuthUser),
    owner: PartyId,
    number: &str,
    fee: i64,
    labour: Vec<NewLabour>,
    send: bool,
) -> JobCard {
    let vehicle = vehicles::create(
        w.state(),
        sami,
        NewVehicle::new(Emirate::Dubai, "A", number, "Toyota", "Hilux", owner.0)
            .vin(&format!("JTHBK2625820{number}"))
            .odometer(52_000),
    )
    .await
    .unwrap();
    let card = job_cards::check_in(
        w.state(),
        sami,
        NewJobCard::new(vehicle.id, "Brake noise")
            .odometer(52_100)
            .condition("Scratch on the left door")
            .acknowledged_by("Mona Hassan")
            .inspection_fee(fee)
            .technician(tareq.0.id)
            .workshop(workshop()),
    )
    .await
    .unwrap();
    job_cards::start_inspection(w.state(), &tareq.1, card.id)
        .await
        .unwrap();
    for line in labour {
        job_cards::add_labour(w.state(), &tareq.1, card.id, line)
            .await
            .unwrap();
    }
    if send {
        job_cards::send_estimate(w.state(), sami, card.id, Promise::default())
            .await
            .unwrap();
    }
    JobCard::find_or_404(w.db(), card.id).await.unwrap()
}
