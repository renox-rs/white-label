//! The lists of the Manpower vertical on the demo agency (#537): the
//! deployments, the requests, the client contracts and the absence cases are
//! `renox::grid`s that are direct children of their pages, search (also the
//! worker's and the client's names, kept by the workforce and the parties
//! modules), filter by client and request, sort and page through the query
//! string, and keep the scope of the role that opens them.

mod grid_support;
mod manpower_screens_support;
mod support;

use grid_support::{encode, grid_is_a_child_of_the_page, load, range};
use manpower_screens_support::{AMAL, HR, demo, person};
use renox::testing::TestApp;

async fn count(app: &TestApp, query: &str) -> i64 {
    renox::db::sql(query.to_owned())
        .scalar::<i64>(app.db())
        .await
        .unwrap()
}

async fn text_of(app: &TestApp, query: &str) -> String {
    renox::db::sql(query.to_owned())
        .scalar::<String>(app.db())
        .await
        .unwrap()
}

/// The page at `path` as `email`.
async fn page_as(app: &TestApp, email: &str, path: &str) -> String {
    let user = person(app, email).await;
    app.acting_as(&user);
    load(app, path).await
}

/// The first record id the page links as `prefix<id>`.
fn first_link(html: &str, prefix: &str) -> Option<i64> {
    let needle = format!("\"{prefix}");
    html.match_indices(&needle).find_map(|(at, _)| {
        let digits: String = html[at + needle.len()..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        digits.parse().ok()
    })
}

/// Every list of the issue is a grid that is a direct child of its page.
#[test]
fn every_manpower_list_is_a_grid_that_fills_the_page() {
    for template in [
        "manpower/views/manpower/deployments.html",
        "manpower/views/manpower/requests.html",
        "manpower/views/manpower/contracts.html",
        "manpower/views/manpower/absence_cases.html",
    ] {
        grid_is_a_child_of_the_page(template);
    }
}

/// The deployments: page size, sort, a filter by client and by request, the
/// worker found by name, and nothing for a name nobody has.
#[renox::test]
async fn the_deployments_filter_by_client_and_request_and_find_the_worker() {
    let app = demo().await;
    let all = page_as(&app, AMAL, "/manpower/deployments?per_page=10").await;
    let (from, to, deployments) = range(&all);
    assert!(deployments >= 1, "the account manager sees deployments");
    assert_eq!((from, to), (1, deployments.min(10)));
    let oldest = page_as(&app, AMAL, "/manpower/deployments?sort=start_date").await;
    assert_eq!(range(&oldest).2, deployments);

    let id = first_link(&all, "/manpower/deployments/").expect("a deployment row links its page");
    let client = count(
        &app,
        &format!("select client_party_id from manpower_deployments where id = {id}"),
    )
    .await;
    let of_client = page_as(
        &app,
        AMAL,
        &format!("/manpower/deployments?in.client_party_id={client}&per_page=100"),
    )
    .await;
    let (_, _, shown) = range(&of_client);
    assert!(shown >= 1 && shown <= deployments);
    assert!(of_client.contains(&format!("/manpower/deployments/{id}\"")));
    let requested = count(
        &app,
        &format!("select coalesce(request_id, 0) from manpower_deployments where id = {id}"),
    )
    .await;
    if requested > 0 {
        let of_request = page_as(
            &app,
            AMAL,
            &format!("/manpower/deployments?in.request_id={requested}&per_page=100"),
        )
        .await;
        let (_, _, shown) = range(&of_request);
        assert!(shown >= 1 && shown <= deployments);
    }

    let worker = text_of(
        &app,
        &format!(
            "select p.display_name from wl_parties_parties p \
             join wl_hr_employees e on e.party_id = p.id \
             join manpower_deployments d on d.employee_id = e.id where d.id = {id}"
        ),
    )
    .await;
    let found = page_as(
        &app,
        AMAL,
        &format!("/manpower/deployments?search={}&per_page=100", encode(&worker)),
    )
    .await;
    assert!(range(&found).2 >= 1, "the worker {worker} is found");
    assert!(found.contains(&format!("/manpower/deployments/{id}\"")));
    let none = page_as(&app, AMAL, "/manpower/deployments?search=zzqqxx-nobody").await;
    assert_eq!(range(&none).2, 0);
}

/// The requests and the contracts filter by client and find the client by
/// name.
#[renox::test]
async fn the_requests_and_contracts_filter_and_search_by_client() {
    let app = demo().await;
    for (list, prefix, table) in [
        ("/manpower/requests", "/manpower/requests/", "manpower_requests"),
        (
            "/manpower/contracts",
            "/manpower/contracts/",
            "manpower_client_contracts",
        ),
    ] {
        let all = page_as(&app, AMAL, &format!("{list}?per_page=10")).await;
        let (_, _, rows) = range(&all);
        assert!(rows >= 1, "{list} has rows");
        let id = first_link(&all, prefix).unwrap_or_else(|| panic!("{list} links its rows"));
        let client = count(
            &app,
            &format!("select client_party_id from {table} where id = {id}"),
        )
        .await;
        let of_client = page_as(
            &app,
            AMAL,
            &format!("{list}?in.client_party_id={client}&per_page=100"),
        )
        .await;
        let (_, _, shown) = range(&of_client);
        assert!(shown >= 1 && shown <= rows, "{list} by client");
        let name = text_of(
            &app,
            &format!("select display_name from wl_parties_parties where id = {client}"),
        )
        .await;
        let found = page_as(
            &app,
            AMAL,
            &format!("{list}?search={}&per_page=100", encode(&name)),
        )
        .await;
        assert!(
            found.contains(&format!("{prefix}{id}\"")),
            "{list}: {name} is found"
        );
        let none = page_as(&app, AMAL, &format!("{list}?search=zzqqxx-nobody")).await;
        assert_eq!(range(&none).2, 0, "{list}");
    }
}

/// The absence cases: HR finds a case by the worker's name and filters by
/// status.
#[renox::test]
async fn the_absence_cases_find_the_worker_and_filter_by_status() {
    let app = demo().await;
    let cases = count(&app, "select count(*) from manpower_absence_cases").await;
    let all = page_as(&app, HR, "/manpower/absence?per_page=10").await;
    assert_eq!(range(&all).2, cases);
    if cases == 0 {
        return;
    }
    let open = count(
        &app,
        "select count(*) from manpower_absence_cases where closed_on is null",
    )
    .await;
    let shown = page_as(&app, HR, "/manpower/absence?in.status=open").await;
    assert_eq!(range(&shown).2, open);
    let worker = text_of(
        &app,
        "select p.display_name from wl_parties_parties p \
         join wl_hr_employees e on e.party_id = p.id \
         join manpower_absence_cases c on c.employee_id = e.id order by c.id limit 1",
    )
    .await;
    let found = page_as(
        &app,
        HR,
        &format!("/manpower/absence?search={}", encode(&worker)),
    )
    .await;
    assert!(range(&found).2 >= 1, "the worker {worker} is found");
}
