//! Booting the auto app in a `TestApp` with the real `wl-scheduling` (the
//! workshop bookings whose check-in opens a job card). The contracts of the
//! other modules (parties, invoices, stock) are answered by the stand-ins in
//! this folder, which keep what they were asked in memory; the host tests
//! (`host/tests/auto_real.rs`) use the real modules.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use auto::JobCardCancelled;
use auto::fleet_contract::FleetContractTerminated;
use auto::job_card::JobCard;
use auto::labour_line::{LabourLine, LabourPricing, LineStatus};
use auto::part_line::{PartCondition, PartLine};
use auto::vehicles::{self, NewVehicle};
use auto::{Emirate, JobCardStatus, Vehicle, permissions as aut};
use renox::auth::{Auth, Permissions, permissions};
use renox::chrono::{NaiveDate, TimeDelta};
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use wl_kernel::calendar::BusinessCalendar;
use wl_kernel::documents::{dubai_date, provide_guard};
use wl_kernel::inventory::{provide_stock_service, provide_supplier_return_drafts};
use wl_kernel::invoicing::provide_invoicer;
use wl_kernel::notify::provide_notifier;
use wl_kernel::parties::provide_directory;
use wl_kernel::payer_bills::provide_payer_bills;
use wl_kernel::rate_cards::provide_rate_cards;
use wl_kernel::tickets::provide_task_creator;
use wl_kernel::voids::RecordVoided;
use wl_scheduling::WlScheduling;

/// The stand-ins of the parties and the invoice book.
pub mod money;

/// The fixtures of the insurance claim tests.
pub mod claimfx;

/// The fixtures of the fleet contract tests.
pub mod fleetfx;

/// The fixtures of the claims that end before the repair is paid.
pub mod stopfx;

/// The stand-ins of the payer bills and the rate cards.
pub mod payer;

/// The stand-in of the stock service.
pub mod stock;

/// The stand-in of the ticket book.
pub mod tickets;

/// The stand-in of the Return to supplier drafts.
pub mod returns;

/// The stand-in of the notifier.
pub mod messages;

/// The stand-in of the document guard.
pub mod documents;

pub use documents::Documents;
pub use messages::Messages;
pub use money::{Books, People};
pub use payer::{Cards, PayerBooks};
pub use returns::Returns;
pub use stock::Stock;
pub use tickets::Tickets;

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

/// The pages extend the app's layout; a package test has no app, so it
/// brings the smallest one.
const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

/// A workshop with scheduling on.
const CLIENT: &str = r#"
[client]
name = "Deira Auto Parts & Garage LLC"
app = "auto"

[modules]
enabled = ["parties", "scheduling"]

[roles]
owner = "owner"

[[legal_entities]]
code = "DAP"
name_en = "Deira Auto Parts & Garage LLC"
default = true
"#;

/// The day `days` from today in Asia/Dubai.
pub fn day(days: i64) -> NaiveDate {
    dubai_date(renox::db::now()) + TimeDelta::days(days)
}

/// Asia/Dubai `date` at `hour:minute`, as an instant.
pub fn at(date: NaiveDate, hour: u32, minute: u32) -> DateTime {
    BusinessCalendar::default().from_local(date.and_hms_opt(hour, minute, 0).expect("a time"))
}

/// Provides the probe route that gives a test the signed-in user, and the
/// stand-ins of the contracts the workshop uses.
struct Harness {
    who: Arc<Mutex<Option<renox::AuthUser>>>,
    people: People,
    books: Books,
    stock: Stock,
    payer_books: PayerBooks,
    cards: Cards,
    tickets: Tickets,
    messages: Messages,
    documents: Documents,
    returns: Returns,
}

impl Module for Harness {
    fn name(&self) -> &'static str {
        "auto-test-harness"
    }

    fn register(&self, app: &mut Registry) {
        provide_directory(app, self.people.clone());
        provide_invoicer(app, self.books.clone());
        provide_stock_service(app, self.stock.clone());
        provide_payer_bills(app, self.payer_books.clone());
        provide_rate_cards(app, self.cards.clone());
        provide_task_creator(app, self.tickets.clone());
        provide_notifier(app, self.messages.clone());
        provide_guard(app, self.documents.clone());
        provide_supplier_return_drafts(app, self.returns.clone());
    }

    fn routes(&self) -> Routes {
        let who = self.who.clone();
        Routes::new().get("/t/whoami", move |user: renox::AuthUser| {
            let who = who.clone();
            async move {
                *who.lock().unwrap() = Some(user);
                "ok"
            }
        })
    }
}

/// The app under test.
pub struct Garage {
    /// The app.
    pub app: TestApp,
    /// The parties the workshop knows.
    pub people: People,
    /// The invoices drafted.
    pub books: Books,
    /// The store of the workshop.
    pub stock: Stock,
    /// The payer bills made.
    pub payer_books: PayerBooks,
    /// The rate cards of the insurers.
    pub cards: Cards,
    /// The warranty tickets and the tasks opened.
    pub tickets: Tickets,
    /// The notifications asked of the notifier.
    pub messages: Messages,
    /// The documents the workshop holds (the document guard).
    pub documents: Documents,
    /// The returns to supplier drafted.
    pub returns: Returns,
    /// `JobCardCancelled`.
    pub cancelled: Recorded<JobCardCancelled>,
    /// `RecordVoided`.
    pub voided: Recorded<RecordVoided>,
    /// `FleetContractTerminated`.
    pub terminated: Recorded<FleetContractTerminated>,
    who: Arc<Mutex<Option<renox::AuthUser>>>,
    /// Keeps the views directory alive.
    _views: tempfile::TempDir,
}

/// Boots the auto app: scheduling with the 08 reason codes.
pub async fn boot() -> Garage {
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();
    let who = Arc::new(Mutex::new(None));
    let (people, books, stock) = (People::default(), Books::default(), Stock::default());
    let (payer_books, cards) = (PayerBooks::default(), Cards::default());
    let tickets = Tickets::default();
    let messages = Messages::default();
    let documents = Documents::default();
    let returns = Returns::default();
    let mut scheduling_module = WlScheduling::new();
    for (code, initiated_by) in auto::reasons::SCHEDULING_REASON_CODES {
        scheduling_module = scheduling_module.reason_code(code, *initiated_by);
    }
    let cancelled = Recorded::<JobCardCancelled>::new();
    let voided = Recorded::<RecordVoided>::new();
    let terminated = Recorded::<FleetContractTerminated>::new();
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(renox::audit::Audit)
        .module(scheduling_module)
        .module(auto::Auto)
        .module(Harness {
            who: who.clone(),
            people: people.clone(),
            books: books.clone(),
            stock: stock.clone(),
            payer_books: payer_books.clone(),
            cards: cards.clone(),
            tickets: tickets.clone(),
            messages: messages.clone(),
            documents: documents.clone(),
            returns: returns.clone(),
        });
    let app = record(app, &cancelled);
    let app = record(app, &voided);
    let app = record(app, &terminated);
    let config = wl_kernel::ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    Garage {
        app,
        people,
        books,
        stock,
        payer_books,
        cards,
        tickets,
        messages,
        documents,
        returns,
        cancelled,
        voided,
        terminated,
        who,
        _views: views,
    }
}

impl Garage {
    /// `user` as the signed-in user the services take.
    pub async fn auth(&self, user: &User) -> renox::AuthUser {
        self.app.acting_as(user);
        self.app.get("/t/whoami").await.assert_ok();
        self.who
            .lock()
            .unwrap()
            .take()
            .expect("the probe route saw the signed-in user")
    }

    /// The database.
    pub fn db(&self) -> &Db {
        self.app.db()
    }

    /// The app state.
    pub fn state(&self) -> &AppState {
        self.app.state()
    }

    /// A user holding exactly `grants` (through a role of their own). The
    /// same name gives the same user.
    pub async fn user(&self, name: &str, grants: &[&str]) -> User {
        let email = format!("{}@example.com", name.to_lowercase());
        if let Some(found) = User::find_by_email(self.db(), &email).await.unwrap() {
            return found;
        }
        let user = User::register(self.db(), name, &email, "password123")
            .await
            .unwrap();
        let role = format!("role_{}", name.to_lowercase());
        permissions::define_role(self.db(), &role, grants)
            .await
            .unwrap();
        user.assign_role(self.db(), &role).await.unwrap();
        user
    }

    /// The service advisor Sami: registers vehicles, checks them in, sends
    /// the estimate and delivers.
    pub async fn advisor_user(&self) -> User {
        self.user(
            "Sami",
            &[
                aut::VEHICLES_VIEW,
                aut::VEHICLES_MANAGE,
                aut::JOB_CARDS_VIEW,
                aut::JOB_CARDS_ADVISE,
            ],
        )
        .await
    }

    /// The service advisor as the signed-in user the services take.
    pub async fn advisor(&self) -> renox::AuthUser {
        let user = self.advisor_user().await;
        self.auth(&user).await
    }

    /// A technician `name`: holds the `technician` role of the host.
    pub async fn technician_user(&self, name: &str) -> User {
        let email = format!("{}@example.com", name.to_lowercase());
        if let Some(found) = User::find_by_email(self.db(), &email).await.unwrap() {
            return found;
        }
        permissions::define_role(self.db(), aut::TECHNICIAN_ROLE, &[aut::JOB_CARDS_WORK])
            .await
            .unwrap();
        let user = User::register(self.db(), name, &email, "password123")
            .await
            .unwrap();
        user.assign_role(self.db(), aut::TECHNICIAN_ROLE)
            .await
            .unwrap();
        user
    }

    /// A technician as the signed-in user the services take.
    pub async fn technician(&self, name: &str) -> renox::AuthUser {
        let user = self.technician_user(name).await;
        self.auth(&user).await
    }

    /// The workshop manager Waleed: quality control and the labour menu.
    pub async fn qc_user(&self) -> User {
        self.user(
            "Waleed",
            &[
                aut::JOB_CARDS_VIEW,
                aut::JOB_CARDS_QC,
                aut::LABOUR_MENU_MANAGE,
            ],
        )
        .await
    }

    /// The workshop manager as the signed-in user the services take.
    pub async fn qc(&self) -> renox::AuthUser {
        let user = self.qc_user().await;
        self.auth(&user).await
    }

    /// A Toyota Hilux, Dubai A 12345, owned by party 7 (Mona Hassan), at
    /// 52,000 km.
    pub async fn hilux(&self) -> Vehicle {
        self.people.add(7, "Mona Hassan");
        if let Some(found) = Vehicle::by_plate(self.db(), Emirate::Dubai, "A", "12345")
            .await
            .unwrap()
        {
            return found;
        }
        let sami = self.advisor().await;
        vehicles::create(
            self.state(),
            &sami,
            NewVehicle::new(Emirate::Dubai, "A", "12345", "Toyota", "Hilux", 7)
                .vin("JTHBK262582012345")
                .year(2022)
                .odometer(52_000),
        )
        .await
        .expect("the vehicle is registered")
    }

    /// A job card of `vehicle` in `status`, inserted the way a fixture is, with
    /// its intake recorded and Tareq (when he exists) as its technician.
    pub async fn card_in(&self, vehicle: &Vehicle, status: JobCardStatus) -> JobCard {
        let mut card = JobCard {
            vehicle_id: vehicle.id,
            customer_party_id: vehicle.owner_party_id,
            complaint: "Brake noise when stopping".to_owned(),
            odometer_in: vehicle.odometer_km,
            intake_condition: Some("Scratch on the left door".to_owned()),
            intake_recorded_at: Some(renox::db::now()),
            workshop_type: Some("scheduling.location".to_owned()),
            workshop_id: Some(1),
            technician_user_id: Some(self.technician_user("Tareq").await.id),
            status,
            ..JobCard::default()
        };
        let mut tx = self.db().begin().await.unwrap();
        card.insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        self.reload(card.id).await
    }

    /// The job card, read again.
    pub async fn reload(&self, id: i64) -> JobCard {
        JobCard::find_or_404(self.db(), id).await.unwrap()
    }

    /// An hourly labour line of `card`, approved: `description` for `hours_milli`
    /// at `rate` fils an hour.
    pub async fn approved_labour(
        &self,
        card: &JobCard,
        description: &str,
        hours_milli: i64,
        rate: i64,
    ) -> LabourLine {
        let mut line = LabourLine {
            job_card_id: card.id,
            description: description.to_owned(),
            pricing: LabourPricing::Hourly,
            hours_milli,
            rate,
            approval: LineStatus::Approved,
            ..LabourLine::default()
        };
        line.save(self.db()).await.unwrap();
        line
    }

    /// A part line of `card`, approved: `quantity_milli` of `description` at
    /// `unit_price` fils, of the stock item `item` when given.
    pub async fn approved_part(
        &self,
        card: &JobCard,
        description: &str,
        item: Option<&str>,
        quantity_milli: i64,
        unit_price: i64,
    ) -> PartLine {
        let mut line = PartLine {
            job_card_id: card.id,
            item_code: item.map(str::to_owned),
            description: description.to_owned(),
            quantity_milli,
            unit_price,
            part_condition: PartCondition::New,
            approval: LineStatus::Approved,
            ..PartLine::default()
        };
        line.save(self.db()).await.unwrap();
        line
    }

    /// The part line, read again.
    pub async fn part_now(&self, id: i64) -> PartLine {
        PartLine::find_or_404(self.db(), id).await.unwrap()
    }

    /// The labour line, read again.
    pub async fn labour_now(&self, id: i64) -> LabourLine {
        LabourLine::find_or_404(self.db(), id).await.unwrap()
    }
}

/// The transition `name` of `card` as the system, for a test that needs one
/// the services do not offer.
pub async fn run_as_system(
    t: &Garage,
    card: &JobCard,
    name: &str,
    input: auto::job_card_machine::JobInput,
) -> Result<JobCard, renox::state_machine::TransitionError> {
    card.transition(name)
        .by_system()
        .input(&input)
        .run(t.state())
        .await
        .map(|_| ())?;
    Ok(t.reload(card.id).await)
}
