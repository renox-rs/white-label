//! Shared by the host tests of the 08 Auto claims that end early (BL-AUT-08 ..
//! BL-AUT-11): the demo workshop booted with the real modules, its staff, the
//! insurer under a payer agreement with an accepted rate card, and a job card
//! under an approved claim with the police report attached and accepted.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use auto::claims::{self, Answer, NewClaim};
use auto::insurers::{self, NewInsurer};
use auto::job_cards::{self, NewJobCard, NewLabour, Promise};
use auto::vehicles::{self, NewVehicle};
use auto::{Emirate, InsuranceClaim, JobCard, LabourLine, WorkStatus};
use host::roles::{
    ACCOUNTANT, BRANCH_MANAGER, INSURANCE_COORDINATOR, PROCUREMENT, SERVICE_ADVISOR, TECHNICIAN,
    WORKSHOP_MANAGER,
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
    let (_, basma) = w.staff("Basma", BRANCH_MANAGER).await;
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
