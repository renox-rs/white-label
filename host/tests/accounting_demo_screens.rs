//! The screen map of the demo firm (`host::accounting_demo::screen_map`, the
//! map `accounting:demo` prints): every screen of F4 opens for the role that
//! owns it, says words and not keys, and holds on a phone (every table of the
//! vertical in its own wrapper).

mod accounting_demo_support;
mod support;

use accounting_demo_support::demo;
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
    let text = visible(html);
    for forbidden in [
        "accounting.",
        "dashboard.",
        "holders of",
        "nav.",
        "client_approval",
        "in_review",
        "pending_approval",
        "review_due",
        "under_review",
        "closed_no_action",
        "voluntary_disclosure",
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

/// A bare `None` (an empty value printed as a word) is never read on a page.
fn no_none(html: &str, url: &str) {
    let text = visible(html);
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
    let screens = host::accounting_demo::screen_map(app.state())
        .await
        .unwrap();
    assert!(screens.len() >= 45, "{} screens", screens.len());
    for group in [
        "Start",
        "Engagements",
        "CDD and AML",
        "Returns",
        "KPI packs",
        "Money",
        "People and documents",
        "Client portal (Marina Bay owner)",
    ] {
        assert!(
            screens.iter().any(|screen| screen.group == group),
            "no screen in `{group}`"
        );
    }
    // A record page of each kind has its real id.
    for prefix in [
        "/accounting/engagements/",
        "/accounting/cdd/",
        "/accounting/aml-cases/",
        "/accounting/returns/",
        "/accounting/kpi-packs/",
        "/accounting/clients/",
        "/accounting/portal/returns/",
        "/accounting/portal/kpi-packs/",
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
        if screen.url.starts_with("/accounting/") || screen.url.starts_with("/portal/") {
            no_none(&html, &screen.url);
            unique_ids(&html, &screen.url);
        }
        if screen.url.starts_with("/accounting/") {
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

/// The client owner finds the return waiting for them on the dashboard, and
/// the finance contact (no approver) does not.
#[renox::test]
async fn the_client_owner_finds_the_return_to_approve_on_the_dashboard() {
    let app = demo().await;
    let owner = browse(
        &app,
        host::accounting_demo::CLIENT_OWNER_EMAIL,
        "/dashboard",
    )
    .await;
    assert_eq!(owner.status.as_u16(), 200);
    let html = owner.text();
    assert!(html.contains("Your accounting firm"), "{}", visible(&html));
    assert!(html.contains("/accounting/portal/returns/"));
    assert!(html.contains("/accounting/portal/kpi-packs/"));
    let finance = browse(
        &app,
        host::accounting_demo::CLIENT_FINANCE_EMAIL,
        "/dashboard",
    )
    .await;
    let html = finance.text();
    assert!(html.contains("Your accounting firm"));
    assert!(!html.contains("/accounting/portal/returns/"));
    // A member of staff has no client block.
    let partner = browse(&app, host::accounting_demo::PARTNER_EMAIL, "/dashboard").await;
    assert!(!partner.text().contains("Your accounting firm"));
}

/// The sidebar of the MLRO: every link has its icon (AML cases had none) and
/// no two links share a label.
#[renox::test]
async fn the_sidebar_has_an_icon_and_a_single_label_per_link() {
    let app = demo().await;
    let page = browse(&app, host::accounting_demo::MLRO_EMAIL, "/dashboard").await;
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
    assert!(labels.iter().any(|label| label == "AML cases"));
}
