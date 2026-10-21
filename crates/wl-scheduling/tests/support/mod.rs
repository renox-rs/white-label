//! Booting wl-scheduling in a `TestApp` with auth: users who hold the
//! permissions of the spec, the kernel events recorded, and the fixtures a
//! booking needs (a practitioner open all week, a service type they deliver).

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use renox::auth::{Auth, Permissions, permissions};
use renox::chrono::{NaiveDate, TimeDelta, Timelike};
use renox::db::DateTime;
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::calendar::BusinessCalendar;
use wl_kernel::notify::{
    self, MessageId, NotificationTypeDef, Notifier, NotifyChannel, NotifyFuture, NotifyRequest,
    provide_notifier,
};
use wl_kernel::payments::{
    AllocationTarget, PaymentFuture, PaymentId, PaymentRecorder, PaymentSummary, RecordPayment,
    RefundId, RefundRequest, ReversalReason, provide_payment_recorder,
};
use wl_kernel::portal::{PortalFuture, PortalUser, PortalUsers, provide_portal_users};
use wl_kernel::scheduling::{
    BookingBooked, BookingCancelled, BookingCheckedIn, BookingCompleted, BookingNoShow,
    BookingReassigned, BookingRescheduled, Eligibility, EligibilityQuery, ResourceEligibility,
    SlotReleased, provide_resource_eligibility,
};
use wl_kernel::scheduling::{
    BookingId, SchedulingFuture, WorkDone, WorkDoneStatus, provide_work_done_status,
};
use wl_kernel::{PartyId, Subject};
use wl_scheduling::permissions as perm;
use wl_scheduling::service::{self, Actor, NewBooking};
use wl_scheduling::{
    AvailabilityRule, Booking, Resource, ResourceType, ServiceType, WlScheduling, resource,
};

/// The pages extend the app's layout; a package test has no app, so it
/// brings the smallest one.
const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

/// A client with the scheduling module (and the parties it needs) on.
const CLIENT: &str = r#"
[client]
name = "Palm Grove Clinic LLC"
app = "property"

[modules]
enabled = ["parties", "scheduling"]

[[legal_entities]]
code = "PGC"
name_en = "Palm Grove Clinic LLC"
default = true
"#;

/// An event log.
#[derive(Clone)]
pub struct Recorded<E>(pub Arc<Mutex<Vec<E>>>);

impl<E> Default for Recorded<E> {
    fn default() -> Self {
        Recorded(Arc::new(Mutex::new(Vec::new())))
    }
}

impl<E: Clone> Recorded<E> {
    /// What was recorded.
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

/// Every event the module emits.
#[derive(Clone, Default)]
pub struct Events {
    pub booked: Recorded<BookingBooked>,
    pub rescheduled: Recorded<BookingRescheduled>,
    pub checked_in: Recorded<BookingCheckedIn>,
    pub completed: Recorded<BookingCompleted>,
    pub no_show: Recorded<BookingNoShow>,
    pub cancelled: Recorded<BookingCancelled>,
    pub reassigned: Recorded<BookingReassigned>,
    pub released: Recorded<SlotReleased>,
}

/// Who acts for whom in the portal: the portal module's grants, as a list.
#[derive(Clone, Default)]
pub struct Portal(Arc<Mutex<Vec<(i64, i64)>>>);

impl Portal {
    /// `user` acts for `party` from now on.
    pub fn allow(&self, user: i64, party: PartyId) {
        self.0.lock().unwrap().push((user, party.0));
    }
}

impl PortalUsers for Portal {
    fn acting_for<'a>(
        &'a self,
        _db: &'a Db,
        user: i64,
        party: PartyId,
        _on: NaiveDate,
    ) -> PortalFuture<'a, Option<PortalUser>> {
        Box::pin(async move {
            let found = self.0.lock().unwrap().contains(&(user, party.0));
            Ok(found.then(|| PortalUser {
                user_id: user,
                person: party,
                acting_for: party,
                roles: vec!["primary_contact".to_owned()],
            }))
        })
    }
}

/// A refund asked of the fake payments module, with the number it returned.
type AskedRefund = (RefundRequest, Option<i64>);

/// `wl-payments` as the module sees it: it records the refunds asked of it,
/// numbers them, and refuses on demand (payments off, an unknown party).
#[derive(Clone, Default)]
pub struct Refunds {
    asked: Arc<Mutex<Vec<AskedRefund>>>,
    refuse: Arc<std::sync::atomic::AtomicBool>,
    crash: Arc<std::sync::atomic::AtomicBool>,
}

impl Refunds {
    /// The refunds asked so far, with the user who asked.
    pub fn asked(&self) -> Vec<(RefundRequest, Option<i64>)> {
        self.asked.lock().unwrap().clone()
    }

    /// The amounts asked so far, in fils.
    pub fn amounts(&self) -> Vec<i64> {
        self.asked()
            .iter()
            .map(|(request, _)| request.amount)
            .collect()
    }

    /// The next refund asked panics, as a process that dies between the claim
    /// of the deposit and the refund's id; the ones after it are accepted.
    pub fn crash_once(&self) {
        self.crash.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// From now on every refund is refused (or accepted again).
    pub fn refuse(&self, refuse: bool) {
        self.refuse
            .store(refuse, std::sync::atomic::Ordering::SeqCst);
    }
}

fn off() -> Error {
    Error::Status(
        StatusCode::UNPROCESSABLE_ENTITY,
        "payments refuses".to_owned(),
    )
}

impl PaymentRecorder for Refunds {
    fn record<'a>(
        &'a self,
        _state: &'a AppState,
        _payment: RecordPayment,
        _by: Option<i64>,
    ) -> PaymentFuture<'a, PaymentSummary> {
        Box::pin(async { Err(off()) })
    }

    fn allocate<'a>(
        &'a self,
        _state: &'a AppState,
        _payment: PaymentId,
        _target: AllocationTarget,
        _amount: i64,
    ) -> PaymentFuture<'a, ()> {
        Box::pin(async { Err(off()) })
    }

    fn reverse<'a>(
        &'a self,
        _state: &'a AppState,
        _payment: PaymentId,
        _reason: ReversalReason,
        _note: Option<String>,
        _by: Option<i64>,
    ) -> PaymentFuture<'a, ()> {
        Box::pin(async { Err(off()) })
    }

    fn find<'a>(
        &'a self,
        _db: &'a Db,
        _id: PaymentId,
    ) -> PaymentFuture<'a, Option<PaymentSummary>> {
        Box::pin(async { Ok(None) })
    }

    fn credit_balance<'a>(
        &'a self,
        _db: &'a Db,
        _party: PartyId,
        _currency: &'a str,
    ) -> PaymentFuture<'a, i64> {
        Box::pin(async { Ok(0) })
    }

    fn payments_for<'a>(
        &'a self,
        _db: &'a Db,
        _prepaid: &'a Subject,
    ) -> PaymentFuture<'a, Vec<PaymentSummary>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn request_refund<'a>(
        &'a self,
        _state: &'a AppState,
        request: RefundRequest,
        by: Option<i64>,
    ) -> PaymentFuture<'a, RefundId> {
        Box::pin(async move {
            if self.crash.swap(false, std::sync::atomic::Ordering::SeqCst) {
                panic!("the process dies while the refund is asked");
            }
            if self.refuse.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(off());
            }
            let mut asked = self.asked.lock().unwrap();
            asked.push((request, by));
            Ok(RefundId(900 + asked.len() as i64))
        })
    }
}

/// The vertical's record of the work done, as the module asks for it: the
/// answer is set per booking, `Nothing` until then.
#[derive(Clone, Default)]
pub struct Work(Arc<Mutex<Vec<(i64, WorkDone)>>>);

impl Work {
    /// From now on the work done for `booking` is `work`.
    pub fn set(&self, booking: i64, work: WorkDone) {
        let mut all = self.0.lock().unwrap();
        all.retain(|(id, _)| *id != booking);
        all.push((booking, work));
    }
}

impl WorkDoneStatus for Work {
    fn status<'a>(
        &'a self,
        _state: &'a AppState,
        booking: BookingId,
        _record: Option<Subject>,
    ) -> SchedulingFuture<'a, WorkDone> {
        Box::pin(async move {
            let all = self.0.lock().unwrap();
            Ok(all
                .iter()
                .find(|(id, _)| *id == booking.0)
                .map_or(WorkDone::Nothing, |(_, work)| *work))
        })
    }
}

/// `wl-notify` as the module sees it: the notifications asked of it.
#[derive(Clone, Default)]
pub struct Messages(Arc<Mutex<Vec<NotifyRequest>>>);

impl Messages {
    /// The notifications sent so far.
    pub fn sent(&self) -> Vec<NotifyRequest> {
        self.0.lock().unwrap().clone()
    }

    /// The English e-mail body of `request`, filled in as `wl-notify` does.
    pub fn body(request: &NotifyRequest) -> String {
        let def: Arc<NotificationTypeDef> =
            notify::notification_type(&request.type_key).expect("the type is registered");
        let template = def
            .template_for(NotifyChannel::Email, "en")
            .expect("an English e-mail text");
        notify::fill(&template.body, &request.data)
    }
}

impl Notifier for Messages {
    fn notify<'a>(
        &'a self,
        _state: &'a AppState,
        request: NotifyRequest,
    ) -> NotifyFuture<'a, Vec<MessageId>> {
        Box::pin(async move {
            let mut sent = self.0.lock().unwrap();
            sent.push(request);
            Ok(vec![MessageId(sent.len() as i64)])
        })
    }
}

/// The vertical's licence check: the resources it refuses.
#[derive(Clone, Default)]
pub struct Licences(Arc<Mutex<Vec<i64>>>);

impl Licences {
    /// From now on `resource` may not deliver the service (or may again).
    pub fn refuse(&self, resource: i64, refused: bool) {
        let mut all = self.0.lock().unwrap();
        all.retain(|id| *id != resource);
        if refused {
            all.push(resource);
        }
    }
}

impl ResourceEligibility for Licences {
    fn check<'a>(
        &'a self,
        _state: &'a AppState,
        query: EligibilityQuery,
    ) -> SchedulingFuture<'a, Eligibility> {
        Box::pin(async move {
            let refused = self.0.lock().unwrap().contains(&query.resource_id);
            Ok(if refused {
                Eligibility::Refused("the licence has expired".to_owned())
            } else {
                Eligibility::Eligible
            })
        })
    }
}

/// A client with the real parties and payments modules on too.
const CLIENT_REAL: &str = r#"
[client]
name = "Palm Grove Clinic LLC"
app = "property"

[modules]
enabled = ["parties", "invoicing", "payments", "scheduling"]

[[legal_entities]]
code = "PGC"
name_en = "Palm Grove Clinic LLC"
default = true
"#;

/// Provides the fakes.
struct Harness {
    /// The real `wl-payments` is mounted: its recorder stays the kernel's.
    real_payments: bool,
    portal: Portal,
    refunds: Refunds,
    work: Work,
    messages: Messages,
    licences: Licences,
}

impl Module for Harness {
    fn name(&self) -> &'static str {
        "scheduling-test-harness"
    }

    fn register(&self, app: &mut Registry) {
        provide_portal_users(app, self.portal.clone());
        if !self.real_payments {
            provide_payment_recorder(app, self.refunds.clone());
        }
        provide_work_done_status(app, self.work.clone());
        provide_notifier(app, self.messages.clone());
        provide_resource_eligibility(app, self.licences.clone());
    }
}

/// The app under test.
pub struct Booked {
    /// The app.
    pub app: TestApp,
    /// The events the module emitted.
    pub events: Events,
    /// Who acts for whom in the portal.
    pub portal: Portal,
    /// The refunds the module asked of `wl-payments`.
    pub refunds: Refunds,
    /// The work done the vertical reports.
    pub work: Work,
    /// The notifications the module sent.
    pub messages: Messages,
    /// The resources the vertical's licence check refuses.
    pub licences: Licences,
    /// Keeps the views directory alive.
    _views: tempfile::TempDir,
    /// Keeps the database file alive (when the test uses one).
    _database: tempfile::TempDir,
}

/// The party that books in the tests.
pub const PATIENT: PartyId = PartyId(70);
/// Another party.
pub const OTHER_PATIENT: PartyId = PartyId(71);

impl Booked {
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

    /// The front desk: sees, manages and checks in.
    pub async fn front_desk(&self) -> User {
        self.user(
            "Fatima",
            &[
                perm::VIEW,
                perm::MANAGE,
                perm::CHECKIN,
                perm::RESOURCES_MANAGE,
            ],
        )
        .await
    }

    /// A user who may only look.
    pub async fn viewer(&self) -> User {
        self.user("Vera", &[perm::VIEW]).await
    }

    /// A resource of `kind`, saved.
    pub async fn resource(&self, name: &str, kind: ResourceType) -> Resource {
        let mut resource = Resource::new(name, kind);
        resource.save(self.db()).await.unwrap();
        resource
    }

    /// Dr A: a person, open every day of the week all day.
    pub async fn doctor(&self, name: &str) -> Resource {
        let doctor = self.resource(name, ResourceType::Person).await;
        self.open_all_week(&doctor).await;
        doctor
    }

    /// Opens `resource` every day of the week, round the clock.
    pub async fn open_all_week(&self, resource: &Resource) {
        for weekday in 1..=7 {
            let mut rule = AvailabilityRule::new(resource.id, weekday, 0, 1440);
            rule.save(self.db()).await.unwrap();
        }
    }

    /// A 30-minute service that `resources` deliver, after `change`.
    pub async fn service_with(
        &self,
        code: &str,
        resources: &[&Resource],
        change: impl FnOnce(&mut ServiceType),
    ) -> ServiceType {
        // A test that builds its scene twice gets the service of that code
        // again (the code is unique) with the new resources offering it.
        let existing = ServiceType::query()
            .where_(ServiceType::CODE.eq(code))
            .first(self.db())
            .await
            .unwrap();
        let service = match existing {
            Some(service) => service,
            None => {
                let mut service = ServiceType::new(code, "Consultation", 30);
                change(&mut service);
                service.save(self.db()).await.unwrap();
                service
            }
        };
        for resource in resources {
            resource::offer(self.db(), service.id, resource.id)
                .await
                .unwrap();
        }
        service
    }

    /// A 30-minute service that `doctor` delivers with a deposit of AED 200
    /// that is forfeited when a party cancels less than 24 hours before; the
    /// party may cancel until 2 hours before. After check-in the rest is
    /// forfeited unless `walk_out_forfeits` is false.
    pub async fn deposit_service(&self, doctor: &Resource, walk_out_forfeits: bool) -> ServiceType {
        self.service_with("consultation", &[doctor], |service| {
            service.deposit_amount = 20_000;
            service.deposit_forfeit_hours = 24;
            service.cancel_cutoff_hours = 2;
            service.walk_out_forfeits = walk_out_forfeits;
        })
        .await
    }

    /// A 30-minute service with a deposit of AED 200 that checks the party
    /// in and tracks the service while it runs.
    pub async fn clinic_service(&self, doctor: &Resource, walk_out_forfeits: bool) -> ServiceType {
        self.service_with("consultation", &[doctor], |service| {
            service.deposit_amount = 20_000;
            service.deposit_forfeit_hours = 24;
            service.cancel_cutoff_hours = 2;
            service.walk_out_forfeits = walk_out_forfeits;
            service.check_in_required = true;
            service.track_in_progress = true;
        })
        .await
    }

    /// A user who may waive a forfeited deposit.
    pub async fn waiver(&self) -> User {
        self.user("Walid", &[perm::VIEW, perm::WAIVE_DEPOSIT]).await
    }

    /// A plain 30-minute service that `doctor` delivers.
    pub async fn service(&self, doctor: &Resource) -> ServiceType {
        self.service_with("consultation", &[doctor], |_| {}).await
    }

    /// A booking of `service` with `resources` from `start`, made by staff.
    pub async fn booking(
        &self,
        service: &ServiceType,
        resources: &[&Resource],
        start: DateTime,
    ) -> Booking {
        self.book_for(PATIENT, service, resources, start).await
    }

    /// Like [`booking`](Booked::booking) for `party`.
    pub async fn book_for(
        &self,
        party: PartyId,
        service: &ServiceType,
        resources: &[&Resource],
        start: DateTime,
    ) -> Booking {
        let ids: Vec<i64> = resources.iter().map(|r| r.id).collect();
        service::book(
            self.state(),
            NewBooking::new(party, service.id, &ids, start),
            Actor::staff(1),
            now(),
        )
        .await
        .expect("the booking is made")
    }

    /// The path and query of a signed URL (the origin cut off), as the test
    /// client posts it.
    pub fn path_of(&self, url: &str) -> String {
        url.trim_start_matches(self.state().config.url.trim_end_matches('/'))
            .to_owned()
    }

    /// The booking, read again.
    pub async fn reload(&self, id: i64) -> Booking {
        Booking::find_or_404(self.db(), id).await.unwrap()
    }

    /// The history rows of the booking: `(transition, to)`.
    pub async fn history(&self, id: i64) -> Vec<(String, String)> {
        renox::state_machine::history(self.db(), Booking::TABLE, id)
            .await
            .unwrap()
            .into_iter()
            .map(|row| (row.transition, row.to_state))
            .collect()
    }
}

/// The real clock, to the minute.
pub fn now() -> DateTime {
    let now = renox::db::now();
    now.with_second(0)
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(now)
}

/// `hours` from the real clock, to the minute.
pub fn in_hours(hours: i64) -> DateTime {
    now() + TimeDelta::hours(hours)
}

/// `minutes` from the real clock, to the minute.
pub fn in_minutes(minutes: i64) -> DateTime {
    now() + TimeDelta::minutes(minutes)
}

/// Asia/Dubai `date` (`2026-11-03`) at `hour:minute`, as an instant.
pub fn dubai(date: &str, hour: u32, minute: u32) -> DateTime {
    let local = date
        .parse::<NaiveDate>()
        .expect("a date")
        .and_hms_opt(hour, minute, 0)
        .expect("a time");
    BusinessCalendar::default().from_local(local)
}

/// `WlScheduling` booted with auth and the events recorded.
pub async fn boot() -> Booked {
    boot_with(WlScheduling::new()).await
}

/// [`boot`] with `module`.
pub async fn boot_with(module: WlScheduling) -> Booked {
    boot_app(module, false).await
}

/// [`boot`] with the real `wl-parties` and `wl-payments` mounted (the
/// refunds are asked of the real module; `Booked::refunds` stays empty) and
/// the demo parties seeded.
pub async fn boot_real() -> Booked {
    boot_app(WlScheduling::new(), true).await
}

/// [`boot`] on a SQLite file in a directory of its own, with ten
/// connections, so that concurrent callers really use separate connections.
pub async fn boot_file_backed() -> Booked {
    boot_app_on(WlScheduling::new(), false, true).await
}

async fn boot_app(module: WlScheduling, real: bool) -> Booked {
    boot_app_on(module, real, false).await
}

async fn boot_app_on(module: WlScheduling, real: bool, file_backed: bool) -> Booked {
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();
    let events = Events::default();
    let portal = Portal::default();
    let refunds = Refunds::default();
    let work = Work::default();
    let messages = Messages::default();
    let licences = Licences::default();
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(renox::audit::Audit);
    let app = if real {
        app.module(wl_parties::WlParties::new())
            .module(wl_payments::WlPayments::new())
    } else {
        app
    };
    let app = app.module(module).module(Harness {
        real_payments: real,
        portal: portal.clone(),
        refunds: refunds.clone(),
        work: work.clone(),
        messages: messages.clone(),
        licences: licences.clone(),
    });
    let app = record(app, &events.booked);
    let app = record(app, &events.rescheduled);
    let app = record(app, &events.checked_in);
    let app = record(app, &events.completed);
    let app = record(app, &events.no_show);
    let app = record(app, &events.cancelled);
    let app = record(app, &events.reassigned);
    let app = record(app, &events.released);
    let client = if real { CLIENT_REAL } else { CLIENT };
    let config = wl_kernel::ClientConfig::parse(client).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    let dir = views.path().to_path_buf();
    let database = tempfile::tempdir().expect("a temporary database directory");
    let url = format!("sqlite://{}", database.path().join("t.db").display());
    let app = TestApp::with_config(app, |c| {
        c.views_path = dir;
        if file_backed {
            c.database_url = url;
            c.database_pool_size = 10;
        }
    })
    .await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    if file_backed {
        assert!(
            database.path().join("t.db").exists(),
            "the test runs on its SQLite file"
        );
    }
    if real {
        wl_parties::seed::run(app.state())
            .await
            .expect("the demo parties seed");
    }
    Booked {
        app,
        events,
        portal,
        refunds,
        work,
        messages,
        licences,
        _views: views,
        _database: database,
    }
}
