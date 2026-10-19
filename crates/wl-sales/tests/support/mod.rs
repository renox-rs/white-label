//! Booting wl-sales in a `TestApp` with auth, as `crates/wl-ticketing/tests/support`
//! boots wl-ticketing.

#![allow(dead_code)]

use renox::auth::{Auth, Permissions, permissions};
use renox::prelude::*;
use renox::testing::TestApp;
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

/// [`boot`] with one more module (a recorder of events or messages).
pub async fn boot_with(extra: impl Module) -> Booted {
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(renox::audit::Audit)
        .module(WlSales::new())
        .module(extra);
    let config = wl_kernel::ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    app.kernel()
        .call("sales:install", Vec::<String>::new())
        .await
        .expect("sales:install runs");
    Booted { app, _views: views }
}
