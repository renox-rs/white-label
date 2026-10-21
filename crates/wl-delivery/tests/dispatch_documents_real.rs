//! BL-DLV-01 (#157), `WRK.DSP.01`, `X-13`, `DOC.ACL.02`: `assign` and run
//! `dispatch` against the real `wl-documents` guard and the real `wl-hr`
//! workforce, no fake. The 03 kinds (driving licence, rider permit, Mulkiya,
//! vehicle insurance) are seeded with their `blocks`; the licence and the
//! permit are attached to the driver's employee, the Mulkiya and the
//! insurance to the vehicle (an `inventory.asset`). The refusals are the
//! real ones: the kind's name, the expiry date, who renews it and, for a kind
//! that allows it, the link that requests an override.

use renox::auth::{Permissions, permissions};
use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::{TestApp, TestResponse};
use wl_delivery::{
    Deliveries, Delivery, DeliveryStatus, NewRun, Run, RunStatus, VehicleType, WlDelivery,
    permissions as perm,
};
use wl_documents::{Document, Documents, KindSeed, NewDocument, WlDocuments};
use wl_hr::{Employee, EmployeeStatus, WlHr};
use wl_kernel::documents::{self, BlockMode, OverrideMode, today};
use wl_kernel::workforce::EmployeeId;
use wl_kernel::{ClientConfig, DeliveryRequest, Subject};

const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

const CLIENT: &str = r#"
[client]
name = "Gulf Express Logistics LLC"
app = "logistics"

[modules]
enabled = ["parties", "documents", "hr", "delivery"]

[[legal_entities]]
code = "GEL"
name_en = "Gulf Express Logistics LLC"
trn = "100000000000003"
default = true
"#;

/// The vehicle (an asset) the runs are driven with.
const VAN: i64 = 7;

const ASSIGN: &str = "delivery.assign";
const DISPATCH: &str = "run.dispatch";

struct Real {
    app: TestApp,
    _views: tempfile::TempDir,
    dispatcher: User,
    driver: User,
    reviewer: User,
    employee: Employee,
}

/// `days` from today (Asia/Dubai); before today when negative.
fn day(days: i64) -> NaiveDate {
    if days >= 0 {
        today() + Days::new(days.unsigned_abs())
    } else {
        today() - Days::new(days.unsigned_abs())
    }
}

fn text(day: NaiveDate) -> String {
    day.format("%Y-%m-%d").to_string()
}

fn pdf() -> Upload {
    Upload::new("scan.pdf", "application/pdf", b"%PDF-1.7 scan".to_vec())
}

/// A 03 kind: a legal block of the two guarded transitions, never overridden,
/// or, with `assign` off, a block of the dispatch only.
fn legal_kind(key: &str, name: &str, subject_type: &str, role: &str, assign: bool) -> KindSeed {
    let kind = KindSeed::new(key, name)
        .subject_type(subject_type)
        .expires()
        .responsible_role(role)
        .legal_block()
        .blocks(DISPATCH, BlockMode::Block);
    if assign {
        kind.blocks(ASSIGN, BlockMode::Block)
    } else {
        kind
    }
}

/// The four 03 kinds.
fn logistics_kinds(assign: bool) -> Vec<KindSeed> {
    vec![
        legal_kind(
            "driving_licence",
            "Driving licence",
            "hr.employee",
            "HR",
            assign,
        ),
        legal_kind("rider_permit", "Rider permit", "hr.employee", "HR", assign),
        legal_kind("mulkiya", "Mulkiya", "inventory.asset", "Fleet", assign),
        legal_kind(
            "vehicle_insurance",
            "Vehicle insurance",
            "inventory.asset",
            "Fleet",
            assign,
        ),
    ]
}

/// A kind that allows an override with an approval.
fn induction_kind() -> KindSeed {
    KindSeed::new("site_induction", "Site induction")
        .subject_type("hr.employee")
        .expires()
        .responsible_role("HR")
        .override_mode(OverrideMode::WithApproval)
        .blocks(ASSIGN, BlockMode::Block)
        .blocks(DISPATCH, BlockMode::Block)
}

async fn user(app: &TestApp, name: &str, grants: &[&str]) -> User {
    let email = format!("{}@example.com", name.to_lowercase());
    let user = User::register(app.db(), name, &email, "password123")
        .await
        .unwrap();
    let role = format!("role_{}", name.to_lowercase());
    permissions::define_role(app.db(), &role, grants)
        .await
        .unwrap();
    user.assign_role(app.db(), &role).await.unwrap();
    user
}

/// The real documents and HR modules with wl-delivery; the driver is an
/// employee of wl-hr.
async fn boot(kinds: &[KindSeed]) -> Real {
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();
    let app = App::new()
        .module(Auth::new())
        .module(renox::audit::Audit)
        .module(Permissions)
        .module(WlDocuments)
        .module(WlHr::new())
        .module(WlDelivery::new());
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    wl_delivery::seed::install(app.db())
        .await
        .expect("the delivery defaults seed");
    wl_documents::kinds::install_seeds(app.db(), kinds)
        .await
        .expect("the kinds install");

    let dispatcher = user(
        &app,
        "Dina",
        &[
            perm::VIEW,
            perm::DISPATCH,
            perm::RUNS_VIEW,
            perm::RUNS_DISPATCH,
        ],
    )
    .await;
    let driver = user(&app, "Dev", &[perm::RUNS_DRIVE, perm::COMPLETE]).await;
    let reviewer = user(
        &app,
        "Rita",
        &[
            wl_documents::permissions::VIEW,
            wl_documents::permissions::REVIEW,
        ],
    )
    .await;
    let mut employee = Employee {
        party_id: 61,
        user_id: Some(driver.id),
        job_role: "driver".into(),
        job_title: "Driver".into(),
        hire_date: day(-400),
        status: EmployeeStatus::Active,
        ..Employee::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    employee
        .insert_in_state(&mut tx, EmployeeStatus::Active, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    Real {
        app,
        _views: views,
        dispatcher,
        driver,
        reviewer,
        employee,
    }
}

impl Real {
    fn employee_subject(&self) -> Subject {
        self.employee.subject()
    }

    fn vehicle(&self) -> Subject {
        Subject::new("inventory.asset", VAN)
    }

    /// Attaches a document of `kind` to `subject` expiring on `expiry` and
    /// has the reviewer accept it, as the review queue does.
    async fn accepted(&self, subject: Subject, kind: &str, expiry: NaiveDate) -> Document {
        let new = NewDocument::new(subject, kind, pdf()).expires_on(expiry);
        let document = Documents::attach(self.app.state(), None, new)
            .await
            .unwrap()
            .expect("the document is attached");
        self.app.acting_as(&self.reviewer);
        self.app
            .htmx()
            .post(&format!("/documents/{}/accept", document.id), &[])
            .await
            .assert_ok();
        Document::find(self.app.db(), document.id)
            .await
            .unwrap()
            .unwrap()
    }

    /// Every document of the driver and the van valid through `until`.
    async fn all_valid(&self, until: NaiveDate) {
        for kind in ["driving_licence", "rider_permit"] {
            self.accepted(self.employee_subject(), kind, until).await;
        }
        for kind in ["mulkiya", "vehicle_insurance"] {
            self.accepted(self.vehicle(), kind, until).await;
        }
    }

    async fn run_on(&self, date: NaiveDate, on_foot: bool) -> Run {
        let new = NewRun::new(date, self.driver.id);
        let new = if on_foot {
            new.vehicle(VehicleType::OnFoot, None)
        } else {
            new.vehicle(VehicleType::Van, Some(VAN))
        };
        wl_delivery::plan::create_run(self.app.state(), new)
            .await
            .expect("the run is planned")
    }

    async fn delivery(&self) -> Delivery {
        let request = DeliveryRequest::new(Subject::new("sales.order", 1), "Mariam Al Hosani")
            .address("Villa 12, Al Barsha 2", "Al Barsha")
            .phone("+971500000000")
            .item(wl_kernel::DeliveryItem::new("Tiles", 10_000));
        Deliveries::request(self.app.state(), request)
            .await
            .expect("the delivery is requested")
    }

    async fn assign(&self, delivery: &Delivery, run: &Run) -> TestResponse {
        self.app.acting_as(&self.dispatcher);
        let run_id = run.id.to_string();
        self.app
            .htmx()
            .post(
                &format!("/deliveries/{}/assign", delivery.id),
                &[("run_id", run_id.as_str())],
            )
            .await
    }

    async fn dispatch(&self, run: &Run) -> TestResponse {
        self.app.acting_as(&self.dispatcher);
        self.app
            .htmx()
            .post(&format!("/runs/{}/dispatch", run.id), &[])
            .await
    }

    async fn status_of(&self, delivery: &Delivery) -> DeliveryStatus {
        Delivery::find_or_404(self.app.db(), delivery.id)
            .await
            .unwrap()
            .status
    }

    async fn run_status(&self, run: &Run) -> RunStatus {
        Run::find_or_404(self.app.db(), run.id)
            .await
            .unwrap()
            .status
    }
}

/// AC1: the driver's licence expires in 10 days and the run is planned 12
/// days out: `assign` is refused naming the licence, its expiry date and who
/// renews it (the real wl-documents text), and the delivery stays pending.
#[renox::test]
async fn assign_is_refused_with_the_real_text_when_the_licence_expires_before_the_run() {
    let t = boot(&logistics_kinds(true)).await;
    t.all_valid(day(400)).await;
    t.accepted(t.employee_subject(), "driving_licence", day(10))
        .await;
    let run = t.run_on(day(12), false).await;
    let delivery = t.delivery().await;

    t.assign(&delivery, &run)
        .await
        .assert_status(422)
        .assert_see(&format!("Driving licence expired on {}", text(day(10))))
        .assert_see("renew: HR");

    assert_eq!(t.status_of(&delivery).await, DeliveryStatus::Pending);
}

/// AC2: the same driver and a run 9 days out: `assign` succeeds, the licence
/// being valid through its expiry date.
#[renox::test]
async fn assign_succeeds_with_the_real_guard_when_the_licence_is_valid_on_the_run_date() {
    let t = boot(&logistics_kinds(true)).await;
    t.all_valid(day(400)).await;
    t.accepted(t.employee_subject(), "driving_licence", day(10))
        .await;
    let run = t.run_on(day(9), false).await;
    let delivery = t.delivery().await;

    t.assign(&delivery, &run).await.assert_ok();
    assert_eq!(t.status_of(&delivery).await, DeliveryStatus::Assigned);

    // The expiry date itself still counts.
    let on_the_day = t.run_on(day(10), false).await;
    let another = t.delivery().await;
    t.assign(&another, &on_the_day).await.assert_ok();
}

/// AC3: a planned run whose insurance expired yesterday: `dispatch` is
/// refused, the run stays planned and the delivery does not leave.
#[renox::test]
async fn dispatch_is_refused_with_the_real_text_when_the_insurance_expired_yesterday() {
    let t = boot(&logistics_kinds(true)).await;
    t.all_valid(day(400)).await;
    let insurance = t.accepted(t.vehicle(), "vehicle_insurance", day(5)).await;
    let run = t.run_on(today(), false).await;
    let delivery = t.delivery().await;
    t.assign(&delivery, &run).await.assert_ok();

    // The insurance runs out between `assign` and `dispatch`.
    Document::query()
        .where_(Document::ID.eq(insurance.id))
        .update(t.app.db(), &[("expiry_date", &Some(day(-1)))])
        .await
        .unwrap();

    t.dispatch(&run)
        .await
        .assert_status(422)
        .assert_see(&format!("Vehicle insurance expired on {}", text(day(-1))))
        .assert_see("renew: Fleet");

    assert_eq!(t.run_status(&run).await, RunStatus::Planned);
    assert_eq!(t.status_of(&delivery).await, DeliveryStatus::Assigned);
}

/// AC4: an on-foot rider with a valid permit and no vehicle: `dispatch`
/// succeeds; no document of any vehicle exists, so asking for one would fail.
#[renox::test]
async fn an_on_foot_rider_is_dispatched_by_the_real_guard_without_a_vehicle() {
    let t = boot(&logistics_kinds(true)).await;
    for kind in ["driving_licence", "rider_permit"] {
        t.accepted(t.employee_subject(), kind, day(400)).await;
    }
    let run = t.run_on(today(), true).await;
    let delivery = t.delivery().await;
    t.assign(&delivery, &run).await.assert_ok();

    t.dispatch(&run).await.assert_ok();

    assert_eq!(t.run_status(&run).await, RunStatus::Dispatched);
    assert_eq!(t.status_of(&delivery).await, DeliveryStatus::OutForDelivery);
}

/// AC5: a 03 client (legal blocks, override mode `never`): the refusal offers
/// no override request. A kind that allows an override with approval offers
/// the link of wl-documents for exactly that kind, the guarded transition and
/// the run.
#[renox::test]
async fn a_legal_block_offers_no_override_link_and_an_overridable_kind_does() {
    let t = boot(&logistics_kinds(true)).await;
    t.all_valid(day(400)).await;
    t.accepted(t.vehicle(), "mulkiya", day(3)).await;
    let run = t.run_on(day(5), false).await;
    let delivery = t.delivery().await;

    let body = t.assign(&delivery, &run).await.assert_status(422).text();
    assert!(body.contains("Mulkiya expired on"), "{body}");
    assert!(body.contains("override: not allowed"), "{body}");
    assert!(!body.contains("overrides/new"), "{body}");

    let mut kinds = logistics_kinds(true);
    kinds.push(induction_kind());
    let t = boot(&kinds).await;
    t.all_valid(day(400)).await;
    t.accepted(t.employee_subject(), "site_induction", day(3))
        .await;
    let run = t.run_on(day(5), false).await;
    let delivery = t.delivery().await;

    t.assign(&delivery, &run)
        .await
        .assert_status(422)
        .assert_see("Site induction expired on")
        .assert_see("/documents/overrides/new?kind=site_induction");

    let refusal = documents::guard(t.app.state())
        .check_as(t.app.state(), ASSIGN, run.id, day(5), None)
        .await
        .unwrap()
        .expect_err("the assignment is refused");
    assert_eq!(
        refusal.override_links(),
        vec![format!(
            "/documents/overrides/new?kind=site_induction&guarded=delivery.assign&record={}",
            run.id
        )]
    );
}

/// AC6: no kind lists `delivery.assign`: `assign` succeeds with no document
/// at all; the kinds that list `run.dispatch` still stop the dispatch.
#[renox::test]
async fn assign_succeeds_when_no_kind_lists_the_transition() {
    let t = boot(&logistics_kinds(false)).await;
    let run = t.run_on(day(5), false).await;
    let delivery = t.delivery().await;

    t.assign(&delivery, &run).await.assert_ok();
    assert_eq!(t.status_of(&delivery).await, DeliveryStatus::Assigned);

    t.dispatch(&run).await.assert_status(422);
    assert_eq!(t.run_status(&run).await, RunStatus::Planned);
}

/// Edge: each kind is checked on the subject it attaches to. With nothing
/// attached, the real guard lists the licence and the permit for the driver's
/// employee and the Mulkiya and the insurance for the vehicle, one failure
/// each: a licence is never asked of the vehicle, nor an insurance of the
/// driver.
#[renox::test]
async fn each_kind_is_checked_on_the_subject_it_attaches_to() {
    let t = boot(&logistics_kinds(true)).await;
    let run = t.run_on(day(5), false).await;

    let refusal = documents::guard(t.app.state())
        .check_as(t.app.state(), ASSIGN, run.id, day(5), None)
        .await
        .unwrap()
        .expect_err("nothing is attached");

    assert_eq!(refusal.failures.len(), 4, "{}", refusal.message());
    for failure in &refusal.failures {
        let expected = match failure.entry.kind.as_str() {
            "driving_licence" | "rider_permit" => t.employee_subject(),
            "mulkiya" | "vehicle_insurance" => t.vehicle(),
            other => panic!("unexpected kind {other}"),
        };
        assert_eq!(failure.subject, expected, "{}", failure.entry.kind);
    }
}

/// A2: wl-hr's `qualification_valid` gives the same answer as the guard the
/// delivery module asks, for a kind that is not bound to a role: valid
/// through the expiry date, not the day after.
#[renox::test]
async fn the_hr_qualification_answers_as_the_guard_does_for_the_licence() {
    let t = boot(&logistics_kinds(true)).await;
    t.all_valid(day(400)).await;
    t.accepted(t.employee_subject(), "driving_licence", day(10))
        .await;
    let id: EmployeeId = t.employee.employee_id();

    for (days, valid) in [(10, true), (11, false)] {
        let hr = wl_hr::qualification::qualification_valid(
            t.app.state(),
            id,
            "driving_licence",
            day(days),
        )
        .await
        .unwrap();
        let run = t.run_on(day(days), false).await;
        let guard = documents::guard(t.app.state())
            .check_as(t.app.state(), ASSIGN, run.id, day(days), None)
            .await
            .unwrap()
            .is_ok();
        assert_eq!(hr, valid, "wl-hr on day {days}");
        assert_eq!(guard, valid, "the guard on day {days}");
    }
}
