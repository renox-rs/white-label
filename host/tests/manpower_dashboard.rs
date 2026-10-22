//! BL-MPW-32 (#512): the dashboard of the demo manpower agency. Each role sees
//! the counts it can act on, each tile linking to its list and left out
//! without the permission; the numbers are the demo seed's; a tile's link
//! opens the list filtered to what it counts; the client approver sees none
//! of them.

mod manpower_screens_support;
mod support;

use manpower_screens_support::*;
use renox::prelude::*;
use renox::testing::TestApp;

/// The figure under `label` on the page and its link.
fn tile(html: &str, label: &str) -> Option<(String, String)> {
    let at = html.find(&format!("<p class=\"rx-stat__label\">{label}</p>"))?;
    let before = &html[..at];
    let link = before.rfind("href=\"")?;
    let href = before[link + 6..].split('"').next()?.to_owned();
    let value = html[at..].split("rx-stat__value\">").nth(1)?;
    let value = value.split("</p>").next()?.trim().to_owned();
    Some((value, href.replace("&amp;", "&")))
}

/// The number under `label`, 0 when the tile is left out.
fn count(html: &str, label: &str) -> u64 {
    tile(html, label).map_or(0, |(value, _)| value.parse().expect("a number"))
}

/// The labels of the manpower tiles, singular and plural.
const LABELS: [(&str, &str); 8] = [
    ("Contract on hold", "Contracts on hold"),
    (
        "Contract ending within 60 days",
        "Contracts ending within 60 days",
    ),
    ("Unfilled request", "Unfilled requests"),
    (
        "Document expiring within 30 days",
        "Documents expiring within 30 days",
    ),
    ("Deployment suspended", "Deployments suspended"),
    ("Open absence case", "Open absence cases"),
    (
        "Timesheet waiting for the client",
        "Timesheets waiting for the client",
    ),
    (
        "Payroll run waiting for approval",
        "Payroll runs waiting for approval",
    ),
];

/// The count under either wording of `pair`.
fn counted(html: &str, pair: (&str, &str)) -> u64 {
    count(html, pair.0).max(count(html, pair.1))
}

/// The link under either wording of `pair`.
fn linked(html: &str, pair: (&str, &str)) -> Option<String> {
    tile(html, pair.0)
        .or_else(|| tile(html, pair.1))
        .map(|t| t.1)
}

/// How many of the eight manpower tiles the page draws.
fn drawn(html: &str) -> usize {
    LABELS
        .iter()
        .filter(|pair| linked(html, **pair).is_some())
        .count()
}

async fn rows(app: &TestApp, query: &str) -> u64 {
    scalar(app, query).await as u64
}

/// AC1: the admin sees every tile with a count, with the demo's numbers, and
/// each link opens.
#[renox::test]
async fn the_admin_sees_every_tile_with_a_count_each_linking_to_its_list() {
    let app = demo().await;
    let html = page(&app, ADMIN, "/dashboard").await;
    no_raw_words(&html);

    let expected = [
        rows(&app, "select count(*) from manpower_client_contracts where status = 'on_hold'").await,
        rows(
            &app,
            &format!(
                "select count(*) from manpower_client_contracts where status in ('active','on_hold') and end_date between '{}' and '{}'",
                day(0),
                day(60)
            ),
        )
        .await,
        rows(&app, "select count(*) from manpower_requests where status in ('open','partially_filled')").await,
        rows(
            &app,
            &format!(
                "select count(*) from wl_documents_documents where subject_type = 'hr.employee' and status = 'accepted' and expiry_date between '{}' and '{}'",
                day(0),
                day(30)
            ),
        )
        .await,
        rows(&app, "select count(*) from manpower_deployments where status = 'suspended'").await,
        rows(&app, "select count(*) from manpower_absence_cases where closed_on is null").await,
        rows(
            &app,
            "select count(*) from wl_attendance_timesheets where status = 'submitted' and escalated_at is null",
        )
        .await,
        rows(&app, "select count(*) from wl_hr_payroll_runs where status = 'calculated'").await,
    ];
    for (pair, expected) in LABELS.iter().zip(expected) {
        assert_eq!(counted(&html, *pair), expected, "{}", pair.1);
        assert_eq!(
            linked(&html, *pair).is_some(),
            expected > 0,
            "{}: a tile with a count, none without",
            pair.1
        );
    }
    // The demo has something to say on contracts, requests, suspension and sheets.
    assert_eq!(counted(&html, LABELS[0]), 1);
    assert_eq!(counted(&html, LABELS[1]), 1);
    assert!(counted(&html, LABELS[2]) >= 1);
    assert!(counted(&html, LABELS[4]) >= 1);
    assert!(counted(&html, LABELS[6]) >= 1);
    assert!(
        !html.contains("Nothing needs your action</"),
        "tiles are shown"
    );

    // Every link opens.
    for pair in LABELS {
        if let Some(url) = linked(&html, pair) {
            let opened = browse(&app, ADMIN, &url).await;
            assert_eq!(opened.status.as_u16(), 200, "{url}");
        }
    }
}

/// AC4: the demo contract on hold reads *1 contract on hold* and its link
/// opens the list filtered to it.
#[renox::test]
async fn the_contract_on_hold_reads_one_and_its_link_lists_only_it() {
    let app = demo().await;
    let html = page(&app, ADMIN, "/dashboard").await;
    let (value, url) = tile(&html, "Contract on hold").expect("the singular label");
    assert_eq!(value, "1");
    assert_eq!(url, "/manpower/contracts?status=on_hold");
    assert!(tile(&html, "Contracts on hold").is_none());

    let list = page(&app, ADMIN, &url).await;
    let held = contract(&app, "CC-DEMO-003").await;
    assert!(
        list.contains(&format!("/manpower/contracts/{held}")),
        "the held contract"
    );
    assert!(
        !list.contains("CC-DEMO-001"),
        "the active one is not listed"
    );
    let all = page(&app, ADMIN, "/manpower/contracts").await;
    assert!(all.contains("CC-DEMO-001"));

    // The 60 days link lists the contract ending in 45.
    let ending = page(&app, ADMIN, "/manpower/contracts?ending=60").await;
    assert!(ending.contains("CC-DEMO-002"));
    assert!(!ending.contains("CC-DEMO-001"));
}

/// AC2: the account manager has no payroll and no absence tile; the contract
/// and request tiles are there.
#[renox::test]
async fn the_account_manager_has_no_payroll_and_no_absence_tile() {
    let app = demo().await;
    let html = page(&app, AMAL, "/dashboard").await;
    assert_eq!(linked(&html, LABELS[7]), None, "payroll");
    assert_eq!(linked(&html, LABELS[5]), None, "absence");
    assert_eq!(linked(&html, LABELS[3]), None, "documents");
    assert!(
        drawn(&html) >= 3,
        "contracts, requests and the rest of his clients"
    );
    assert_eq!(counted(&html, LABELS[0]), 1);
}

/// AC3: HR sees the absence and payroll tiles when there is something, and
/// not the tiles of a permission HR does not hold.
#[renox::test]
async fn hr_sees_absence_and_payroll_and_not_the_contract_tiles() {
    let app = demo().await;
    // One open absence case and one calculated run to count.
    let html = page(&app, HR, "/dashboard").await;
    let absence = rows(
        &app,
        "select count(*) from manpower_absence_cases where closed_on is null",
    )
    .await;
    let runs = rows(
        &app,
        "select count(*) from wl_hr_payroll_runs where status = 'calculated'",
    )
    .await;
    assert_eq!(counted(&html, LABELS[5]), absence);
    assert_eq!(counted(&html, LABELS[7]), runs);
    assert_eq!(
        linked(&html, LABELS[5]).is_some(),
        absence > 0,
        "absence tile"
    );
    // HR holds neither contracts.view nor deployments.view.
    assert_eq!(linked(&html, LABELS[0]), None);
    assert_eq!(linked(&html, LABELS[1]), None);
    assert_eq!(linked(&html, LABELS[4]), None);
}

/// AC5: a worker document that ends in 20 days is counted, at 40 days it is
/// not.
#[renox::test]
async fn a_document_ending_in_twenty_days_counts_and_in_forty_does_not() {
    let app = demo().await;
    let id = scalar(
        &app,
        "select id from wl_documents_documents where subject_type = 'hr.employee' and status = 'accepted' and expiry_date is not null order by id limit 1",
    )
    .await;
    let set = |on: String| {
        format!("update wl_documents_documents set expiry_date = '{on}' where id = {id}")
    };
    let others = rows(
        &app,
        &format!(
            "select count(*) from wl_documents_documents where subject_type = 'hr.employee' and status = 'accepted' and id <> {id} and expiry_date between '{}' and '{}'",
            day(0),
            day(30)
        ),
    )
    .await;
    renox::db::sql(set(day(20).to_string()))
        .execute(app.db())
        .await
        .unwrap();
    let html = page(&app, HR, "/dashboard").await;
    assert_eq!(counted(&html, LABELS[3]), others + 1);
    renox::db::sql(set(day(40).to_string()))
        .execute(app.db())
        .await
        .unwrap();
    let html = page(&app, HR, "/dashboard").await;
    assert_eq!(counted(&html, LABELS[3]), others);
}

/// AC6: an account manager scoped to one client counts that client only.
#[renox::test]
async fn an_account_manager_of_one_client_counts_that_client_only() {
    let app = demo().await;
    let all = page(&app, MD, "/dashboard").await;
    assert_eq!(counted(&all, LABELS[0]), 1, "the hold is Al Quoz's");

    let mut viewer =
        renox::auth::User::register(app.db(), "Vera", "vera@test.example", "secret123")
            .await
            .unwrap();
    viewer.email_verified_at = Some(renox::db::now());
    viewer.save(app.db()).await.unwrap();
    renox::auth::permissions::define_role(
        app.db(),
        "tile_viewer",
        &[
            manpower::permissions::CONTRACTS_VIEW,
            manpower::permissions::REQUESTS_VIEW,
            manpower::permissions::DEPLOYMENTS_VIEW,
        ],
    )
    .await
    .unwrap();
    let tower = client(&app, "Gulf Tower Facilities LLC").await;
    viewer
        .assign_role_in(
            app.db(),
            "tile_viewer",
            &manpower::permissions::client_scope(tower.0),
        )
        .await
        .unwrap();
    app.acting_as(&viewer);
    let response = app
        .request()
        .header("accept", "text/html")
        .get("/dashboard")
        .await;
    response.assert_ok();
    let html = response.text();
    assert_eq!(
        counted(&html, LABELS[0]),
        0,
        "Al Quoz's hold is not counted"
    );
    assert_eq!(
        counted(&html, LABELS[1]),
        0,
        "Marina Bay's contract is not counted"
    );
    let towers = rows(
        &app,
        &format!(
            "select count(*) from manpower_requests where status in ('open','partially_filled') and client_party_id = {}",
            tower.0
        ),
    )
    .await;
    assert_eq!(counted(&html, LABELS[2]), towers);
}

/// AC6 (the demo's own account manager): when his role is scoped to Gulf Tower
/// only, he counts Gulf Tower's contracts, requests and timesheets and nothing
/// of the other clients, and has no payroll, absence or document tile (their
/// permissions are global).
#[renox::test]
async fn the_demo_account_manager_of_one_client_sees_that_clients_tiles_only() {
    let app = demo().await;
    let before = page(&app, AMAL, "/dashboard").await;
    assert_eq!(
        counted(&before, LABELS[0]),
        1,
        "all three clients: the hold"
    );
    assert_eq!(
        counted(&before, LABELS[6]),
        2,
        "all three clients: two sheets"
    );

    let amal = person(&app, AMAL).await;
    for other in ["Marina Bay Hospitality LLC", "Al Quoz Logistics Park LLC"] {
        let party = client(&app, other).await;
        amal.remove_role_in(
            app.db(),
            host::roles::ACCOUNT_MANAGER,
            &manpower::permissions::client_scope(party.0),
        )
        .await
        .unwrap();
    }
    let tower = client(&app, "Gulf Tower Facilities LLC").await;
    let html = page(&app, AMAL, "/dashboard").await;
    assert_eq!(
        counted(&html, LABELS[0]),
        0,
        "the hold is Al Quoz's, not counted"
    );
    let towers = rows(
        &app,
        &format!(
            "select count(*) from manpower_requests where status in ('open','partially_filled') and client_party_id = {}",
            tower.0
        ),
    )
    .await;
    assert!(towers > 0);
    assert_eq!(counted(&html, LABELS[2]), towers, "Gulf Tower's requests");
    let ending = rows(
        &app,
        &format!(
            "select count(*) from manpower_client_contracts where client_party_id = {} and status = 'active'",
            tower.0
        ),
    )
    .await;
    assert!(
        counted(&html, LABELS[1]) <= ending,
        "no contract of another client"
    );
    assert_eq!(counted(&html, LABELS[6]), 1, "Gulf Tower's one sheet");
    for (pair, what) in [
        (LABELS[3], "documents"),
        (LABELS[5], "absence"),
        (LABELS[7], "payroll"),
    ] {
        assert_eq!(linked(&html, pair), None, "{what}");
    }
}

/// AC7: the client approver sees none of these tiles.
#[renox::test]
async fn the_client_approver_sees_none_of_the_agency_tiles() {
    let app = demo().await;
    let html = page(&app, "approver@gulftower.example", "/dashboard").await;
    assert_eq!(drawn(&html), 0);
    assert!(
        !html.contains("Nothing needs your action</"),
        "no agency block to be quiet about"
    );
}

/// AC8: the tiles are one grid of links, no table that would scroll at 390 px.
#[renox::test]
async fn the_tiles_are_a_grid_of_links_without_a_table() {
    let app = demo().await;
    let html = page(&app, ADMIN, "/dashboard").await;
    assert!(html.contains("rx-stats"));
    assert!(!html.contains("<table"), "no table on the dashboard");
}
