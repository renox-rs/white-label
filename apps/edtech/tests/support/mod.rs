//! Booting the edtech app in a `TestApp` with the real `wl-scheduling` (the
//! resources a batch names and the availability the sessions supply to) and
//! the real `wl-hr`.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use renox::auth::{Auth, Permissions, permissions};
use renox::chrono::{NaiveDate, TimeDelta};
use renox::db::DateTime;
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::calendar::BusinessCalendar;
use wl_kernel::documents::dubai_date;
use wl_kernel::invoicing::provide_invoicer;
use wl_kernel::notify::provide_notifier;
use wl_kernel::parties::provide_directory;
use wl_kernel::payments::{provide_payment_links, provide_payment_recorder};
use wl_kernel::sales::{provide_package_ledger, provide_package_sales};
use wl_kernel::subscriptions::provide_subscriptions;
use wl_scheduling::{Resource, ResourceType, WlScheduling};

/// The stand-ins of the parties, the invoice book, the subscriptions and the
/// packages the enrolments talk to.
pub mod money;

pub use money::{Books, Cash, Messages, Packages, People, Subs};

/// The pages extend the app's layout; a package test has no app, so it
/// brings the smallest one.
const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

/// A centre with scheduling and HR on; `{holidays}` is filled in.
const CLIENT: &str = r#"
[client]
name = "Gulf Skills Training Centre LLC"
app = "edtech"

[modules]
enabled = ["parties", "hr", "scheduling"]

[roles]
owner = "owner"

[[legal_entities]]
code = "GST"
name_en = "Gulf Skills Training Centre LLC"
default = true

[calendar]
holidays = [{holidays}]
"#;

/// The day `days` from today in Asia/Dubai.
pub fn day(days: i64) -> NaiveDate {
    dubai_date(renox::db::now()) + TimeDelta::days(days)
}

/// The first `weekday` (1 = Monday ... 7 = Sunday) on or after `from`.
pub fn next_weekday(from: NaiveDate, weekday: u32) -> NaiveDate {
    use renox::chrono::Datelike;
    let mut date = from;
    while date.weekday().number_from_monday() != weekday {
        date += TimeDelta::days(1);
    }
    date
}

/// Asia/Dubai `date` at `hour:minute`, as an instant.
pub fn at(date: NaiveDate, hour: u32, minute: u32) -> DateTime {
    BusinessCalendar::default().from_local(date.and_hms_opt(hour, minute, 0).expect("a time"))
}

/// Provides the probe route that gives a test the signed-in user, and the
/// stand-ins of the contracts the enrolments use.
struct Harness {
    who: Arc<Mutex<Option<renox::AuthUser>>>,
    people: People,
    books: Books,
    subs: Subs,
    packages: Packages,
    cash: Cash,
    messages: Messages,
}

impl Module for Harness {
    fn name(&self) -> &'static str {
        "edtech-test-harness"
    }

    fn register(&self, app: &mut Registry) {
        provide_directory(app, self.people.clone());
        provide_invoicer(app, self.books.clone());
        provide_subscriptions(app, self.subs.clone());
        provide_package_ledger(app, self.packages.clone());
        provide_package_sales(app, self.packages.clone());
        provide_payment_recorder(app, self.cash.clone());
        provide_payment_links(app, self.cash.clone());
        provide_notifier(app, self.messages.clone());
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
pub struct Centre {
    /// The app.
    pub app: TestApp,
    /// The parties the centre knows.
    pub people: People,
    /// The invoices drafted.
    pub books: Books,
    /// The subscriptions started and cancelled.
    pub subs: Subs,
    /// The packages sold.
    pub packages: Packages,
    /// The payments the centre asked about and the refunds and links it asked.
    pub cash: Cash,
    /// The notifications asked of the notifier.
    pub messages: Messages,
    who: Arc<Mutex<Option<renox::AuthUser>>>,
    /// Keeps the views directory alive.
    _views: tempfile::TempDir,
}

/// Boots the edtech app: scheduling and HR, with the 10 reason codes.
pub async fn boot() -> Centre {
    boot_with_holidays(&[]).await
}

/// Like [`boot`] with `holidays` as the centre's public holidays.
pub async fn boot_with_holidays(holidays: &[NaiveDate]) -> Centre {
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();
    let who = Arc::new(Mutex::new(None));
    let (people, books, subs, packages) = (
        People::default(),
        Books::default(),
        Subs::default(),
        Packages::default(),
    );
    let (cash, messages) = (Cash::default(), Messages::default());
    let mut scheduling_module = WlScheduling::new();
    for (code, initiated_by) in edtech::reasons::SCHEDULING_REASON_CODES {
        scheduling_module = scheduling_module.reason_code(code, *initiated_by);
    }
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(renox::audit::Audit)
        .module(wl_hr::WlHr::new())
        .module(scheduling_module)
        .module(edtech::EdTech)
        .module(Harness {
            who: who.clone(),
            people: people.clone(),
            books: books.clone(),
            subs: subs.clone(),
            packages: packages.clone(),
            cash: cash.clone(),
            messages: messages.clone(),
        });
    let listed: Vec<String> = holidays.iter().map(|d| format!("\"{d}\"")).collect();
    let client = CLIENT.replace("{holidays}", &listed.join(", "));
    let config = wl_kernel::ClientConfig::parse(&client).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    Centre {
        app,
        people,
        books,
        subs,
        packages,
        cash,
        messages,
        who,
        _views: views,
    }
}

impl Centre {
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

    /// The academic coordinator: plans and opens batches.
    pub async fn coordinator_user(&self) -> User {
        self.user(
            "Amal",
            &[
                edtech::permissions::COURSES_VIEW,
                edtech::permissions::COURSES_MANAGE,
                edtech::permissions::BATCHES_VIEW,
                edtech::permissions::BATCHES_MANAGE,
                edtech::permissions::SESSIONS_VIEW,
            ],
        )
        .await
    }

    /// The academic coordinator as the signed-in user the services take.
    pub async fn coordinator(&self) -> renox::AuthUser {
        let user = self.coordinator_user().await;
        self.auth(&user).await
    }

    /// A tutor: teaches the sessions assigned, plans nothing.
    pub async fn teacher_user(&self) -> User {
        self.user(
            "Tariq",
            &[
                edtech::permissions::BATCHES_VIEW,
                edtech::permissions::SESSIONS_VIEW,
                edtech::permissions::SESSIONS_TEACH,
            ],
        )
        .await
    }

    /// A tutor as the signed-in user the services take.
    pub async fn teacher(&self) -> renox::AuthUser {
        let user = self.teacher_user().await;
        self.auth(&user).await
    }

    /// The counsellor: looks at courses, batches and sessions.
    pub async fn counsellor_user(&self) -> User {
        self.user(
            "Cara",
            &[
                edtech::permissions::COURSES_VIEW,
                edtech::permissions::BATCHES_VIEW,
                edtech::permissions::SESSIONS_VIEW,
            ],
        )
        .await
    }

    /// The counsellor as the signed-in user the services take.
    pub async fn counsellor(&self) -> renox::AuthUser {
        let user = self.counsellor_user().await;
        self.auth(&user).await
    }

    /// A person resource (a tutor) of wl-scheduling.
    pub async fn tutor(&self, name: &str) -> Resource {
        let mut resource = Resource::new(name, ResourceType::Person);
        resource.save(self.db()).await.unwrap();
        resource
    }

    /// A room of wl-scheduling seating `capacity` persons.
    pub async fn room(&self, name: &str, capacity: i64) -> Resource {
        let mut resource = Resource::new(name, ResourceType::Room);
        resource.capacity = capacity;
        resource.save(self.db()).await.unwrap();
        resource
    }
}

use edtech::batches::{self, NewBatch};
use edtech::courses::{self, NewCourse};
use edtech::{Batch, Course, Session};

impl Centre {
    /// The course `IELTS-B2` (defined once).
    pub async fn course(&self) -> Course {
        if let Some(found) = Course::by_code(self.db(), "IELTS-B2").await.unwrap() {
            return found;
        }
        let amal = self.coordinator().await;
        courses::create(
            self.state(),
            &amal,
            NewCourse::new("IELTS-B2", "IELTS preparation", "English language", 40).level("B2"),
        )
        .await
        .expect("the course is defined")
    }

    /// A batch of 12 seats planned by the coordinator on `weekdays` at 18:00
    /// for 90 minutes over four weeks from `first`, with `tutor` and `room`.
    pub async fn batch_of(
        &self,
        tutor: &Resource,
        room: &Resource,
        first: NaiveDate,
        weekdays: &[i64],
    ) -> Batch {
        let course = self.course().await;
        let amal = self.coordinator().await;
        batches::create(
            self.state(),
            &amal,
            NewBatch::new(
                course.id,
                12,
                (first, first + TimeDelta::days(27)),
                weekdays,
                "18:00",
                90,
            )
            .tutor(tutor.id, None)
            .room(room.id, Some(room.capacity)),
        )
        .await
        .expect("the batch is planned")
    }

    /// A batch of the Monday and Wednesday pattern, opened.
    pub async fn open_batch(&self, tutor: &Resource, room: &Resource, first: NaiveDate) -> Batch {
        let batch = self.batch_of(tutor, room, first, &[1, 3]).await;
        let amal = self.coordinator().await;
        batches::open(self.state(), &amal, batch.id)
            .await
            .expect("the batch opens")
    }

    /// The sessions of `batch`, in date order.
    pub async fn sessions_of(&self, batch: &Batch) -> Vec<Session> {
        Session::of_batch(self.db(), batch.id).await.unwrap()
    }

    /// A `scheduled` session of `batch` that started `days` days ago at 10:00
    /// for an hour, with the batch's tutor and room.
    pub async fn past_session(&self, batch: &Batch, number: i64, days: i64) -> Session {
        let date = day(-days);
        let mut session = Session {
            batch_id: batch.id,
            number,
            session_date: date,
            starts_at: at(date, 10, 0),
            ends_at: at(date, 11, 0),
            tutor_resource_id: batch.tutor_resource_id,
            room_resource_id: batch.room_resource_id,
            ..Session::default()
        };
        session.save(self.db()).await.unwrap();
        session
    }
}

use edtech::enrolments::{self, NewEnrolment};
use edtech::seats::{self, Limit};
use edtech::{Enrolment, EnrolmentStatus, SessionStatus};
use renox::state_machine::StateMachine;

/// The price of the course the enrolment tests sell: AED 2,520 with VAT.
pub const PRICE: i64 = 252_000;

impl Centre {
    /// A counsellor at the front desk: enrols and withdraws, never exceeds a
    /// batch's capacity or approves a discount.
    pub async fn desk_user(&self) -> User {
        self.user(
            "Dina",
            &[
                edtech::permissions::COURSES_VIEW,
                edtech::permissions::BATCHES_VIEW,
                edtech::permissions::SESSIONS_VIEW,
                edtech::permissions::ENROLMENTS_VIEW,
                edtech::permissions::ENROLMENTS_MANAGE,
                edtech::permissions::CERTIFICATES_VIEW,
            ],
        )
        .await
    }

    /// The desk counsellor as the signed-in user the services take.
    pub async fn desk(&self) -> renox::AuthUser {
        let user = self.desk_user().await;
        self.auth(&user).await
    }

    /// The centre manager: enrols above capacity up to the room's limit and
    /// approves a discount.
    pub async fn manager_user(&self) -> User {
        self.user(
            "Mona",
            &[
                edtech::permissions::COURSES_VIEW,
                edtech::permissions::BATCHES_VIEW,
                edtech::permissions::BATCHES_MANAGE,
                edtech::permissions::BATCHES_APPROVE,
                edtech::permissions::SEATS_EXCEED,
                edtech::permissions::SESSIONS_VIEW,
                edtech::permissions::ENROLMENTS_VIEW,
                edtech::permissions::ENROLMENTS_MANAGE,
                edtech::permissions::ENROLMENTS_APPROVE,
                edtech::permissions::CERTIFICATES_VIEW,
                edtech::permissions::CERTIFICATES_ISSUE,
            ],
        )
        .await
    }

    /// The centre manager as the signed-in user the services take.
    pub async fn manager(&self) -> renox::AuthUser {
        let user = self.manager_user().await;
        self.auth(&user).await
    }

    /// The coordinator who issues certificates.
    pub async fn registrar_user(&self) -> User {
        self.user(
            "Rana",
            &[
                edtech::permissions::CERTIFICATES_VIEW,
                edtech::permissions::CERTIFICATES_ISSUE,
                edtech::permissions::ENROLMENTS_VIEW,
            ],
        )
        .await
    }

    /// The registrar as the signed-in user the services take.
    pub async fn registrar(&self) -> renox::AuthUser {
        let user = self.registrar_user().await;
        self.auth(&user).await
    }

    /// The company admin: revokes certificates.
    pub async fn company_admin_user(&self) -> User {
        self.user(
            "Zayed",
            &[
                edtech::permissions::CERTIFICATES_VIEW,
                edtech::permissions::CERTIFICATES_REVOKE,
            ],
        )
        .await
    }

    /// The company admin as the signed-in user the services take.
    pub async fn company_admin(&self) -> renox::AuthUser {
        let user = self.company_admin_user().await;
        self.auth(&user).await
    }

    /// Party `id` named `name` in the directory.
    pub fn person(&self, id: i64, name: &str) -> i64 {
        self.people.add(id, name);
        id
    }

    /// A course enrolment of `student` in `batch` at [`PRICE`], upfront, with
    /// the terms accepted, made by the desk counsellor.
    pub async fn enrol(&self, student: i64, batch: &Batch) -> Enrolment {
        let desk = self.desk().await;
        enrolments::create(
            self.state(),
            &desk,
            NewEnrolment::course(student, batch.id, PRICE).terms_accepted(),
        )
        .await
        .expect("the student is enrolled")
    }

    /// An enrolment of `student` in `batch` inserted in `status` (a fixture),
    /// holding a seat when the status takes one.
    pub async fn enrolment_in(
        &self,
        batch: &Batch,
        student: i64,
        status: EnrolmentStatus,
    ) -> Enrolment {
        let mut row = Enrolment {
            batch_id: Some(batch.id),
            student_party_id: student,
            payer_party_id: student,
            price: PRICE,
            discount_approved: true,
            sessions_total: 8,
            instalments: 1,
            terms_accepted_at: Some(renox::db::now()),
            seat_held: status.takes_a_seat(),
            seat_hold_until: (status == EnrolmentStatus::PendingPayment)
                .then(|| renox::db::now() + TimeDelta::hours(48)),
            status,
            ..Enrolment::default()
        };
        let mut tx = self.db().begin().await.unwrap();
        if status.takes_a_seat() {
            seats::take(&mut tx, batch.id, 1, Limit::Room).await.unwrap();
        }
        row.insert_in_state(&mut tx, status, "fixture").await.unwrap();
        tx.commit().await.unwrap();
        Enrolment::find_or_404(self.db(), row.id).await.unwrap()
    }

    /// The seat hold of `enrolment` runs out and it lapses: `lapse` run a
    /// minute after the hold ends.
    pub async fn lapse(&self, enrolment: &Enrolment) -> Enrolment {
        let until = enrolment.seat_hold_until.expect("a seat hold");
        edtech::lapse::lapse_one(self.state(), enrolment.id, until + TimeDelta::minutes(1))
            .await
            .expect("the run works")
            .expect("the enrolment lapses")
    }

    /// A `held` session of `batch` that started `days` days ago (a fixture).
    pub async fn held_session(&self, batch: &Batch, number: i64, days: i64) -> Session {
        let date = day(-days);
        let mut session = Session {
            batch_id: batch.id,
            number,
            session_date: date,
            starts_at: at(date, 10, 0),
            ends_at: at(date, 11, 0),
            tutor_resource_id: batch.tutor_resource_id,
            room_resource_id: batch.room_resource_id,
            held_at: Some(renox::db::now()),
            status: SessionStatus::Held,
            ..Session::default()
        };
        let mut tx = self.db().begin().await.unwrap();
        session
            .insert_in_state(&mut tx, SessionStatus::Held, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        session
    }
}
