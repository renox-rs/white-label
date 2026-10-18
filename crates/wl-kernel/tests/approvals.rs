//! Acceptance checks of #25 (BL-KRN-01): an approval request that ends
//! without a yes sends its subject back to the state it had before.
//!
//! The kernel cannot depend on business modules, so the subjects are
//! test-only models with their own migration: a refund **with** a waiting
//! state (`requested → pending_approval`), a booking **without** one (its
//! `confirm` is deferred by the kernel's interceptor) that reserves a slot,
//! and a material request **with** one that holds bags of cement.

use std::time::Duration;

use renox::auth::{Permissions, permissions};
use renox::db::{Json, Migration, Transaction, sql};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx, history};
use renox::testing::{TestApp, TestResponse};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{self, End, Require, Step};
use wl_kernel::machines::{BoxFuture, Hold};
use wl_kernel::{
    ApprovalDecided, ApprovalEnd, ApprovalPolicy, ApprovalRequest, ApprovalRequested,
    ApprovalStatus, ClientConfig, PolicyDefault, Subject, SubjectMachine,
};

const REFUND: &str = "payments.refund";
const BOOKING: &str = "bookings.booking";
const MATERIAL: &str = "materials.material_request";
/// The refund's action key (and its transition `refund`).
const REFUND_ACTION: &str = "payments.refund";
/// The material request's action key (and its transition `order`).
const ORDER_ACTION: &str = "materials.order";
/// The guard message of `refund_approved`.
const OVER_BALANCE: &str = "The amount exceeds the remaining balance.";
/// What the subjects' approval notices are.
const REQUESTED: &str = "approval-requested";
const DECIDED: &str = "approval-decided";

// ---------------------------------------------------------------------------
// The test-only subjects.
// ---------------------------------------------------------------------------

#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
enum RefundStatus {
    #[default]
    Requested,
    PendingApproval,
    Refunded,
    Cancelled,
}

/// A refund of `amount` fils from a deposit with `balance` fils left.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_refunds", state_machine)]
struct Refund {
    id: i64,
    status: RefundStatus,
    amount: i64,
    balance: i64,
}

impl StateMachine for Refund {
    type State = RefundStatus;
    const COLUMN: renox::db::Col<Refund, RefundStatus> = Refund::STATUS;

    fn machine() -> Machine<Self> {
        use RefundStatus::{Cancelled, PendingApproval, Refunded, Requested};
        Machine::new()
            .initial([Requested])
            .state(Requested)
            .state(PendingApproval)
            .tone("warning")
            .state(Refunded)
            .terminal()
            .state(Cancelled)
            .terminal()
            // Into the waiting state: the effect asks for approval.
            .transition(
                Transition::new("refund")
                    .from([Requested])
                    .to(PendingApproval)
                    .effect(submit_refund),
            )
            .transition(
                Transition::new("refund_approved")
                    .from([PendingApproval])
                    .to(Refunded)
                    .system()
                    .guard("within_balance", within_balance)
                    .effect(|refund: &Refund, _ctx, tx| effect(tx, REFUND, refund.id, "paid_out")),
            )
            .transition(
                Transition::new("back_to_requested")
                    .from([PendingApproval])
                    .to(Requested)
                    .system(),
            )
            .transition(
                Transition::new("cancel")
                    .from([Requested, PendingApproval])
                    .to(Cancelled),
            )
    }
}

fn submit_refund<'a>(
    refund: &'a Refund,
    ctx: &'a TransitionCtx,
    tx: &'a mut Transaction,
) -> BoxFuture<'a, Result> {
    Box::pin(async move {
        let requester = ctx.actor().map(|user| user.id).ok_or(Error::Unauthorized)?;
        let require = Require::new(
            REFUND_ACTION,
            Subject::new(REFUND, refund.id),
            "refund",
            requester,
            refund.status.to_string(),
        )
        .payload(json!({ "amount": refund.amount }));
        approvals::require_in(tx, &require).await?;
        Ok(())
    })
}

/// The payload snapshot's amount must still fit the balance left.
fn within_balance<'a>(
    refund: &'a Refund,
    ctx: &'a TransitionCtx,
) -> BoxFuture<'a, std::result::Result<(), String>> {
    Box::pin(async move {
        let amount = ctx.input()["amount"].as_i64().unwrap_or(i64::MAX);
        if amount <= refund.balance {
            Ok(())
        } else {
            Err(OVER_BALANCE.to_owned())
        }
    })
}

#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
enum BookingStatus {
    #[default]
    Requested,
    Confirmed,
}

/// A booking of a slot; it has no waiting state.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_bookings", state_machine)]
struct Booking {
    id: i64,
    status: BookingStatus,
    slot: String,
}

impl StateMachine for Booking {
    type State = BookingStatus;
    const COLUMN: renox::db::Col<Booking, BookingStatus> = Booking::STATUS;

    fn machine() -> Machine<Self> {
        use BookingStatus::{Confirmed, Requested};
        Machine::new()
            .initial([Requested])
            .state(Requested)
            .state(Confirmed)
            .terminal()
            .transition(
                Transition::new("confirm")
                    .from([Requested])
                    .to(Confirmed)
                    .effect(|booking: &Booking, _ctx, tx| {
                        effect(tx, BOOKING, booking.id, "confirmed")
                    }),
            )
            .transition(
                Transition::new("confirm_approved")
                    .from([Requested])
                    .to(Confirmed)
                    .system()
                    .effect(|booking: &Booking, _ctx, tx| {
                        effect(tx, BOOKING, booking.id, "confirmed")
                    }),
            )
    }
}

#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
enum MaterialStatus {
    #[default]
    Draft,
    PendingApproval,
    Ordered,
}

/// A request for `bags` bags of cement.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_material_requests", state_machine)]
struct MaterialRequest {
    id: i64,
    status: MaterialStatus,
    bags: i64,
}

impl StateMachine for MaterialRequest {
    type State = MaterialStatus;
    const COLUMN: renox::db::Col<MaterialRequest, MaterialStatus> = MaterialRequest::STATUS;

    fn machine() -> Machine<Self> {
        use MaterialStatus::{Draft, Ordered, PendingApproval};
        Machine::new()
            .initial([Draft])
            .state(Draft)
            .state(PendingApproval)
            .state(Ordered)
            .terminal()
            .transition(
                Transition::new("submit")
                    .from([Draft])
                    .to(PendingApproval)
                    .effect(submit_material),
            )
            .transition(
                Transition::new("order_approved")
                    .from([PendingApproval])
                    .to(Ordered)
                    .system(),
            )
            .transition(
                Transition::new("back_to_draft")
                    .from([PendingApproval])
                    .to(Draft)
                    .system(),
            )
    }
}

fn submit_material<'a>(
    request: &'a MaterialRequest,
    ctx: &'a TransitionCtx,
    tx: &'a mut Transaction,
) -> BoxFuture<'a, Result> {
    Box::pin(async move {
        let requester = ctx.actor().map(|user| user.id).ok_or(Error::Unauthorized)?;
        let require = Require::new(
            ORDER_ACTION,
            Subject::new(MATERIAL, request.id),
            "order",
            requester,
            request.status.to_string(),
        )
        .payload(json!({ "bags": request.bags }));
        approvals::require_in(tx, &require).await?;
        Ok(())
    })
}

#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
enum CarelessStatus {
    #[default]
    Draft,
    Waiting,
}

/// A machine whose `back_to_draft` a user could run: refused at boot.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_careless", state_machine)]
struct Careless {
    id: i64,
    status: CarelessStatus,
}

impl StateMachine for Careless {
    type State = CarelessStatus;
    const COLUMN: renox::db::Col<Careless, CarelessStatus> = Careless::STATUS;

    fn machine() -> Machine<Self> {
        use CarelessStatus::{Draft, Waiting};
        Machine::new()
            .state(Draft)
            .state(Waiting)
            .transition(Transition::new("submit").from([Draft]).to(Waiting))
            .transition(Transition::new("back_to_draft").from([Waiting]).to(Draft))
    }
}

/// Records that an action's effect ran.
fn effect<'a>(
    tx: &'a mut Transaction,
    subject_type: &'static str,
    id: i64,
    what: &'static str,
) -> BoxFuture<'a, Result> {
    Box::pin(async move {
        sql("INSERT INTO test_effects (subject, effect) VALUES (?, ?)")
            .bind(Subject::new(subject_type, id).to_string())
            .bind(what)
            .execute(&mut *tx)
            .await?;
        Ok(())
    })
}

/// A reservation made at submit, keyed by the request.
async fn hold(tx: &mut Transaction, hold: &Hold, what: &str, qty: i64) -> Result {
    sql("INSERT INTO test_holds (request_id, subject, what, qty) VALUES (?, ?, ?, ?)")
        .bind(hold.request_id)
        .bind(hold.subject.to_string())
        .bind(what)
        .bind(qty)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

fn reserve_refund<'a>(tx: &'a mut Transaction, held: &'a Hold) -> BoxFuture<'a, Result> {
    Box::pin(async move {
        let amount = held.payload["amount"].as_i64().unwrap_or_default();
        hold(tx, held, "refund", amount).await
    })
}

fn reserve_slot<'a>(tx: &'a mut Transaction, held: &'a Hold) -> BoxFuture<'a, Result> {
    Box::pin(async move {
        let slot = sql("SELECT slot FROM test_bookings WHERE id = ?")
            .bind(held.subject.subject_id)
            .scalar::<String>(&mut *tx)
            .await?;
        hold(tx, held, &format!("slot {slot}"), 1).await
    })
}

fn hold_cement<'a>(tx: &'a mut Transaction, held: &'a Hold) -> BoxFuture<'a, Result> {
    Box::pin(async move {
        let bags = held.payload["bags"].as_i64().unwrap_or_default();
        hold(tx, held, "cement", bags).await
    })
}

/// Every subject's release: counts its calls, fails while the flag
/// `fail_release` is set, and frees the request's holds (idempotent).
fn release<'a>(tx: &'a mut Transaction, held: &'a Hold) -> BoxFuture<'a, Result> {
    Box::pin(async move {
        sql("INSERT INTO test_releases (request_id) VALUES (?)")
            .bind(held.request_id)
            .execute(&mut *tx)
            .await?;
        let failing = sql("SELECT COUNT(*) FROM test_flags WHERE name = 'fail_release'")
            .scalar::<i64>(&mut *tx)
            .await?;
        if failing > 0 {
            return Err(renox::anyhow::anyhow!("the store is offline").into());
        }
        sql("UPDATE test_holds SET released = 1 WHERE request_id = ? AND released = 0")
            .bind(held.request_id)
            .execute(&mut *tx)
            .await?;
        Ok(())
    })
}

const UP: &str = r#"
CREATE TABLE "test_refunds" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    status TEXT NOT NULL DEFAULT 'requested',
    amount INTEGER NOT NULL DEFAULT 0,
    balance INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE "test_bookings" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    status TEXT NOT NULL DEFAULT 'requested',
    slot TEXT NOT NULL DEFAULT ''
);
CREATE TABLE "test_material_requests" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    status TEXT NOT NULL DEFAULT 'draft',
    bags INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE "test_holds" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id INTEGER NOT NULL,
    subject TEXT NOT NULL,
    what TEXT NOT NULL,
    qty INTEGER NOT NULL,
    released INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE "test_releases" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id INTEGER NOT NULL
);
CREATE TABLE "test_effects" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    subject TEXT NOT NULL,
    effect TEXT NOT NULL
);
CREATE TABLE "test_flags" (name TEXT PRIMARY KEY);
"#;

const DOWN: &str = r#"
DROP TABLE "test_flags";
DROP TABLE "test_effects";
DROP TABLE "test_releases";
DROP TABLE "test_holds";
DROP TABLE "test_material_requests";
DROP TABLE "test_bookings";
DROP TABLE "test_refunds";
"#;

const MIGRATIONS: &[Migration] = &[Migration::new(
    "20261018130099_create_test_approval_subjects",
    UP,
    Some(DOWN),
)];

/// The module of the test subjects: their tables, machines, default
/// policies and pages.
struct Subjects;

impl Module for Subjects {
    fn name(&self) -> &'static str {
        "test-subjects"
    }

    fn routes(&self) -> Routes {
        Routes::new()
            .group(
                "/refunds",
                "refunds.",
                wl_kernel::machines::routes::<Refund>(),
            )
            .group(
                "/bookings",
                "bookings.",
                wl_kernel::machines::routes::<Booking>(),
            )
            .group(
                "/materials",
                "materials.",
                wl_kernel::machines::routes::<MaterialRequest>(),
            )
            .get("/refunds/{id}", show_refund)
            .get("/refunds/{id}/actions", refund_actions)
            .get("/approval-requests/{id}/actions", request_actions)
    }

    fn migrations(&self) -> &'static [Migration] {
        MIGRATIONS
    }

    fn register(&self, app: &mut Registry) {
        app.model::<Refund>()
            .model::<Booking>()
            .model::<MaterialRequest>();
        SubjectMachine::of::<Refund>(REFUND)
            .waiting(RefundStatus::PendingApproval)
            .reserve(reserve_refund)
            .release(release)
            .register();
        SubjectMachine::of::<Booking>(BOOKING)
            .reserve(reserve_slot)
            .release(release)
            .register();
        SubjectMachine::of::<MaterialRequest>(MATERIAL)
            .waiting(MaterialStatus::PendingApproval)
            .reserve(hold_cement)
            .release(release)
            .register();
        PolicyDefault::new(REFUND_ACTION)
            .step(["finance"])
            .register();
        PolicyDefault::new("bookings.confirm")
            .step(["manager"])
            .register();
        PolicyDefault::new(ORDER_ACTION)
            .step(["site_manager"])
            .register();
    }
}

/// The refund's page: its approval cards.
async fn show_refund(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    lang: Lang,
) -> Result<View> {
    let approvals =
        approvals::cards_for(&state.db, &Subject::new(REFUND, id), &user, &lang).await?;
    Ok(view("refunds/show.html", context! { approvals }))
}

/// The names of the transitions the viewer may run on the refund now.
async fn refund_actions(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<String> {
    let refund = Refund::find(&state.db, id).await?.ok_or(Error::NotFound)?;
    let names: Vec<String> = refund
        .available(&user, &state.db)
        .await?
        .into_iter()
        .map(|action| action.name)
        .collect();
    Ok(names.join(","))
}

/// The names of the transitions the viewer may run on the approval request
/// now (none: the kernel's are all system transitions).
async fn request_actions(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<String> {
    let request = ApprovalRequest::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    let names: Vec<String> = request
        .available(&user, &state.db)
        .await?
        .into_iter()
        .map(|action| action.name)
        .collect();
    Ok(names.join(","))
}

// ---------------------------------------------------------------------------
// The world each check starts from.
// ---------------------------------------------------------------------------

const CLIENT: &str = r#"
[client]
name = "Acme Properties LLC"
app = "property"

[modules]
enabled = []

[[legal_entities]]
code = "ACME"
name_en = "Acme Properties LLC"
default = true
"#;

const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

const REFUND_PAGE: &str = r#"<rx-page layout="layouts/app.html" title="Refund">
  <rx-include template="wl-kernel/approval_card.html"/>
</rx-page>
"#;

/// The app and its people: Aisha asks (she holds `finance` too, to show she
/// cannot approve her own request), Bilal approves (`finance`, `manager`,
/// `site_manager`), Chen approves (`finance`, `owner`), Dana is an admin.
struct World {
    app: TestApp,
    a: User,
    b: User,
    c: User,
    admin: User,
    _views: tempfile::TempDir,
}

async fn world() -> World {
    let views = tempfile::tempdir().expect("a temporary views directory");
    for (name, text) in [
        ("layouts/app.html", LAYOUT),
        ("refunds/show.html", REFUND_PAGE),
    ] {
        let path = views.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(
        App::new()
            .module(Auth::new())
            .module(Permissions)
            .module(Subjects),
        config,
    )
    .expect("the kernel applies");
    let app = TestApp::with_config(app, |c| c.views_path = views.path().to_path_buf()).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    app.fake_events();

    let db = app.db();
    let view = wl_kernel::permissions::APPROVALS_VIEW;
    for role in ["finance", "owner", "manager", "site_manager", "admin"] {
        permissions::define_role(db, role, &[view]).await.unwrap();
    }
    let user = |name: &'static str, roles: &'static [&'static str]| async move {
        let email = format!("{}@example.com", name.to_lowercase());
        let user = User::register(db, name, &email, "password123")
            .await
            .unwrap();
        for role in roles {
            user.assign_role(db, role).await.unwrap();
        }
        user
    };
    let a = user("Aisha", &["finance"]).await;
    let b = user("Bilal", &["finance", "manager", "site_manager"]).await;
    let c = user("Chen", &["finance", "owner"]).await;
    let admin = user("Dana", &["admin"]).await;
    World {
        app,
        a,
        b,
        c,
        admin,
        _views: views,
    }
}

impl World {
    fn state(&self) -> &AppState {
        self.app.state()
    }

    fn db(&self) -> &Db {
        self.app.db()
    }

    /// `user` runs `transition` on the record at `path` (e.g. `/refunds/1`)
    /// through the kernel's transition route.
    async fn run(&self, user: &User, path: &str, transition: &str) -> TestResponse {
        self.app.acting_as(user);
        self.app
            .post(&format!("{path}/transitions/{transition}"), &[])
            .await
    }

    /// A refund of `amount` fils from a deposit with `balance` left.
    async fn refund(&self, amount: i64, balance: i64) -> Refund {
        Refund::create(
            self.db(),
            Refund {
                amount,
                balance,
                ..Default::default()
            },
        )
        .await
        .unwrap()
    }

    /// A refund of AED 500 that Aisha sent for approval; its request.
    async fn refund_sent(&self) -> (Refund, ApprovalRequest) {
        let refund = self.refund(50_000, 100_000).await;
        let res = self
            .run(&self.a, &format!("/refunds/{}", refund.id), "refund")
            .await;
        assert!(
            res.status.is_redirection(),
            "{}: {}",
            res.status,
            res.text()
        );
        let request = self.request_about(&Subject::new(REFUND, refund.id)).await;
        (refund, request)
    }

    /// The latest request about `subject`.
    async fn request_about(&self, subject: &Subject) -> ApprovalRequest {
        ApprovalRequest::query()
            .where_(ApprovalRequest::SUBJECT_TYPE.eq(subject.subject_type.as_str()))
            .where_(ApprovalRequest::SUBJECT_ID.eq(subject.subject_id))
            .order_by_desc(ApprovalRequest::ID)
            .first(self.db())
            .await
            .unwrap()
            .expect("a request about the subject")
    }

    async fn reload(&self, request: &ApprovalRequest) -> ApprovalRequest {
        ApprovalRequest::find(self.db(), request.id)
            .await
            .unwrap()
            .unwrap()
    }

    async fn refund_status(&self, id: i64) -> RefundStatus {
        Refund::find(self.db(), id).await.unwrap().unwrap().status
    }

    /// The refund's history, oldest first.
    async fn refund_history(&self, id: i64) -> Vec<renox::state_machine::StateTransition> {
        history(self.db(), Refund::TABLE, id).await.unwrap()
    }

    /// `user`'s notices of `kind`, newest first, as JSON text.
    async fn notices(&self, user: &User, kind: &str) -> Vec<String> {
        user.notifications(self.db(), 50)
            .await
            .unwrap()
            .into_iter()
            .filter(|n| n.kind == kind)
            .map(|n| n.data.to_string())
            .collect()
    }

    /// The holds of `request` not released yet.
    async fn open_holds(&self, request: &ApprovalRequest) -> Vec<(String, i64)> {
        let rows = sql("SELECT what, qty FROM test_holds WHERE request_id = ? AND released = 0")
            .bind(request.id)
            .fetch_all(self.db())
            .await
            .unwrap();
        rows.iter()
            .map(|row| {
                (
                    row.try_get::<String>("what").unwrap(),
                    row.try_get::<i64>("qty").unwrap(),
                )
            })
            .collect()
    }

    /// How many times the release handler ran (and committed) for `request`.
    async fn releases(&self, request: &ApprovalRequest) -> i64 {
        sql("SELECT COUNT(*) FROM test_releases WHERE request_id = ?")
            .bind(request.id)
            .scalar::<i64>(self.db())
            .await
            .unwrap()
    }

    async fn effects(&self) -> i64 {
        sql("SELECT COUNT(*) FROM test_effects")
            .scalar::<i64>(self.db())
            .await
            .unwrap()
    }

    /// The `ApprovalDecided` of `request`.
    fn decided(&self, request: &ApprovalRequest) -> Vec<ApprovalDecided> {
        self.app
            .emitted::<ApprovalDecided>()
            .into_iter()
            .filter(|e| e.request == request.id)
            .collect()
    }

    /// Gives the refund policy two steps: `finance`, then `finance` or
    /// `owner` (an admin's change; pending requests keep their copy).
    async fn two_step_refunds(&self) {
        let mut policy = ApprovalPolicy::query()
            .where_(ApprovalPolicy::ACTION.eq(REFUND_ACTION))
            .first(self.db())
            .await
            .unwrap()
            .expect("the refund policy is installed");
        policy.steps = Json(vec![
            Step::one_of(["finance"]),
            Step::one_of(["finance", "owner"]),
        ]);
        policy.save(self.db()).await.unwrap();
    }
}

#[track_caller]
fn assert_forbidden<T: std::fmt::Debug>(result: Result<T>) {
    match result {
        Err(Error::Forbidden) => {}
        other => panic!("expected 403, got {other:?}"),
    }
}

#[track_caller]
fn assert_refused<T: std::fmt::Debug>(result: Result<T>, message: &str) {
    match result {
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, said)) => {
            assert!(said.contains(message), "422 `{said}`, expected `{message}`");
        }
        other => panic!("expected 422 `{message}`, got {other:?}"),
    }
}

const ALREADY_DECIDED: &str = "This request was already decided.";

// ---------------------------------------------------------------------------
// The acceptance criteria.
// ---------------------------------------------------------------------------

/// AC1: a blocked action becomes a pending request with its snapshot; the
/// step-1 approvers are told; nothing of the action has run.
#[renox::test]
async fn ac1_a_blocked_action_becomes_a_pending_request_and_nothing_of_it_runs() {
    let w = world().await;
    let (refund, request) = w.refund_sent().await;

    assert_eq!(
        w.refund_status(refund.id).await,
        RefundStatus::PendingApproval
    );
    assert_eq!(request.status, ApprovalStatus::Pending);
    assert_eq!(request.action, REFUND_ACTION);
    assert_eq!(request.transition, "refund");
    assert_eq!(request.requester_id, w.a.id);
    assert_eq!(request.payload.0, json!({ "amount": 50_000 }));
    assert_eq!(request.subject_state_before, "requested");
    assert_eq!(request.steps.0, vec![Step::one_of(["finance"])]);
    assert!(request.sod);
    let created = request.created_at.expect("a creation time");
    let expires = request.expires_at.expect("an expiry time");
    // 72 hours from submit (the row's own timestamp is taken a moment later).
    let waits = (expires - created).num_seconds();
    assert!((72 * 3600 - 60..=72 * 3600).contains(&waits), "{waits}s");

    w.app.assert_emitted::<ApprovalRequested>(|e| {
        e.request == request.id
            && e.subject == Subject::new(REFUND, refund.id)
            && e.step_approvers == vec![w.b.id, w.c.id]
    });
    assert_eq!(w.notices(&w.b, REQUESTED).await.len(), 1);
    assert_eq!(w.notices(&w.c, REQUESTED).await.len(), 1);
    assert!(
        w.notices(&w.a, REQUESTED).await.is_empty(),
        "SoD: not the requester"
    );
    assert_eq!(w.effects().await, 0, "the refund is not paid out");
    assert_eq!(
        w.open_holds(&request).await,
        [("refund".to_owned(), 50_000)]
    );
    let badge = approvals::pending_for(w.db(), &Subject::new(REFUND, refund.id))
        .await
        .unwrap();
    assert_eq!(badge.len(), 1);

    // Without a waiting state the interceptor defers the user's run: the
    // booking stays where it is and its effect does not run.
    let booking = Booking::create(
        w.db(),
        Booking {
            slot: "2026-11-03 10:00".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let res = w
        .run(&w.a, &format!("/bookings/{}", booking.id), "confirm")
        .await;
    assert!(res.status.is_redirection(), "{}", res.text());
    let held = w.request_about(&Subject::new(BOOKING, booking.id)).await;
    assert!(held.is_pending());
    assert_eq!(held.subject_state_before, "requested");
    let booking = Booking::find(w.db(), booking.id).await.unwrap().unwrap();
    assert_eq!(booking.status, BookingStatus::Requested);
    assert_eq!(w.effects().await, 0, "the booking is not confirmed");
    w.app.assert_emitted::<ApprovalRequested>(|e| {
        e.request == held.id && e.step_approvers == vec![w.b.id]
    });

    // A second request for the same subject and action is refused.
    let mut tx = w.db().begin().await.unwrap();
    let again = Require::new(
        REFUND_ACTION,
        Subject::new(REFUND, refund.id),
        "refund",
        w.a.id,
        "requested",
    );
    assert_refused(
        approvals::require_in(&mut tx, &again).await,
        "already waiting for approval",
    );
}

/// AC2: segregation of duties; the last approval runs `<action>_approved`.
#[renox::test]
async fn ac2_sod_holds_and_the_last_approval_runs_the_action() {
    let w = world().await;
    w.two_step_refunds().await;
    let (refund, request) = w.refund_sent().await;
    assert_eq!(request.steps.len(), 2);

    // Aisha holds `finance` but asked: she may not approve.
    assert_forbidden(approvals::approve(w.state(), request.id, &w.a, "").await);
    let step = approvals::approve(w.state(), request.id, &w.b, "fine")
        .await
        .unwrap();
    assert!(step.is_none(), "the request waits for step 2");
    assert_eq!(w.reload(&request).await.current_step, 2);
    // Bilal approved step 1 and holds a step-2 role: still not him.
    assert_forbidden(approvals::approve(w.state(), request.id, &w.b, "").await);
    assert_eq!(
        w.refund_status(refund.id).await,
        RefundStatus::PendingApproval
    );

    // Chen approves the last step through the card's button.
    w.app.acting_as(&w.c);
    w.app
        .htmx()
        .post(
            &format!("/approvals/{}/approve", request.id),
            &[("comment", "")],
        )
        .await
        .assert_ok();
    let request = w.reload(&request).await;
    assert_eq!(request.status, ApprovalStatus::Approved);
    assert!(request.decided_at.is_some());
    assert_eq!(w.refund_status(refund.id).await, RefundStatus::Refunded);
    assert_eq!(w.effects().await, 1, "paid out once approved");
    let last = w.refund_history(refund.id).await.pop().unwrap();
    assert_eq!(last.transition, "refund_approved");
    assert_eq!(last.actor_id, None, "run as the system");
    assert_eq!(last.metadata["requester_id"], w.a.id);
    assert_eq!(
        w.decided(&request),
        [ApprovalDecided {
            request: request.id,
            decision: ApprovalEnd::Approved,
            approvers: vec![w.b.id, w.c.id],
            subject: Subject::new(REFUND, refund.id),
            state_returned_to: None,
        }]
    );
    assert_eq!(w.notices(&w.a, DECIDED).await.len(), 1);
    assert_eq!(w.releases(&request).await, 0, "a yes releases nothing");
}

/// AC3: rejected with "duplicate", the refund is back in `requested`.
#[renox::test]
async fn ac3_a_rejected_refund_goes_back_with_the_comment_as_reason() {
    let w = world().await;
    let (refund, request) = w.refund_sent().await;

    w.app.acting_as(&w.b);
    w.app
        .htmx()
        .post(
            &format!("/approvals/{}/reject", request.id),
            &[("comment", "duplicate")],
        )
        .await
        .assert_ok();

    assert_eq!(w.refund_status(refund.id).await, RefundStatus::Requested);
    let back = w.refund_history(refund.id).await.pop().unwrap();
    assert_eq!(back.transition, "back_to_requested");
    assert_eq!(back.reason.as_deref(), Some("duplicate"));
    let request = w.reload(&request).await;
    assert_eq!(request.status, ApprovalStatus::Rejected);
    assert_eq!(request.state_returned_to.as_deref(), Some("requested"));
    let told = w.notices(&w.a, DECIDED).await;
    assert_eq!(told.len(), 1);
    assert!(told[0].contains("duplicate"), "{}", told[0]);
    assert_eq!(
        w.decided(&request),
        [ApprovalDecided {
            request: request.id,
            decision: ApprovalEnd::Rejected,
            approvers: vec![w.b.id],
            subject: Subject::new(REFUND, refund.id),
            state_returned_to: Some("requested".into()),
        }]
    );
    assert!(w.open_holds(&request).await.is_empty());
}

/// AC4: withdrawn by Aisha, the refund is back; the approvers are told.
#[renox::test]
async fn ac4_a_withdrawn_refund_goes_back_and_the_approvers_are_told() {
    let w = world().await;
    let (refund, request) = w.refund_sent().await;

    // Only the requester may withdraw.
    assert_forbidden(approvals::withdraw(w.state(), request.id, &w.b).await);
    w.app.acting_as(&w.a);
    let res = w
        .app
        .post(&format!("/approvals/{}/withdraw", request.id), &[])
        .await;
    assert!(
        res.status.is_redirection(),
        "{}: {}",
        res.status,
        res.text()
    );

    assert_eq!(w.refund_status(refund.id).await, RefundStatus::Requested);
    let back = w.refund_history(refund.id).await.pop().unwrap();
    assert_eq!(back.transition, "back_to_requested");
    assert_eq!(
        back.reason.as_deref(),
        Some("approval withdrawn by requester")
    );
    assert_eq!(w.reload(&request).await.status, ApprovalStatus::Withdrawn);
    assert_eq!(w.notices(&w.b, DECIDED).await.len(), 1);
    assert_eq!(w.notices(&w.c, DECIDED).await.len(), 1);
    assert!(w.notices(&w.a, DECIDED).await.is_empty());
    let decided = w.decided(&request);
    assert_eq!(decided.len(), 1);
    assert_eq!(decided[0].decision, ApprovalEnd::Withdrawn);
    assert_eq!(decided[0].state_returned_to.as_deref(), Some("requested"));
}

/// AC5: `expire` waits for `expires_at`, then sends the refund back.
#[renox::test]
async fn ac5_expire_is_refused_before_its_time_and_then_sends_the_refund_back() {
    let w = world().await;
    let (refund, request) = w.refund_sent().await;

    assert_refused(
        approvals::expire(w.state(), request.id).await,
        "has not reached its expiry time",
    );
    assert!(w.reload(&request).await.is_pending());

    w.app.travel(Duration::from_secs(73 * 60 * 60));
    let decided = w
        .app
        .at_travelled_time(approvals::expire(w.state(), request.id))
        .await
        .unwrap();
    assert_eq!(decided.decision, ApprovalEnd::Expired);
    assert_eq!(decided.state_returned_to.as_deref(), Some("requested"));
    assert_eq!(w.refund_status(refund.id).await, RefundStatus::Requested);
    let back = w.refund_history(refund.id).await.pop().unwrap();
    assert_eq!(back.reason.as_deref(), Some("approval expired"));
    assert_eq!(w.reload(&request).await.status, ApprovalStatus::Expired);
    assert_eq!(w.notices(&w.a, DECIDED).await.len(), 1);
    assert_eq!(w.decided(&request).len(), 1);
}

/// AC6: the balance dropped below the amount: the final approval fails and
/// the refund goes back.
#[renox::test]
async fn ac6_a_final_approval_whose_guard_fails_ends_in_failed() {
    let w = world().await;
    let (refund, request) = w.refund_sent().await;
    sql("UPDATE test_refunds SET balance = 30000 WHERE id = ?")
        .bind(refund.id)
        .execute(w.db())
        .await
        .unwrap();

    let decided = approvals::approve(w.state(), request.id, &w.b, "")
        .await
        .unwrap()
        .expect("the request ended");
    assert_eq!(decided.decision, ApprovalEnd::Failed);
    assert_eq!(decided.state_returned_to.as_deref(), Some("requested"));
    assert_eq!(w.reload(&request).await.status, ApprovalStatus::Failed);
    assert_eq!(w.refund_status(refund.id).await, RefundStatus::Requested);
    assert_eq!(w.effects().await, 0, "nothing was paid out");
    let reason = format!("approval failed: {OVER_BALANCE}");
    let back = w.refund_history(refund.id).await.pop().unwrap();
    assert_eq!(back.transition, "back_to_requested");
    assert_eq!(back.reason.as_deref(), Some(reason.as_str()));
    for user in [&w.a, &w.b] {
        let told = w.notices(user, DECIDED).await;
        assert_eq!(told.len(), 1, "user {}", user.name);
        assert!(told[0].contains(OVER_BALANCE), "{}", told[0]);
    }
    assert_eq!(w.decided(&request).len(), 1);
    assert!(w.open_holds(&request).await.is_empty());
}

/// A booking Aisha asked to confirm (deferred, as bookings have no waiting
/// state), with the 10:00 slot of 2026-11-03 reserved; its request.
async fn booking_sent(w: &World) -> (Booking, ApprovalRequest) {
    let booking = Booking::create(
        w.db(),
        Booking {
            slot: "2026-11-03 10:00".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let res = w
        .run(&w.a, &format!("/bookings/{}", booking.id), "confirm")
        .await;
    assert!(res.status.is_redirection(), "{}", res.text());
    let request = w.request_about(&Subject::new(BOOKING, booking.id)).await;
    (booking, request)
}

/// AC7: without a waiting state, a reject moves nothing and the badge goes.
#[renox::test]
async fn ac7_without_a_waiting_state_a_reject_only_clears_the_badge() {
    let w = world().await;
    let (booking, request) = booking_sent(&w).await;
    let subject = Subject::new(BOOKING, booking.id);
    assert_eq!(
        approvals::pending_for(w.db(), &subject)
            .await
            .unwrap()
            .len(),
        1
    );
    let rows = history(w.db(), Booking::TABLE, booking.id)
        .await
        .unwrap()
        .len();

    let decided = approvals::reject(w.state(), request.id, &w.b, "fully booked")
        .await
        .unwrap();
    assert_eq!(decided.state_returned_to, None);
    let after = Booking::find(w.db(), booking.id).await.unwrap().unwrap();
    assert_eq!(after.status, BookingStatus::Requested);
    assert_eq!(
        history(w.db(), Booking::TABLE, booking.id)
            .await
            .unwrap()
            .len(),
        rows,
        "no history row"
    );
    assert!(
        approvals::pending_for(w.db(), &subject)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(w.reload(&request).await.state_returned_to, None);
}

/// AC8: the booking's reserved slot is free again after the reject.
#[renox::test]
async fn ac8_a_rejected_booking_frees_its_reserved_slot() {
    let w = world().await;
    let (booking, request) = booking_sent(&w).await;
    assert_eq!(
        w.open_holds(&request).await,
        [("slot 2026-11-03 10:00".to_owned(), 1)]
    );

    approvals::reject(w.state(), request.id, &w.b, "fully booked")
        .await
        .unwrap();
    let after = Booking::find(w.db(), booking.id).await.unwrap().unwrap();
    assert_eq!(after.status, BookingStatus::Requested);
    assert!(
        approvals::pending_for(w.db(), &Subject::new(BOOKING, booking.id))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(w.open_holds(&request).await.is_empty(), "the slot is free");
    assert!(w.reload(&request).await.released_at.is_some());
    assert_eq!(w.effects().await, 0, "never confirmed");
}

/// AC9: the expired material request is back in `draft` and its cement is
/// released in the same transaction: a failing release undoes the end.
#[renox::test]
async fn ac9_an_expired_material_request_goes_back_and_releases_its_hold_atomically() {
    let w = world().await;
    let material = MaterialRequest::create(
        w.db(),
        MaterialRequest {
            bags: 20,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let res = w
        .run(&w.a, &format!("/materials/{}", material.id), "submit")
        .await;
    assert!(res.status.is_redirection(), "{}", res.text());
    let request = w.request_about(&Subject::new(MATERIAL, material.id)).await;
    assert_eq!(w.open_holds(&request).await, [("cement".to_owned(), 20)]);
    let status = || async {
        MaterialRequest::find(w.db(), material.id)
            .await
            .unwrap()
            .unwrap()
            .status
    };
    assert_eq!(status().await, MaterialStatus::PendingApproval);
    w.app.travel(Duration::from_secs(73 * 60 * 60));

    // The release fails: nothing of the end stays.
    sql("INSERT INTO test_flags (name) VALUES ('fail_release')")
        .execute(w.db())
        .await
        .unwrap();
    let failed = w
        .app
        .at_travelled_time(approvals::expire(w.state(), request.id))
        .await;
    assert!(failed.is_err(), "{failed:?}");
    assert!(w.reload(&request).await.is_pending());
    assert_eq!(status().await, MaterialStatus::PendingApproval);
    assert_eq!(w.open_holds(&request).await.len(), 1);
    assert_eq!(w.releases(&request).await, 0);
    assert!(w.decided(&request).is_empty());

    sql("DELETE FROM test_flags").execute(w.db()).await.unwrap();
    let decided = w
        .app
        .at_travelled_time(approvals::expire(w.state(), request.id))
        .await
        .unwrap();
    assert_eq!(decided.state_returned_to.as_deref(), Some("draft"));
    assert_eq!(status().await, MaterialStatus::Draft);
    let back = history(w.db(), MaterialRequest::TABLE, material.id)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(back.transition, "back_to_draft");
    assert_eq!(back.reason.as_deref(), Some("approval expired"));
    assert!(
        w.open_holds(&request).await.is_empty(),
        "the cement is free"
    );
    assert_eq!(w.releases(&request).await, 1);
}

/// AC10: a refund cancelled while waiting stays cancelled; the request says
/// where it went; the reservation is still released.
#[renox::test]
async fn ac10_a_subject_that_already_moved_is_left_where_it_is() {
    let w = world().await;
    let (refund, request) = w.refund_sent().await;
    let res = w
        .run(&w.a, &format!("/refunds/{}", refund.id), "cancel")
        .await;
    assert!(res.status.is_redirection(), "{}", res.text());
    assert_eq!(w.refund_status(refund.id).await, RefundStatus::Cancelled);
    let rows = w.refund_history(refund.id).await.len();

    let decided = approvals::reject(w.state(), request.id, &w.b, "duplicate")
        .await
        .unwrap();
    assert_eq!(decided.state_returned_to, None);
    assert_eq!(w.refund_status(refund.id).await, RefundStatus::Cancelled);
    assert_eq!(w.refund_history(refund.id).await.len(), rows, "not moved");
    let request = w.reload(&request).await;
    assert_eq!(request.subject_moved_to.as_deref(), Some("cancelled"));
    assert_eq!(request.state_returned_to, None);
    let lang = w.state().lang("en");
    let card = approvals::card(w.db(), &request, &w.a, &lang)
        .await
        .unwrap();
    assert_eq!(
        card.subject_moved.as_deref(),
        Some("Subject already moved to `cancelled`.")
    );
    assert!(w.open_holds(&request).await.is_empty());
    let told = w.notices(&w.a, DECIDED).await;
    assert_eq!(told.len(), 1);
    assert!(
        told[0].contains("Subject already moved to `cancelled`"),
        "{}",
        told[0]
    );
}

/// AC11: a reject needs a comment that is not blank.
#[renox::test]
async fn ac11_a_reject_without_a_comment_is_refused() {
    let w = world().await;
    let (refund, request) = w.refund_sent().await;

    for comment in ["", "   "] {
        assert_refused(
            approvals::reject(w.state(), request.id, &w.b, comment).await,
            "A comment is required to reject or return.",
        );
        let mut tx = w.db().begin().await.unwrap();
        let ended = approvals::end_in(&mut tx, request.id, End::reject(w.b.id, 1, comment)).await;
        assert_refused(
            ended.map(|e| e.request.id),
            "A comment is required to reject or return.",
        );
    }
    w.app.acting_as(&w.b);
    w.app
        .htmx()
        .post(
            &format!("/approvals/{}/reject", request.id),
            &[("comment", "  ")],
        )
        .await
        .assert_invalid("comment");

    assert!(w.reload(&request).await.is_pending());
    assert_eq!(
        w.refund_status(refund.id).await,
        RefundStatus::PendingApproval
    );
    assert!(w.decided(&request).is_empty());
}

/// AC12: a reject and a withdraw at the same time: exactly one wins.
#[renox::test]
async fn ac12_a_reject_racing_a_withdraw_ends_the_request_once() {
    let w = world().await;
    let (refund, request) = w.refund_sent().await;

    let (rejected, withdrawn) = renox::tokio::join!(
        approvals::reject(w.state(), request.id, &w.b, "duplicate"),
        approvals::withdraw(w.state(), request.id, &w.a),
    );
    assert!(
        rejected.is_ok() != withdrawn.is_ok(),
        "exactly one wins: {rejected:?} / {withdrawn:?}"
    );
    let lost = if rejected.is_ok() {
        withdrawn
    } else {
        rejected
    };
    assert_refused(lost, ALREADY_DECIDED);

    let request = w.reload(&request).await;
    assert!(matches!(
        request.status,
        ApprovalStatus::Rejected | ApprovalStatus::Withdrawn
    ));
    let ends = history(w.db(), ApprovalRequest::TABLE, request.id)
        .await
        .unwrap()
        .into_iter()
        .filter(|row| row.from_state.as_deref() == Some("pending"))
        .count();
    assert_eq!(ends, 1, "one end state");
    let backs = w
        .refund_history(refund.id)
        .await
        .into_iter()
        .filter(|row| row.transition.starts_with("back_to_"))
        .count();
    assert_eq!(backs, 1, "one back transition");
    assert_eq!(w.releases(&request).await, 1);
    assert_eq!(w.decided(&request).len(), 1);
}

/// AC13: rejected at step 2, the step-1 decision stays.
#[renox::test]
async fn ac13_a_reject_at_step_two_keeps_the_step_one_decision() {
    let w = world().await;
    w.two_step_refunds().await;
    let (refund, request) = w.refund_sent().await;
    approvals::approve(w.state(), request.id, &w.b, "")
        .await
        .unwrap();
    approvals::reject(w.state(), request.id, &w.c, "over budget")
        .await
        .unwrap();

    assert_eq!(w.refund_status(refund.id).await, RefundStatus::Requested);
    let request = w.reload(&request).await;
    let decisions: Vec<_> = request
        .decisions(w.db())
        .await
        .unwrap()
        .into_iter()
        .map(|d| (d.step, d.approver_id, d.decision.to_string(), d.comment))
        .collect();
    assert_eq!(
        decisions,
        [
            (1, w.b.id, "approve".to_owned(), None),
            (
                2,
                w.c.id,
                "reject".to_owned(),
                Some("over budget".to_owned())
            ),
        ]
    );
    let lang = w.state().lang("en");
    let timeline = approvals::timeline_for(w.db(), &Subject::new(REFUND, refund.id), &lang)
        .await
        .unwrap();
    let titles: Vec<&str> = timeline
        .iter()
        .filter_map(|item| item["title"].as_str())
        .collect();
    assert_eq!(
        titles,
        [
            "Approval requested: payments.refund",
            "Approved at step 1 by Bilal",
            "Rejected at step 2 by Chen",
            "Approval rejected: payments.refund",
        ]
    );
    assert_eq!(timeline[3]["body"], "over budget");
    assert_eq!(w.decided(&request)[0].approvers, vec![w.b.id, w.c.id]);
}

/// AC14: nobody sees or runs `back_to_*` and `*_approved`.
#[renox::test]
async fn ac14_no_user_sees_or_runs_the_kernels_transitions() {
    let w = world().await;
    let (refund, _request) = w.refund_sent().await;
    let path = format!("/refunds/{}", refund.id);

    for user in [&w.admin, &w.a, &w.b] {
        w.app.acting_as(user);
        let actions = w.app.get(&format!("{path}/actions")).await;
        actions.assert_ok();
        assert_eq!(actions.text(), "cancel", "user {}", user.name);
    }
    for name in ["back_to_requested", "refund_approved"] {
        w.run(&w.admin, &path, name).await.assert_forbidden();
    }
    assert_eq!(
        w.refund_status(refund.id).await,
        RefundStatus::PendingApproval
    );
}

/// AC14: a `back_to_*` a user could run is refused when declared.
#[test]
#[should_panic(expected = "must be `.system()`")]
fn ac14_a_back_transition_that_is_not_system_panics() {
    let _ = SubjectMachine::of::<Careless>("tests.careless");
}

/// AC15: the release handler runs once; a second end is refused.
#[renox::test]
async fn ac15_one_release_per_request_and_no_second_end() {
    let w = world().await;
    let (refund, request) = w.refund_sent().await;
    approvals::reject(w.state(), request.id, &w.b, "duplicate")
        .await
        .unwrap();

    assert_refused(
        approvals::withdraw(w.state(), request.id, &w.a).await,
        ALREADY_DECIDED,
    );
    w.app.travel(Duration::from_secs(73 * 60 * 60));
    assert_refused(
        w.app
            .at_travelled_time(approvals::expire(w.state(), request.id))
            .await,
        ALREADY_DECIDED,
    );
    let mut tx = w.db().begin().await.unwrap();
    let again = approvals::end_in(&mut tx, request.id, End::withdraw(w.a.id)).await;
    assert_refused(again.map(|e| e.request.id), ALREADY_DECIDED);
    drop(tx);

    assert_eq!(w.releases(&request).await, 1);
    assert_eq!(w.reload(&request).await.status, ApprovalStatus::Rejected);
    assert_eq!(w.refund_status(refund.id).await, RefundStatus::Requested);
    assert_eq!(w.decided(&request).len(), 1);
}

/// The approval card shows each viewer the buttons they may use.
#[renox::test]
async fn the_approval_card_shows_each_viewer_their_buttons() {
    let w = world().await;
    let (refund, request) = w.refund_sent().await;
    let page = format!("/refunds/{}", refund.id);
    let approve = format!("/approvals/{}/approve", request.id);
    let reject = format!("/approvals/{}/reject", request.id);
    let withdraw = format!("/approvals/{}/withdraw", request.id);

    w.app.acting_as(&w.b);
    let res = w.app.get(&page).await;
    res.assert_ok()
        .assert_see("Approval: payments.refund")
        .assert_see("Pending approval")
        .assert_see("Aisha")
        .assert_see(&approve)
        .assert_see(&reject)
        .assert_dont_see(&withdraw);

    w.app.acting_as(&w.a);
    w.app
        .get(&page)
        .await
        .assert_ok()
        .assert_see(&withdraw)
        .assert_dont_see(&approve)
        .assert_dont_see(&reject);

    w.app.acting_as(&w.admin);
    w.app
        .get(&page)
        .await
        .assert_ok()
        .assert_see("Approval: payments.refund")
        .assert_dont_see(&approve)
        .assert_dont_see(&withdraw);

    // Once ended, no card is left.
    approvals::reject(w.state(), request.id, &w.b, "duplicate")
        .await
        .unwrap();
    w.app.acting_as(&w.b);
    w.app
        .get(&page)
        .await
        .assert_ok()
        .assert_dont_see("Approval: payments.refund");
}

// ---------------------------------------------------------------------------
// #26 part 2: resubmission links.
// ---------------------------------------------------------------------------

/// A resubmission after a return links to it, restarts at step 1 and closes
/// the task; a second return is followed by `earlier_decisions` newest
/// first.
#[renox::test]
async fn a_resubmission_links_to_the_returned_request_and_closes_its_task() {
    let w = world().await;
    let (refund, first) = w.refund_sent().await;
    assert!(first.resubmission_of.is_none());

    approvals::return_request(w.state(), first.id, &w.b, "attach the photos")
        .await
        .unwrap();
    let tasks = approvals::needs_action(w.db(), &w.a).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].request_id, first.id);
    assert!(
        approvals::needs_action(w.db(), &w.b)
            .await
            .unwrap()
            .is_empty()
    );

    let path = format!("/refunds/{}", refund.id);
    let res = w.run(&w.a, &path, "refund").await;
    assert!(res.status.is_redirection(), "{}", res.text());
    let second = w.request_about(&Subject::new(REFUND, refund.id)).await;
    assert_eq!(second.resubmission_of, Some(first.id));
    assert_eq!(second.current_step, 1);
    assert!(second.is_pending());
    assert!(second.decisions(w.db()).await.unwrap().is_empty());
    assert_eq!(w.reload(&first).await.status, ApprovalStatus::Returned);
    assert!(
        approvals::needs_action(w.db(), &w.a)
            .await
            .unwrap()
            .is_empty()
    );
    let closed = approvals::close_not_resubmitting(w.db(), first.id, &w.a).await;
    assert!(closed.is_err(), "the resubmission already closed the task");

    let earlier = approvals::earlier_decisions(w.db(), &second).await.unwrap();
    assert_eq!(earlier.len(), 1);
    assert_eq!(earlier[0].status, ApprovalStatus::Returned);
    assert_eq!(earlier[0].approver, w.b.id);
    assert_eq!(earlier[0].comment, "attach the photos");

    approvals::return_request(w.state(), second.id, &w.b, "still missing")
        .await
        .unwrap();
    let res = w.run(&w.a, &path, "refund").await;
    assert!(res.status.is_redirection(), "{}", res.text());
    let third = w.request_about(&Subject::new(REFUND, refund.id)).await;
    assert_eq!(third.resubmission_of, Some(second.id));
    let comments: Vec<_> = approvals::earlier_decisions(w.db(), &third)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.comment)
        .collect();
    assert_eq!(comments, ["still missing", "attach the photos"]);
}

/// A rejection is linked only when the payload is the same; "not
/// resubmitting" is for the requester and works once.
#[renox::test]
async fn a_rejection_links_only_an_equal_payload_and_not_resubmitting_closes_once() {
    let w = world().await;
    let (refund, first) = w.refund_sent().await;
    approvals::reject(w.state(), first.id, &w.b, "no")
        .await
        .unwrap();
    let path = format!("/refunds/{}", refund.id);
    let res = w.run(&w.a, &path, "refund").await;
    assert!(res.status.is_redirection(), "{}", res.text());
    let second = w.request_about(&Subject::new(REFUND, refund.id)).await;
    assert_eq!(second.resubmission_of, Some(first.id));

    approvals::reject(w.state(), second.id, &w.b, "no again")
        .await
        .unwrap();
    let mut changed = Refund::find(w.db(), refund.id).await.unwrap().unwrap();
    changed.amount = 60_000;
    changed.save(w.db()).await.unwrap();
    let res = w.run(&w.a, &path, "refund").await;
    assert!(res.status.is_redirection(), "{}", res.text());
    let third = w.request_about(&Subject::new(REFUND, refund.id)).await;
    assert_eq!(third.resubmission_of, None, "a different amount");
    assert!(
        approvals::earlier_decisions(w.db(), &third)
            .await
            .unwrap()
            .is_empty()
    );

    let (_, returned) = w.refund_sent().await;
    approvals::return_request(w.state(), returned.id, &w.b, "fix it")
        .await
        .unwrap();
    let denied = approvals::close_not_resubmitting(w.db(), returned.id, &w.b).await;
    assert!(matches!(denied, Err(Error::Forbidden)));
    let task = approvals::close_not_resubmitting(w.db(), returned.id, &w.a)
        .await
        .unwrap();
    assert_eq!(
        task.closed_reason,
        Some(approvals::TaskClosed::NotResubmitting)
    );
    assert!(
        approvals::needs_action(w.db(), &w.a)
            .await
            .unwrap()
            .is_empty()
    );
    assert_refused(
        approvals::close_not_resubmitting(w.db(), returned.id, &w.a).await,
        "This task is already closed.",
    );
}

// ---------------------------------------------------------------------------
// #26 part 4: the acceptance checks of returning a request for changes.
// ---------------------------------------------------------------------------

const RETURN_COMMENT: &str = "attach the inspection photos";

impl World {
    /// Aisha sends the refund for approval (again); the latest request.
    async fn resubmit(&self, refund: &Refund) -> ApprovalRequest {
        let res = self
            .run(&self.a, &format!("/refunds/{}", refund.id), "refund")
            .await;
        assert!(
            res.status.is_redirection(),
            "{}: {}",
            res.status,
            res.text()
        );
        self.request_about(&Subject::new(REFUND, refund.id)).await
    }

    /// Sets the time of every decision of `request` to `at` (RFC 3339).
    async fn decided_on(&self, request: &ApprovalRequest, at: &str) {
        let at: DateTime = at.parse().expect("an instant");
        sql("UPDATE wl_approval_decisions SET created_at = ? WHERE request_id = ?")
            .bind(at)
            .bind(request.id)
            .execute(self.db())
            .await
            .unwrap();
    }

    /// A two-step deduction of AED 6,000 that Aisha sent; Bilal approved
    /// step 1.
    async fn deduction_at_step_two(&self) -> (Refund, ApprovalRequest) {
        self.two_step_refunds().await;
        let refund = self.refund(600_000, 1_000_000).await;
        let request = self.resubmit(&refund).await;
        approvals::approve(self.state(), request.id, &self.b, "")
            .await
            .unwrap();
        (refund, request)
    }

    /// The refund's page as `user` sees it.
    async fn card_page(&self, refund: &Refund, user: &User) -> TestResponse {
        self.app.acting_as(user);
        self.app.get(&format!("/refunds/{}", refund.id)).await
    }

    /// The one task of a request.
    async fn task_of(&self, request_id: i64) -> approvals::ResubmitTask {
        approvals::ResubmitTask::query()
            .where_(approvals::ResubmitTask::REQUEST_ID.eq(request_id))
            .first(self.db())
            .await
            .unwrap()
            .expect("the request has a task")
    }
}

/// AC1: returned at step 2 with a comment.
#[renox::test]
async fn ac1_a_return_at_step_two_sends_the_subject_back_and_gives_a_task() {
    let w = world().await;
    let (refund, request) = w.deduction_at_step_two().await;
    assert_eq!(w.reload(&request).await.current_step, 2);

    let decided = approvals::return_request(w.state(), request.id, &w.c, RETURN_COMMENT)
        .await
        .unwrap();

    let request = w.reload(&request).await;
    assert_eq!(request.status, ApprovalStatus::Returned);
    assert_eq!(w.refund_status(refund.id).await, RefundStatus::Requested);
    let back = w.refund_history(refund.id).await.pop().unwrap();
    assert_eq!(back.transition, "back_to_requested");
    assert_eq!(back.reason.as_deref(), Some(RETURN_COMMENT));
    assert_eq!(w.releases(&request).await, 1);
    let last = request.decisions(w.db()).await.unwrap().pop().unwrap();
    assert_eq!(
        (last.step, last.approver_id, last.decision.to_string()),
        (2, w.c.id, "return".to_owned())
    );
    assert_eq!(last.comment.as_deref(), Some(RETURN_COMMENT));

    let tasks = approvals::needs_action(w.db(), &w.a).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].request_id, request.id);
    assert!(tasks[0].is_open());
    let told = w.notices(&w.a, DECIDED).await;
    assert_eq!(told.len(), 1);
    assert!(told[0].contains(RETURN_COMMENT), "{}", told[0]);
    assert!(told[0].contains("Returned for changes"), "{}", told[0]);

    let event = ApprovalDecided {
        request: request.id,
        decision: ApprovalEnd::Returned,
        approvers: vec![w.b.id, w.c.id],
        subject: Subject::new(REFUND, refund.id),
        state_returned_to: Some("requested".into()),
    };
    assert_eq!(decided, event);
    assert_eq!(w.decided(&request), [event]);
}

/// AC2: no comment, or a non-approver, and nothing moves.
#[renox::test]
async fn ac2_a_return_needs_a_comment_and_an_approver_of_the_step() {
    let w = world().await;
    let (refund, request) = w.deduction_at_step_two().await;

    for comment in ["", "   "] {
        assert_refused(
            approvals::return_request(w.state(), request.id, &w.c, comment).await,
            "A comment is required to reject or return.",
        );
    }
    w.app.acting_as(&w.c);
    w.app
        .htmx()
        .post(
            &format!("/approvals/{}/return", request.id),
            &[("comment", "  ")],
        )
        .await
        .assert_invalid("comment");
    // The requester and an admin without the role are not approvers.
    assert_forbidden(approvals::return_request(w.state(), request.id, &w.a, RETURN_COMMENT).await);
    assert_forbidden(
        approvals::return_request(w.state(), request.id, &w.admin, RETURN_COMMENT).await,
    );
    w.app.acting_as(&w.a);
    w.app
        .htmx()
        .post(
            &format!("/approvals/{}/return", request.id),
            &[("comment", RETURN_COMMENT)],
        )
        .await
        .assert_forbidden();

    let request = w.reload(&request).await;
    assert!(request.is_pending());
    assert_eq!(request.current_step, 2);
    assert_eq!(
        w.refund_status(refund.id).await,
        RefundStatus::PendingApproval
    );
    assert!(
        approvals::needs_action(w.db(), &w.a)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(w.decided(&request).is_empty());
    assert_eq!(w.releases(&request).await, 0);
}

/// AC3: a resubmission is a new request at step 1; only Aisha is excluded.
#[renox::test]
async fn ac3_a_resubmission_is_a_fresh_request_that_closes_the_task() {
    let w = world().await;
    let (refund, first) = w.deduction_at_step_two().await;
    approvals::return_request(w.state(), first.id, &w.c, RETURN_COMMENT)
        .await
        .unwrap();

    let second = w.resubmit(&refund).await;
    assert_ne!(second.id, first.id);
    assert!(second.is_pending());
    assert_eq!(second.resubmission_of, Some(first.id));
    assert_eq!(second.current_step, 1);
    assert!(second.decisions(w.db()).await.unwrap().is_empty());

    let task = w.task_of(first.id).await;
    assert_eq!(task.status, approvals::TaskStatus::Closed);
    assert_eq!(task.closed_reason, Some(approvals::TaskClosed::Resubmitted));
    assert_eq!(task.resubmission_id, Some(second.id));
    assert_eq!(w.reload(&first).await.status, ApprovalStatus::Returned);

    // Only the requester is excluded: the returning approver may decide the
    // new request.
    assert_forbidden(approvals::approve(w.state(), second.id, &w.a, "").await);
    approvals::approve(w.state(), second.id, &w.c, "")
        .await
        .unwrap();
    assert_eq!(w.reload(&second).await.current_step, 2);
}

/// AC4: a rejection is linked to a same-amount resubmission, not to a
/// different amount.
#[renox::test]
async fn ac4_a_rejected_refund_is_linked_when_it_is_asked_for_again() {
    let w = world().await;
    let refund = w.refund(120_000, 500_000).await;
    let first = w.resubmit(&refund).await;
    approvals::reject(w.state(), first.id, &w.b, "duplicate")
        .await
        .unwrap();
    w.decided_on(&first, "2026-11-02T08:00:00Z").await;

    // A different reason is the same request (`reason` is not compared).
    assert!(approvals::same_payload(
        &json!({ "amount": 120_000, "reason": "broken lock" }),
        &json!({ "amount": 120_000, "reason": "water damage" }),
    ));
    let second = w.resubmit(&refund).await;
    assert_eq!(second.resubmission_of, Some(first.id));
    w.card_page(&refund, &w.b)
        .await
        .assert_ok()
        .assert_see("rejected on 2026-11-02 by Bilal: duplicate");

    // A request for AED 1,250 instead is a new one.
    approvals::reject(w.state(), second.id, &w.b, "no")
        .await
        .unwrap();
    let mut changed = Refund::find(w.db(), refund.id).await.unwrap().unwrap();
    changed.amount = 125_000;
    changed.save(w.db()).await.unwrap();
    let third = w.resubmit(&refund).await;
    assert_eq!(third.resubmission_of, None);
    w.card_page(&refund, &w.b)
        .await
        .assert_ok()
        .assert_see("Approval: payments.refund")
        .assert_dont_see("Earlier decisions")
        .assert_dont_see("rejected on");
}

/// AC5: an unchanged resubmission after a return; two returns are listed
/// newest first.
#[renox::test]
async fn ac5_the_card_lists_the_returns_newest_first() {
    let w = world().await;
    let (refund, first) = w.refund_sent().await;
    approvals::return_request(w.state(), first.id, &w.c, RETURN_COMMENT)
        .await
        .unwrap();
    w.decided_on(&first, "2026-11-02T08:00:00Z").await;

    let second = w.resubmit(&refund).await;
    assert_eq!(second.resubmission_of, Some(first.id));
    w.card_page(&refund, &w.b)
        .await
        .assert_ok()
        .assert_see("Earlier decisions")
        .assert_see("returned on 2026-11-02 by Chen: attach the inspection photos");

    approvals::return_request(w.state(), second.id, &w.b, "still blurry")
        .await
        .unwrap();
    w.decided_on(&second, "2026-11-05T08:00:00Z").await;
    let third = w.resubmit(&refund).await;
    assert_eq!(third.resubmission_of, Some(second.id));
    let res = w.card_page(&refund, &w.c).await;
    res.assert_ok()
        .assert_see("returned on 2026-11-05 by Bilal: still blurry")
        .assert_see("returned on 2026-11-02 by Chen: attach the inspection photos");
    let text = res.text();
    assert!(
        text.find("2026-11-05").unwrap() < text.find("2026-11-02").unwrap(),
        "newest first"
    );
}

/// AC6: a returned request is terminal for everyone.
#[renox::test]
async fn ac6_a_returned_request_offers_nothing_and_refuses_every_decision() {
    let w = world().await;
    let (refund, request) = w.refund_sent().await;
    approvals::return_request(w.state(), request.id, &w.b, RETURN_COMMENT)
        .await
        .unwrap();
    let request = w.reload(&request).await;

    for user in [&w.admin, &w.a, &w.b, &w.c] {
        w.app.acting_as(user);
        let actions = w
            .app
            .get(&format!("/approval-requests/{}/actions", request.id))
            .await;
        actions.assert_ok();
        assert_eq!(actions.text(), "", "user {}", user.name);
    }
    assert_refused(
        approvals::approve(w.state(), request.id, &w.c, "").await,
        ALREADY_DECIDED,
    );
    assert_refused(
        approvals::reject(w.state(), request.id, &w.c, "no").await,
        ALREADY_DECIDED,
    );
    assert_refused(
        approvals::return_request(w.state(), request.id, &w.c, "again").await,
        ALREADY_DECIDED,
    );
    assert_refused(
        approvals::withdraw(w.state(), request.id, &w.a).await,
        ALREADY_DECIDED,
    );
    for action in ["approve", "reject", "return"] {
        w.app.acting_as(&w.c);
        let res = w
            .app
            .htmx()
            .post(
                &format!("/approvals/{}/{action}", request.id),
                &[("comment", "x")],
            )
            .await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{action}");
    }
    assert_eq!(w.reload(&request).await.status, ApprovalStatus::Returned);
    assert_eq!(w.refund_status(refund.id).await, RefundStatus::Requested);
    assert_eq!(w.releases(&request).await, 1);
    assert_eq!(w.decided(&request).len(), 1);
}

/// AC7: "Not resubmitting" is for the requester and works once.
#[renox::test]
async fn ac7_not_resubmitting_closes_the_task_once_for_the_requester_only() {
    let w = world().await;
    let (_, request) = w.refund_sent().await;
    approvals::return_request(w.state(), request.id, &w.b, RETURN_COMMENT)
        .await
        .unwrap();
    let path = format!("/approvals/{}/not-resubmitting", request.id);

    w.app.acting_as(&w.b);
    w.app.htmx().post(&path, &[]).await.assert_forbidden();
    assert_eq!(
        approvals::needs_action(w.db(), &w.a).await.unwrap().len(),
        1
    );

    w.app.acting_as(&w.a);
    w.app.htmx().post(&path, &[]).await.assert_ok();
    let task = w.task_of(request.id).await;
    assert_eq!(task.status, approvals::TaskStatus::Closed);
    assert_eq!(
        task.closed_reason,
        Some(approvals::TaskClosed::NotResubmitting)
    );
    assert_eq!(task.closed_by, Some(w.a.id));
    assert!(
        approvals::needs_action(w.db(), &w.a)
            .await
            .unwrap()
            .is_empty()
    );

    let again = w.app.htmx().post(&path, &[]).await;
    assert_eq!(again.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(again.text().contains("This task is already closed."));
    assert_eq!(w.reload(&request).await.status, ApprovalStatus::Returned);
}

/// AC8 (card): the Return button is for an approver of the current step.
#[renox::test]
async fn ac8_the_card_shows_return_only_to_an_approver_of_the_step() {
    let w = world().await;
    let (refund, request) = w.refund_sent().await;
    let ret = format!("/approvals/{}/return", request.id);

    w.card_page(&refund, &w.b)
        .await
        .assert_ok()
        .assert_see(&ret)
        .assert_see("Return for changes");
    w.card_page(&refund, &w.a)
        .await
        .assert_ok()
        .assert_dont_see(&ret);
    w.card_page(&refund, &w.admin)
        .await
        .assert_ok()
        .assert_dont_see(&ret);

    // Once returned, the card shows its task, and the button to the
    // requester only.
    approvals::return_request(w.state(), request.id, &w.b, RETURN_COMMENT)
        .await
        .unwrap();
    let not_resubmitting = format!("/approvals/{}/not-resubmitting", request.id);
    w.card_page(&refund, &w.a)
        .await
        .assert_ok()
        .assert_see("Returned for changes: waiting")
        .assert_see(&not_resubmitting)
        .assert_dont_see(&ret);
    w.card_page(&refund, &w.b)
        .await
        .assert_ok()
        .assert_dont_see(&not_resubmitting)
        .assert_dont_see(&ret);
}

/// "Needs your action": a pending request is awaiting its current step's
/// approvers only (not the requester, not a user without the role), and
/// leaves the list once it is decided.
#[renox::test]
async fn awaiting_lists_the_pending_requests_a_user_may_decide() {
    let w = world().await;
    let (refund, request) = w.refund_sent().await;

    let ids = |items: Vec<ApprovalRequest>| items.into_iter().map(|r| r.id).collect::<Vec<_>>();
    assert_eq!(
        ids(approvals::awaiting(w.db(), &w.b).await.unwrap()),
        [request.id]
    );
    assert_eq!(
        ids(approvals::awaiting(w.db(), &w.c).await.unwrap()),
        [request.id]
    );
    assert!(
        approvals::awaiting(w.db(), &w.a).await.unwrap().is_empty(),
        "SoD: not the requester"
    );
    assert!(
        approvals::awaiting(w.db(), &w.admin)
            .await
            .unwrap()
            .is_empty(),
        "not a holder of the step's role"
    );

    approvals::register_subject_link(REFUND, |id| format!("/refunds/{id}"));
    let items = approvals::awaiting_items(w.db(), &w.b, &w.state().lang("en"))
        .await
        .unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].url.as_deref(),
        Some(format!("/refunds/{}", refund.id).as_str())
    );

    approvals::approve(w.state(), request.id, &w.b, "")
        .await
        .unwrap();
    assert!(approvals::awaiting(w.db(), &w.b).await.unwrap().is_empty());
}
