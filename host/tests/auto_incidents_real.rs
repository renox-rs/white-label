//! BL-AUT-06 (#402, `AUT.ACL.22`) against the REAL wl-ticketing of the host:
//! a vehicle incident is a ticket of the category `vehicle_incident` linked to
//! the job card; nobody resolves it from the ticket's own page; the branch
//! manager decides the liability on the job card and the ticket is resolved
//! with the decision; an involved manager cannot decide. The app's own tests
//! (`apps/auto/tests/aut_06_incidents.rs`) show the same points against a
//! stand-in.

mod support;

use std::sync::{Arc, Mutex};

use auto::incidents::{self, NewIncident};
use auto::job_cards::{self, NewJobCard};
use auto::test_drives::{self, NewDrive};
use auto::vehicles::{self, NewVehicle};
use auto::{Emirate, JobCard, JobCardStatus};
use host::roles::{BRANCH_MANAGER, OWNER, SERVICE_ADVISOR, TECHNICIAN};
use renox::prelude::*;
use renox::testing::TestApp;
use support::{person, tenant};
use wl_kernel::Subject;
use wl_ticketing::{Ticket, TicketStatus};

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
        "auto-incidents-real-who"
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

/// JC-60 in progress, assigned to Tareq (opened by the advisor).
async fn jc60(w: &World, sami: &AuthUser) -> (JobCard, AuthUser) {
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
        NewJobCard::new(vehicle.id, "Brake noise")
            .odometer(52_100)
            .condition("Scratch on the left door")
            .acknowledged_by("Mona Hassan")
            .technician(tareq_user.id)
            .workshop(Subject::new("scheduling.location", 1)),
    )
    .await
    .unwrap();
    (card, tareq)
}

fn words(error: &Error) -> String {
    error.to_string()
}

/// #402 points 1 to 3 with the real wl-ticketing: the technician records a
/// test drive and raises the incident (a real ticket of the category, about the
/// job card); the ticket cannot be resolved from the ticketing side; the branch
/// manager decides *workshop* and the ticket is resolved with the decision and
/// the internal job card exists for the same vehicle.
#[renox::test]
async fn aut_06_real_the_incident_ticket_is_resolved_by_the_branch_managers_decision() {
    let w = boot().await;
    let (_, sami) = w.staff("Sami", SERVICE_ADVISOR).await;
    let (_, basma) = w.staff("Basma", BRANCH_MANAGER).await;
    let (card, tareq) = jc60(&w, &sami).await;
    // The job card is checked in: the drive is recorded while it is open.
    let drive = test_drives::record(w.state(), &tareq, card.id, NewDrive::out(52_110))
        .await
        .unwrap();
    test_drives::finish(w.state(), &tareq, drive.id, 52_122, None)
        .await
        .unwrap();

    let incident = incidents::report(
        w.state(),
        &tareq,
        card.id,
        NewIncident::new("Scratch on the right door").road_accident(),
    )
    .await
    .unwrap();
    let ticket_id = incident.ticket_id.expect("a real ticket");
    let ticket = Ticket::find_or_404(w.db(), ticket_id).await.unwrap();
    assert_eq!(ticket.subject(), card.subject());
    assert!(incident.ticket_number.as_deref().unwrap().starts_with("TKT-"));

    // From the ticketing side nobody resolves it.
    let refusal = wl_ticketing::work::resolve(w.state(), &basma, &ticket, renox::db::now())
        .await
        .unwrap_err();
    assert!(words(&refusal.into_error()).contains("job card"));

    // The technician and the advisor cannot decide.
    assert!(matches!(
        incidents::decide(w.state(), &tareq, incident.id, "workshop", "x")
            .await
            .unwrap_err(),
        Error::Forbidden
    ));
    assert!(matches!(
        incidents::decide(w.state(), &sami, incident.id, "workshop", "x")
            .await
            .unwrap_err(),
        Error::Forbidden
    ));
    let decided = incidents::decide(
        w.state(),
        &basma,
        incident.id,
        "workshop",
        "The technician hit the post",
    )
    .await
    .unwrap();
    let internal = JobCard::find_or_404(w.db(), decided.internal_job_card_id.unwrap())
        .await
        .unwrap();
    assert!(internal.internal && internal.accident_damage);
    assert_eq!(internal.vehicle_id, card.vehicle_id);
    assert_eq!(internal.status, JobCardStatus::CheckedIn);
    let after = Ticket::find_or_404(w.db(), ticket_id).await.unwrap();
    assert!(matches!(
        after.status,
        TicketStatus::Resolved | TicketStatus::Closed
    ));
}

/// #402 point 6 with the real wl-ticketing: the branch manager who drove the
/// test drive cannot decide the incident; the owner can.
#[renox::test]
async fn aut_06_real_an_involved_branch_manager_cannot_decide_but_the_owner_can() {
    let w = boot().await;
    let (_, sami) = w.staff("Sami", SERVICE_ADVISOR).await;
    let (_, basma) = w.staff("Basma", BRANCH_MANAGER).await;
    let (_, owner) = w.staff("Omar", OWNER).await;
    let (card, tareq) = jc60(&w, &sami).await;
    test_drives::record(
        w.state(),
        &basma,
        card.id,
        NewDrive::out(52_110),
    )
    .await
    .unwrap();
    let incident = incidents::report(w.state(), &tareq, card.id, NewIncident::new("Dent"))
        .await
        .unwrap();
    let refusal = incidents::decide(w.state(), &basma, incident.id, "workshop", "Our fault")
        .await
        .unwrap_err();
    assert!(words(&refusal).contains("involved"), "{refusal}");
    let decided = incidents::decide(w.state(), &owner, incident.id, "not_workshop", "Customer's")
        .await
        .unwrap();
    assert_eq!(decided.decided_by, Some(owner.id));
    assert_eq!(decided.internal_job_card_id, None);
}
