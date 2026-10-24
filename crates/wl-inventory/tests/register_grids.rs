//! The inventory registers as `renox::grid`s (F7 grid standard): dispatches,
//! goods receipts, purchase orders, transfers, stock (items and lots), holds
//! and bills of materials sort, filter, search and page on the server (no
//! silent cap), say so in words when a filter matches nothing, and show a
//! user only the rows of the scopes the user holds the permission in.

mod support;

use renox::auth::permissions::Scope;
use renox::prelude::*;
use renox::testing::TestResponse;
use wl_inventory::goods_receipt::DraftLine;
use wl_inventory::transfer::NewTransferLine;
use wl_inventory::{Item, Location, NewDispatchLine, PurchaseOrder};
use wl_kernel::Subject;

use support::{Booted, ORDER, boot};

/// A user with `role` in the scope of `scope` only.
async fn scoped(t: &Booted, name: &str, role: &str, scope: &Subject) -> User {
    let email = format!("{}@example.com", name.to_lowercase());
    let user = User::register(t.db(), name, &email, "password123")
        .await
        .unwrap();
    user.assign_role_in(
        t.db(),
        role,
        &Scope::new(&scope.subject_type, scope.subject_id),
    )
    .await
    .unwrap();
    user
}

/// `path` fetched as a browser, as `by`, must be a page that says all of
/// `words` and none of `absent`.
async fn sees(t: &Booted, by: &User, path: &str, words: &[&str], absent: &[&str]) -> TestResponse {
    let page = t.browse(by, path).await;
    page.assert_ok();
    for word in words {
        assert!(page.text().contains(word), "{path} lacks `{word}`");
    }
    for word in absent {
        assert!(!page.text().contains(word), "{path} shows `{word}`");
    }
    page
}

/// The link of a record's page as a row writes it, closed by its quote so
/// that `/boms/1"` is not found in `/boms/12"`.
fn link(prefix: &str, id: i64) -> String {
    format!("{prefix}/{id}\"")
}

/// Twenty-seven dispatches, oldest first, of a pump at `store`.
async fn many_dispatches(t: &Booted, store: &Location) -> (Item, Vec<wl_inventory::Dispatch>) {
    let pump = t.priced_item("PUMP", false, 5_000).await;
    let mut dispatches = Vec::new();
    for n in 0..27 {
        dispatches.push(
            t.draft_dispatch(store, ORDER + n, vec![NewDispatchLine::of(pump.id, 1_000)])
                .await,
        );
    }
    (pump, dispatches)
}

/// /dispatches: 27 rows are two pages (no cap), `sort=` reorders, a search
/// and a status filter narrow, a filter that matches nothing offers to clear
/// the filters; JSON answers every row; a scoped dispatcher sees only the
/// dispatches of the scope.
#[renox::test]
async fn the_dispatches_grid_pages_sorts_filters_and_respects_the_scope() {
    let t = boot().await;
    let store = t.named_location("STORE-1", "Jebel Ali warehouse").await;
    let (pump, dispatches) = many_dispatches(&t, &store).await;
    let number = |n: usize| dispatches[n].number.clone().unwrap();
    let dispatcher = t.dispatcher("Dina").await;

    sees(
        &t,
        &dispatcher,
        "/dispatches",
        &[&number(26)],
        &[&number(0)],
    )
    .await;
    sees(
        &t,
        &dispatcher,
        "/dispatches?page=2",
        &[&number(0)],
        &[&number(26)],
    )
    .await;
    sees(
        &t,
        &dispatcher,
        "/dispatches?sort=id",
        &[&number(0)],
        &[&number(26)],
    )
    .await;
    sees(
        &t,
        &dispatcher,
        &format!("/dispatches?search={}", number(12)),
        &[&number(12)],
        &[&number(0), &number(26)],
    )
    .await;
    sees(
        &t,
        &dispatcher,
        "/dispatches?in.status=draft&per_page=10",
        &[&number(26)],
        &[&number(0)],
    )
    .await;
    sees(
        &t,
        &dispatcher,
        "/dispatches?in.status=delivered",
        &["Clear filters"],
        &[&number(26)],
    )
    .await;
    // The JSON answer is every dispatch the filters match, not a page.
    t.app.acting_as(&dispatcher);
    let json = t.app.request().get("/dispatches").await;
    json.assert_ok();
    assert_eq!(json.text().matches("\"status_code\":\"draft\"").count(), 27);

    // A dispatcher of another scope sees none of them; one of this scope
    // sees only the dispatch of the scope.
    let site = Subject::new("construction.project", 4);
    let on_site = t.location("SITE", Some(&site)).await;
    let site_dispatch = t
        .draft_dispatch(
            &on_site,
            ORDER + 100,
            vec![NewDispatchLine::of(pump.id, 1_000)],
        )
        .await;
    let on_site_user = scoped(&t, "Sid", "dispatcher", &site).await;
    sees(
        &t,
        &on_site_user,
        "/dispatches",
        &[site_dispatch.number.as_deref().unwrap()],
        &[&number(26), &number(0)],
    )
    .await;
}

/// /purchase-orders and /goods-receipts: 27 of each are two pages, the
/// drafts lead, `sort=` and a search narrow, nothing matching offers to
/// clear the filters, and a storekeeper of a scope sees only the receipts of
/// the scope.
#[renox::test]
async fn the_orders_and_receipts_grids_page_sort_and_filter() {
    let t = boot().await;
    let store = t.named_location("STORE-1", "Jebel Ali warehouse").await;
    let pump = t.priced_item("PUMP", false, 5_000).await;
    let mut orders = Vec::new();
    let mut receipts = Vec::new();
    for n in 0..27 {
        let (order, line) = t.order(&store, &pump, 10_000, 5_000).await;
        // A number, so that each row can be told from the others.
        PurchaseOrder::query()
            .where_(PurchaseOrder::ID.eq(order.id))
            .update(t.db(), &[("number", &Some(format!("PO-T-{n:02}")))])
            .await
            .unwrap();
        // Every order but the first has a draft receipt started.
        if !orders.is_empty() {
            receipts.push(
                t.receipt(&order, &store, vec![DraftLine::on(line.id, 10_000, 10_000)])
                    .await,
            );
        }
        orders.push(order);
    }
    let keeper = t.storekeeper("Kim").await;

    // Purchase orders: newest first, 25 to a page; `sort=` and a search narrow.
    sees(&t, &keeper, "/purchase-orders", &["PO-T-26"], &["PO-T-00"]).await;
    sees(
        &t,
        &keeper,
        "/purchase-orders?page=2",
        &["PO-T-00"],
        &["PO-T-26"],
    )
    .await;
    sees(
        &t,
        &keeper,
        "/purchase-orders?sort=number",
        &["PO-T-00"],
        &["PO-T-26"],
    )
    .await;
    sees(
        &t,
        &keeper,
        "/purchase-orders?search=PO-T-13",
        &["PO-T-13"],
        &["PO-T-00", "PO-T-26"],
    )
    .await;
    sees(
        &t,
        &keeper,
        "/purchase-orders?in.status=partially_received",
        &["Clear filters"],
        &["PO-T-26"],
    )
    .await;
    // The order with a draft receipt opens it; the one without starts one.
    let page = t
        .browse(&keeper, "/purchase-orders?sort=number&per_page=10")
        .await;
    assert!(page.text().contains(&format!("receive-{}", orders[0].id)));
    assert!(
        page.text()
            .contains(&link("/goods-receipts", receipts[0].id))
    );

    // Receipts: drafts first, 26 receipts are two pages.
    let page_one = t.browse(&keeper, "/goods-receipts").await;
    page_one.assert_ok();
    for receipt in &receipts[1..] {
        assert!(
            page_one
                .text()
                .contains(&link("/goods-receipts", receipt.id)),
            "receipt {} is on the first page",
            receipt.id
        );
    }
    assert!(
        !page_one
            .text()
            .contains(&link("/goods-receipts", receipts[0].id))
    );
    let by_id = t
        .browse(&keeper, "/goods-receipts?sort=-id&per_page=10")
        .await;
    assert!(
        by_id
            .text()
            .contains(&link("/goods-receipts", receipts[25].id))
    );
    assert!(
        !by_id
            .text()
            .contains(&link("/goods-receipts", receipts[0].id))
    );
    let paged = t
        .browse(&keeper, "/goods-receipts?sort=id&per_page=10&page=3")
        .await;
    assert!(
        paged
            .text()
            .contains(&link("/goods-receipts", receipts[20].id))
    );
    sees(
        &t,
        &keeper,
        "/goods-receipts?in.status=reversed",
        &["Clear filters"],
        &[],
    )
    .await;
    sees(
        &t,
        &keeper,
        "/goods-receipts?search=zzz-no-note",
        &["Clear filters"],
        &[],
    )
    .await;
    // Those who may receive nowhere are refused.
    let nobody = t.clerk("Carl").await;
    t.browse(&nobody, "/goods-receipts")
        .await
        .assert_forbidden();
    t.browse(&nobody, "/purchase-orders")
        .await
        .assert_forbidden();

    // A storekeeper of another scope sees only the receipt of the scope.
    let site = Subject::new("construction.project", 4);
    let on_site = t.location("SITE", Some(&site)).await;
    let (site_order, site_line) = t.order(&on_site, &pump, 10_000, 5_000).await;
    let site_receipt = t
        .receipt(
            &site_order,
            &on_site,
            vec![DraftLine::on(site_line.id, 10_000, 10_000)],
        )
        .await;
    let on_site_keeper = scoped(&t, "Sid", "storekeeper", &site).await;
    sees(
        &t,
        &on_site_keeper,
        "/goods-receipts",
        &[&link("/goods-receipts", site_receipt.id)],
        &[&link("/goods-receipts", receipts[25].id)],
    )
    .await;
}

/// /stock/transfers: 27 transfers are two pages, `sort=` and a status filter
/// narrow, nothing matching offers to clear the filters.
#[renox::test]
async fn the_transfers_grid_pages_sorts_and_filters() {
    let t = boot().await;
    let from = t.named_location("STORE-1", "Jebel Ali warehouse").await;
    let to = t.named_location("STORE-2", "Sharjah depot").await;
    let pump = t.priced_item("PUMP", false, 5_000).await;
    let mut transfers = Vec::new();
    for _ in 0..27 {
        transfers.push(
            t.transfer(&from, &to, vec![NewTransferLine::of(pump.id, 1_000)])
                .await,
        );
    }
    let mover = t.mover("Mo").await;
    let number = |n: usize| transfers[n].number.clone().unwrap();
    sees(
        &t,
        &mover,
        "/stock/transfers",
        &[&number(26)],
        &[&number(0)],
    )
    .await;
    sees(
        &t,
        &mover,
        "/stock/transfers?page=2",
        &[&number(0)],
        &[&number(26)],
    )
    .await;
    sees(
        &t,
        &mover,
        "/stock/transfers?sort=id",
        &[&number(0)],
        &[&number(26)],
    )
    .await;
    sees(
        &t,
        &mover,
        "/stock/transfers?in.status=draft&per_page=10",
        &[&number(26)],
        &[&number(0)],
    )
    .await;
    sees(
        &t,
        &mover,
        "/stock/transfers?in.status=received",
        &["Clear filters"],
        &[&number(26)],
    )
    .await;
    sees(
        &t,
        &mover,
        "/stock/transfers?search=Sharjah",
        &[&number(26)],
        &[],
    )
    .await;
    sees(
        &t,
        &mover,
        "/stock/transfers?search=Nowhere",
        &["Clear filters"],
        &[&number(26)],
    )
    .await;
    // A draft transfer shows its Send button to a mover of the source.
    sees(&t, &mover, "/stock/transfers", &["Send"], &[]).await;
    t.browse(&t.clerk("Carl").await, "/stock/transfers")
        .await
        .assert_forbidden();
}

/// /stock: the items and the lots of a store are grids of their own
/// (`items.` and `lots.` names), 27 each are two pages, sorted by what is on
/// hand, searched, and nothing matching offers to clear the filters.
#[renox::test]
async fn the_stock_items_and_lots_are_grids() {
    let t = boot().await;
    let store = t.named_location("STORE-1", "Jebel Ali warehouse").await;
    for n in 0..27i64 {
        let item = t.priced_item(&format!("ITEM-{n:02}"), true, 100).await;
        let lot = t.lot_expiring(&item, &format!("LOT-{n:02}"), 60 + n).await;
        t.stock_in(&item, &store, Some(&lot), 1_000 * (n + 1), 100)
            .await;
    }
    let viewer = t.finance("Fay").await;
    let path = |query: &str| format!("/stock?location={}{query}", store.id);
    // The lots grid and the items grid show the item names; each is told
    // apart by emptying the other.
    let no_lots = "&lots.search=no-lot-is-called-this";
    let no_items = "&items.search=no-item-is-called-this";
    // Items by name: 25 to a page.
    sees(&t, &viewer, &path(no_lots), &["ITEM-00"], &["ITEM-26"]).await;
    sees(
        &t,
        &viewer,
        &path(&format!("&items.page=2{no_lots}")),
        &["ITEM-26"],
        &["ITEM-00"],
    )
    .await;
    // Sorted by what is on hand, the most first.
    sees(
        &t,
        &viewer,
        &path(&format!("&items.sort=-on_hand&items.per_page=10{no_lots}")),
        &["ITEM-26"],
        &["ITEM-00"],
    )
    .await;
    sees(
        &t,
        &viewer,
        &path(&format!("&items.search=ITEM-13{no_lots}")),
        &["ITEM-13"],
        &["ITEM-00", "ITEM-26"],
    )
    .await;
    sees(&t, &viewer, &path(no_items), &["Clear filters"], &[]).await;
    // The lots: soonest expiry first, 25 to a page.
    sees(&t, &viewer, &path(no_items), &["LOT-00"], &["LOT-26"]).await;
    sees(
        &t,
        &viewer,
        &path(&format!("&lots.page=2{no_items}")),
        &["LOT-26"],
        &["LOT-00"],
    )
    .await;
    sees(
        &t,
        &viewer,
        &path(&format!(
            "&lots.sort=-expiry_date&lots.per_page=10{no_items}"
        )),
        &["LOT-26"],
        &["LOT-00"],
    )
    .await;
    sees(
        &t,
        &viewer,
        &path(&format!("&lots.search=LOT-13{no_items}")),
        &["LOT-13"],
        &["LOT-00", "LOT-26"],
    )
    .await;
    // A user who may not see stock is refused.
    t.browse(&t.clerk("Carl").await, "/stock")
        .await
        .assert_forbidden();
}

/// /stock/holds: 27 holds are two pages with the open ones first, a status
/// filter narrows, and nothing matching offers to clear the filters.
#[renox::test]
async fn the_holds_grid_pages_and_filters() {
    let t = boot().await;
    let pump = t.priced_item("PUMP", false, 100).await;
    let holder = t.holder("Hana").await;
    for n in 0..27 {
        let store = t
            .named_location(&format!("S-{n:02}"), &format!("Store {n:02}"))
            .await;
        t.stock_in(&pump, &store, None, 5_000, 100).await;
        t.hold_as(
            &holder,
            &[
                ("reason", "temperature"),
                ("reference", &format!("ticketing.ticket:{}", n + 1)),
                ("location", &store.id.to_string()),
            ],
        )
        .await
        .assert_ok();
    }
    let holds = t.all_holds().await;
    assert_eq!(holds.len(), 27);
    let oldest = holds.iter().min_by_key(|h| h.id).unwrap();
    let newest = holds.iter().max_by_key(|h| h.id).unwrap();
    let (oldest, newest) = (
        oldest.number.clone().unwrap(),
        newest.number.clone().unwrap(),
    );
    sees(&t, &holder, "/stock/holds", &[&newest], &[&oldest]).await;
    sees(&t, &holder, "/stock/holds?page=2", &[&oldest], &[&newest]).await;
    sees(&t, &holder, "/stock/holds?sort=id", &[&oldest], &[&newest]).await;
    sees(
        &t,
        &holder,
        &format!("/stock/holds?search={oldest}"),
        &[&oldest],
        &[&newest],
    )
    .await;
    sees(
        &t,
        &holder,
        "/stock/holds?in.status=cancelled",
        &["Clear filters"],
        &[&newest],
    )
    .await;
    t.browse(&t.clerk("Carl").await, "/stock/holds")
        .await
        .assert_forbidden();
}

/// /boms: 27 versions are two pages, `?status=` and `in.status=` narrow, a
/// search finds one, the JSON answer lists every version, and nothing
/// matching offers to clear the filters.
#[renox::test]
async fn the_boms_grid_pages_sorts_and_filters() {
    let t = boot().await;
    let keeper = t.bom_keeper("Bea").await;
    let mut ids = Vec::new();
    for n in 0..27 {
        let item = t.priced_item(&format!("MADE-{n:02}"), false, 0).await;
        t.app.acting_as(&keeper);
        let made = t
            .app
            .htmx()
            .post("/boms", &[("item", &item.id.to_string()), ("output", "1")])
            .await;
        let id = made
            .header("location")
            .and_then(|l| l.rsplit('/').next().map(str::to_owned))
            .and_then(|id| id.parse::<i64>().ok())
            .expect("a redirect to the bill");
        ids.push(id);
    }
    let bom = |n: usize| link("/boms", ids[n]);
    sees(&t, &keeper, "/boms", &[&bom(26)], &[&bom(0)]).await;
    sees(&t, &keeper, "/boms?page=2", &[&bom(0)], &[&bom(26)]).await;
    sees(&t, &keeper, "/boms?sort=id", &[&bom(0)], &[&bom(26)]).await;
    sees(
        &t,
        &keeper,
        "/boms?search=MADE-13",
        &[&bom(13)],
        &[&bom(0), &bom(26)],
    )
    .await;
    sees(
        &t,
        &keeper,
        "/boms?in.status=draft&per_page=10",
        &[&bom(26)],
        &[&bom(0)],
    )
    .await;
    // The links of the tabs and of the dashboards keep their meaning.
    sees(&t, &keeper, "/boms?status=draft", &[&bom(26)], &[]).await;
    sees(&t, &keeper, "/boms?status=released", &[], &[&bom(26)]).await;
    sees(
        &t,
        &keeper,
        "/boms?in.status=obsolete",
        &["Clear filters"],
        &[&bom(26)],
    )
    .await;
    t.app.acting_as(&keeper);
    let json = t.app.request().get("/boms").await;
    json.assert_ok();
    assert_eq!(json.text().matches("\"status_code\":\"draft\"").count(), 27);
    // A user who may not view bills is refused.
    t.browse(&t.clerk("Carl").await, "/boms")
        .await
        .assert_forbidden();
}
