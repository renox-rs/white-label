//! Shared by the host tests of the Manpower screens (BL-MPW-22 and the issues
//! that follow): the demo agency booted and seeded as `manpower:demo` does, a
//! person of the agency by e-mail, a GET the way a browser sends it, a form
//! post the way htmx sends it, and the words a page is checked for. A screen
//! test opens the page as a role of the demo agency, posts the form of one of
//! its sheets, and asserts the words on the page, not only the status.

#![allow(dead_code)]

use manpower::ClientContract;
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::testing::{TestApp, TestResponse};
use wl_invoicing::rate_card::Unit;
use wl_invoicing::rate_cards::{self, LineInput};
use wl_kernel::documents::today;
use wl_kernel::{ClientConfig, PartyId};

use crate::support::noon_clock;

/// The demo client of the Manpower vertical.
const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-manpower/client.toml"
);

/// The account manager of the demo agency.
pub const AMAL: &str = "accountmanager@desertstar.example";
/// The managing director.
pub const MD: &str = "md@desertstar.example";
/// The company admin.
pub const ADMIN: &str = "admin@desertstar.example";
/// The site supervisor.
pub const SUPERVISOR: &str = "supervisor@desertstar.example";
/// HR.
pub const HR: &str = "hr@desertstar.example";

/// The demo agency, seeded.
pub async fn demo() -> TestApp {
    let config = ClientConfig::load(CLIENT).expect("the demo client of the Manpower vertical");
    let app = TestApp::new(
        host::app_for(config)
            .expect("the app")
            .provide(noon_clock()),
    )
    .await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    host::manpower_demo::seed(app.kernel()).await.unwrap();
    app
}

/// The person with this e-mail.
pub async fn person(app: &TestApp, email: &str) -> User {
    User::find_by_email(app.db(), email)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("the demo login {email}"))
}

/// A GET as `email` the way a browser sends it: the page, not the JSON.
pub async fn browse(app: &TestApp, email: &str, path: &str) -> TestResponse {
    let user = person(app, email).await;
    app.acting_as(&user);
    app.request()
        .header("accept", "text/html,application/xhtml+xml,*/*;q=0.8")
        .get(path)
        .await
}

/// The text of the page `path` as `email`; the page must answer 200.
pub async fn page(app: &TestApp, email: &str, path: &str) -> String {
    let response = browse(app, email, path).await;
    assert_eq!(
        response.status.as_u16(),
        200,
        "{path} for {email}: {}",
        response.text()
    );
    let html = response.text();
    // `MPW_DUMP=/some/dir cargo test ...` keeps the pages for a look.
    if let Ok(dir) = std::env::var("MPW_DUMP") {
        let name = path.trim_matches('/').replace(['/', '?', '='], "_");
        let _ = std::fs::write(format!("{dir}/{name}.html"), &html);
    }
    html
}

/// A form post as `email` the way an action sheet sends it (htmx).
pub async fn post(app: &TestApp, email: &str, path: &str, form: &[(&str, &str)]) -> TestResponse {
    let user = person(app, email).await;
    app.acting_as(&user);
    app.htmx().post(path, form).await
}

/// A number from a query.
pub async fn scalar(app: &TestApp, query: &str) -> i64 {
    renox::db::sql(query.to_owned())
        .scalar::<i64>(app.db())
        .await
        .unwrap()
}

/// The id of the demo contract `reference` (`CC-DEMO-001`).
pub async fn contract(app: &TestApp, reference: &str) -> i64 {
    scalar(
        app,
        &format!("select id from manpower_client_contracts where reference = '{reference}'"),
    )
    .await
}

/// The contract `id` as it is stored.
pub async fn stored(app: &TestApp, id: i64) -> ClientContract {
    ClientContract::find_or_404(app.db(), id).await.unwrap()
}

/// The party id of the demo client called `name`.
pub async fn client(app: &TestApp, name: &str) -> PartyId {
    PartyId(
        scalar(
            app,
            &format!("select id from wl_parties_parties where legal_name_en = '{name}'"),
        )
        .await,
    )
}

/// A date as the pages write it: `01 Nov 2026`.
pub fn words(date: NaiveDate) -> String {
    date.format("%d %b %Y").to_string()
}

/// `n` days from today (negative: before).
pub fn day(n: i64) -> NaiveDate {
    if n >= 0 {
        today() + renox::chrono::Days::new(n as u64)
    } else {
        today() - renox::chrono::Days::new(n.unsigned_abs())
    }
}

/// The id a 303 sent the browser to: `/manpower/contracts/12` is 12.
pub fn landed(response: &TestResponse) -> i64 {
    response
        .header("location")
        .and_then(|location| location.rsplit('/').next())
        .and_then(|id| id.parse().ok())
        .unwrap_or_else(|| panic!("a redirect to a page: {:?}", response.header("location")))
}

/// A draft contract of Gulf Tower, made through the form of *New contract* by
/// the account manager, ending in a year.
pub async fn draft_contract(app: &TestApp, reference: &str) -> i64 {
    let tower = client(app, "Gulf Tower Facilities LLC").await;
    let start = day(0).to_string();
    let end = day(365).to_string();
    let created = post(
        app,
        AMAL,
        "/manpower/contracts",
        &[
            ("client_party_id", &tower.0.to_string()),
            ("reference", reference),
            ("start_date", &start),
            ("end_date", &end),
            ("notice_period_days", "30"),
        ],
    )
    .await;
    created.assert_status(303);
    landed(&created)
}

/// A rate card for contract `id` with a version effective from `from`, the
/// client's acceptance recorded by the admin (the signed acceptance).
pub async fn accepted_rate_card(app: &TestApp, id: i64, from: NaiveDate) {
    let contract = stored(app, id).await;
    let card = rate_cards::create_card(app.db(), None, contract.client(), &contract.subject())
        .await
        .unwrap();
    let version = rate_cards::new_version(app.db(), None, card.id, from, false)
        .await
        .unwrap();
    rate_cards::add_line(
        app.db(),
        version.id,
        LineInput::new("security_guard", "day", Unit::Hour, 2_800),
    )
    .await
    .unwrap();
    post(
        app,
        ADMIN,
        &format!("/rate-card-versions/{}/accept", version.id),
        &[("document", "signed-acceptance.pdf")],
    )
    .await
    .assert_status(303);
}

/// Asserts that `html` says every word of `wanted`.
pub fn says(html: &str, wanted: &[&str]) {
    for word in wanted {
        assert!(html.contains(word), "the page does not say `{word}`");
    }
}

/// The text a person reads on `html`: what is between the tags, without
/// scripts and styles.
pub fn visible(html: &str) -> String {
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

/// Asserts that the text of `html` says none of the raw words a screen must
/// not show.
pub fn no_raw_words(html: &str) {
    let html = visible(html);
    let html = html.as_str();
    for forbidden in [
        "security_guard",
        "manpower.release",
        "manpower.terminate",
        "hr.employee",
        "None",
        "null",
        "undefined",
        "on_hold",
    ] {
        if let Some(at) = html.find(forbidden) {
            let from = at.saturating_sub(80);
            let to = (at + 80).min(html.len());
            panic!(
                "the page shows `{forbidden}`: ...{}...",
                html.get(from..to).unwrap_or_default()
            );
        }
    }
}

/// The id of the employee called `name` in the demo.
pub async fn employee(app: &TestApp, name: &str) -> i64 {
    scalar(
        app,
        &format!(
            "select e.id from wl_hr_employees e join wl_parties_parties p on p.id = e.party_id \
             where p.legal_name_en = '{name}'"
        ),
    )
    .await
}

/// The first deployment of the worker called `name` in the demo.
pub async fn deployment_of(app: &TestApp, name: &str) -> i64 {
    let employee = employee(app, name).await;
    scalar(
        app,
        &format!("select min(id) from manpower_deployments where employee_id = {employee}"),
    )
    .await
}

/// Asserts the phone structure of `html`: every table scrolls inside its own
/// box (the kit's table wrapper), so nothing widens the page.
pub fn fits_a_phone(html: &str) {
    assert_eq!(
        html.matches("<table").count(),
        html.matches("rx-table-wrap").count(),
        "every table scrolls inside its own box"
    );
}
