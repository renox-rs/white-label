//! The lists of wl-sales are standard grids (#526): each one fills the page
//! (the grid follows the header, with no stack, card or pane around it),
//! searches, sorts, pages through the query string (`?sort=`, `?per_page=`,
//! `?page=`) and filters by its select and date columns (`?in.channel=`,
//! `?from.business_date=`).

mod support;

use renox::chrono::Duration;
use wl_kernel::documents::today;
use wl_kernel::sales::OpenTill;
use wl_sales::permissions as perm;
use wl_sales::till_sessions;

use support::floor::Floor;
use support::{Booted, sell};

/// The page at `path`, as the signed-in user gets it.
async fn load(t: &Booted, path: &str) -> String {
    let page = t.app.get(path).await;
    page.assert_ok();
    page.text()
}

/// Where `text` first stands in the page; text that is not there is last.
fn at(html: &str, text: &str) -> usize {
    html.find(text).unwrap_or(usize::MAX)
}

/// Whether the grid is the page's own block: no stack, card or pane between
/// the header and the grid.
fn fills_the_page(html: &str) -> bool {
    let (Some(header), Some(grid)) = (
        html.find("class=\"rx-page-header\""),
        html.find("<form class=\"rx-grid"),
    ) else {
        return false;
    };
    let between = &html[header..grid];
    header < grid
        && !between.contains("rx-stack")
        && !between.contains("rx-card")
        && !between.contains("rx-pane")
}

/// Sales orders: twelve drafts, seven by phone and five from the website;
/// the channel and the status filter, the number searches, the newest come
/// first and the pages hold what `per_page` says.
#[renox::test]
async fn sal_grid_the_sales_orders_filter_sort_search_and_page() {
    let (t, _) = sell::boot().await;
    let seller = sell::seller(&t).await;
    t.app.acting_as(&seller);
    let mut numbers = Vec::new();
    for n in 0..12 {
        let channel = if n < 7 { "phone" } else { "website" };
        let order = sell::draft(&t, &seller, channel, "on_account", 10_000 + n).await;
        numbers.push(order.number.clone());
    }

    let first = load(&t, "/sales-orders?sort=-id&per_page=10").await;
    assert!(fills_the_page(&first));
    assert!(first.contains("1–10 of 12"));
    assert!(first.contains(&numbers[11]) && !first.contains(&numbers[0]));
    assert!(at(&first, &numbers[11]) < at(&first, &numbers[10]), "newest first");
    assert!(first.contains("Party 7"), "the customer by name");
    let second = load(&t, "/sales-orders?sort=-id&per_page=10&page=2").await;
    assert!(second.contains("11–12 of 12") && second.contains(&numbers[0]));
    let oldest = load(&t, "/sales-orders?sort=id&per_page=25").await;
    assert!(at(&oldest, &numbers[0]) < at(&oldest, &numbers[1]), "oldest first");

    let website = load(&t, "/sales-orders?in.channel=website").await;
    assert!(website.contains("1–5 of 5"));
    assert!(website.contains("Website"), "the channel in words");
    assert!(!website.contains(&numbers[0]));
    let drafts = load(&t, "/sales-orders?in.status=draft&per_page=25").await;
    assert!(drafts.contains("1–12 of 12"));
    let none = load(&t, "/sales-orders?in.status=confirmed").await;
    assert!(!none.contains(&numbers[0]));

    let found = load(&t, &format!("/sales-orders?search={}", numbers[3])).await;
    assert!(found.contains("1–1 of 1") && found.contains(&numbers[3]));
}

/// Loyalty members: twelve members, searched by name or by mobile number,
/// sorted and paged.
#[renox::test]
async fn sal_grid_the_loyalty_members_search_by_name_or_mobile() {
    let t = support::boot().await;
    let floor = Floor::new(&t).await;
    floor.turn_on(&t, &[]).await;
    for n in 1..=12 {
        floor
            .enrol(&t, 100 + n, &format!("+97155000{n:04}"))
            .await;
    }
    t.app.acting_as(&floor.mia);

    let first = load(&t, "/loyalty?per_page=10").await;
    assert!(fills_the_page(&first));
    assert!(first.contains("1–10 of 12"));
    let second = load(&t, "/loyalty?per_page=10&page=2").await;
    assert!(second.contains("11–12 of 12"));

    let by_name = load(&t, "/loyalty?search=Party%20107").await;
    assert!(by_name.contains("1–1 of 1") && by_name.contains("Party 107"));
    let by_mobile = load(&t, "/loyalty?search=550000011").await;
    assert!(by_mobile.contains("1–1 of 1") && by_mobile.contains("Party 111"));

    let desc = load(&t, "/loyalty?sort=-mobile&per_page=25").await;
    assert!(at(&desc, "+971550000012") < at(&desc, "+971550000001"));
    let asc = load(&t, "/loyalty?sort=mobile&per_page=25").await;
    assert!(at(&asc, "+971550000001") < at(&asc, "+971550000012"));
}

/// Till sessions: twelve sessions on twelve days, filtered by a range of
/// days, sorted by day and paged.
#[renox::test]
async fn sal_grid_the_till_sessions_filter_by_day_sort_and_page() {
    let t = support::boot().await;
    let manager = t
        .user("Tariq", &[perm::POS_OPEN_TILL, perm::POS_REVIEW])
        .await;
    let day = |n: i64| today() - Duration::days(n);
    for n in 0..12 {
        till_sessions::open(
            t.state(),
            None,
            OpenTill {
                till: format!("T{n:02}"),
                location: Some("marina".to_owned()),
                legal_entity_id: None,
                business_date: day(n),
                cashier_user: manager.id,
                float: 10_000,
            },
        )
        .await
        .unwrap();
    }
    t.app.acting_as(&manager);

    let first = load(&t, "/till-sessions?per_page=10").await;
    assert!(fills_the_page(&first));
    assert!(first.contains("1–10 of 12"));
    assert!(at(&first, ">T00<") < at(&first, ">T01<"), "the latest day first");
    let second = load(&t, "/till-sessions?per_page=10&page=2").await;
    assert!(second.contains("11–12 of 12") && second.contains(">T11<"));

    let range = load(
        &t,
        &format!(
            "/till-sessions?from.business_date={}&to.business_date={}",
            day(5),
            day(3)
        ),
    )
    .await;
    assert!(range.contains("1–3 of 3"));
    assert!(range.contains(">T04<"));
    assert!(!range.contains(">T06<") && !range.contains(">T02<"));

    let oldest = load(&t, "/till-sessions?sort=business_date&per_page=25").await;
    assert!(at(&oldest, ">T11<") < at(&oldest, ">T00<"));
    let found = load(&t, "/till-sessions?search=T07").await;
    assert!(found.contains("1–1 of 1"));
}

/// Lead sources: the nine seeded sources, sorted by code both ways, paged
/// and searched.
#[renox::test]
async fn sal_grid_the_lead_sources_sort_search_and_page() {
    let t = support::boot().await;
    let manager = t.user("Maya", &[perm::CAMPAIGNS_MANAGE]).await;
    t.app.acting_as(&manager);

    let first = load(&t, "/sales/lead-sources?per_page=5").await;
    assert!(fills_the_page(&first));
    assert!(first.contains("1–5 of 9"));
    let second = load(&t, "/sales/lead-sources?per_page=5&page=2").await;
    assert!(second.contains("6–9 of 9"));
    let asc = load(&t, "/sales/lead-sources?sort=code&per_page=25").await;
    assert!(at(&asc, "Aggregator") < at(&asc, "Website"));
    let desc = load(&t, "/sales/lead-sources?sort=-code&per_page=25").await;
    assert!(at(&desc, "Website") < at(&desc, "Aggregator"));
    let found = load(&t, "/sales/lead-sources?search=paid").await;
    assert!(found.contains("1–1 of 1") && found.contains("Paid ads"));
}

/// The other lists open as grids that fill the page for those who may see
/// them, and their search runs.
#[renox::test]
async fn sal_grid_every_list_is_a_grid_that_fills_the_page() {
    let t = support::boot().await;
    let manager = t
        .user(
            "Lina",
            &[
                perm::POS_SELL,
                perm::RETURNS_REQUEST,
                perm::LOYALTY_VIEW,
                perm::LOYALTY_MANAGE,
                perm::SEGMENTS_MANAGE,
                perm::ACQUISITION_REPORTS_VIEW,
                perm::CAMPAIGNS_MANAGE,
            ],
        )
        .await;
    t.app.acting_as(&manager);
    for path in [
        "/counter-sales",
        "/sales-returns",
        "/loyalty/adjustments",
        "/loyalty/review",
        "/segments",
        "/sales/acquisition-exceptions",
        "/sales/acquisition-exceptions?tab=changes",
    ] {
        let html = load(&t, path).await;
        assert!(fills_the_page(&html), "{path} fills the page");
        let separator = if path.contains('?') { '&' } else { '?' };
        let searched = load(
            &t,
            &format!("{path}{separator}search=nothing-like-it&per_page=10"),
        )
        .await;
        assert!(searched.contains("<form class=\"rx-grid"), "{path} searches");
    }
    // The accounts to review export for a manager.
    t.app
        .get("/loyalty/review?export=csv")
        .await
        .assert_ok();
}
