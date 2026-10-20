//! The shell every signed-in page shares: the sidebar labels of the host and
//! the brand (the client's name, not the framework's).

use renox::prelude::*;
use renox::testing::TestApp;

async fn admin(app: &TestApp) -> User {
    let mut user = User::register(app.db(), "Rana", "rana@example.com", "secret123")
        .await
        .unwrap();
    user.email_verified_at = Some(renox::db::now());
    user.save(app.db()).await.unwrap();
    host::roles::define(app.db()).await.unwrap();
    user.assign_role(app.db(), host::roles::ADMIN)
        .await
        .unwrap();
    user
}

/// The raw text keys (`nav.something`) left in a page's HTML.
fn raw_nav_keys(html: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find("nav.") {
        let tail = &rest[at..];
        let end = tail
            .find(|c: char| !(c.is_ascii_lowercase() || c == '_' || c == '.'))
            .unwrap_or(tail.len());
        // `nav.` inside a word (`.rx-sidebar__nav.x`) is not a key.
        let before = rest[..at].chars().last();
        if end > 4 && !before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-') {
            found.push(tail[..end].to_owned());
        }
        rest = &tail[4..];
    }
    found
}

/// Every link the modules add to the sidebar (the admin sees all of them)
/// has a text: no `nav.…` key is drawn instead of a label.
#[renox::test]
async fn every_sidebar_label_of_the_host_resolves_to_a_text() {
    let app = TestApp::new(host::app()).await;
    app.kernel().seed().await.unwrap();
    let admin = admin(&app).await;
    app.acting_as(&admin);
    let page = app.get("/dashboard").await;
    page.assert_ok();
    let html = page.text();
    assert_eq!(raw_nav_keys(&html), Vec::<String>::new());
    // The links that had no text before: the vertical's and the modules'.
    for label in [
        "Enquiries",
        "Supplier bills",
        "Expense claims",
        "Compliance flags",
        "Broadcasts",
        "Ticket categories",
    ] {
        assert!(html.contains(label), "the sidebar has no `{label}` link");
    }
}

/// Every link of the sidebar has an icon the kit knows: an unknown name
/// draws nothing and leaves the label out of line with the others.
#[renox::test]
async fn every_sidebar_link_has_an_icon() {
    let app = TestApp::new(host::app()).await;
    app.kernel().seed().await.unwrap();
    let admin = admin(&app).await;
    app.acting_as(&admin);
    let page = app.get("/dashboard").await;
    let html = page.text();
    let mut links = 0;
    for chunk in html.split("<a class=\"rx-sidebar__link\"").skip(1) {
        let link = &chunk[..chunk.find("</a>").expect("a closed link")];
        links += 1;
        assert!(link.contains("<svg"), "a sidebar link has no icon: {link}");
    }
    assert!(links > 30, "the admin sidebar lists {links} links");
}

/// The brand is the client's name (`[client] name` of client.toml), not the
/// framework's, in the sidebar and on the public pages.
#[renox::test]
async fn the_brand_is_the_clients_name() {
    let app = TestApp::new(host::app()).await;
    app.kernel().seed().await.unwrap();
    let admin = admin(&app).await;
    app.acting_as(&admin);
    let page = app.get("/dashboard").await;
    page.assert_ok();
    let html = page.text();
    assert!(html.contains("Palm Grove Properties LLC"), "no client name");
    assert!(!html.contains(">Renox<"), "the framework's name is shown");
    // The mark is the name's first letter.
    assert!(html.contains("rx-brand-mark"));
    let guest = TestApp::new(host::app()).await;
    guest
        .get("/")
        .await
        .assert_ok()
        .assert_see("Palm Grove Properties LLC");
    guest
        .get("/login")
        .await
        .assert_ok()
        .assert_see("Palm Grove Properties LLC");
}

/// One link of the sidebar is the current one, the most specific for the
/// page: the kit lit every link of a module (`property.*`) at once.
#[renox::test]
async fn one_sidebar_link_marks_the_current_page() {
    let app = TestApp::new(host::app()).await;
    app.kernel().seed().await.unwrap();
    let admin = admin(&app).await;
    app.acting_as(&admin);
    for (path, current) in [
        ("/property/leases", "/property/leases"),
        ("/property", "/property"),
        ("/parties", "/parties"),
        ("/parties/block-effects", "/parties/block-effects"),
        ("/dashboard", "/dashboard"),
    ] {
        let html = app.get(path).await.text();
        let marked: Vec<&str> = html
            .split("<a class=\"rx-sidebar__link\" href=\"")
            .skip(1)
            .filter(|chunk| {
                let tag = &chunk[..chunk.find('>').unwrap_or(chunk.len())];
                tag.contains("aria-current=\"page\"")
            })
            .map(|chunk| &chunk[..chunk.find('"').unwrap()])
            .collect();
        assert_eq!(marked, vec![current], "the current link of {path}");
    }
}
