//! Booting the auto app in a `TestApp` with the real `wl-scheduling` (the
//! workshop bookings whose check-in opens a job card). The contracts of the
//! other modules (parties, invoices, stock) are answered by the stand-ins in
//! this folder, which keep what they were asked in memory.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use renox::auth::{Auth, Permissions, permissions};
use renox::chrono::{NaiveDate, TimeDelta};
use renox::db::DateTime;
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::calendar::BusinessCalendar;
use wl_kernel::documents::dubai_date;
use wl_scheduling::WlScheduling;

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

/// Provides the probe route that gives a test the signed-in user.
struct Harness {
    who: Arc<Mutex<Option<renox::AuthUser>>>,
}

impl Module for Harness {
    fn name(&self) -> &'static str {
        "auto-test-harness"
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
    let mut scheduling_module = WlScheduling::new();
    for (code, initiated_by) in auto::reasons::SCHEDULING_REASON_CODES {
        scheduling_module = scheduling_module.reason_code(code, *initiated_by);
    }
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(renox::audit::Audit)
        .module(scheduling_module)
        .module(auto::Auto)
        .module(Harness { who: who.clone() });
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
}
