//! Booting wl-sales in a `TestApp` with auth, as `crates/wl-ticketing/tests/support`
//! boots wl-ticketing.

#![allow(dead_code)]

pub mod changes;
pub mod credit;
pub mod floor;
pub mod online;
pub mod retention;
pub mod sell;
pub mod shop;

use std::sync::{Arc, Mutex};

use renox::auth::{Auth, Permissions, permissions};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::sales::{
    CustomerAcquired, CustomerAcquisitionChanged, LeadCreated, LeadLost, LeadWinReverted, LeadWon,
};
use wl_sales::WlSales;

/// The pages extend the app's layout; a package test has no app, so it
/// brings the smallest one.
const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

/// A client with the sales module (and the parties it needs) on.
const CLIENT: &str = r#"
[client]
name = "Palm Grove Properties LLC"
app = "property"

[modules]
enabled = ["parties", "sales"]

[[legal_entities]]
code = "PGP"
name_en = "Palm Grove Properties LLC"
default = true
"#;

/// The app under test.
pub struct Booted {
    /// The app.
    pub app: TestApp,
    /// Keeps the views directory alive.
    _views: tempfile::TempDir,
}

impl Booted {
    /// The database.
    pub fn db(&self) -> &Db {
        self.app.db()
    }

    /// The app state.
    pub fn state(&self) -> &AppState {
        self.app.state()
    }

    /// A user holding exactly `grants` (through a role of their own).
    pub async fn user(&self, name: &str, grants: &[&str]) -> User {
        let email = format!("{}@example.com", name.to_lowercase());
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

/// `WlSales` booted with auth and the defaults installed.
pub async fn boot() -> Booted {
    boot_with(Noop).await
}

/// A module that adds nothing.
struct Noop;

impl Module for Noop {
    fn name(&self) -> &'static str {
        "noop"
    }
}

/// A second module that adds nothing.
struct Spare;

impl Module for Spare {
    fn name(&self) -> &'static str {
        "spare"
    }
}

/// [`boot`] with one more module (a recorder of events or messages).
pub async fn boot_with(extra: impl Module) -> Booted {
    boot_configured(WlSales::new(), extra).await
}

/// [`boot_with`] with `sales` as the sales module, for a test that changes
/// the settings.
pub async fn boot_configured(sales: WlSales, extra: impl Module) -> Booted {
    boot_pair(sales, extra, Spare).await
}

/// [`boot_configured`] with a second extra module (the real wl-notify and the
/// fakes it needs).
pub async fn boot_pair(sales: WlSales, first: impl Module, second: impl Module) -> Booted {
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(renox::audit::Audit)
        .module(sales)
        .module(first)
        .module(second);
    let config = wl_kernel::ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    app.kernel()
        .call("sales:install", Vec::<String>::new())
        .await
        .expect("sales:install runs");
    Booted { app, _views: views }
}

/// The lead and acquisition events a test saw, recorded by a module that
/// listens to them.
#[derive(Clone, Default)]
pub struct Events {
    /// `LeadCreated`.
    pub created: Arc<Mutex<Vec<LeadCreated>>>,
    /// `LeadWon`.
    pub won: Arc<Mutex<Vec<LeadWon>>>,
    /// `LeadLost`.
    pub lost: Arc<Mutex<Vec<LeadLost>>>,
    /// `LeadWinReverted`.
    pub reverted: Arc<Mutex<Vec<LeadWinReverted>>>,
    /// `CustomerAcquired`.
    pub acquired: Arc<Mutex<Vec<CustomerAcquired>>>,
    /// `CustomerAcquisitionChanged`.
    pub changed: Arc<Mutex<Vec<CustomerAcquisitionChanged>>>,
}

/// Records `event`s of type `E` into `log`.
fn record<E: Event + Clone + Send + Sync + 'static>(app: &mut Registry, log: &Arc<Mutex<Vec<E>>>) {
    let log = log.clone();
    app.listen(move |event: E, _state: AppState| {
        let log = log.clone();
        async move {
            log.lock().unwrap().push(event);
            Ok(())
        }
    });
}

impl Module for Events {
    fn name(&self) -> &'static str {
        "events"
    }

    fn register(&self, app: &mut Registry) {
        record(app, &self.created);
        record(app, &self.won);
        record(app, &self.lost);
        record(app, &self.reverted);
        record(app, &self.acquired);
        record(app, &self.changed);
    }
}

/// `WlSales` with the recorded events.
pub async fn boot_events() -> (Booted, Events) {
    boot_events_with(WlSales::new()).await
}

/// [`boot_events`] with `sales` as the sales module.
pub async fn boot_events_with(sales: WlSales) -> (Booted, Events) {
    let events = Events::default();
    let t = boot_configured(sales, events.clone()).await;
    (t, events)
}

/// `WlSales` with the fakes of the other modules (notifier, invoicer, payer
/// bills, workforce) and the loyalty events recorded.
pub async fn boot_shop() -> (Booted, shop::Shop) {
    boot_shop_with(WlSales::new()).await
}

/// [`boot_shop`] with `sales` as the sales module.
pub async fn boot_shop_with(sales: WlSales) -> (Booted, shop::Shop) {
    let shop = shop::Shop::default();
    let t = boot_configured(sales, shop.clone()).await;
    (t, shop)
}
