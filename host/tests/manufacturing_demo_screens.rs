//! The screen map of the demo company (`host::manufacturing_demo::screen_map`,
//! the map `manufacturing:demo` prints): every screen of F7 opens for the role
//! that owns it, says words and not keys, and holds on a phone (every table in
//! its own wrapper).

mod manufacturing_demo_support;
mod support;

use manufacturing_demo_support::demo;
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
        "manufacturing.",
        "holders of",
        "nav.",
        "finish_to_stock",
        "return_to_stock",
        "customer_specific",
        "in_progress",
        "on_hold",
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

/// An item or a lot shown as its id (`Item 6`, `Lot 10`) instead of its name
/// or number.
fn no_bare_ids(html: &str, url: &str) {
    let text = visible(html);
    for word in ["Item ", "Lot "] {
        for (at, _) in text.match_indices(word) {
            let after = text[at + word.len()..].chars().next();
            assert!(
                !after.is_some_and(|c| c.is_ascii_digit()),
                "{url} shows `{word}<id>`: ...{}...",
                text.get(at.saturating_sub(40)..(at + 60).min(text.len()))
                    .unwrap_or_default()
            );
        }
    }
}

/// A bare `None` (an empty value printed as a word) is never read on a page.
/// The "None" choice of a grid filter select is the grid's own label, not a
/// value.
fn no_none(html: &str, url: &str) {
    let html = html
        .replace("<option value=\"\">None</option>", "")
        .replace("<option value=\"\" selected>None</option>", "");
    let text = visible(&html);
    let bare = text
        .split(|c: char| !c.is_alphanumeric())
        .any(|word| word == "None");
    assert!(!bare, "{url} prints `None` for an empty value");
}

/// A time printed the ISO way (`2026-10-23 22:15`) instead of `23 Oct 2026
/// 22:15`.
fn no_iso_times(html: &str, url: &str) {
    // A date-time column of the standard grid prints `2026-10-23 22:15` (the
    // grid's own format, in the app's time zone): that cell is not a time
    // printed by a screen.
    let mut plain = String::new();
    for (index, part) in html.split("<span class=\"rx-grid__val\">").enumerate() {
        let cell = part
            .get(16..)
            .is_some_and(|rest| rest.starts_with("</span>"))
            && part.as_bytes()[4] == b'-'
            && part.as_bytes()[10] == b' ';
        if index > 0 {
            plain.push_str("<span class=\"rx-grid__val\">");
        }
        plain.push_str(if cell { &part[16..] } else { part });
    }
    let text: Vec<char> = visible(&plain).chars().collect();
    let digit = |at: usize, count: usize| {
        (at..at + count).all(|i| text.get(i).is_some_and(char::is_ascii_digit))
    };
    for at in 0..text.len() {
        let iso = digit(at, 4)
            && text.get(at + 4) == Some(&'-')
            && digit(at + 5, 2)
            && text.get(at + 7) == Some(&'-')
            && digit(at + 8, 2)
            && text.get(at + 10) == Some(&' ')
            && digit(at + 11, 2)
            && text.get(at + 13) == Some(&':');
        assert!(
            !iso,
            "{url} prints a time the ISO way: {}",
            text[at..(at + 16).min(text.len())]
                .iter()
                .collect::<String>()
        );
    }
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
    let screens = host::manufacturing_demo::screen_map(app.state())
        .await
        .unwrap();
    assert!(screens.len() >= 35, "{} screens", screens.len());
    for group in [
        "Start",
        "Work orders",
        "Bills of materials",
        "Quality",
        "Sales",
        "Stock and shipping",
        "Money",
        "People and documents",
        "Customer portal (Al Noor buyer)",
    ] {
        assert!(
            screens.iter().any(|screen| screen.group == group),
            "no screen in `{group}`"
        );
    }
    // A record page of each kind has its real id.
    for prefix in [
        "/manufacturing/work-orders/",
        "/manufacturing/ncrs/",
        "/boms/",
        "/sales-orders/",
        "/dispatches/",
        "/recalls/",
        "/manufacturing/recalls/",
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
        // The settings pages name the registered keys on purpose.
        if !screen.url.starts_with("/settings/") {
            no_raw_words(&html, &screen.url);
            no_bare_ids(&html, &screen.url);
            // The stock pages are wl-inventory's; the invoice grids are not.
            if [
                "/stock",
                "/dispatches",
                "/manufacturing/",
                "/boms",
                "/recalls",
            ]
            .iter()
            .any(|prefix| screen.url.starts_with(prefix))
            {
                no_none(&html, &screen.url);
                no_iso_times(&html, &screen.url);
            }
        }
        // The pages of the vertical keep every table in the kit's wrapper;
        // the shared grids (invoices, stock) are the platform's.
        if screen.url.contains("/manufacturing/") || screen.url.contains("/portal") {
            unique_ids(&html, &screen.url);
        }
        if screen.url.contains("/manufacturing/") {
            assert_eq!(
                html.matches("<table").count(),
                html.matches("rx-table-wrap").count()
                    + html.matches("<div class=\"rx-grid__scroll\"").count(),
                "{}: every table scrolls inside its own box (a grid has its own)",
                screen.url
            );
        }
        assert!(!screen.what.is_empty());
    }
}

/// The accountant follows the exports and attaches the exit evidence: the
/// "Attach evidence" link of the export evidence page opens for that login.
#[renox::test]
async fn the_accountant_opens_the_documents_of_an_export_dispatch() {
    let app = demo().await;
    let accountant = "accountant@emiratespack.example";
    let page = browse(&app, accountant, "/manufacturing/exports").await;
    assert_eq!(page.status.as_u16(), 200);
    let html = page.text();
    let link = html
        .split('"')
        .find(|part| part.starts_with("/documents/of/inventory.dispatch/"))
        .expect("an Attach evidence link")
        .to_owned();
    let response = browse(&app, accountant, &link).await;
    assert_eq!(response.status.as_u16(), 200, "{link} as the accountant");
}

/// The sidebar of the production manager: every link has its icon and no two
/// links share a label (Recalls was listed twice).
#[renox::test]
async fn the_sidebar_has_an_icon_and_a_single_label_per_link() {
    let app = demo().await;
    let page = browse(&app, host::manufacturing_demo::MANAGER_EMAIL, "/dashboard").await;
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
    assert!(labels.iter().any(|label| label == "Work orders"));
}
