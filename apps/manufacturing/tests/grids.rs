//! The list screens of the vertical are the standard grid (#556, #523): rows
//! from the database, the scope of the person in the query, search, sort,
//! filters, paging, the legacy `?status=` links, the empty state in words,
//! and a dispatch that is a link only for who may open it.

mod support;

use manufacturing::WorkOrderStatus;
use support::{Booted, PANEL, boot};

/// The place of `needle` in `text`.
fn at(text: &str, needle: &str) -> usize {
    text.find(needle)
        .unwrap_or_else(|| panic!("`{needle}` is not on the page"))
}

/// The rows of a grid page: how many links to `prefix<id>` it holds.
fn rows(text: &str, prefix: &str) -> usize {
    let mut seen = std::collections::BTreeSet::new();
    for part in text.split(&format!("href=\"{prefix}")).skip(1) {
        seen.insert(part.split('"').next().unwrap_or_default().to_owned());
    }
    seen.len()
}

/// The labels of the buttons in the page header.
fn header_labels(html: &str) -> Vec<String> {
    let start = html
        .find("rx-page-header__actions")
        .expect("header actions");
    let header = &html[start
        ..html[start..]
            .find("</div>")
            .map_or(html.len(), |e| start + e)];
    header
        .split("rx-button__label\">")
        .skip(1)
        .map(|part| part.split('<').next().unwrap_or_default().to_owned())
        .collect()
}

/// The stock module's item, as the grids search it.
async fn item(t: &Booted) {
    renox::db::sql(format!(
        "INSERT INTO wl_inventory_items (id, code, name_en, base_unit) VALUES ({PANEL}, 'PNL-9', 'Moulded panel', 'pcs')"
    ))
    .execute(t.db())
    .await
    .unwrap();
}

/// The work order list: only the plants of the person, 25 a page, sorted,
/// searched on the number, the line and the item, filtered by the grid and by
/// the old `?status=` links, JSON for the rest.
#[renox::test]
async fn the_work_order_list_is_a_grid_with_the_scope_in_the_query() {
    let t = boot().await;
    item(&t).await;
    let (a, b) = (t.plant("Plant A").await, t.plant("Plant B").await);
    let (line_a, line_b) = (t.line(&a, "LA").await, t.line(&b, "LB").await);
    let planner = t.planner("Amal", &a).await;
    let mut ours = Vec::new();
    for _ in 0..27 {
        ours.push(t.fixture(&a, &line_a, WorkOrderStatus::Planned).await);
    }
    let running = t.fixture(&a, &line_a, WorkOrderStatus::InProgress).await;
    let done = t.fixture(&a, &line_a, WorkOrderStatus::Completed).await;
    let theirs = t.fixture(&b, &line_b, WorkOrderStatus::Planned).await;
    let all = 29;

    let page = t.browse(&planner, "/manufacturing/work-orders").await;
    page.assert_ok();
    let text = page.text();
    assert!(text.contains("rx-grid"), "the list is a grid");
    assert_eq!(rows(&text, "/manufacturing/work-orders/"), 25);
    assert!(
        text.contains(&format!("of {all}")),
        "the pager counts {all}"
    );
    assert!(
        !text.contains(&theirs.number),
        "the other plant's order is hidden"
    );
    // Newest first; page 2 holds the oldest.
    assert!(text.contains(&done.number));
    let second = t
        .browse(&planner, "/manufacturing/work-orders?page=2")
        .await;
    let second = second.text();
    assert_eq!(rows(&second, "/manufacturing/work-orders/"), all - 25);
    assert!(second.contains(&ours[0].number));
    assert!(!second.contains(&done.number));

    // Sort on the number, both ways.
    let up = t
        .browse(
            &planner,
            "/manufacturing/work-orders?sort=number&per_page=50",
        )
        .await
        .text();
    assert!(at(&up, &ours[0].number) < at(&up, &ours[5].number));
    let down = t
        .browse(
            &planner,
            "/manufacturing/work-orders?sort=-number&per_page=50",
        )
        .await
        .text();
    assert!(at(&down, &ours[5].number) < at(&down, &ours[0].number));

    // Search: the number, the line code and the item.
    let found = t
        .browse(
            &planner,
            &format!("/manufacturing/work-orders?search={}", ours[3].number),
        )
        .await
        .text();
    assert!(found.contains(&ours[3].number) && !found.contains(&ours[4].number));
    assert_eq!(rows(&found, "/manufacturing/work-orders/"), 1);
    let by_item = t
        .browse(&planner, "/manufacturing/work-orders?search=Moulded")
        .await
        .text();
    assert!(by_item.contains("of 29"), "every order is of the panel");
    let nothing = t
        .browse(&planner, "/manufacturing/work-orders?search=zzzz")
        .await
        .text();
    assert_eq!(rows(&nothing, "/manufacturing/work-orders/"), 0);
    assert!(nothing.contains("Nothing matches"), "{nothing}");

    // The grid's status filter and the links of the dashboard tiles.
    for link in ["?in.status=in_progress", "?status=in_progress"] {
        let only = t
            .browse(&planner, &format!("/manufacturing/work-orders{link}"))
            .await
            .text();
        assert_eq!(rows(&only, "/manufacturing/work-orders/"), 1, "{link}");
        assert!(only.contains(&running.number));
    }
    let open = t
        .browse(
            &planner,
            "/manufacturing/work-orders?status=open&per_page=50",
        )
        .await
        .text();
    assert!(open.contains(&running.number) && !open.contains(&done.number));
    assert_eq!(rows(&open, "/manufacturing/work-orders/"), 28);

    // JSON for everything else keeps the scope and the filter.
    let json = t
        .get(&planner, "/manufacturing/work-orders?status=completed")
        .await;
    assert!(json.text().contains(&done.number));
    assert!(!json.text().contains(&running.number));
    // A person of plant B sees only its own.
    let other = t.planner("Bilal", &b).await;
    let mine = t.browse(&other, "/manufacturing/work-orders").await.text();
    assert_eq!(rows(&mine, "/manufacturing/work-orders/"), 1);
    assert!(mine.contains(&theirs.number));
}

/// With no work order the list says what it is and what fills it.
#[renox::test]
async fn the_empty_lists_say_what_they_are_for() {
    let t = boot().await;
    let plant = t.plant("Plant A").await;
    let planner = t.planner("Amal", &plant).await;
    let inspector = t.inspector("Ines", &plant).await;
    let sales = t.sales("Sara").await;
    let manager = t.manager("Mona", &plant).await;
    for (who, path, words) in [
        (&planner, "/manufacturing/work-orders", "No work orders"),
        (&inspector, "/manufacturing/ncrs", "No NCRs"),
        (&manager, "/manufacturing/aborted", "No aborted work orders"),
        (&planner, "/manufacturing/conformity", "No item is flagged"),
        (&inspector, "/manufacturing/inspections", "No lot came back"),
        (&sales, "/manufacturing/returns", "Nothing came back"),
    ] {
        let page = t.browse(who, path).await;
        page.assert_ok();
        let text = page.text();
        assert!(text.contains("rx-grid"), "{path} is a grid");
        assert!(text.contains(words), "{path} says `{words}`");
    }
}

/// The NCR list: the plant's NCRs only, one header link per meaning (the
/// *Open* state is the grid's status filter, the header says *To work on*),
/// sort and the legacy `?status=active` link.
#[renox::test]
async fn the_ncr_list_has_no_label_twice_and_keeps_the_scope() {
    let t = boot().await;
    let (a, b) = (t.plant("Plant A").await, t.plant("Plant B").await);
    let ours = t.inspector("Ines", &a).await;
    let theirs = t.inspector("Ivan", &b).await;
    t.lots.put(5, "L5", None, true, 20_000, 2_000, "released");
    t.lots.put(6, "L6", None, true, 20_000, 2_000, "released");
    let first = t.ncr(&ours, &a, 5, "4").await;
    let second = t.ncr(&ours, &a, 6, "9").await;
    let other = t.ncr(&theirs, &b, 6, "2").await;
    let closed = t.ncr_now(second.id).await;
    renox::db::sql(format!(
        "UPDATE manufacturing_ncrs SET status = 'closed' WHERE id = {}",
        closed.id
    ))
    .execute(t.db())
    .await
    .unwrap();

    let page = t.browse(&ours, "/manufacturing/ncrs").await;
    page.assert_ok();
    let html = page.text();
    assert!(html.contains(&first.number) && html.contains(&second.number));
    assert!(
        !html.contains(&other.number),
        "the other plant's NCR is hidden"
    );
    let labels = header_labels(&html);
    let mut unique = labels.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(
        unique.len(),
        labels.len(),
        "a header label twice: {labels:?}"
    );
    assert!(!labels.contains(&"Open".to_owned()), "{labels:?}");
    assert!(labels.contains(&"To work on".to_owned()), "{labels:?}");

    // Sort by quantity, both ways.
    let up = t
        .browse(&ours, "/manufacturing/ncrs?sort=quantity_milli")
        .await
        .text();
    assert!(at(&up, &first.number) < at(&up, &second.number));
    let down = t
        .browse(&ours, "/manufacturing/ncrs?sort=-quantity_milli")
        .await
        .text();
    assert!(at(&down, &second.number) < at(&down, &first.number));
    // The old link and the grid filter.
    let active = t
        .browse(&ours, "/manufacturing/ncrs?status=active")
        .await
        .text();
    assert!(active.contains(&first.number) && !active.contains(&second.number));
    let by_filter = t
        .browse(&ours, "/manufacturing/ncrs?in.status=closed")
        .await
        .text();
    assert!(by_filter.contains(&second.number) && !by_filter.contains(&first.number));
    let found = t
        .browse(
            &ours,
            &format!("/manufacturing/ncrs?search={}", first.number),
        )
        .await
        .text();
    assert_eq!(rows(&found, "/manufacturing/ncrs/"), 1);
}

/// The aborted work orders: the plants of the person only, the costs only for
/// who may see them of every order listed.
#[renox::test]
async fn the_aborted_list_shows_the_plants_of_the_person_and_hides_costs_from_a_planner() {
    let t = boot().await;
    let (a, b) = (t.plant("Plant A").await, t.plant("Plant B").await);
    let (line_a, line_b) = (t.line(&a, "LA").await, t.line(&b, "LB").await);
    let ours = t.fixture(&a, &line_a, WorkOrderStatus::Aborted).await;
    let theirs = t.fixture(&b, &line_b, WorkOrderStatus::Aborted).await;
    t.fixture(&a, &line_a, WorkOrderStatus::Planned).await;
    renox::db::sql(format!(
        "UPDATE manufacturing_work_orders SET abort_reason = 'Wrong mould', abort_loss = 58600, abort_scrap_value = 1200, aborted_at = '2026-10-20 08:00:00' WHERE id = {}",
        ours.id
    ))
    .execute(t.db())
    .await
    .unwrap();
    let manager = t.manager("Mona", &a).await;
    let planner = t.planner("Amal", &a).await;

    let page = t.browse(&manager, "/manufacturing/aborted").await;
    page.assert_ok();
    let text = page.text();
    assert!(text.contains(&ours.number) && text.contains("Wrong mould"));
    assert!(!text.contains(&theirs.number));
    assert_eq!(rows(&text, "/manufacturing/work-orders/"), 1);
    assert!(text.contains("Loss (AED)"), "the manager sees the loss");
    let planned = t.browse(&planner, "/manufacturing/aborted").await.text();
    assert!(planned.contains(&ours.number));
    assert!(
        !planned.contains("Loss (AED)"),
        "a planner does not see costs"
    );
    let found = t
        .browse(&manager, "/manufacturing/aborted?search=mould")
        .await
        .text();
    assert_eq!(rows(&found, "/manufacturing/work-orders/"), 1);
    let none = t
        .browse(&manager, "/manufacturing/aborted?search=zzzz")
        .await
        .text();
    assert_eq!(rows(&none, "/manufacturing/work-orders/"), 0);
}
