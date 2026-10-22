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
use wl_scheduling::{Resource, ResourceType, WlScheduling};

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

/// Provides the probe route that gives a test the signed-in user.
struct Harness {
    who: Arc<Mutex<Option<renox::AuthUser>>>,
}

impl Module for Harness {
    fn name(&self) -> &'static str {
        "edtech-test-harness"
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
        .module(Harness { who: who.clone() });
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
    pub async fn coordinator(&self) -> renox::AuthUser {
        let user = self
            .user(
                "Amal",
                &[
                    edtech::permissions::COURSES_VIEW,
                    edtech::permissions::COURSES_MANAGE,
                    edtech::permissions::BATCHES_VIEW,
                    edtech::permissions::BATCHES_MANAGE,
                    edtech::permissions::SESSIONS_VIEW,
                ],
            )
            .await;
        self.auth(&user).await
    }

    /// A tutor: teaches the sessions assigned.
    pub async fn tutor_user(&self) -> renox::AuthUser {
        let user = self
            .user(
                "Tariq",
                &[
                    edtech::permissions::BATCHES_VIEW,
                    edtech::permissions::SESSIONS_VIEW,
                    edtech::permissions::SESSIONS_TEACH,
                ],
            )
            .await;
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
