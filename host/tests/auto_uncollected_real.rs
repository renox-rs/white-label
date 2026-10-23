//! BL-AUT-05 (#401, `AUT.RUL.18`) against the REAL wl-invoicing and
//! wl-ticketing of the host: the storage fee of a vehicle that was not
//! collected is invoiced at release on a real invoice (6 days at AED 50 =
//! AED 300 + VAT), no fee after a total loss, and the branch manager gets a
//! real task at 30 days. The app's own tests
//! (`apps/auto/tests/aut_05_uncollected.rs`) show the same points against
//! stand-ins.

mod support;

use std::sync::{Arc, Mutex};

use auto::job_cards::{self, NewJobCard};
use auto::release::{self, Release};
use auto::vehicles::{self, NewVehicle};
use auto::{Emirate, JobCard, JobCardStatus, settings, storage};
use host::roles::{BRANCH_MANAGER, SERVICE_ADVISOR};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use renox::testing::TestApp;
use support::{person, tenant};
use wl_kernel::Subject;
use wl_kernel::documents::dubai_date;
use wl_kernel::invoicing::{InvoiceId, invoicer};
use wl_kernel::tickets::task_creator;

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
        "auto-uncollected-real-who"
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

/// A cancelled job card that began to wait `back` days ago, with a storage fee
/// of AED 50 a day agreed at check-in.
async fn waiting(w: &World, sami: &AuthUser, back: i64, code: &str) -> JobCard {
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
            .storage_fee(5_000)
            .workshop(Subject::new("scheduling.location", 1)),
    )
    .await
    .unwrap();
    JobCard::query()
        .where_(JobCard::ID.eq(card.id))
        .update(
            w.db(),
            &[
                ("status", &JobCardStatus::Cancelled),
                ("closed_at", &Some(renox::db::now() - TimeDelta::days(back))),
                ("closed_code", &Some(code.to_owned())),
            ],
        )
        .await
        .unwrap();
    JobCard::find_or_404(w.db(), card.id).await.unwrap()
}

/// #401 point 2 with the real invoicing: storage on at AED 50 a day and agreed
/// at check-in, the first reminder fell due 6 days ago, release today: one
/// real invoice of AED 300 + VAT = AED 315.
#[renox::test]
async fn aut_05_real_release_invoices_six_days_of_storage() {
    let w = boot().await;
    let (_, sami) = w.staff("Sami", SERVICE_ADVISOR).await;
    settings::set(w.db(), settings::STORAGE_FEE_ENABLED, "1", None)
        .await
        .unwrap();
    let card = waiting(&w, &sami, 9, "customer_stopped").await;
    let start = storage::start_of(&card).expect("cancelled");
    storage::process(w.state(), start + renox::chrono::Duration::days(3))
        .await
        .unwrap();
    let released = release::release(w.state(), &sami, card.id, Release::safe("Mona Hassan"))
        .await
        .unwrap();
    assert_eq!(released.storage_days, 6);
    let invoice = invoicer(w.state())
        .find(w.db(), InvoiceId(released.storage_invoice_id.expect("an invoice")))
        .await
        .unwrap()
        .expect("the invoice is in the books");
    assert_eq!(invoice.gross_total, 31_500);
    assert_eq!(invoice.party.0, released.customer_party_id);
}

/// #401 points 3 and 4 with the real invoicing: switched off, or after a total
/// loss, release raises no storage invoice.
#[renox::test]
async fn aut_05_real_no_storage_invoice_when_off_or_after_a_total_loss() {
    let w = boot().await;
    let (_, sami) = w.staff("Sami", SERVICE_ADVISOR).await;
    let off = waiting(&w, &sami, 9, "customer_stopped").await;
    let start = storage::start_of(&off).unwrap();
    storage::process(w.state(), start + renox::chrono::Duration::days(3))
        .await
        .unwrap();
    let released = release::release(w.state(), &sami, off.id, Release::safe("Mona Hassan"))
        .await
        .unwrap();
    assert!(released.storage_invoice_id.is_none());

    settings::set(w.db(), settings::STORAGE_FEE_ENABLED, "1", None)
        .await
        .unwrap();
    let lost = waiting(&w, &sami, 9, "total_loss").await;
    storage::process(w.state(), start + renox::chrono::Duration::days(3))
        .await
        .unwrap();
    let released = release::release(w.state(), &sami, lost.id, Release::safe("Mona Hassan"))
        .await
        .unwrap();
    assert!(released.storage_invoice_id.is_none());
}

/// #401 point 1 with the real ticketing: 30 days after the vehicle began to
/// wait the branch manager has a real task that names the open legal steps.
#[renox::test]
async fn aut_05_real_the_branch_manager_has_a_task_at_thirty_days() {
    let w = boot().await;
    let (_, sami) = w.staff("Sami", SERVICE_ADVISOR).await;
    let (_, _basma) = w.staff("Basma", BRANCH_MANAGER).await;
    let card = waiting(&w, &sami, 31, "customer_stopped").await;
    let start = dubai_date(card.closed_at.expect("closed"));
    storage::process(w.state(), start + renox::chrono::Duration::days(29))
        .await
        .unwrap();
    let key = format!("auto.uncollected:{}", card.id);
    assert!(task_creator(w.state())
        .find_open(w.state(), &key)
        .await
        .unwrap()
        .is_none());
    storage::process(w.state(), start + renox::chrono::Duration::days(30))
        .await
        .unwrap();
    assert!(task_creator(w.state())
        .find_open(w.state(), &key)
        .await
        .unwrap()
        .is_some());
}
