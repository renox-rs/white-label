//! Booting wl-parties in a `TestApp`, as `crates/wl-kernel/tests/package.rs`
//! boots the kernel, with the events it emits recorded.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use renox::auth::{Permissions, permissions};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::{ClientConfig, PartyBlocked, PartyCreated, PartyUnblocked};
use wl_parties::{Party, WlParties, permissions as perm};

/// The pages extend the app's layout; a package test has no app, so it
/// brings the smallest one.
const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

/// A client with the parties module on.
pub const CLIENT: &str = r#"
[client]
name = "Palm Grove Properties LLC"
app = "property"

[modules]
enabled = ["parties"]

[[legal_entities]]
code = "PGP"
name_en = "Palm Grove Properties LLC"
default = true
"#;

/// The app under test with what it emitted.
pub struct Booted {
    /// The app.
    pub app: TestApp,
    /// Keeps the views directory alive.
    _views: tempfile::TempDir,
    created: Arc<Mutex<Vec<PartyCreated>>>,
    blocked: Arc<Mutex<Vec<PartyBlocked>>>,
    unblocked: Arc<Mutex<Vec<PartyUnblocked>>>,
}

impl Booted {
    /// Every `PartyCreated` so far.
    pub fn created(&self) -> Vec<PartyCreated> {
        self.created.lock().unwrap().clone()
    }

    /// Every `PartyBlocked` so far.
    pub fn blocked(&self) -> Vec<PartyBlocked> {
        self.blocked.lock().unwrap().clone()
    }

    /// Every `PartyUnblocked` so far.
    pub fn unblocked(&self) -> Vec<PartyUnblocked> {
        self.unblocked.lock().unwrap().clone()
    }

    /// The database.
    pub fn db(&self) -> &Db {
        self.app.db()
    }

    /// The party with this English legal name.
    pub async fn party(&self, legal_name_en: &str) -> Party {
        Party::query()
            .where_(Party::LEGAL_NAME_EN.eq(legal_name_en))
            .first(self.db())
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("no party `{legal_name_en}`"))
    }

    /// The party with this id, as stored now.
    pub async fn reload(&self, party: &Party) -> Party {
        Party::find(self.db(), party.id).await.unwrap().unwrap()
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

    /// User A: blocks parties, does not see restricted reasons.
    pub async fn user_a(&self) -> User {
        self.user("Amal", &[perm::VIEW, perm::BLOCK]).await
    }

    /// The MLRO (the compliance role): sees and uses restricted reasons.
    pub async fn mlro(&self) -> User {
        self.user(
            "Mariam",
            &[perm::VIEW, perm::BLOCK, perm::BLOCK_REASON_VIEW],
        )
        .await
    }

    /// User B: sees, blocks and exports parties, but not restricted reasons.
    pub async fn user_b(&self) -> User {
        self.user("Bilal", &[perm::VIEW, perm::BLOCK, perm::EXPORT])
            .await
    }
}

/// A user holding `role` in `scope` only (the role grants `parties.view`).
pub async fn staff_in(
    t: &Booted,
    name: &str,
    role: &str,
    scope: &renox::auth::permissions::Scope,
) -> User {
    let email = format!("{}@example.com", name.to_lowercase());
    let user = User::register(t.db(), name, &email, "password123")
        .await
        .unwrap();
    permissions::define_role(t.db(), role, &[perm::VIEW])
        .await
        .unwrap();
    user.assign_role_in(t.db(), role, scope).await.unwrap();
    user
}

/// `user` blocks `party` through the real route.
pub async fn block_party(t: &Booted, user: &User, party: &Party, code: &str, note: &str) {
    t.app.acting_as(user);
    t.app
        .htmx()
        .post(
            &format!("/parties/{}/block", party.id),
            &[("reason_code", code), ("note", note)],
        )
        .await
        .assert_ok();
}

/// wl-parties with the default block reasons, booted and seeded.
pub async fn boot() -> Booted {
    boot_with(WlParties::new(), true).await
}

/// wl-parties with the default block reasons and no party yet.
pub async fn boot_unseeded() -> Booted {
    boot_with(WlParties::new(), false).await
}

/// `module` booted in an app with auth and the kernel, with the demo
/// parties when `seed`.
pub async fn boot_with(module: WlParties, seed: bool) -> Booted {
    boot_custom(module, seed, |app| app).await
}

/// [`boot_with`], with `customize` adding to the app (a test-only listener)
/// before the kernel applies.
pub async fn boot_custom(
    module: WlParties,
    seed: bool,
    customize: impl FnOnce(App) -> App,
) -> Booted {
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();

    let created = Arc::new(Mutex::new(Vec::new()));
    let blocked = Arc::new(Mutex::new(Vec::new()));
    let unblocked = Arc::new(Mutex::new(Vec::new()));
    let (c, b, u) = (
        Arc::clone(&created),
        Arc::clone(&blocked),
        Arc::clone(&unblocked),
    );
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(module)
        .listen(move |event: PartyCreated, _state: AppState| {
            let c = Arc::clone(&c);
            async move {
                c.lock().unwrap().push(event);
                Ok(())
            }
        })
        .listen(move |event: PartyBlocked, _state: AppState| {
            let b = Arc::clone(&b);
            async move {
                b.lock().unwrap().push(event);
                Ok(())
            }
        })
        .listen(move |event: PartyUnblocked, _state: AppState| {
            let u = Arc::clone(&u);
            async move {
                u.lock().unwrap().push(event);
                Ok(())
            }
        });
    let app = customize(app);
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| {
        c.views_path = dir;
        // Above the module's own 20 MB limit on evidence files, so that
        // limit is the one a test meets.
        c.upload_max_size = 32 * 1024 * 1024;
    })
    .await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    if seed {
        wl_parties::seed::run(app.state())
            .await
            .expect("the demo parties seed");
    }
    created.lock().unwrap().clear();
    Booted {
        app,
        _views: views,
        created,
        blocked,
        unblocked,
    }
}
