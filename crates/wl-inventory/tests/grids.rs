//! The stock lists on the standard grid (#528): stock, goods receipts,
//! purchase orders, transfers, holds, pending issues, recalls, dispatches and
//! preferred-supplier requests are `renox::grid`s that are direct children of
//! their pages. The figures of the stock page are sorted, filtered and
//! exported by the grid (not only the rows of the page), the supplier is a
//! filter, and a grid refresh (htmx, `Accept: */*`) gets the grid, not JSON.

mod support;

use renox::prelude::*;
use wl_inventory::DraftLine;
use wl_kernel::PartyId;

use support::{Booted, SUPPLIER, boot};

/// The footer of the grid in `html`: first row, last row and total
/// (`1–10 of 18`); `(0, 0, 0)` when the grid shows no footer (no rows).
fn range(html: &str) -> (i64, i64, i64) {
    let marker = "rx-grid__range\">";
    let Some(at) = html.find(marker) else {
        return (0, 0, 0);
    };
    let rest = &html[at + marker.len()..];
    let text = &rest[..rest.find("</p>").unwrap()];
    let numbers: Vec<i64> = text
        .split(|c: char| !c.is_ascii_digit() && c != ',')
        .filter(|part| !part.replace(',', "").is_empty())
        .map(|part| part.replace(',', "").parse().unwrap())
        .collect();
    (numbers[0], numbers[1], numbers[2])
}

/// The page at `path` as `user`, as a browser asks for it.
async fn page(t: &Booted, user: &User, path: &str) -> String {
    let response = t.browse(user, path).await;
    assert_eq!(response.status.as_u16(), 200, "{path}: {}", response.text());
    response.text()
}

/// The template `name` of the crate puts its `<rx-grid>` directly in
/// `<rx-page>`, after the header, with nothing but dialogs after it.
fn grid_is_a_child_of_the_page(name: &str) {
    let path = format!("{}/views/wl-inventory/{name}", env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{path}"));
    let lines: Vec<&str> = source.lines().collect();
    let at = lines
        .iter()
        .position(|line| line.trim_start().starts_with("<rx-grid"))
        .unwrap_or_else(|| panic!("{name} has no grid"));
    assert!(
        lines[at].starts_with("  <rx-grid"),
        "{name}: the grid is a direct child of <rx-page>"
    );
    let child = |line: &str| line.starts_with("  <") && !line.starts_with("   ");
    for line in lines[..at].iter().filter(|line| child(line)) {
        assert!(
            ["  <rx-page-header", "  <rx-include", "  </rx-page-header"]
                .iter()
                .any(|ok| line.starts_with(ok)),
            "{name}: `{line}` wraps or comes before the grid"
        );
    }
    for line in lines[at + 1..].iter().filter(|line| child(line)) {
        assert!(
            [
                "  </rx-grid",
                "  <rx-action-sheet",
                "  </rx-action-sheet",
                "  <template",
                "  </template",
            ]
            .iter()
            .any(|ok| line.starts_with(ok)),
            "{name}: `{line}` is below the grid"
        );
    }
    assert!(
        !source.contains("<rx-table"),
        "{name}: no table beside the grid"
    );
}

/// Every list of the issue is a grid that is a direct child of its page.
#[test]
fn every_stock_list_is_a_grid_that_fills_the_page() {
    for name in [
        "stock.html",
        "receipts.html",
        "orders.html",
        "transfers.html",
        "holds.html",
        "pending.html",
        "recalls.html",
        "dispatches.html",
        "preferred_requests.html",
    ] {
        grid_is_a_child_of_the_page(name);
    }
}

/// The stock of a store: page size and pages, the search on the code, the
/// sort on what is on hand (all the store's items, not the page's), the
/// level a filter, and the export with the figures for the storekeeper who
/// adjusts the stock only.
#[renox::test]
async fn the_stock_sorts_and_filters_on_its_figures_and_exports_them() {
    let t = boot().await;
    let store = t.named_location("STORE-1", "Jumeirah store").await;
    for (code, quantity) in [("GLOVES", 40_000), ("MASKS", 5_000), ("GOWNS", 12_000)] {
        let item = t.item(code).await;
        t.stock_in(&item, &store, None, quantity, 1_000).await;
    }
    let mut masks = wl_inventory::Item::query()
        .where_(wl_inventory::Item::CODE.eq("MASKS"))
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    masks.reorder_point_milli = 10_000;
    masks.save(t.db()).await.unwrap();
    let viewer = t.finance("Fiona").await;
    let here = format!("/stock?location={}", store.id);

    let all = page(&t, &viewer, &format!("{here}&per_page=10")).await;
    assert_eq!(range(&all), (1, 3, 3));
    assert!(all.contains("40.000 kg"), "the quantity in the item's unit");

    let most = page(&t, &viewer, &format!("{here}&sort=-on_hand_milli")).await;
    let (gloves, gowns, masks_at) = (
        most.find("GLOVES").unwrap(),
        most.find("GOWNS").unwrap(),
        most.find("MASKS").unwrap(),
    );
    assert!(gloves < gowns && gowns < masks_at, "most on hand first");

    let found = page(&t, &viewer, &format!("{here}&search=glov")).await;
    assert_eq!(range(&found).2, 1);
    let low = page(&t, &viewer, &format!("{here}&in.level=low")).await;
    assert_eq!(range(&low).2, 1, "only the item at its reorder point");
    assert!(low.contains("MASKS"));
    let plenty = page(&t, &viewer, &format!("{here}&min.on_hand_milli=10")).await;
    assert_eq!(range(&plenty).2, 2, "the figure filters all the items");

    t.browse(&viewer, &format!("{here}&export=csv"))
        .await
        .assert_forbidden();
    let clerk = t.finance("Karim").await;
    clerk.assign_role(t.db(), "stock_clerk").await.unwrap();
    let file = t.browse(&clerk, &format!("{here}&export=csv")).await;
    file.assert_ok();
    assert!(file.text().contains("GLOVES"));
    assert!(file.text().contains("40"), "the figures are in the export");
}

/// The orders and receipts: the supplier is a filter on both.
#[renox::test]
async fn the_orders_and_receipts_filter_by_supplier() {
    let t = boot().await;
    let store = t.named_location("STORE-1", "Jumeirah store").await;
    let item = t.item("GLOVES").await;
    let (first, first_line) = t.order(&store, &item, 10_000, 100).await;
    let other = PartyId(999);
    let (second, second_line) = t.order_from(other, &store, &item, 5_000, 100).await;
    t.receipt(&first, &store, vec![DraftLine::on(first_line.id, 10_000, 10_000)])
        .await;
    t.receipt(&second, &store, vec![DraftLine::on(second_line.id, 5_000, 5_000)])
        .await;
    let keeper = t.storekeeper("Sami").await;

    let orders = page(&t, &keeper, "/purchase-orders?per_page=10").await;
    assert_eq!(range(&orders).2, 2);
    let of_one = page(&t, &keeper, &format!("/purchase-orders?in.supplier={}", SUPPLIER.0)).await;
    assert_eq!(range(&of_one).2, 1);
    let receipts = page(&t, &keeper, "/goods-receipts?per_page=10&sort=-when").await;
    assert_eq!(range(&receipts).2, 2);
    let of_other = page(&t, &keeper, "/goods-receipts?in.supplier=999").await;
    assert_eq!(range(&of_other).2, 1);
}

/// A grid refresh asks with htmx and `Accept: */*`: the dispatches, the
/// pending issues and the preferred-supplier requests answer the grid, a
/// plain JSON client still gets JSON.
#[renox::test]
async fn a_grid_refresh_gets_the_grid_not_json() {
    let t = boot().await;
    let dispatcher = t.dispatcher("Dina").await;
    t.app.acting_as(&dispatcher);
    let refresh = t
        .app
        .request()
        .header("hx-request", "true")
        .header("accept", "*/*")
        .get("/dispatches?search=nothing")
        .await;
    refresh.assert_ok();
    assert!(refresh.text().contains("rx-grid"), "{}", refresh.text());
    let json = t.app.request().header("accept", "*/*").get("/dispatches").await;
    json.assert_ok();
    assert!(json.text().trim_start().starts_with('['), "{}", json.text());

    let clerk = t.user_with("Cleo", "stock_clerk").await;
    t.app.acting_as(&clerk);
    let refresh = t
        .app
        .request()
        .header("hx-request", "true")
        .header("accept", "*/*")
        .get("/pending-issues?per_page=10")
        .await;
    refresh.assert_ok();
    assert!(refresh.text().contains("rx-grid"), "{}", refresh.text());
}
