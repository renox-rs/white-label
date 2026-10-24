//! #533: the Logistics lists are standard grids that fill the page: the
//! shipments, the COD remittances (with the bank account changes on a page
//! of their own), the COD differences and the reversed deliveries. Each
//! searches, sorts, pages and changes its page size through the query
//! string, and the grid is the page's content right under its header.

mod support;

use logistics::Shipment;
use logistics::account::{AccountChange, AccountChangeStatus};
use logistics::reversal::{self, Reversal};
use renox::prelude::*;

use support::{SOUQ, World, boot};

/// The page at `path` as `by` gets it, 200.
async fn load(w: &World, by: &User, path: &str) -> String {
    let page = w.get(by, path).await;
    assert_eq!(page.status.as_u16(), 200, "{path}");
    page.text()
}

/// The footer of the grid in `html` reads `from–to of total`.
fn shows_range(html: &str, from: usize, to: usize, total: usize) -> bool {
    html.contains(&format!("{from}–{to} of {total}"))
}

/// The grid of `html` sits right under the page header, with no stack, card
/// or old table around it, and it has the search box and the page sizes.
fn fills_the_page(html: &str, path: &str) {
    let header = html
        .find("class=\"rx-page-header\"")
        .unwrap_or_else(|| panic!("{path}: a page header"));
    let grid = html
        .find("<form class=\"rx-grid")
        .unwrap_or_else(|| panic!("{path}: a grid"));
    assert!(header < grid, "{path}: the header comes first");
    let between = &html[header..grid];
    for wrapper in ["rx-stack", "rx-card", "rx-table", "rx-panes"] {
        assert!(
            !between.contains(wrapper),
            "{path}: {wrapper} before the grid"
        );
    }
    assert!(
        !html.contains("rx-table-wrap"),
        "{path}: an old table is left"
    );
    assert_eq!(
        html.matches("<form class=\"rx-grid").count(),
        1,
        "{path}: one grid"
    );
    assert!(html.contains("name=\"search\""), "{path}: a search box");
    assert!(html.contains("per_page"), "{path}: a page size");
}

/// Every list of the vertical is a grid right under its header.
#[renox::test]
async fn every_logistics_list_is_a_grid_that_fills_the_page() {
    let w = boot().await;
    for (by, path) in [
        (&w.dispatcher, "/shipments"),
        (&w.amal, "/remittances"),
        (&w.amal, "/remittances/accounts"),
        (&w.amal, "/cod-differences"),
        (&w.dispatcher, "/reversals"),
    ] {
        let html = load(&w, by, path).await;
        fills_the_page(&html, path);
    }
}

/// The shipments grid finds a shipment by the recipient's phone, pages ten
/// at a time, sorts on the number and filters by state.
#[renox::test]
async fn the_shipments_grid_searches_the_phone_pages_sorts_and_filters() {
    let w = boot().await;
    let mut numbers = Vec::new();
    for n in 0..11 {
        numbers.push(w.book_as(&w.noor, SOUQ, &format!("ORD-{n}")).await.number);
    }
    let odd = w
        .book_with(
            &w.noor,
            SOUQ,
            "ORD-PHONE",
            &[
                ("recipient_phone", "+971509998877"),
                ("recipient_name", "Khalid Rahman"),
            ],
        )
        .await;

    let first = load(&w, &w.dispatcher, "/shipments?per_page=10&sort=number").await;
    assert!(shows_range(&first, 1, 10, 12));
    assert!(first.contains(&numbers[0]));
    let second = load(
        &w,
        &w.dispatcher,
        "/shipments?per_page=10&sort=number&page=2",
    )
    .await;
    assert!(shows_range(&second, 11, 12, 12));
    assert!(second.contains(&odd.number) && !second.contains(&format!(">{}<", numbers[0])));
    let all = load(&w, &w.dispatcher, "/shipments?per_page=25").await;
    assert!(shows_range(&all, 1, 12, 12));

    // The phone (a hidden column) and the recipient find the shipment.
    let by_phone = load(&w, &w.dispatcher, "/shipments?search=9998877").await;
    assert!(shows_range(&by_phone, 1, 1, 1) && by_phone.contains(&odd.number));
    let by_name = load(&w, &w.dispatcher, "/shipments?search=Khalid").await;
    assert!(shows_range(&by_name, 1, 1, 1));

    // Newest first by default; the state filter.
    let newest = load(&w, &w.dispatcher, "/shipments?sort=-number").await;
    assert!(newest.find(&odd.number).unwrap() < newest.find(&numbers[10]).unwrap());
    let created = load(
        &w,
        &w.dispatcher,
        "/shipments?in.status=created&per_page=25",
    )
    .await;
    assert!(shows_range(&created, 1, 12, 12));
    let delivered = load(&w, &w.dispatcher, "/shipments?in.status=delivered").await;
    assert!(!delivered.contains(&odd.number));
}

/// The bank account changes waiting are a grid of their own, linked from the
/// remittances' header with their number; the remittances page lists no
/// table above its grid.
#[renox::test]
async fn the_account_changes_have_their_own_grid() {
    let w = boot().await;
    let now = renox::db::now();
    let mut change = AccountChange {
        client_party_id: SOUQ.0,
        account: "AE070331234567890123456".to_owned(),
        requested_by: w.amal.id,
        requested_at: now,
        via: "staff".to_owned(),
        contact: "accounts@souq.example".to_owned(),
        expires_at: now,
        ..Default::default()
    };
    // A new change starts pending, the machine's first state.
    assert_eq!(change.status, AccountChangeStatus::Pending);
    change.save(w.db()).await.unwrap();

    let remittances = load(&w, &w.bashar, "/remittances").await;
    assert!(remittances.contains("Bank account changes (1)"));
    assert!(remittances.contains("/remittances/accounts"));

    let changes = load(&w, &w.bashar, "/remittances/accounts").await;
    assert!(shows_range(&changes, 1, 1, 1));
    assert!(changes.contains("AE070331234567890123456"));
    assert!(changes.contains("Waiting for approval and confirmation"));
    // The decisions are in the row's menu, posting to the change.
    assert!(changes.contains(&format!("/remittances/accounts/{}/approve", change.id)));
    assert!(changes.contains("id=\"reject-account\""));
    let none = load(&w, &w.bashar, "/remittances/accounts?search=AE99").await;
    assert!(!none.contains("AE070331234567890123456"));
}

/// The COD differences say who absorbed them in words and filter by it.
#[renox::test]
async fn the_cod_differences_filter_by_who_absorbed_them() {
    let w = boot().await;
    let a = w.book_as(&w.noor, SOUQ, "ORD-A").await;
    let b = w.book_as(&w.noor, SOUQ, "ORD-B").await;
    for (shipment, absorber) in [(&a, "business"), (&b, "driver")] {
        Shipment::query()
            .where_(Shipment::ID.eq(shipment.id))
            .update(w.db(), &[("cod_difference", &5_000_i64)])
            .await
            .unwrap();
        Shipment::query()
            .where_(Shipment::ID.eq(shipment.id))
            .update(w.db(), &[("cod_absorbed_by", &absorber.to_owned())])
            .await
            .unwrap();
    }
    let all = load(&w, &w.amal, "/cod-differences").await;
    assert!(shows_range(&all, 1, 2, 2));
    assert!(all.contains(">Business<") && all.contains(">Driver<"));
    let business = load(&w, &w.amal, "/cod-differences?in.cod_absorbed_by=business").await;
    assert!(shows_range(&business, 1, 1, 1));
    assert!(business.contains(&a.number) && !business.contains(&format!(">{}<", b.number)));
    let found = load(
        &w,
        &w.amal,
        &format!("/cod-differences?search={}", b.number),
    )
    .await;
    assert!(shows_range(&found, 1, 1, 1));
}

/// A reversed delivery's row names the driver, offers the two decisions in
/// its menu and opens its shipment.
#[renox::test]
async fn a_reversal_row_names_the_driver_and_opens_its_shipment() {
    let w = boot().await;
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-REV").await;
    let mut row = Reversal {
        shipment_id: shipment.id,
        reason: "Recipient refused after payment".to_owned(),
        liability: "driver".to_owned(),
        cash_amount: 12_000,
        driver_user_id: Some(w.driver.id),
        target_state: "at_hub".to_owned(),
        task_status: reversal::TASK_OPEN.to_owned(),
        ..Default::default()
    };
    row.save(w.db()).await.unwrap();

    let html = load(&w, &w.dispatcher, "/reversals").await;
    assert!(shows_range(&html, 1, 1, 1));
    assert!(html.contains(&shipment.number));
    assert!(html.contains(&w.driver.name), "the driver by name");
    assert!(html.contains(&format!("/reversals/{}/raise-shortage", row.id)));
    assert!(html.contains(&format!("/reversals/{}/hr-case", row.id)));
    assert!(html.contains(&format!("/reversals/{}/shipment", row.id)));
    assert!(!html.contains("{shipment_id}"), "no link left unfilled");
    let found = load(&w, &w.dispatcher, "/reversals?search=refused").await;
    assert!(shows_range(&found, 1, 1, 1));

    let open = w
        .get(&w.dispatcher, &format!("/reversals/{}/shipment", row.id))
        .await;
    assert_eq!(open.status.as_u16(), 303);
    let location = open.header("location").unwrap_or_default().to_owned();
    assert!(
        location.ends_with(&format!("/shipments/{}", shipment.id)),
        "{location}"
    );
    w.get(&w.amal, &format!("/reversals/{}/shipment", row.id))
        .await
        .assert_forbidden();
}
