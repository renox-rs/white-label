//! Acceptance checks of #29: integration health and the manual fallback.
//!
//! The connection `asp.acme` is registered by a test-only invoicing module
//! (owner role *accountant*, screens *invoice list* and *invoice detail*,
//! N = 15). Its send handler fails while `test_asp.mode` is `down`. Time is
//! the real clock moved with `TestApp::travel`; "10:00" is the moment of the
//! first failed call.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use renox::auth::{Permissions, permissions};
use renox::chrono::Duration as Span;
use renox::db::{DateTime, Migration, now, sql};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::history;
use renox::testing::TestApp;
use wl_kernel::integrations::{self, Submitted, call_failed, service, web};
use wl_kernel::machines::BoxFuture;
use wl_kernel::notify::{
    MessageId, Notifier, NotifyFuture, NotifyRecipient, NotifyRequest, provide_notifier,
};
use wl_kernel::{
    ClientConfig, ConnectionStatus, Delivery, IntegrationConnection, IntegrationDef,
    IntegrationStatusChanged, Outbound, QueuedCall, RuleRun,
};

const KEY: &str = "asp.acme";
const LIST: &str = "invoicing.invoice_list";
const DETAIL: &str = "invoicing.invoice_detail";

/// The pages extend the app's layout; a test has no app, so it brings the
/// smallest one.
const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

const UP: &str = r#"
CREATE TABLE "test_asp" (mode TEXT NOT NULL);
INSERT INTO "test_asp" (mode) VALUES ('down');
CREATE TABLE "test_sent" (id INTEGER PRIMARY KEY AUTOINCREMENT, n INTEGER NOT NULL);
"#;

const DOWN: &str = r#"
DROP TABLE "test_sent";
DROP TABLE "test_asp";
"#;

const MIGRATIONS: &[Migration] = &[Migration::new(
    "20261019100199_create_test_asp",
    UP,
    Some(DOWN),
)];

/// The module's send handler: fails while the ASP is down, else records the
/// call.
fn send<'a>(state: &'a AppState, call: &'a Outbound) -> BoxFuture<'a, Result<Delivery>> {
    Box::pin(async move {
        let mode = sql("SELECT mode FROM test_asp LIMIT 1")
            .scalar_optional::<String>(&state.db)
            .await?;
        if mode.as_deref() == Some("down") {
            return Err(call_failed("503 service unavailable"));
        }
        sql("INSERT INTO test_sent (n) VALUES (?)")
            .bind(call.payload["n"].as_i64().unwrap_or_default())
            .execute(&state.db)
            .await?;
        Ok(Delivery::Sent)
    })
}

/// A recording notifier.
#[derive(Clone, Default)]
struct Notices(Arc<Mutex<Vec<NotifyRequest>>>);

impl Notices {
    fn all(&self) -> Vec<NotifyRequest> {
        self.0.lock().unwrap().clone()
    }

    /// The user ids the `type_key` messages went to, sorted.
    fn users(&self, type_key: &str) -> Vec<i64> {
        let mut ids: Vec<i64> = self
            .all()
            .iter()
            .filter(|r| r.type_key == type_key)
            .flat_map(|r| r.recipients.clone())
            .filter_map(|r| match r {
                NotifyRecipient::User { id } => Some(id),
                _ => None,
            })
            .collect();
        ids.sort_unstable();
        ids
    }

    fn count(&self, type_key: &str) -> usize {
        self.all().iter().filter(|r| r.type_key == type_key).count()
    }
}

impl Notifier for Notices {
    fn notify<'a>(
        &'a self,
        _state: &'a AppState,
        request: NotifyRequest,
    ) -> NotifyFuture<'a, Vec<MessageId>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(request);
            Ok(Vec::new())
        })
    }
}

struct Invoicing {
    notices: Notices,
}

/// `GET /banner/{screen}`: the banner of a screen.
async fn banner(
    State(db): State<Db>,
    lang: Lang,
    Path(screen): Path<String>,
) -> Result<Json<Value>> {
    let banner = web::banner_for(&db, &screen, &lang).await?;
    Ok(Json(banner.unwrap_or(Value::Null)))
}

impl Module for Invoicing {
    fn name(&self) -> &'static str {
        "test-invoicing"
    }

    fn routes(&self) -> Routes {
        Routes::new().get("/banner/{screen}", banner)
    }

    fn migrations(&self) -> &'static [Migration] {
        MIGRATIONS
    }

    fn register(&self, app: &mut Registry) {
        provide_notifier(app, self.notices.clone());
        IntegrationDef::of(KEY, "invoicing", send)
            .owner_role("accountant")
            .screens([LIST, DETAIL])
            .fallback("File the invoice on the ASP portal by hand.", Some("قدّم الفاتورة يدويًا."))
            .register();
    }
}

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

struct World {
    app: TestApp,
    notices: Notices,
    /// Keeps the views directory alive.
    _views: tempfile::TempDir,
    admin: User,
    accountant: User,
    manager: User,
    viewer: User,
    nobody: User,
    /// The moment of the first failed call.
    t0: DateTime,
}

async fn world() -> World {
    let dir = tempfile::tempdir().expect("a temporary views directory");
    let layout = dir.path().join("layouts/app.html");
    std::fs::create_dir_all(layout.parent().expect("a path")).expect("a directory");
    std::fs::write(layout, LAYOUT).expect("a layout");

    let notices = Notices::default();
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(
        App::new()
            .module(Auth::new())
            .module(Permissions)
            .module(Invoicing {
                notices: notices.clone(),
            }),
        config,
    )
    .expect("the kernel applies");
    let views = dir.path().to_path_buf();
    let app = TestApp::with_config(app, move |c| c.views_path = views.clone()).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    app.fake_events();

    let db = app.db();
    permissions::define_role(db, "admin", &[]).await.unwrap();
    permissions::define_role(db, "accountant", &[]).await.unwrap();
    permissions::define_role(
        db,
        "integrations_manager",
        &[
            wl_kernel::permissions::INTEGRATIONS_VIEW,
            wl_kernel::permissions::INTEGRATIONS_MANAGE,
        ],
    )
    .await
    .unwrap();
    permissions::define_role(
        db,
        "integrations_viewer",
        &[wl_kernel::permissions::INTEGRATIONS_VIEW],
    )
    .await
    .unwrap();
    permissions::define_role(db, "clerk", &[wl_kernel::permissions::APPROVALS_VIEW])
        .await
        .unwrap();
    let user = |name: &'static str, role: &'static str| async move {
        let email = format!("{}@example.com", name.to_lowercase());
        let user = User::register(db, name, &email, "password123")
            .await
            .unwrap();
        user.assign_role(db, role).await.unwrap();
        user
    };
    World {
        admin: user("Ada", "admin").await,
        accountant: user("Amir", "accountant").await,
        manager: user("Mona", "integrations_manager").await,
        viewer: user("Vik", "integrations_viewer").await,
        nobody: user("Nora", "clerk").await,
        t0: now(),
        notices,
        _views: dir,
        app,
    }
}

impl World {
    fn call(&self, n: i64) -> Outbound {
        Outbound::new(KEY, "invoice", json!({ "n": n }))
    }

    /// Submits call `n` at the travelled time.
    async fn submit(&self, n: i64) -> Submitted {
        self.app
            .at_travelled_time(service::submit(self.app.state(), self.call(n)))
            .await
            .expect("the call is handed over")
    }

    /// Moves the clock `minutes` forward and runs the scheduled task.
    async fn tick(&self, minutes: u64) {
        self.app.travel(Duration::from_secs(minutes * 60));
        self.app
            .run_scheduled(service::TASK)
            .await
            .expect("the task runs");
    }

    async fn set_mode(&self, mode: &str) {
        sql("UPDATE test_asp SET mode = ?")
            .bind(mode)
            .execute(self.app.db())
            .await
            .unwrap();
    }

    async fn connection(&self) -> IntegrationConnection {
        IntegrationConnection::by_key(self.app.db(), KEY)
            .await
            .unwrap()
            .expect("the connection is stored")
    }

    async fn waiting(&self) -> Vec<QueuedCall> {
        QueuedCall::waiting(self.app.db(), KEY).await.unwrap()
    }

    async fn sent(&self) -> Vec<i64> {
        sql("SELECT n FROM test_sent ORDER BY id")
            .scalars::<i64>(self.app.db())
            .await
            .unwrap()
    }

    /// Whole minutes from the first failure to `at`.
    fn minutes(&self, at: Option<DateTime>) -> i64 {
        (at.expect("a time") - self.t0).num_seconds().div_euclid(60)
            + i64::from((at.expect("a time") - self.t0).num_seconds().rem_euclid(60) >= 30)
    }

    async fn banner(&self, screen: &str) -> renox::testing::TestResponse {
        self.app.acting_as(&self.nobody);
        self.app.get(&format!("/banner/{screen}")).await
    }

    fn changes(&self) -> Vec<(String, String)> {
        self.app
            .emitted::<IntegrationStatusChanged>()
            .into_iter()
            .map(|e| (e.from, e.to))
            .collect()
    }
}

/// AC1: a call that fails at 10:00 degrades the connection, `failed since` is
/// 10:00 and the call is queued and retried at 10:01, 10:03, 10:07 and 10:15.
#[renox::test]
async fn a_failed_call_degrades_and_is_retried_with_the_backoff() {
    let w = world().await;
    let queued = w.submit(1).await;
    assert!(matches!(queued, Submitted::Queued { .. }));

    let degraded = w.connection().await;
    assert_eq!(degraded.status, ConnectionStatus::Degraded);
    let since = degraded.failed_since.expect("failed since");
    assert!(since - w.t0 < Span::seconds(30));
    assert_eq!(degraded.last_error.as_deref(), Some("503 service unavailable"));
    assert_eq!(w.changes(), [("up".to_owned(), "degraded".to_owned())]);

    let mut next = Vec::new();
    for (step, expected) in [(1, 3), (2, 7), (4, 15), (8, 30), (15, 45)] {
        // Before the wait is over nothing is tried.
        let before = w.waiting().await[0].attempts;
        w.tick(step - 1).await;
        if step > 1 {
            assert_eq!(w.waiting().await[0].attempts, before, "tried too early");
        }
        w.tick(1).await;
        let head = &w.waiting().await[0];
        assert_eq!(head.attempts, before + 1);
        next.push((w.minutes(head.next_attempt_at), expected));
    }
    for (found, expected) in next {
        assert_eq!(found, expected);
    }
    assert!(w.sent().await.is_empty());
}

/// AC2: with no success until 10:15, `go_down` runs: the screens show the
/// banner, the admin and the accountant are alerted once, and the event is
/// emitted.
#[renox::test]
async fn after_n_minutes_the_connection_goes_down_and_the_owner_is_alerted_once() {
    let w = world().await;
    w.submit(1).await;
    w.tick(14).await;
    assert_eq!(w.connection().await.status, ConnectionStatus::Degraded);
    assert!(w.notices.all().is_empty());
    w.banner(LIST).await.assert_json(Value::Null);

    w.tick(1).await;
    let down = w.connection().await;
    assert_eq!(down.status, ConnectionStatus::Down);
    assert!(down.failed_since.is_some());

    let mut expected = vec![w.admin.id, w.accountant.id];
    expected.sort_unstable();
    assert_eq!(w.notices.users("kernel.integration_down"), expected);
    assert_eq!(
        w.changes(),
        [
            ("up".to_owned(), "degraded".to_owned()),
            ("degraded".to_owned(), "down".to_owned())
        ]
    );

    // The invoice list and the invoice detail show the banner; other screens
    // do not.
    for screen in [LIST, DETAIL] {
        let res = w.banner(screen).await;
        res.assert_ok();
        res.assert_json_path("0.connection", KEY);
        assert!(
            res.text().contains("File the invoice on the ASP portal by hand."),
            "{}",
            res.text()
        );
    }
    w.banner("invoicing.payment_list")
        .await
        .assert_json(Value::Null);

    // One rule run, and no second alert at the next checks.
    w.tick(1).await;
    w.tick(1).await;
    assert_eq!(w.notices.count("kernel.integration_down"), 1);
    let runs = RuleRun::query()
        .where_(RuleRun::RULE.eq("PLT.RUL.05"))
        .count(w.app.db())
        .await
        .unwrap();
    assert_eq!(runs, 1);
}

/// AC3: after `go_down` the queued call is still retried every 15 minutes
/// and never dropped.
#[renox::test]
async fn retries_go_on_every_15_minutes_while_down() {
    let w = world().await;
    w.submit(1).await;
    w.tick(15).await;
    assert_eq!(w.connection().await.status, ConnectionStatus::Down);

    for _ in 0..2 {
        let before = w.waiting().await[0].attempts;
        w.tick(15).await;
        let waiting = w.waiting().await;
        assert_eq!(waiting.len(), 1, "never dropped");
        assert_eq!(waiting[0].attempts, before + 1);
    }
    assert_eq!(w.connection().await.status, ConnectionStatus::Down);
}

/// AC4: with N = 5 for the connection, the same happens at 10:05.
#[renox::test]
async fn n_is_set_per_connection() {
    let w = world().await;
    IntegrationConnection::where_eq("key", KEY)
        .update(w.app.db(), &[("down_after_minutes", &5_i64)])
        .await
        .unwrap();
    w.submit(1).await;
    w.tick(4).await;
    assert_eq!(w.connection().await.status, ConnectionStatus::Degraded);
    w.tick(1).await;
    assert_eq!(w.connection().await.status, ConnectionStatus::Down);
    assert_eq!(w.notices.count("kernel.integration_down"), 1);
}

/// An idle connection with no calls never goes down.
#[renox::test]
async fn an_idle_connection_never_goes_down() {
    let w = world().await;
    w.tick(24 * 60).await;
    assert_eq!(w.connection().await.status, ConnectionStatus::Up);
    assert!(w.notices.all().is_empty());
}

/// AC5: `disable` without a reason is refused; with a reason nothing is sent
/// and the queue keeps its 12 items.
#[renox::test]
async fn disable_needs_a_reason_and_keeps_the_queue() {
    let w = world().await;
    for n in 1..=12 {
        w.submit(n).await;
    }
    assert_eq!(w.waiting().await.len(), 12);

    w.app.acting_as(&w.manager);
    let blank = w
        .app
        .htmx()
        .post("/integrations/asp.acme/disable", &[("reason", "  ")])
        .await;
    assert_eq!(blank.status.as_u16(), 422, "{}", blank.text());
    blank.assert_invalid("reason");
    assert_eq!(w.connection().await.status, ConnectionStatus::Degraded);

    // Without the permission it is refused.
    w.app.acting_as(&w.viewer);
    let denied = w
        .app
        .post("/integrations/asp.acme/disable", &[("reason", "contract ended")])
        .await;
    assert_eq!(denied.status.as_u16(), 403, "{}", denied.text());

    w.app.acting_as(&w.manager);
    let ok = w
        .app
        .post("/integrations/asp.acme/disable", &[("reason", "contract ended")])
        .await;
    assert_eq!(ok.status.as_u16(), 303, "{}", ok.text());
    let disabled = w.connection().await;
    assert_eq!(disabled.status, ConnectionStatus::Disabled);
    assert_eq!(disabled.disabled_reason.as_deref(), Some("contract ended"));

    // Nothing is sent while it is disabled, even when the ASP is back.
    w.set_mode("ok").await;
    w.tick(30).await;
    assert!(w.sent().await.is_empty());
    assert_eq!(w.waiting().await.len(), 12);
    // A new call is queued, not made.
    assert!(matches!(w.submit(13).await, Submitted::Queued { .. }));
    assert!(w.sent().await.is_empty());
    assert_eq!(w.waiting().await.len(), 13);

    let rows = history(w.app.db(), IntegrationConnection::TABLE, disabled.id)
        .await
        .unwrap();
    assert_eq!(rows.last().map(|r| r.transition.as_str()), Some("disable"));
    assert_eq!(rows.last().and_then(|r| r.reason.as_deref()), Some("contract ended"));
}

/// AC6: a user without `integrations.view` cannot open the connection page.
#[renox::test]
async fn only_holders_of_integrations_view_open_the_page() {
    let w = world().await;
    w.app.acting_as(&w.nobody);
    let denied = w.app.get("/integrations/asp.acme").await;
    assert_eq!(denied.status.as_u16(), 403, "{}", denied.text());
    let denied = w.app.get("/integrations").await;
    assert_eq!(denied.status.as_u16(), 403, "{}", denied.text());

    w.app.acting_as(&w.viewer);
    let page = w.app.get("/integrations/asp.acme").await;
    page.assert_ok();
    assert!(page.text().contains(KEY), "{}", page.text());
    let list = w.app.get("/integrations").await;
    list.assert_ok();
}

/// A call to an `up` connection is made at once and nothing is queued.
#[renox::test]
async fn a_call_to_a_healthy_connection_is_sent_at_once() {
    let w = world().await;
    w.set_mode("ok").await;
    assert_eq!(w.submit(1).await, Submitted::Sent);
    assert_eq!(w.sent().await, [1]);
    assert!(w.waiting().await.is_empty());
    let up = w.connection().await;
    assert_eq!(up.status, ConnectionStatus::Up);
    assert!(up.last_success_at.is_some());
}

/// A call that arrives while the connection is degraded waits behind the
/// older ones: calls go out in the order they were queued.
#[renox::test]
async fn calls_are_never_sent_out_of_order() {
    let w = world().await;
    for n in 1..=3 {
        assert!(matches!(w.submit(n).await, Submitted::Queued { .. }));
    }
    w.set_mode("ok").await;
    // A new call does not overtake the three that wait.
    assert!(matches!(w.submit(4).await, Submitted::Queued { .. }));
    assert!(w.sent().await.is_empty());

    w.tick(1).await;
    assert_eq!(w.sent().await, [1, 2, 3, 4]);
    assert!(w.waiting().await.is_empty());
}

/// The first success recovers a degraded or down connection: the banner goes
/// and `IntegrationStatusChanged { from: down, to: up }` is emitted.
#[renox::test]
async fn a_success_takes_the_connection_up_and_removes_the_banner() {
    let w = world().await;
    w.submit(1).await;
    w.tick(15).await;
    assert_eq!(w.connection().await.status, ConnectionStatus::Down);
    w.banner(LIST).await.assert_json_path("0.connection", KEY);

    w.set_mode("ok").await;
    let recovered = w
        .app
        .at_travelled_time(service::report_success(w.app.state(), KEY))
        .await
        .unwrap();
    assert!(recovered);
    let up = w.connection().await;
    assert_eq!(up.status, ConnectionStatus::Up);
    assert!(up.failed_since.is_none() && up.last_error.is_none());
    assert_eq!(
        w.changes().last(),
        Some(&("down".to_owned(), "up".to_owned()))
    );
    w.banner(LIST).await.assert_json(Value::Null);
}

/// Several failed calls record one `degrade`; the later ones only note their
/// error (of two workers, the second finds the connection degraded).
#[renox::test]
async fn concurrent_failures_degrade_once() {
    let w = world().await;
    let state = w.app.state();
    service::report_failure(state, KEY, "timeout").await.unwrap();
    service::report_failure(state, KEY, "connection reset")
        .await
        .unwrap();

    let degraded = w.connection().await;
    assert_eq!(degraded.status, ConnectionStatus::Degraded);
    let rows = history(w.app.db(), IntegrationConnection::TABLE, degraded.id)
        .await
        .unwrap();
    assert_eq!(rows.iter().filter(|r| r.transition == "degrade").count(), 1);
    assert_eq!(
        w.app.emitted::<IntegrationStatusChanged>().len(),
        1,
        "one status change"
    );
}

#[renox::test]
async fn the_kernel_templates_compile() {
    let w = world().await;
    w.app.assert_views_compile();
    let _ = integrations::all();
}
