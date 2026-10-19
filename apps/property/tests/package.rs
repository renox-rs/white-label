use std::path::Path;

use property::Property;
use property::permissions;
use renox::auth::permissions as grants;
use renox::auth::{Auth, Permissions};
use renox::prelude::*;
use renox::testing::TestApp;

/// The pages extend the app's layout; a package test has no app, so it
/// brings the smallest one.
const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

/// The kernel's client file: the pages include its approval card.
const CLIENT: &str = r#"
[client]
name = "Palm Grove Properties LLC"
app = "property"

[modules]
enabled = []

[[legal_entities]]
code = "PGP"
name_en = "Palm Grove Properties LLC"
default = true
"#;

/// A views directory holding the layout and `files` (the app's own views).
fn views(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temporary views directory");
    for (name, text) in [("layouts/app.html", LAYOUT)].iter().chain(files) {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().expect("a file path")).expect("a directory");
        std::fs::write(path, text).expect("a view file");
    }
    dir
}

/// The module mounted in an app with sign-in and roles, that uses `views`.
async fn boot(views: &Path) -> TestApp {
    let config = wl_kernel::ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(
        App::new()
            .module(Auth::new())
            .module(Permissions)
            .module(Property),
        config,
    )
    .expect("the kernel applies");
    TestApp::with_config(app, |config| {
        config.views_path = views.to_path_buf();
    })
    .await
}

/// A user who may see the portfolio.
async fn viewer(app: &TestApp) -> User {
    grants::define_role(app.db(), "viewer", &[permissions::UNITS_VIEW])
        .await
        .unwrap();
    let user = User::register(app.db(), "Vera", "vera@example.com", "password123")
        .await
        .unwrap();
    user.assign_role(app.db(), "viewer").await.unwrap();
    user
}

#[renox::test]
async fn the_portfolio_answers_a_user_who_may_see_it() {
    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let user = viewer(&app).await;
    app.acting_as(&user);
    app.get("/property")
        .await
        .assert_ok()
        .assert_view("property/index.html")
        .assert_see("No buildings yet");
}

#[renox::test]
async fn a_user_without_the_permission_is_refused() {
    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let user = User::register(app.db(), "Nora", "nora@example.com", "password123")
        .await
        .unwrap();
    app.acting_as(&user);
    app.get("/property").await.assert_forbidden();
}

#[renox::test]
async fn the_models_match_the_schema() {
    let dir = views(&[]);
    boot(dir.path()).await.assert_models_match_schema().await;
}

#[renox::test]
async fn every_view_compiles() {
    let dir = views(&[]);
    boot(dir.path()).await.assert_views_compile();
}

#[renox::test]
async fn an_app_file_of_the_same_name_wins() {
    let dir = views(&[(
        "property/index.html",
        "<rx-page layout=\"layouts/app.html\" title=\"Mine\">The app's own page.</rx-page>",
    )]);
    let app = boot(dir.path()).await;
    let user = viewer(&app).await;
    app.acting_as(&user);
    app.get("/property")
        .await
        .assert_ok()
        .assert_see("The app's own page.")
        .assert_dont_see("No buildings yet");
}
