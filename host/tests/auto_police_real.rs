//! BL-AUT-07 (#403, `AUT.ACL.20`, D-08) against the REAL wl-documents (and the
//! real wl-parties) of the host: the police accident report is a document of
//! the job card or its claim that a second user accepts; until then the job
//! card `approve` and the claim's `book_survey`, `submit_estimate` and
//! `approve` are refused with "police report required"; a report that is
//! archived refuses again; a report on the claim counts for the job card; no
//! override request can be made for the kind. The app's own tests
//! (`apps/auto/tests/aut_07_police_report.rs`) show the same points against a
//! stand-in guard.

mod support;

use std::sync::{Arc, Mutex};

use auto::claims::{self, Answer, NewClaim};
use auto::insurers::{self, NewInsurer};
use auto::job_cards::{self, NewJobCard, NewLabour, Promise};
use auto::vehicles::{self, NewVehicle};
use auto::{ClaimStatus, Emirate, InsuranceClaim, JobCard, JobCardStatus, LabourLine};
use host::roles::{
    BRANCH_MANAGER, INSURANCE_COORDINATOR, SERVICE_ADVISOR, TECHNICIAN, WORKSHOP_MANAGER,
};
use renox::prelude::*;
use renox::testing::TestApp;
use renox::upload::Upload;
use support::{company, day, person, tenant};
use wl_documents::{Document, DocumentStatus, Documents, NewDocument};
use wl_kernel::Subject;

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
        "auto-police-real-who"
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

/// An accident job card (report number typed at check-in) with a door repair
/// estimate sent, and its labour line.
async fn accident_card(w: &World, sami: &AuthUser) -> (JobCard, LabourLine) {
    let (tareq_user, tareq) = w.staff("Tareq", TECHNICIAN).await;
    let owner = tenant(&w.app, "Mona Hassan").await;
    let vehicle = vehicles::create(
        w.state(),
        sami,
        NewVehicle::new(Emirate::Dubai, "A", "12345", "Toyota", "Hilux", owner.0)
            .vin("JTHBK262582012345")
            .odometer(52_000),
    )
    .await
    .unwrap();
    let card = job_cards::check_in(
        w.state(),
        sami,
        NewJobCard::new(vehicle.id, "Right side hit")
            .odometer(52_100)
            .condition("Right door dented")
            .acknowledged_by("Mona Hassan")
            .technician(tareq_user.id)
            .workshop(Subject::new("scheduling.location", 1))
            .accident("DXB-PR-70"),
    )
    .await
    .unwrap();
    job_cards::start_inspection(w.state(), &tareq, card.id)
        .await
        .unwrap();
    let door = job_cards::add_labour(
        w.state(),
        &tareq,
        card.id,
        NewLabour::hourly("Door panel repair", 2_000, 20_000),
    )
    .await
    .unwrap();
    let card = job_cards::send_estimate(w.state(), sami, card.id, Promise::default())
        .await
        .unwrap();
    assert_eq!(card.status, JobCardStatus::AwaitingApproval);
    assert!(card.accident_damage);
    (card, door)
}

/// The police report on `subject`, uploaded by `by` and waiting for review.
async fn upload_report(w: &World, by: &AuthUser, subject: Subject) -> Document {
    let upload = Upload::new(
        "police-report.pdf",
        "application/pdf",
        b"%PDF-1.7 police report".to_vec(),
    );
    Documents::attach(
        w.state(),
        Some(by),
        NewDocument::new(subject, "police_report", upload).number("DXB-PR-70"),
    )
    .await
    .unwrap()
    .expect("the report is attached")
}

fn words(error: &Error) -> String {
    error.to_string()
}

/// #403 points 1 and 4 with the real wl-documents: JC-70 is flagged and the
/// report is uploaded but not yet accepted, so the customer's approval is
/// refused with "police report required"; the advisor who uploaded it cannot
/// accept it; the workshop manager accepts it and the approval goes through.
#[renox::test]
async fn aut_07_real_an_accident_job_card_is_approved_once_a_second_user_accepts_the_report() {
    let w = boot().await;
    let (_, sami) = w.staff("Sami", SERVICE_ADVISOR).await;
    let (_, wanda) = w.staff("Wanda", WORKSHOP_MANAGER).await;
    let (card, _door) = accident_card(&w, &sami).await;

    let refusal = job_cards::approve_recorded(w.state(), &sami, card.id, "Mona", "recorded", None)
        .await
        .unwrap_err();
    assert!(
        words(&refusal).contains("Police report required"),
        "{refusal}"
    );

    let report = upload_report(&w, &sami, card.subject()).await;
    assert_eq!(report.status, DocumentStatus::PendingReview);
    let still = job_cards::approve_recorded(w.state(), &sami, card.id, "Mona", "recorded", None)
        .await
        .unwrap_err();
    assert!(
        words(&still).contains("Police report required"),
        "a report waiting for review is not accepted: {still}"
    );
    assert!(
        Documents::accept(w.state(), &sami, &report).await.is_err(),
        "the uploader does not accept"
    );

    Documents::accept(w.state(), &wanda, &report).await.unwrap();
    let approved = job_cards::approve_recorded(w.state(), &sami, card.id, "Mona", "recorded", None)
        .await
        .unwrap();
    assert_eq!(approved.status, JobCardStatus::InProgress);
}

/// #403 points 2, 3 and 4 with the real wl-documents: CL-7 `registered`
/// without a report refuses `book_survey` and `submit_estimate`; a report on
/// the claim, accepted by a second user, lets both go; the report is then
/// archived and the claim `approve` is refused; another report accepted lets
/// the claim approve and starts the job card with the report that sits on the
/// claim.
#[renox::test]
async fn aut_07_real_the_claim_steps_follow_the_report_and_an_archived_report_refuses_again() {
    let w = boot().await;
    let (_, sami) = w.staff("Sami", SERVICE_ADVISOR).await;
    let (_, wanda) = w.staff("Wanda", WORKSHOP_MANAGER).await;
    let (_, bashir) = w.staff("Bashir", BRANCH_MANAGER).await;
    let (_, carla) = w.staff("Carla", INSURANCE_COORDINATOR).await;
    let (card, door) = accident_card(&w, &sami).await;
    let orient = company(&w.app, "Orient Insurance").await;
    let insurer = insurers::register(w.state(), &carla, NewInsurer::new(orient.0))
        .await
        .unwrap();
    let vehicle = card.vehicle_id;
    let claim = claims::register(
        w.state(),
        &carla,
        NewClaim::new(insurer.id, vehicle, "POL-7788", "OR-2026-118", day(-3))
            .job_card(card.id)
            .police_report("DXB-PR-70", Some(day(-2)))
            .excess(20_000),
    )
    .await
    .unwrap();

    let survey = claims::book_survey(w.state(), &carla, claim.id)
        .await
        .unwrap_err();
    assert!(
        words(&survey).contains("Police report required"),
        "{survey}"
    );
    let submit = claims::submit_estimate(w.state(), &carla, claim.id)
        .await
        .unwrap_err();
    assert!(
        words(&submit).contains("Police report required"),
        "{submit}"
    );

    let report = upload_report(&w, &carla, claim.subject()).await;
    Documents::accept(w.state(), &wanda, &report).await.unwrap();
    claims::book_survey(w.state(), &carla, claim.id)
        .await
        .unwrap();
    let submitted = claims::submit_estimate(w.state(), &carla, claim.id)
        .await
        .unwrap();
    assert_eq!(submitted.status, ClaimStatus::EstimateSubmitted);

    // The report is removed from the claim: archived by the branch manager.
    let accepted = Document::find_or_404(w.db(), report.id).await.unwrap();
    Documents::archive(w.state(), &bashir, &accepted, Some("Wrong file"))
        .await
        .unwrap();
    let answer = || Answer::new("LPO-5521", "orient-approval-1").labour(door.id, 40_000);
    let refusal = claims::approve(w.state(), &carla, claim.id, answer())
        .await
        .unwrap_err();
    assert!(
        words(&refusal).contains("Police report required"),
        "{refusal}"
    );
    assert_eq!(
        InsuranceClaim::find_or_404(w.db(), claim.id)
            .await
            .unwrap()
            .status,
        ClaimStatus::EstimateSubmitted
    );

    // Another report is accepted: the claim approves and the job card starts.
    let second = upload_report(&w, &carla, claim.subject()).await;
    Documents::accept(w.state(), &wanda, &second).await.unwrap();
    let approved = claims::approve(w.state(), &carla, claim.id, answer())
        .await
        .unwrap();
    assert_eq!(approved.status, ClaimStatus::Approved);
    assert_eq!(
        JobCard::find_or_404(w.db(), card.id).await.unwrap().status,
        JobCardStatus::InProgress
    );
}

/// #403 point 5 with the real wl-documents: the kind is a legal block with
/// override mode never, so no override request can be created for the job card
/// `approve`, the roles to attach and accept the report exist, and the
/// refusal offers no override link.
#[renox::test]
async fn aut_07_real_no_override_can_be_requested_for_the_police_report() {
    let w = boot().await;
    let (_, sami) = w.staff("Sami", SERVICE_ADVISOR).await;
    let (wanda_user, _) = w.staff("Wanda", WORKSHOP_MANAGER).await;
    let (card, _door) = accident_card(&w, &sami).await;

    let kind = wl_documents::DocumentKind::by_key(w.db(), "police_report")
        .await
        .unwrap()
        .expect("the seed stored the police report kind");
    assert!(kind.legal_block);
    assert_eq!(
        kind.override_mode,
        wl_kernel::documents::OverrideMode::Never
    );

    let asked = Documents::request_override(
        w.state(),
        &wanda_user,
        "police_report",
        "job_card.approve",
        card.id,
        "The customer is in a hurry",
    )
    .await
    .unwrap_err();
    assert!(words(&asked).contains("cannot be overridden"), "{asked}");

    let refusal = wl_kernel::documents::guard(w.state())
        .check_as(
            w.state(),
            "job_card.approve",
            card.id,
            wl_kernel::documents::today(),
            Some(wanda_user.id),
        )
        .await
        .unwrap()
        .unwrap_err();
    assert!(refusal.override_links().is_empty());
}
