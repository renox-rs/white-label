//! The screen map of the demo company (`host::auto_demo::screen_map`, the map
//! `auto:demo` prints): every screen of F8 opens for the role that owns it,
//! says words and not keys, and holds on a phone (every table of the vertical
//! in its own wrapper).

mod auto_demo_support;
mod support;

use auto_demo_support::demo;
use renox::prelude::*;
use renox::testing::{TestApp, TestResponse};

/// A GET as `email` the way a browser sends it.
async fn browse(app: &TestApp, email: &str, path: &str) -> TestResponse {
    let user = User::find_by_email(app.db(), email)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("the demo login {email}"));
    app.acting_as(&user);
    app.request()
        .header("accept", "text/html,application/xhtml+xml,*/*;q=0.8")
        .get(path)
        .await
}

/// The text a person reads on `html`: what is between the tags, without
/// scripts and styles.
fn visible(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        rest = &rest[open..];
        let skip = ["script", "style"]
            .iter()
            .find(|tag| rest[1..].starts_with(**tag))
            .copied();
        match skip {
            Some(tag) => {
                let close = format!("</{tag}>");
                rest = rest.find(&close).map_or("", |at| &rest[at + close.len()..]);
            }
            None => rest = rest.find('>').map_or("", |at| &rest[at + 1..]),
        }
        out.push(' ');
    }
    out.push_str(rest);
    out
}

/// The raw words a screen must not show.
fn no_raw_words(html: &str, url: &str) {
    // The company's e-mail domain is not a text key.
    let text = visible(html).replace("alnoorauto.", "");
    for forbidden in [
        "auto.",
        "dashboard.",
        "holders of",
        "nav.",
        "checked_in",
        "awaiting_approval",
        "awaiting_parts",
        "quality_check",
        "in_progress",
        "estimate_submitted",
        "non_agency",
        "periodic_service",
        "Some(",
        "null",
        "undefined",
    ] {
        if let Some(at) = text.find(forbidden) {
            let from = at.saturating_sub(80);
            let to = (at + 200).min(text.len());
            panic!(
                "{url} shows `{forbidden}`: ...{}...",
                text.get(from..to).unwrap_or_default()
            );
        }
    }
}

/// A bare `None` (an empty value printed as a word) is never read on a page;
/// the insurer's "None accepted" (no rate card) is a sentence.
fn no_none(html: &str, url: &str) {
    let text = visible(html).replace("None accepted", "");
    let bare = text
        .split(|c: char| !c.is_alphanumeric())
        .any(|word| word == "None");
    assert!(!bare, "{url} prints `None` for an empty value");
}

/// No two elements of a page share an `id` (a sheet per row needs its own).
fn unique_ids(html: &str, url: &str) {
    let mut seen = std::collections::BTreeSet::new();
    for part in html.split(" id=\"").skip(1) {
        let id = part.split('"').next().unwrap_or_default();
        assert!(seen.insert(id.to_owned()), "{url} uses the id `{id}` twice");
    }
}

/// Every screen of the map answers 200 for its owner, shows visible words
/// only, and keeps its tables in their wrappers (390 px).
#[renox::test]
async fn every_screen_of_the_map_opens_for_its_owner_in_words() {
    let app = demo().await;
    let screens = host::auto_demo::screen_map(app.state()).await.unwrap();
    assert!(screens.len() >= 45, "{} screens", screens.len());
    for group in [
        "Start",
        "Workshop",
        "Vehicles",
        "Insurance",
        "Fleet",
        "Parts and sales",
        "Money",
        "People",
    ] {
        assert!(
            screens.iter().any(|screen| screen.group == group),
            "no screen in `{group}`"
        );
    }
    // A record page of each kind has its real id.
    for prefix in [
        "/auto/job-cards/",
        "/auto/vehicles/",
        "/auto/claims/",
        "/auto/insurers/",
        "/auto/fleet/",
        "/auto/recalls/",
        "/counter-sales/",
        "/recalls/",
    ] {
        assert!(
            screens.iter().any(|screen| screen.url.starts_with(prefix)),
            "no screen at {prefix}<id>"
        );
    }
    for screen in &screens {
        let response = browse(&app, screen.login, &screen.url).await;
        assert_eq!(
            response.status.as_u16(),
            200,
            "{} as {} ({})",
            screen.url,
            screen.login,
            screen.what
        );
        let html = response.text();
        no_raw_words(&html, &screen.url);
        if screen.url.starts_with("/auto/") {
            no_none(&html, &screen.url);
            unique_ids(&html, &screen.url);
            assert_eq!(
                html.matches("<table").count(),
                html.matches("rx-table-wrap").count(),
                "{}: every table scrolls inside its own box",
                screen.url
            );
        }
        assert!(!screen.what.is_empty());
    }
}

/// The delivered job card names its invoice by number, not by row id; the
/// fleet plan names its service, not its code.
#[renox::test]
async fn the_job_card_names_its_invoice_and_the_plan_its_service() {
    let app = demo().await;
    let screens = host::auto_demo::screen_map(app.state()).await.unwrap();
    let delivered = screens
        .iter()
        .find(|s| s.url.starts_with("/auto/job-cards/") && s.what.starts_with("Delivered"))
        .expect("the delivered job card");
    let html = browse(&app, delivered.login, &delivered.url).await.text();
    assert!(visible(&html).contains("INV-"), "{}", visible(&html));
    let fleet = screens
        .iter()
        .find(|s| s.url.starts_with("/auto/fleet/") && s.what.starts_with("Active"))
        .expect("the active contract");
    let html = browse(&app, fleet.login, &fleet.url).await.text();
    assert!(visible(&html).contains("Periodic service"));
}

/// The sidebar of the owner: every link has its icon and no two links share
/// a label.
#[renox::test]
async fn the_sidebar_has_an_icon_and_a_single_label_per_link() {
    let app = demo().await;
    let page = browse(&app, host::auto_demo::OWNER_EMAIL, "/dashboard").await;
    assert_eq!(page.status.as_u16(), 200);
    let html = page.text();
    let start = html.find("<nav").expect("a sidebar");
    let nav = &html[start
        ..html[start..]
            .find("</nav>")
            .map_or(html.len(), |end| start + end)];
    let mut labels = Vec::new();
    for link in nav.split("<a ").skip(1) {
        let link = link.split("</a>").next().unwrap_or_default();
        if link.contains("rx-sidebar__brand") {
            continue;
        }
        assert!(
            link.contains("<svg"),
            "a sidebar link without an icon: {link}"
        );
        let label = visible(link.split_once('>').map_or("", |(_, rest)| rest))
            .trim()
            .to_owned();
        assert!(!labels.contains(&label), "`{label}` is listed twice");
        labels.push(label);
    }
    assert!(labels.iter().any(|label| label == "Job cards"));
}
