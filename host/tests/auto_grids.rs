//! The lists of the Auto vertical on the demo workshop (#557): each is a
//! `renox::grid` that is a direct child of the page (it fills the screen),
//! opens for the role that owns it, searches and pages through the query
//! string, and exports only for the roles that manage the list; the
//! technician's own list never exports.

mod grid_support;

use grid_support::{grid_is_a_child_of_the_page, load, range};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::ClientConfig;

/// The demo client of the Auto vertical.
const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-auto/client.toml"
);

/// The demo workshop, seeded.
async fn demo() -> TestApp {
    let config = ClientConfig::load(CLIENT).expect("the demo client of the Auto vertical");
    let app = TestApp::new(host::app_for(config).expect("the app")).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    host::auto_demo::seed(app.kernel()).await.unwrap();
    app
}

/// Signs in the demo login `email`.
async fn sign_in(app: &TestApp, email: &str) {
    let user = User::find_by_email(app.db(), email)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("the demo login {email}"));
    app.acting_as(&user);
}

/// Each list with its template and the demo login that works from it.
const LISTS: [(&str, &str, &str); 9] = [
    (
        "/auto/job-cards",
        "job_cards.html",
        "waleed@alnoorauto.example",
    ),
    (
        "/auto/vehicles",
        "vehicles.html",
        "hussein@alnoorauto.example",
    ),
    ("/auto/claims", "claims.html", "carla@alnoorauto.example"),
    (
        "/auto/fleet",
        "fleet_contracts.html",
        "fatima@alnoorauto.example",
    ),
    (
        "/auto/supplier-claims",
        "supplier_claims.html",
        "pia@alnoorauto.example",
    ),
    (
        "/auto/export-sales",
        "export_sales.html",
        "aisha@alnoorauto.example",
    ),
    (
        "/auto/technician",
        "technician.html",
        "tareq@alnoorauto.example",
    ),
    (
        "/auto/insurers",
        "insurers.html",
        "carla@alnoorauto.example",
    ),
    (
        "/auto/labour-menu",
        "labour_menu.html",
        "sami@alnoorauto.example",
    ),
];

/// Every list of the issue is a grid that is a direct child of its page.
#[test]
fn every_auto_list_is_a_grid_that_fills_the_page() {
    for (_, template, _) in LISTS {
        grid_is_a_child_of_the_page(&format!("auto/views/auto/{template}"));
    }
}

/// Each list answers 200 for its owner, draws the grid, pages one row at a
/// time and finds nothing for a nonsense search.
#[renox::test]
async fn every_auto_list_opens_as_a_grid_for_its_owner() {
    let app = demo().await;
    for (path, _, login) in LISTS {
        sign_in(&app, login).await;
        let html = load(&app, path).await;
        assert!(html.contains("class=\"rx-grid"), "{path} draws a grid");
        let (_, _, rows) = range(&html);
        if rows > 0 {
            let one = load(&app, &format!("{path}?per_page=1")).await;
            assert_eq!(range(&one), (1, 1, rows), "{path} pages one row");
        }
        let none = load(&app, &format!("{path}?search=zzqqxx-nothing")).await;
        assert_eq!(range(&none).2, 0, "{path} finds nothing for nonsense");
    }
}

/// The insurance coordinator exports the claims; a technician exports
/// neither the workshop's job cards nor their own list (`AUT.ACL.08`).
#[renox::test]
async fn the_export_is_for_the_roles_that_manage_the_list() {
    let app = demo().await;
    sign_in(&app, "carla@alnoorauto.example").await;
    let file = app
        .request()
        .header("accept", "text/html")
        .get("/auto/claims?export=csv")
        .await;
    assert_eq!(file.status.as_u16(), 200);
    assert!(
        file.header("content-type")
            .is_some_and(|kind| kind.contains("text/csv")),
        "a CSV file"
    );

    sign_in(&app, "tareq@alnoorauto.example").await;
    for path in ["/auto/technician?export=csv", "/auto/job-cards?export=csv"] {
        let page = app.request().header("accept", "text/html").get(path).await;
        assert_eq!(page.status.as_u16(), 403, "{path} as a technician");
    }
}
