//! The list screens of F7 on the demo company (#556, #523): each is the
//! standard grid with rows from the seed, sorted, searched, filtered and
//! paged through its query string, restricted to the viewer, and without a
//! link that ends in a 403.

mod manufacturing_demo_support;
mod support;

use host::manufacturing_demo::{
    ACCOUNTANT_EMAIL, MANAGER_EMAIL, OPERATOR_EMAIL, OWNER_EMAIL, PLANNER_EMAIL, QC_EMAIL,
    SALES_EMAIL, SALES_MANAGER_EMAIL,
};
use manufacturing_demo_support::demo;
use renox::prelude::*;
use renox::testing::TestApp;

/// The page of `path` for `email`, as a browser asks for it.
async fn page(app: &TestApp, email: &str, path: &str) -> String {
    let user = User::find_by_email(app.db(), email)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("the demo login {email}"));
    app.acting_as(&user);
    let response = app
        .request()
        .header("accept", "text/html,application/xhtml+xml,*/*;q=0.8")
        .get(path)
        .await;
    assert_eq!(response.status.as_u16(), 200, "{path} as {email}");
    response.text()
}

/// How many different records the rows of a page link to under `prefix`.
fn rows(html: &str, prefix: &str) -> usize {
    let mut seen = std::collections::BTreeSet::new();
    for part in html.split(&format!("href=\"{prefix}")).skip(1) {
        seen.insert(part.split('"').next().unwrap_or_default().to_owned());
    }
    seen.len()
}

/// The place of `needle` in `html`.
fn at(html: &str, needle: &str) -> usize {
    html.find(needle)
        .unwrap_or_else(|| panic!("`{needle}` is not on the page"))
}

async fn scalar(app: &TestApp, query: &str) -> i64 {
    renox::db::sql(query.to_owned())
        .scalar::<i64>(app.db())
        .await
        .unwrap()
}

/// 33 work orders: 25 on the first page, the rest on the second; the tile
/// links (`?status=`) and the grid's own filter narrow the same query; sorted
/// and searched on the number, the line and the item.
#[renox::test]
async fn the_work_orders_page_search_sort_and_filter() {
    let app = demo().await;
    let total = scalar(&app, "select count(*) from manufacturing_work_orders").await as usize;
    assert!(total > 25, "{total} work orders: more than a page");
    let path = "/manufacturing/work-orders";
    let first = page(&app, MANAGER_EMAIL, path).await;
    assert!(first.contains("rx-grid"));
    assert_eq!(rows(&first, "/manufacturing/work-orders/"), 25);
    assert!(
        first.contains(&format!("of {total}")),
        "the pager counts {total}"
    );
    let second = page(&app, MANAGER_EMAIL, &format!("{path}?page=2")).await;
    assert_eq!(rows(&second, "/manufacturing/work-orders/"), total - 25);
    // Newest first by default; the number sorts both ways.
    let up = page(
        &app,
        MANAGER_EMAIL,
        &format!("{path}?sort=number&per_page=50"),
    )
    .await;
    assert!(at(&up, "WO-2026-0001") < at(&up, "WO-2026-0002"));
    let down = page(
        &app,
        MANAGER_EMAIL,
        &format!("{path}?sort=-number&per_page=50"),
    )
    .await;
    assert!(at(&down, "WO-2026-0002") < at(&down, "WO-2026-0001"));
    // Search: the number, and the item's name through the stock module.
    let one = page(&app, MANAGER_EMAIL, &format!("{path}?search=WO-2026-0003")).await;
    assert_eq!(rows(&one, "/manufacturing/work-orders/"), 1);
    let boxes = page(
        &app,
        MANAGER_EMAIL,
        &format!("{path}?search=Food-grade&per_page=50"),
    )
    .await;
    let food = scalar(
        &app,
        "select count(*) from manufacturing_work_orders w join wl_inventory_items i on i.id = w.item_id where i.code = 'BOX-FOOD'",
    )
    .await as usize;
    assert_eq!(rows(&boxes, "/manufacturing/work-orders/"), food);
    // The links of the dashboard tiles and the grid filter.
    for link in ["?status=on_hold", "?in.status=on_hold"] {
        let held = page(&app, MANAGER_EMAIL, &format!("{path}{link}")).await;
        assert_eq!(rows(&held, "/manufacturing/work-orders/"), 1, "{link}");
    }
    let open = page(
        &app,
        MANAGER_EMAIL,
        &format!("{path}?status=open&per_page=50"),
    )
    .await;
    let open_n = scalar(
        &app,
        "select count(*) from manufacturing_work_orders where status in ('planned','released','in_progress','on_hold')",
    )
    .await as usize;
    assert_eq!(rows(&open, "/manufacturing/work-orders/"), open_n);
    let none = page(&app, MANAGER_EMAIL, &format!("{path}?search=zzzz")).await;
    assert_eq!(rows(&none, "/manufacturing/work-orders/"), 0);
    assert!(none.contains("Nothing matches"));
    // The operator of line 1 sees work orders of line 1 only.
    let operator = page(&app, OPERATOR_EMAIL, &format!("{path}?per_page=50")).await;
    let on_line_1 = scalar(
        &app,
        "select count(*) from manufacturing_work_orders w join manufacturing_lines l on l.id = w.line_id where l.code = 'L1'",
    )
    .await as usize;
    assert_eq!(rows(&operator, "/manufacturing/work-orders/"), on_line_1);
    assert!(on_line_1 < total, "line 2 is hidden from the operator");
}

/// The aborted work order the production manager approved is listed with who
/// asked and who approved, and the loss for who may see costs.
#[renox::test]
async fn the_aborted_list_has_the_approved_abort() {
    let app = demo().await;
    let manager = page(&app, MANAGER_EMAIL, "/manufacturing/aborted").await;
    assert_eq!(rows(&manager, "/manufacturing/work-orders/"), 1);
    for word in [
        "Customer withdrew the order",
        "Amal Youssef",
        "Mona Farouk",
        "Loss (AED)",
        "Scrap value (AED)",
    ] {
        assert!(manager.contains(word), "{word}");
    }
    let planner = page(&app, PLANNER_EMAIL, "/manufacturing/aborted").await;
    assert_eq!(rows(&planner, "/manufacturing/work-orders/"), 1);
    assert!(
        !planner.contains("Loss (AED)"),
        "a planner does not see costs"
    );
    let found = page(
        &app,
        MANAGER_EMAIL,
        "/manufacturing/aborted?search=withdrew",
    )
    .await;
    assert_eq!(rows(&found, "/manufacturing/work-orders/"), 1);
    let none = page(&app, MANAGER_EMAIL, "/manufacturing/aborted?search=zzzz").await;
    assert_eq!(rows(&none, "/manufacturing/work-orders/"), 0);
}

/// The returned lots: one waiting, one passed, one failed; the dispatch is
/// plain text for the QC inspector (a link would answer 403); the NCR list has
/// no label twice.
#[renox::test]
async fn the_returned_lots_show_each_state_and_no_dead_link() {
    let app = demo().await;
    let all = page(&app, QC_EMAIL, "/manufacturing/inspections").await;
    for word in [
        "Waiting",
        "Passed",
        "Failed",
        "damaged in transit",
        "boxes arrived wet",
    ] {
        assert!(all.contains(word), "{word}");
    }
    assert!(
        !all.contains("href=\"/dispatches/"),
        "no link to a page the QC may not open"
    );
    assert!(all.contains("DSP-2026-0001"), "the number is still read");
    for (filter, word, other) in [
        ("?status=open", "damaged in transit", "boxes arrived wet"),
        (
            "?in.status=passed",
            "printing out of register",
            "boxes arrived wet",
        ),
        (
            "?in.status=failed",
            "boxes arrived wet",
            "printing out of register",
        ),
    ] {
        let some = page(
            &app,
            QC_EMAIL,
            &format!("/manufacturing/inspections{filter}"),
        )
        .await;
        assert!(some.contains(word) && !some.contains(other), "{filter}");
    }
    let up = page(
        &app,
        QC_EMAIL,
        "/manufacturing/inspections?sort=quantity_milli",
    )
    .await;
    assert!(at(&up, "boxes arrived wet") < at(&up, "damaged in transit"));
    let found = page(&app, QC_EMAIL, "/manufacturing/inspections?search=register").await;
    assert!(found.contains("printing out of register"));
    assert!(!found.contains("boxes arrived wet"));
    // The failed one raised an NCR; the list of NCRs says each label once.
    let ncrs = page(&app, QC_EMAIL, "/manufacturing/ncrs").await;
    let start = at(&ncrs, "rx-page-header__actions");
    let header = &ncrs[start..start + ncrs[start..].find("</div>").unwrap()];
    let labels: Vec<&str> = header
        .split("rx-button__label\">")
        .skip(1)
        .map(|part| part.split('<').next().unwrap())
        .collect();
    let mut unique = labels.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), labels.len(), "{labels:?}");
    assert!(!labels.contains(&"Open"), "{labels:?}");
}

/// The export evidence: the accountant reads the dispatch number as text, the
/// state is a filter and the days sort by the day the goods left.
#[renox::test]
async fn the_export_evidence_has_no_dead_dispatch_link_and_filters_by_state() {
    let app = demo().await;
    let all = page(&app, ACCOUNTANT_EMAIL, "/manufacturing/exports").await;
    assert!(
        !all.contains("href=\"/dispatches/"),
        "the accountant may not open dispatches"
    );
    assert!(all.contains("DSP-2026-0004") && all.contains("DSP-2026-0005"));
    assert!(all.contains("Attach evidence"));
    let warned = page(
        &app,
        ACCOUNTANT_EMAIL,
        "/manufacturing/exports?in.state=warned",
    )
    .await;
    assert!(warned.contains("DSP-2026-0004") && !warned.contains("DSP-2026-0005"));
    let done = page(
        &app,
        ACCOUNTANT_EMAIL,
        "/manufacturing/exports?in.state=evidence",
    )
    .await;
    assert!(done.contains("DSP-2026-0005") && !done.contains("DSP-2026-0004"));
    let old_first = page(
        &app,
        ACCOUNTANT_EMAIL,
        "/manufacturing/exports?sort=dispatched_on",
    )
    .await;
    assert!(at(&old_first, "DSP-2026-0004") < at(&old_first, "DSP-2026-0005"));
    let new_first = page(
        &app,
        ACCOUNTANT_EMAIL,
        "/manufacturing/exports?sort=-dispatched_on",
    )
    .await;
    assert!(at(&new_first, "DSP-2026-0005") < at(&new_first, "DSP-2026-0004"));
    let found = page(
        &app,
        ACCOUNTANT_EMAIL,
        "/manufacturing/exports?search=DSP-2026-0005",
    )
    .await;
    assert!(found.contains("DSP-2026-0005") && !found.contains("DSP-2026-0004"));
}

/// Returned goods: sales sees every line, filters and searches them, the
/// dispatch and the order link (and open).
#[renox::test]
async fn the_returned_goods_search_and_filter() {
    let app = demo().await;
    let all = page(&app, SALES_EMAIL, "/manufacturing/returns").await;
    for word in [
        "SO-2026-0001",
        "damaged in transit",
        "boxes arrived wet",
        "New dispatch",
    ] {
        assert!(all.contains(word), "{word}");
    }
    assert!(all.contains("href=\"/sales-orders/"), "the order links");
    // Sales holds `stock.view` everywhere, so the dispatch is a link, and the
    // link opens.
    let link = all
        .split("href=\"/dispatches/")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("a dispatch link for sales")
        .to_owned();
    page(&app, SALES_EMAIL, &format!("/dispatches/{link}")).await;
    let wet = page(&app, SALES_EMAIL, "/manufacturing/returns?search=wet").await;
    assert!(wet.contains("boxes arrived wet") && !wet.contains("damaged in transit"));
    let open = page(&app, SALES_EMAIL, "/manufacturing/returns?status=open").await;
    assert!(open.contains("damaged in transit"));
    let closed = page(&app, SALES_EMAIL, "/manufacturing/returns?in.status=closed").await;
    assert!(!closed.contains("damaged in transit"));
    let up = page(
        &app,
        SALES_EMAIL,
        "/manufacturing/returns?sort=refused_milli",
    )
    .await;
    assert!(at(&up, "boxes arrived wet") < at(&up, "damaged in transit"));
}

/// The flagged items, the recalls (one reaching a shipped lot) and the BOMs.
#[renox::test]
async fn conformity_recalls_and_boms_are_grids() {
    let app = demo().await;
    let conformity = page(&app, MANAGER_EMAIL, "/manufacturing/conformity").await;
    for code in ["BOX-FOOD", "BOX-RTL", "SLV-EPH", "BOX-EXP"] {
        assert!(conformity.contains(code), "{code}");
    }
    let found = page(
        &app,
        MANAGER_EMAIL,
        "/manufacturing/conformity?search=SLV-EPH",
    )
    .await;
    assert!(found.contains("SLV-EPH") && !found.contains("BOX-EXP"));
    let sorted = page(
        &app,
        MANAGER_EMAIL,
        "/manufacturing/conformity?sort=-item_code",
    )
    .await;
    assert!(at(&sorted, "SLV-EPH") < at(&sorted, "BOX-EXP"));

    let recalls = page(&app, MANAGER_EMAIL, "/manufacturing/recalls").await;
    assert_eq!(rows(&recalls, "/manufacturing/recalls/"), 2);
    let peel = page(&app, MANAGER_EMAIL, "/manufacturing/recalls?search=peel").await;
    assert_eq!(rows(&peel, "/manufacturing/recalls/"), 1);
    let traced = page(
        &app,
        MANAGER_EMAIL,
        "/manufacturing/recalls?in.status=traced",
    )
    .await;
    assert_eq!(rows(&traced, "/manufacturing/recalls/"), 2);
    let none = page(
        &app,
        MANAGER_EMAIL,
        "/manufacturing/recalls?in.status=closed",
    )
    .await;
    assert_eq!(rows(&none, "/manufacturing/recalls/"), 0);
    // The recall of the finished lot reaches a shipped dispatch and one that
    // has not left, flagged for a re-pick.
    let finished = scalar(
        &app,
        "select id from wl_inventory_recalls where reason like '%peel test%'",
    )
    .await;
    let trace = page(
        &app,
        MANAGER_EMAIL,
        &format!("/manufacturing/recalls/{finished}"),
    )
    .await;
    assert!(
        trace.contains("Re-pick asked of the storekeeper"),
        "the re-pick flag"
    );
    assert!(trace.contains("Shipped"));

    let boms = page(&app, MANAGER_EMAIL, "/boms").await;
    assert_eq!(rows(&boms, "/boms/"), 3);
    assert!(boms.contains("Materials per unit (AED)"));
    let found = page(&app, MANAGER_EMAIL, "/boms?search=SLV-EPH").await;
    assert_eq!(rows(&found, "/boms/"), 1);
    let draftless = page(&app, MANAGER_EMAIL, "/boms?in.status=draft").await;
    assert_eq!(rows(&draftless, "/boms/"), 0);
    let released = page(&app, MANAGER_EMAIL, "/boms?status=released").await;
    assert_eq!(rows(&released, "/boms/"), 3);
    let planner = page(&app, PLANNER_EMAIL, "/boms").await;
    assert!(
        !planner.contains("Materials per unit"),
        "no cost for a planner"
    );
}

/// The credit accounts: five customers, found by name, each linking to its
/// account; the limit sorts; the owed figure is drawn.
#[renox::test]
async fn the_credit_accounts_are_a_grid_with_names_and_links() {
    let app = demo().await;
    let all = page(&app, SALES_MANAGER_EMAIL, "/credit/accounts").await;
    assert_eq!(rows(&all, "/credit/accounts/"), 5);
    for name in [
        "Al Noor Trading LLC",
        "Sahara Retail LLC",
        "Atlas Gulf Export FZE",
    ] {
        assert!(all.contains(name), "{name}");
    }
    assert!(all.contains("Credit limit (AED)") && all.contains("Owes now (AED)"));
    let found = page(&app, SALES_MANAGER_EMAIL, "/credit/accounts?search=Sahara").await;
    assert_eq!(rows(&found, "/credit/accounts/"), 1);
    assert!(found.contains("Sahara Retail LLC") && !found.contains("Al Noor"));
    let big = page(
        &app,
        SALES_MANAGER_EMAIL,
        "/credit/accounts?sort=-credit_limit",
    )
    .await;
    assert!(at(&big, "Atlas Gulf Export FZE") < at(&big, "Emirates Print House LLC"));
    let none = page(&app, SALES_MANAGER_EMAIL, "/credit/accounts?search=zzzz").await;
    assert_eq!(rows(&none, "/credit/accounts/"), 0);
    let good = page(
        &app,
        SALES_MANAGER_EMAIL,
        "/credit/accounts?in.standing=good",
    )
    .await;
    assert_eq!(rows(&good, "/credit/accounts/"), 5, "no account is on hold");
    // The owner opens none of the lists of the plant.
    let owner = page(&app, OWNER_EMAIL, "/recalls").await;
    assert!(owner.contains("rx-grid"));
}
