//! Acceptance checks of #31 (BL-KRN-07): monitor site devices by heartbeat.
//!
//! The device connection `fnb.kds.marina` (a kitchen display of outlet
//! Marina, owner role *outlet manager*, heartbeat every minute, N = 5) and the
//! group `fnb.kds.jbr` (two displays) are registered by a test-only module.
//! Time is the real clock moved with `TestApp::travel`; "12:00" is the moment
//! of the first heartbeat. The active window is set through the settings page
//! relative to the clock, because a test cannot choose the time of day.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use renox::auth::{Permissions, permissions};
use renox::db::{DateTime, now};
use renox::prelude::*;
use renox::serde_json::Value;
use renox::testing::TestApp;
use wl_kernel::integrations::{self, devices, service};
use wl_kernel::notify::{
    MessageId, Notifier, NotifyFuture, NotifyRecipient, NotifyRequest, provide_notifier,
};
use wl_kernel::{
    ClientConfig, ConnectionStatus, DeviceBeat, IntegrationConnection, IntegrationDef,
    IntegrationStatusChanged,
};

const MARINA: &str = "fnb.kds.marina";
const JBR: &str = "fnb.kds.jbr";
const BOARD: &str = "fnb.kitchen_board";

const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

const CLIENT: &str = r#"
[client]
name = "Cloud Kitchens LLC"
app = "fnb"

[modules]
enabled = []

[[legal_entities]]
code = "CK"
name_en = "Cloud Kitchens LLC"
default = true
"#;

/// A recording notifier.
#[derive(Clone, Default)]
struct Notices(Arc<Mutex<Vec<NotifyRequest>>>);

impl Notices {
    fn users(&self, type_key: &str) -> Vec<i64> {
        let mut ids: Vec<i64> = self
            .0
            .lock()
            .unwrap()
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
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.type_key == type_key)
            .count()
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

struct Kitchen {
    notices: Notices,
}

/// `GET /banner/{screen}`: the banner of a screen.
async fn banner(
    State(db): State<Db>,
    lang: Lang,
    Path(screen): Path<String>,
) -> Result<Json<Value>> {
    let banner = integrations::web::banner_for(&db, &screen, &lang).await?;
    Ok(Json(banner.unwrap_or(Value::Null)))
}

impl Module for Kitchen {
    fn name(&self) -> &'static str {
        "test-kitchen"
    }

    fn routes(&self) -> Routes {
        Routes::new().get("/banner/{screen}", banner)
    }

    fn register(&self, app: &mut Registry) {
        provide_notifier(app, self.notices.clone());
        IntegrationDef::device(MARINA, "fnb")
            .scope("outlet:marina")
            .owner_role("outlet_manager")
            .screens([BOARD])
            .fallback("Run the kitchen from paper tickets.", None)
            .heartbeat_seconds(60)
            .down_after_minutes(5)
            .register();
        IntegrationDef::device(JBR, "fnb")
            .scope("outlet:jbr")
            .owner_role("outlet_manager")
            .fallback("Run the kitchen from paper tickets.", None)
            .down_after_minutes(5)
            .group(["kds-1", "kds-2"])
            .register();
    }
}

struct World {
    app: TestApp,
    notices: Notices,
    _views: tempfile::TempDir,
    admin: User,
    outlet_manager: User,
    manager: User,
    device: User,
    viewer: User,
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
            .module(Kitchen {
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
    permissions::define_role(db, "outlet_manager", &[])
        .await
        .unwrap();
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
        "kitchen_device",
        &[wl_kernel::permissions::INTEGRATIONS_HEARTBEAT],
    )
    .await
    .unwrap();
    permissions::define_role(db, "viewer", &[wl_kernel::permissions::INTEGRATIONS_VIEW])
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
        outlet_manager: user("Omar", "outlet_manager").await,
        manager: user("Mona", "integrations_manager").await,
        device: user("Kds", "kitchen_device").await,
        viewer: user("Vik", "viewer").await,
        notices,
        _views: dir,
        app,
    }
}

impl World {
    /// The clock, moved by `travel`.
    async fn clock(&self) -> DateTime {
        self.app.at_travelled_time(async { now() }).await
    }

    /// A heartbeat of `device` of connection `key` now.
    async fn beat(&self, key: &str, device: Option<&str>) -> devices::Beat {
        self.app
            .at_travelled_time(devices::heartbeat(self.app.state(), key, device))
            .await
            .expect("the heartbeat is recorded")
    }

    /// Moves the clock `seconds` forward and runs the scheduled task.
    async fn tick(&self, seconds: u64) {
        self.app.travel(Duration::from_secs(seconds));
        self.app
            .run_scheduled(service::TASK)
            .await
            .expect("the task runs");
    }

    async fn connection(&self, key: &str) -> IntegrationConnection {
        IntegrationConnection::by_key(self.app.db(), key)
            .await
            .unwrap()
            .expect("the connection is stored")
    }

    async fn status(&self, key: &str) -> ConnectionStatus {
        self.connection(key).await.status
    }

    async fn last_heartbeat(&self, key: &str, device: &str) -> DateTime {
        DeviceBeat::of_connection(self.app.db(), key)
            .await
            .unwrap()
            .into_iter()
            .find(|d| d.device_key == device)
            .and_then(|d| d.last_heartbeat_at)
            .expect("the device sent a heartbeat")
    }

    fn changes(&self, key: &str) -> Vec<(String, String)> {
        self.app
            .emitted::<IntegrationStatusChanged>()
            .into_iter()
            .filter(|e| e.connection == key)
            .map(|e| (e.from, e.to))
            .collect()
    }

    /// Sets the active window of `key` to `from`-`to` (minutes after
    /// midnight, Asia/Dubai) through the settings page.
    async fn set_window(&self, key: &str, from: i64, to: i64) {
        self.app.acting_as(&self.manager);
        let from = devices::clock_text(from);
        let to = devices::clock_text(to);
        let res = self
            .app
            .post(
                &format!("/integrations/{key}/settings"),
                &[
                    ("heartbeat_seconds", "60"),
                    ("window_from", from.as_str()),
                    ("window_to", to.as_str()),
                    ("down_after_minutes", "5"),
                ],
            )
            .await;
        assert_eq!(res.status.as_u16(), 303, "{}", res.text());
    }

    /// Minutes after midnight in Asia/Dubai `minutes` from now.
    async fn dubai_minute(&self, minutes: i64) -> i64 {
        let at = self.clock().await + renox::chrono::Duration::minutes(minutes);
        let local = at + renox::chrono::Duration::hours(4);
        use renox::chrono::Timelike;
        i64::from(local.hour()) * 60 + i64::from(local.minute())
    }
}

/// AC1: Given a kitchen display at outlet "Marina" with interval 1 minute and
/// N = 5, whose last heartbeat arrives at 12:00:00, then the connection is
/// `degraded` at 12:01:30 with `failed since` = 12:00:00, and `down` at
/// 12:05:00.
#[renox::test]
async fn a_missed_heartbeat_degrades_the_device_and_n_minutes_of_silence_take_it_down() {
    let w = world().await;
    let kind = w.connection(MARINA).await;
    assert_eq!(kind.kind, "device");
    assert_eq!(kind.scope.as_deref(), Some("outlet:marina"));
    w.beat(MARINA, None).await;
    let heard = w.last_heartbeat(MARINA, MARINA).await;

    // Within one interval and a half nothing happens.
    w.tick(60).await;
    assert_eq!(w.status(MARINA).await, ConnectionStatus::Up);
    assert!(w.changes(MARINA).is_empty());

    // 12:01:30: the heartbeat is missed.
    w.tick(30).await;
    let degraded = w.connection(MARINA).await;
    assert_eq!(degraded.status, ConnectionStatus::Degraded);
    assert_eq!(degraded.failed_since, Some(heard));
    assert!(degraded.last_error.unwrap_or_default().contains(MARINA));
    assert_eq!(
        w.changes(MARINA),
        [("up".to_owned(), "degraded".to_owned())]
    );

    // 12:05:00: N minutes of silence from the last heartbeat.
    w.tick(60 * 3).await;
    assert_eq!(w.status(MARINA).await, ConnectionStatus::Degraded);
    w.tick(30).await;
    assert_eq!(w.status(MARINA).await, ConnectionStatus::Down);
    assert_eq!(w.connection(MARINA).await.failed_since, Some(heard));
    let mut expected = vec![w.admin.id, w.outlet_manager.id];
    expected.sort_unstable();
    assert_eq!(w.notices.users("kernel.integration_down"), expected);
    assert_eq!(
        w.changes(MARINA),
        [
            ("up".to_owned(), "degraded".to_owned()),
            ("degraded".to_owned(), "down".to_owned())
        ]
    );
    // The kitchen board shows the banner of the manual fallback.
    w.app.acting_as(&w.viewer);
    let board = w.app.get(&format!("/banner/{BOARD}")).await;
    board.assert_ok();
    assert!(
        board.text().contains("Run the kitchen from paper tickets."),
        "{}",
        board.text()
    );
}

/// AC2: A heartbeat at 12:07 runs `recover`.
#[renox::test]
async fn a_heartbeat_after_the_silence_recovers_the_connection() {
    let w = world().await;
    w.beat(MARINA, None).await;
    w.tick(60 * 5 + 30).await;
    assert_eq!(w.status(MARINA).await, ConnectionStatus::Down);

    w.app.travel(Duration::from_secs(60 * 2));
    let outcome = w.beat(MARINA, None).await;

    assert_eq!(outcome, devices::Beat::Recovered);
    let up = w.connection(MARINA).await;
    assert_eq!(up.status, ConnectionStatus::Up);
    assert_eq!(up.failed_since, None);
    assert!(up.last_success_at.is_some());
    assert_eq!(
        w.changes(MARINA).last().cloned(),
        Some(("down".to_owned(), "up".to_owned()))
    );
    // The banner is gone.
    w.app.acting_as(&w.viewer);
    w.app
        .get(&format!("/banner/{BOARD}"))
        .await
        .assert_json(Value::Null);
}

/// AC3: Silence from 23:00 to 10:00 produces no transition; silence continuing
/// after 10:00 makes it `degraded` at 10:01:30 with `failed since` = 10:00:00
/// and `down` at 10:05:00. The window opens in about two hours here.
#[renox::test]
async fn silence_outside_the_window_is_ignored_and_counts_from_the_window_start() {
    let w = world().await;
    let start = w.dubai_minute(120).await;
    w.set_window(MARINA, start, (start + 600) % 1440).await;
    w.beat(MARINA, None).await;

    // Hours of silence before the window opens: no transition, no alert.
    for _ in 0..3 {
        w.tick(30 * 60).await;
    }
    let window = devices::window_of(&w.connection(MARINA).await);
    assert!(!devices::in_window(window, w.clock().await));
    assert_eq!(w.status(MARINA).await, ConnectionStatus::Up);
    assert!(w.changes(MARINA).is_empty());

    // The window opens: step minute by minute to the first moment inside it.
    let mut opened = None;
    for _ in 0..180 {
        w.app.travel(Duration::from_secs(60));
        let at = w.clock().await;
        if devices::in_window(window, at) {
            opened = devices::window_opened_at(window, at);
            break;
        }
    }
    let opened = opened.expect("the window opens");
    assert_eq!(w.status(MARINA).await, ConnectionStatus::Up);

    // Silence counts from the window start: 90 seconds later it is degraded.
    let missing = 91 - (w.clock().await - opened).num_seconds().max(0);
    w.tick(u64::try_from(missing.max(1)).unwrap()).await;
    let degraded = w.connection(MARINA).await;
    assert_eq!(degraded.status, ConnectionStatus::Degraded);
    assert_eq!(degraded.failed_since, Some(opened));

    // And five minutes after the window start it is down.
    let to_down = 5 * 60 - (w.clock().await - opened).num_seconds();
    w.tick(u64::try_from(to_down.max(1)).unwrap()).await;
    assert_eq!(w.status(MARINA).await, ConnectionStatus::Down);
}

/// Edge: a connection that is `down` when its window closes stays `down`,
/// sends no new alerts, and recovers on a heartbeat even outside the window.
#[renox::test]
async fn a_connection_down_at_closing_time_stays_down_without_new_alerts() {
    let w = world().await;
    let start = w.dubai_minute(-30).await;
    w.set_window(MARINA, start, (start + 60) % 1440).await;
    w.beat(MARINA, None).await;
    w.tick(60 * 5 + 30).await;
    assert_eq!(w.status(MARINA).await, ConnectionStatus::Down);
    assert_eq!(w.notices.count("kernel.integration_down"), 1);

    // Past the closing time, hours later.
    w.tick(3 * 3600).await;
    w.tick(3600).await;
    let window = devices::window_of(&w.connection(MARINA).await);
    assert!(!devices::in_window(window, w.clock().await));
    assert_eq!(w.status(MARINA).await, ConnectionStatus::Down);
    assert_eq!(w.notices.count("kernel.integration_down"), 1);

    // A heartbeat outside the window still recovers it.
    assert_eq!(w.beat(MARINA, None).await, devices::Beat::Recovered);
    assert_eq!(w.status(MARINA).await, ConnectionStatus::Up);
}

/// AC4: Given a group of 2 devices where one is silent, the group is not
/// `down`; it is `degraded`, and it goes `down` N minutes after the last device
/// went quiet.
#[renox::test]
async fn a_group_is_down_only_when_no_device_answers() {
    let w = world().await;
    w.beat(JBR, Some("kds-1")).await;
    w.beat(JBR, Some("kds-2")).await;

    // kds-1 goes on beating, kds-2 is silent.
    for _ in 0..8 {
        w.app.travel(Duration::from_secs(60));
        w.beat(JBR, Some("kds-1")).await;
        w.app
            .run_scheduled(service::TASK)
            .await
            .expect("the task runs");
    }
    let group = w.connection(JBR).await;
    assert_eq!(
        group.status,
        ConnectionStatus::Degraded,
        "one device answers"
    );
    assert_eq!(
        group.failed_since,
        Some(w.last_heartbeat(JBR, "kds-1").await),
        "N counts from the last answer"
    );
    assert_eq!(w.notices.count("kernel.integration_down"), 0);

    // Nobody answers for N minutes: the group is down.
    w.tick(60 * 5).await;
    assert_eq!(w.status(JBR).await, ConnectionStatus::Down);
    assert_eq!(w.notices.count("kernel.integration_down"), 1);

    // One device is back: the group recovers and degrades while the other is
    // still silent.
    w.beat(JBR, Some("kds-2")).await;
    assert_eq!(w.status(JBR).await, ConnectionStatus::Degraded);
    // Both back: up.
    w.beat(JBR, Some("kds-1")).await;
    assert_eq!(w.status(JBR).await, ConnectionStatus::Up);
}

/// Edge: a group with all devices answering stays up.
#[renox::test]
async fn a_group_whose_devices_all_answer_stays_up() {
    let w = world().await;
    for _ in 0..5 {
        w.beat(JBR, Some("kds-1")).await;
        w.beat(JBR, Some("kds-2")).await;
        w.tick(60).await;
    }
    assert_eq!(w.status(JBR).await, ConnectionStatus::Up);
    assert!(w.changes(JBR).is_empty());
}

/// The company admin sets the interval, the window, N and the grouping in the
/// connection settings; others cannot.
#[renox::test]
async fn the_admin_sets_the_heartbeat_interval_window_and_grouping() {
    let w = world().await;
    w.app.acting_as(&w.manager);
    let page = w.app.get(&format!("/integrations/{MARINA}")).await;
    page.assert_ok();
    assert!(page.text().contains("device-settings"), "{}", page.text());
    assert!(page.text().contains("outlet:marina"), "{}", page.text());

    let bad = w
        .app
        .htmx()
        .post(
            &format!("/integrations/{MARINA}/settings"),
            &[
                ("heartbeat_seconds", "5"),
                ("window_from", ""),
                ("window_to", ""),
                ("down_after_minutes", "5"),
            ],
        )
        .await;
    assert_eq!(bad.status.as_u16(), 422, "{}", bad.text());
    let half = w
        .app
        .htmx()
        .post(
            &format!("/integrations/{MARINA}/settings"),
            &[
                ("heartbeat_seconds", "60"),
                ("window_from", "10:00"),
                ("window_to", ""),
                ("down_after_minutes", "5"),
            ],
        )
        .await;
    assert_eq!(half.status.as_u16(), 422, "{}", half.text());
    let ok = w
        .app
        .post(
            &format!("/integrations/{MARINA}/settings"),
            &[
                ("heartbeat_seconds", "120"),
                ("window_from", "10:00"),
                ("window_to", "23:00"),
                ("down_after_minutes", "8"),
                ("grouped", "on"),
            ],
        )
        .await;
    assert_eq!(ok.status.as_u16(), 303, "{}", ok.text());
    let saved = w.connection(MARINA).await;
    assert_eq!(saved.heartbeat_seconds, 120);
    assert_eq!(
        (saved.window_start, saved.window_end),
        (Some(600), Some(1380))
    );
    assert_eq!(saved.down_after_minutes, 8);
    assert!(saved.grouped);

    w.app.acting_as(&w.viewer);
    let denied = w
        .app
        .post(
            &format!("/integrations/{MARINA}/settings"),
            &[("heartbeat_seconds", "60"), ("down_after_minutes", "5")],
        )
        .await;
    assert_eq!(denied.status.as_u16(), 403, "{}", denied.text());
}

/// A longer interval moves the point a heartbeat counts as missed (one and a
/// half intervals).
#[renox::test]
async fn the_interval_decides_when_a_heartbeat_is_missed() {
    let w = world().await;
    w.app.acting_as(&w.manager);
    w.app
        .post(
            &format!("/integrations/{MARINA}/settings"),
            &[("heartbeat_seconds", "120"), ("down_after_minutes", "5")],
        )
        .await
        .assert_status(303);
    w.beat(MARINA, None).await;
    w.tick(120).await;
    assert_eq!(
        w.status(MARINA).await,
        ConnectionStatus::Up,
        "2 minutes < 3"
    );
    w.tick(61).await;
    assert_eq!(w.status(MARINA).await, ConnectionStatus::Degraded);
}

/// A device sends its heartbeat itself; it needs `integrations.heartbeat`,
/// and only a device connection takes one.
#[renox::test]
async fn a_device_sends_its_heartbeat_with_its_own_permission() {
    let w = world().await;
    w.beat(MARINA, None).await;
    w.tick(60 * 6).await;
    assert_eq!(w.status(MARINA).await, ConnectionStatus::Down);

    w.app.acting_as(&w.viewer);
    let denied = w
        .app
        .post(&format!("/devices/{MARINA}/heartbeat"), &[("x", "")])
        .await;
    assert_eq!(denied.status.as_u16(), 403, "{}", denied.text());

    w.app.acting_as(&w.device);
    let ok = w
        .app
        .post(&format!("/devices/{MARINA}/heartbeat"), &[("x", "")])
        .await;
    ok.assert_ok();
    ok.assert_json_path("status", "up");
    ok.assert_json_path("recovered", true);
    assert_eq!(w.status(MARINA).await, ConnectionStatus::Up);

    // A connection the platform calls is not a device.
    let refused = w
        .app
        .post("/devices/unknown.service/heartbeat", &[("x", "")])
        .await;
    assert!(refused.status.is_client_error(), "{}", refused.text());
}

/// Edge: a disabled device connection is left alone: no degrade, no recover.
#[renox::test]
async fn a_disabled_device_connection_ignores_silence_and_heartbeats() {
    let w = world().await;
    w.app.acting_as(&w.manager);
    w.app
        .post(
            &format!("/integrations/{MARINA}/disable"),
            &[("reason", "outlet closed for refit")],
        )
        .await
        .assert_status(303);
    w.tick(60 * 10).await;
    assert_eq!(w.status(MARINA).await, ConnectionStatus::Disabled);
    assert_eq!(w.beat(MARINA, None).await, devices::Beat::Ignored);
    assert_eq!(w.status(MARINA).await, ConnectionStatus::Disabled);
    assert_eq!(w.notices.count("kernel.integration_down"), 0);
}
