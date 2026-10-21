//! The dashboard of the Construction client and of the clients whose vertical
//! app has no figures of its own: the demo company's MD sees every tile of
//! the business, each linking to its page; a role with fewer permissions sees
//! fewer; and a client that is neither Property nor Construction opens its
//! dashboard without reading the Property tables.

mod support;

use renox::prelude::*;
use renox::testing::TestApp;
use support::{accounting_booted, person};
use wl_kernel::ClientConfig;

/// The demo client of the Construction vertical.
const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-construction/client.toml"
);

/// The Construction demo, seeded as `construction:demo` does.
async fn demo() -> TestApp {
    let config = ClientConfig::load(CLIENT).expect("the demo client of the Construction vertical");
    let app = TestApp::new(host::app_for(config).expect("the app")).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    host::construction_demo::seed(app.kernel()).await.unwrap();
    app
}

/// The text of the figure under `label` on the page, and its link.
fn tile(html: &str, label: &str) -> Option<(String, String)> {
    let at = html.find(&format!("<p class=\"rx-stat__label\">{label}</p>"))?;
    let before = &html[..at];
    let link = before.rfind("href=\"")?;
    let href = before[link + 6..].split('"').next()?.to_owned();
    let after = &html[at..];
    let value = after.split("rx-stat__value\">").nth(1)?;
    let value = value.split("</p>").next()?.trim().to_owned();
    Some((value, href))
}

/// How many tiles the page draws.
fn tiles(html: &str) -> usize {
    html.matches("rx-stat__label").count()
}

/// The number under `label`.
fn count(html: &str, label: &str) -> u32 {
    tile(html, label)
        .unwrap_or_else(|| panic!("no tile {label}"))
        .0
        .parse()
        .unwrap_or_else(|_| panic!("{label} is not a number"))
}

async fn login(app: &TestApp, email: &str) -> User {
    let user = User::find_by_email(app.db(), email)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("the demo login {email}"));
    app.acting_as(&user);
    user
}

#[renox::test]
async fn the_md_sees_the_tiles_of_the_business_each_linking_to_its_page() {
    let app = demo().await;
    login(&app, "admin@alwaha.example").await;
    let page = app.get("/dashboard").await;
    page.assert_ok();
    let html = page.text();

    // The projects and what waits on them.
    assert_eq!(count(&html, "Projects in progress"), 4);
    assert_eq!(
        tile(&html, "Projects in progress").unwrap().1,
        "/construction/projects"
    );
    assert!(count(&html, "Payment applications waiting over 14 days") >= 2);
    assert!(count(&html, "Guarantees ending in 60 days") >= 1);
    let _ = count(&html, "Variations unanswered over 14 days");
    assert_eq!(count(&html, "Open absence cases"), 1);
    assert!(count(&html, "Permits ending in 30 days") >= 1);
    assert_eq!(
        tile(&html, "Open absence cases").unwrap().1,
        "/hr/employees"
    );
    let _ = count(&html, "Orders awaiting approval");
    assert_eq!(
        tile(&html, "Cost codes at 90% of budget").unwrap().1,
        "/budget-report"
    );
    // The money tiles of the shared modules.
    let (overdue, link) = tile(&html, "Overdue invoices").expect("overdue invoices");
    assert!(overdue.starts_with("AED"), "{overdue}");
    assert_eq!(link, "/invoices?in.status=overdue");
    assert!(count(&html, "Bounced cheques") >= 1);
    // Nothing of Property, and the welcome says the right thing.
    assert!(!html.contains("Units available"), "no Property tile");
    assert!(!html.contains("Leases ending"), "no Property tile");
    assert!(html.contains("Where your projects stand today"), "{html}");

    // The pages the tiles lead to open.
    for url in [
        "/construction/projects",
        "/hr/employees",
        "/budget-report",
        "/invoices",
        "/cheques",
    ] {
        app.get(url).await.assert_ok();
    }
}

#[renox::test]
async fn a_role_with_fewer_permissions_sees_fewer_tiles() {
    let app = demo().await;
    login(&app, "admin@alwaha.example").await;
    let md = tiles(&app.get("/dashboard").await.text());

    // The project manager sees the projects but not the money or the people.
    login(&app, "project_manager@alwaha.example").await;
    let page = app.get("/dashboard").await;
    page.assert_ok();
    let html = page.text();
    assert!(tile(&html, "Projects in progress").is_some());
    assert!(tile(&html, "Payment applications waiting over 14 days").is_some());
    assert!(tile(&html, "Overdue invoices").is_none());
    assert!(tile(&html, "Bounced cheques").is_none());
    assert!(tile(&html, "Open absence cases").is_none());
    assert!(tiles(&html) < md, "{} against the MD's {md}", tiles(&html));

    // HR sees the people, not the projects.
    login(&app, "hr@alwaha.example").await;
    let html = app.get("/dashboard").await.text();
    assert!(tile(&html, "Open absence cases").is_some());
    assert!(tile(&html, "Permits ending in 30 days").is_some());
    assert!(tile(&html, "Projects in progress").is_none());

    // The site supervisor's page opens with no tile, but with the welcome
    // and the activity.
    login(&app, "supervisor@alwaha.example").await;
    let page = app.get("/dashboard").await;
    page.assert_ok();
    let html = page.text();
    assert_eq!(tiles(&html), 0, "{html}");
    assert!(html.contains("Welcome back"));
    assert!(html.contains("Your recent activity"));
}

#[renox::test]
async fn a_client_of_another_vertical_opens_a_generic_dashboard() {
    let app = accounting_booted().await;
    let admin = person(&app, "Rana", host::roles::ADMIN).await;
    app.acting_as(&admin);
    let page = app.get("/dashboard").await;
    page.assert_ok();
    let html = page.text();
    assert!(html.contains("What waits for you today."), "{html}");
    assert!(html.contains("Your recent activity"));
    assert!(!html.contains("Units available"), "no Property tile");
    assert!(!html.contains("Buildings flagged"), "no Property tile");
}
