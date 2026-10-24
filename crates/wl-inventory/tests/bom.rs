//! The bill of materials (Base 07, `STK.BOM.01`, 07 §7.1 *BOM*, `MFG.ACL.06`):
//! a draft is started, filled and released, the cost of the materials is
//! rolled up, the older released version becomes obsolete, a released version
//! is locked, and the work order reads it through the kernel's
//! `BillsOfMaterials`.

mod support;

use renox::prelude::*;
use renox::testing::TestResponse;
use wl_inventory::{Bom, BomStatus, Item};
use wl_kernel::Subject;
use wl_kernel::production::{BOM_SUBJECT_TYPE, boms};

use support::{Booted, boot};

/// A form post as `by`, the way htmx sends it.
async fn post(t: &Booted, by: &User, path: &str, form: &[(&str, &str)]) -> TestResponse {
    t.app.acting_as(by);
    t.app.htmx().post(path, form).await
}

/// The id a redirect landed on.
fn landed(response: &TestResponse) -> i64 {
    response
        .header("location")
        .and_then(|location| location.rsplit('/').next())
        .and_then(|id| id.parse().ok())
        .unwrap_or_else(|| panic!("a redirect to a page: {:?}", response.header("location")))
}

/// The items of a panel: resin at 8.00 a kg and film at 3.00 a m.
async fn items(t: &Booted) -> (Item, Item, Item) {
    let panel = t.priced_item("PANEL", false, 0).await;
    let resin = t.priced_item("RESIN", false, 800).await;
    let film = t.priced_item("FILM", false, 300).await;
    (panel, resin, film)
}

/// A draft of the panel that makes 2 at 90% yield in 1 hour from 3.6 of
/// resin with 5% scrap and 2 of film, started and filled through the forms
/// as the keeper.
async fn drafted(t: &Booted, keeper: &User, panel: &Item, resin: &Item, film: &Item) -> i64 {
    let made = post(
        t,
        keeper,
        "/boms",
        &[
            ("item", &panel.id.to_string()),
            ("output", "2"),
            ("yield_percent", "90"),
            ("hours", "1"),
        ],
    )
    .await;
    let id = landed(&made);
    for (component, quantity, scrap) in [(resin, "3.6", "5"), (film, "2", "")] {
        post(
            t,
            keeper,
            &format!("/boms/{id}/lines"),
            &[
                ("item", &component.id.to_string()),
                ("quantity", quantity),
                ("scrap", scrap),
            ],
        )
        .await
        .assert_ok();
    }
    id
}

/// The visible text of a page: tags dropped.
fn visible(html: &str) -> String {
    let mut text = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                text.push(' ');
            }
            c if !in_tag => text.push(c),
            _ => {}
        }
    }
    text.replace("&#39;", "'").replace("&amp;", "&")
}

/// A draft is filled and released: the cost of the materials of one unit is
/// rolled up at the average cost with the scrap and the yield, and the kernel
/// gives the work order the components of one unit, the cost and the hours.
#[renox::test]
async fn a_released_bom_gives_the_work_order_its_components_cost_and_hours() {
    let t = boot().await;
    let (panel, resin, film) = items(&t).await;
    let keeper = t.bom_keeper("Kim").await;
    let releaser = t.bom_releaser("Rae").await;
    let id = drafted(&t, &keeper, &panel, &resin, &film).await;
    let subject = Subject::new(BOM_SUBJECT_TYPE, id);

    // A draft is not released for a work order.
    let draft = boms(t.state())
        .version(t.state(), &subject)
        .await
        .unwrap()
        .expect("the draft is known");
    assert!(!draft.released);

    post(&t, &releaser, &format!("/boms/{id}/release"), &[])
        .await
        .assert_ok();
    let released = Bom::find_or_404(t.db(), id).await.unwrap();
    assert_eq!(released.status, BomStatus::Released);
    assert_eq!(released.version, 1);
    // Resin 3600 x 1.05 x 800 / (2 x 0.9) = 1680; film 2000 x 300 / 1800 = 333.
    assert_eq!(released.material_cost_per_unit, 1_680 + 333);
    assert_eq!(released.released_by, Some(releaser.id));

    let version = boms(t.state())
        .version(t.state(), &subject)
        .await
        .unwrap()
        .expect("the version");
    assert!(version.released);
    assert_eq!(version.item, Subject::new("inventory.item", panel.id));
    assert_eq!(version.material_cost_per_unit, 2_013);
    // One hour for two units.
    assert_eq!(version.standard_milli_hours_per_unit, 500);
    assert_eq!(version.components.len(), 2);
    assert_eq!(version.components[0].item.subject_id, resin.id);
    // 3.6 kg for 2 units at 90% yield: 2 kg of resin a good unit, 5% scrap on top.
    assert_eq!(version.components[0].quantity_milli, 2_000);
    assert_eq!(version.components[0].scrap_bps, 500);
    assert_eq!(version.components[1].quantity_milli, 1_111);

    // Another kind of subject, or an unknown number, is not a BOM.
    assert!(
        boms(t.state())
            .version(t.state(), &Subject::new(BOM_SUBJECT_TYPE, 999))
            .await
            .unwrap()
            .is_none()
    );
}

/// A new version copies the lines of a released one; releasing it makes the
/// older version obsolete and only the new one is released for a work order.
#[renox::test]
async fn releasing_a_new_version_makes_the_older_one_obsolete() {
    let t = boot().await;
    let (panel, resin, film) = items(&t).await;
    let keeper = t.bom_keeper("Kim").await;
    let releaser = t.bom_releaser("Rae").await;
    let first = drafted(&t, &keeper, &panel, &resin, &film).await;
    post(&t, &releaser, &format!("/boms/{first}/release"), &[])
        .await
        .assert_ok();

    let copy = post(&t, &keeper, &format!("/boms/{first}/new-version"), &[]).await;
    let second = landed(&copy);
    let draft = Bom::find_or_404(t.db(), second).await.unwrap();
    assert_eq!((draft.version, draft.status), (2, BomStatus::Draft));
    assert_eq!(
        wl_inventory::bom::lines_of(t.db(), second)
            .await
            .unwrap()
            .len(),
        2
    );
    // One draft at a time.
    let again = post(&t, &keeper, &format!("/boms/{first}/new-version"), &[]).await;
    again.assert_status(422);
    assert!(again.text().contains("has a draft version already"));

    post(&t, &releaser, &format!("/boms/{second}/release"), &[])
        .await
        .assert_ok();
    let old = Bom::find_or_404(t.db(), first).await.unwrap();
    assert_eq!(old.status, BomStatus::Obsolete);
    assert!(old.obsoleted_at.is_some());
    let new = Bom::find_or_404(t.db(), second).await.unwrap();
    assert_eq!(new.status, BomStatus::Released);
    let known = boms(t.state());
    let released = |id: i64| {
        let known = known.clone();
        let state = t.state().clone();
        async move {
            known
                .version(&state, &Subject::new(BOM_SUBJECT_TYPE, id))
                .await
                .unwrap()
                .unwrap()
                .released
        }
    };
    assert!(!released(first).await);
    assert!(released(second).await);
}

/// A bill without a component is not released, and the refusal says so in
/// words; so are a component that is the item made, one listed twice and
/// quantities that are not above zero.
#[renox::test]
async fn a_bom_with_nothing_to_release_or_a_bad_line_is_refused_in_words() {
    let t = boot().await;
    let (panel, resin, _film) = items(&t).await;
    let keeper = t.bom_keeper("Kim").await;
    let releaser = t.bom_releaser("Rae").await;
    let made = post(&t, &keeper, "/boms", &[("item", &panel.id.to_string())]).await;
    let id = landed(&made);
    let empty = post(&t, &releaser, &format!("/boms/{id}/release"), &[]).await;
    empty.assert_status(422);
    assert!(empty.text().contains("needs at least one component"));
    assert_eq!(
        Bom::find_or_404(t.db(), id).await.unwrap().status,
        BomStatus::Draft
    );

    let lines = format!("/boms/{id}/lines");
    let itself = post(
        &t,
        &keeper,
        &lines,
        &[("item", &panel.id.to_string()), ("quantity", "1")],
    )
    .await;
    itself.assert_status(422);
    assert!(itself.text().contains("cannot be its own component"));
    let zero = post(
        &t,
        &keeper,
        &lines,
        &[("item", &resin.id.to_string()), ("quantity", "0")],
    )
    .await;
    zero.assert_status(422);
    assert!(zero.text().contains("quantity must be above zero"));
    let scrap = post(
        &t,
        &keeper,
        &lines,
        &[
            ("item", &resin.id.to_string()),
            ("quantity", "1"),
            ("scrap", "120"),
        ],
    )
    .await;
    scrap.assert_status(422);
    assert!(scrap.text().contains("scrap must be between"));
    post(
        &t,
        &keeper,
        &lines,
        &[("item", &resin.id.to_string()), ("quantity", "1")],
    )
    .await
    .assert_ok();
    let twice = post(
        &t,
        &keeper,
        &lines,
        &[("item", &resin.id.to_string()), ("quantity", "2")],
    )
    .await;
    twice.assert_status(422);
    assert!(twice.text().contains("is a component already"));
    let yields = post(
        &t,
        &keeper,
        &format!("/boms/{id}/edit"),
        &[("output", "1"), ("yield_percent", "0"), ("hours", "0")],
    )
    .await;
    yields.assert_status(422);
    assert!(yields.text().contains("yield must be above 0%"));
}

/// A released bill is locked (`MFG.ACL.06`): no component can be added or
/// removed and the header is not changed; a draft's lines can be removed.
#[renox::test]
async fn a_released_bom_is_locked_and_a_draft_can_lose_a_line() {
    let t = boot().await;
    let (panel, resin, film) = items(&t).await;
    let keeper = t.bom_keeper("Kim").await;
    let releaser = t.bom_releaser("Rae").await;
    let id = drafted(&t, &keeper, &panel, &resin, &film).await;
    let lines = wl_inventory::bom::lines_of(t.db(), id).await.unwrap();
    post(
        &t,
        &keeper,
        &format!("/boms/{id}/lines/{}/remove", lines[1].id),
        &[],
    )
    .await
    .assert_ok();
    assert_eq!(
        wl_inventory::bom::lines_of(t.db(), id).await.unwrap().len(),
        1
    );
    post(&t, &releaser, &format!("/boms/{id}/release"), &[])
        .await
        .assert_ok();

    for (path, form) in [
        (
            format!("/boms/{id}/lines"),
            vec![("item", film.id.to_string()), ("quantity", "1".to_owned())],
        ),
        (
            format!("/boms/{id}/lines/{}/remove", lines[0].id),
            Vec::new(),
        ),
        (format!("/boms/{id}/edit"), vec![("output", "5".to_owned())]),
    ] {
        let form: Vec<(&str, &str)> = form.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let refused = post(&t, &keeper, &path, &form).await;
        refused.assert_status(422);
        assert!(
            refused.text().contains("Only a draft can be changed"),
            "{path}"
        );
    }
    let release_again = post(&t, &releaser, &format!("/boms/{id}/release"), &[]).await;
    assert!(release_again.status.as_u16() >= 400);
}

/// A released version is made obsolete by hand by the releaser; a draft is
/// not.
#[renox::test]
async fn a_released_version_is_made_obsolete_and_a_draft_cannot_be() {
    let t = boot().await;
    let (panel, resin, film) = items(&t).await;
    let keeper = t.bom_keeper("Kim").await;
    let releaser = t.bom_releaser("Rae").await;
    let id = drafted(&t, &keeper, &panel, &resin, &film).await;
    assert!(
        post(&t, &releaser, &format!("/boms/{id}/obsolete"), &[])
            .await
            .status
            .as_u16()
            >= 400
    );
    post(&t, &releaser, &format!("/boms/{id}/release"), &[])
        .await
        .assert_ok();
    post(&t, &releaser, &format!("/boms/{id}/obsolete"), &[])
        .await
        .assert_ok();
    assert_eq!(
        Bom::find_or_404(t.db(), id).await.unwrap().status,
        BomStatus::Obsolete
    );
}

/// Who may do what: keeping needs `boms.manage`, releasing `boms.release`,
/// seeing `boms.view`; a user with none of them gets no list.
#[renox::test]
async fn the_bom_permissions_are_checked_in_the_service() {
    let t = boot().await;
    let (panel, resin, film) = items(&t).await;
    let keeper = t.bom_keeper("Kim").await;
    let releaser = t.bom_releaser("Rae").await;
    let viewer = t.bom_viewer("Vic").await;
    let nobody = t.clerk("Carl").await;
    let id = drafted(&t, &keeper, &panel, &resin, &film).await;

    let item = panel.id.to_string();
    for by in [&viewer, &releaser, &nobody] {
        post(&t, by, "/boms", &[("item", &item)])
            .await
            .assert_status(403);
        post(
            &t,
            by,
            &format!("/boms/{id}/lines"),
            &[("item", &film.id.to_string()), ("quantity", "1")],
        )
        .await
        .assert_status(403);
    }
    for by in [&keeper, &viewer, &nobody] {
        post(&t, by, &format!("/boms/{id}/release"), &[])
            .await
            .assert_status(403);
    }
    t.browse(&nobody, "/boms").await.assert_status(403);
    t.browse(&viewer, "/boms").await.assert_ok();
    assert_eq!(
        Bom::find_or_404(t.db(), id).await.unwrap().status,
        BomStatus::Draft
    );
}

/// The list and the record page: a browser gets pages, others JSON; the
/// record page shows the actions the person may run for the state and a
/// disabled one says why.
#[renox::test]
async fn the_bom_pages_show_the_versions_lines_and_the_actions_for_the_person() {
    let t = boot().await;
    let (panel, resin, film) = items(&t).await;
    let keeper = t.bom_keeper("Kim").await;
    let releaser = t.bom_releaser("Rae").await;
    let viewer = t.bom_viewer("Vic").await;
    let first = drafted(&t, &keeper, &panel, &resin, &film).await;

    let list = t.browse(&keeper, "/boms").await;
    list.assert_ok();
    let text = visible(&list.text());
    for word in [
        "Bills of materials",
        "PANEL",
        "Version",
        "Draft",
        "New bill",
    ] {
        assert!(text.contains(word), "{word}: {text}");
    }
    t.app.acting_as(&keeper);
    let json = t.app.request().get("/boms").await;
    json.assert_ok();
    assert!(json.text().contains("\"status_code\":\"draft\""));

    let draft_page = t.browse(&keeper, &format!("/boms/{first}")).await;
    draft_page.assert_ok();
    let text = visible(&draft_page.text());
    for word in [
        "PANEL",
        "RESIN",
        "FILM",
        "3.6 pcs",
        "5%",
        "90%",
        "Add a component",
        "Change the bill",
        "Worked out at release",
    ] {
        assert!(text.contains(word), "{word}: {text}");
    }
    // The keeper cannot release; the releaser can and is told nothing blocks.
    assert!(!text.contains("Locks the version"));
    let releaser_page = visible(&t.browse(&releaser, &format!("/boms/{first}")).await.text());
    assert!(releaser_page.contains("Locks the version"));
    assert!(!releaser_page.contains("Add a component"));
    let viewer_page = visible(&t.browse(&viewer, &format!("/boms/{first}")).await.text());
    assert!(!viewer_page.contains("Add a component"));
    assert!(!viewer_page.contains("Locks the version"));

    // Released: locked, a new version is offered; with a draft already it is
    // disabled and says why.
    post(&t, &releaser, &format!("/boms/{first}/release"), &[])
        .await
        .assert_ok();
    let released = visible(&t.browse(&keeper, &format!("/boms/{first}")).await.text());
    assert!(released.contains("A released bill is locked"));
    assert!(released.contains("New version"));
    // The reason "Change the bill" is disabled is on the page, not only in
    // the button's tooltip.
    assert!(
        released.contains("Only a draft can be changed."),
        "{released}"
    );
    assert!(released.contains("AED 20.13"), "{released}");
    assert!(!released.contains("Add a component"));
    post(&t, &keeper, &format!("/boms/{first}/new-version"), &[])
        .await
        .assert_status(303);
    let blocked = visible(&t.browse(&keeper, &format!("/boms/{first}")).await.text());
    assert!(blocked.contains("This item has a draft version already."));
}

/// The distinct records the rows of a grid page link to under `/boms/`.
fn bom_rows(html: &str) -> usize {
    let mut seen = std::collections::BTreeSet::new();
    for part in html.split("href=\"/boms/").skip(1) {
        seen.insert(part.split('"').next().unwrap_or_default().to_owned());
    }
    seen.len()
}

/// The list is the standard grid: searched on the item's code and name,
/// sorted, filtered by status (the old `?status=` link too), paged, with the
/// cost only for who sees what materials cost, and an empty state in words.
#[renox::test]
async fn the_bom_list_is_a_grid_that_pages_searches_and_filters() {
    let t = boot().await;
    let keeper = t.bom_keeper("Kim").await;
    let releaser = t.bom_releaser("Rae").await;
    let viewer = t.bom_viewer("Vic").await;
    let empty = t.browse(&keeper, "/boms").await.text();
    assert!(empty.contains("rx-grid") && empty.contains("No bill of materials yet"));

    let (panel, resin, film) = items(&t).await;
    let released = drafted(&t, &keeper, &panel, &resin, &film).await;
    post(&t, &releaser, &format!("/boms/{released}/release"), &[])
        .await
        .assert_ok();
    for n in 0..12 {
        let item = t.priced_item(&format!("FG-{n:02}"), false, 0).await;
        post(&t, &keeper, "/boms", &[("item", &item.id.to_string())]).await;
    }

    let first = t.browse(&keeper, "/boms?per_page=10").await.text();
    assert_eq!(bom_rows(&first), 10);
    assert!(first.contains("of 13"), "the pager counts 13");
    let second = t.browse(&keeper, "/boms?per_page=10&page=2").await.text();
    assert_eq!(bom_rows(&second), 3);
    // Search on the item's code and name.
    let found = t.browse(&keeper, "/boms?search=FG-03").await.text();
    assert_eq!(bom_rows(&found), 1);
    let found = found
        .split("<table class=\"rx-grid__table\"")
        .nth(1)
        .unwrap_or_default();
    assert!(found.contains("FG-03") && !found.contains("FG-04"));
    let none = t.browse(&keeper, "/boms?search=zzzz").await.text();
    assert_eq!(bom_rows(&none), 0);
    assert!(none.contains("Nothing matches"));
    // The status: the grid's filter and the old link.
    for link in ["?in.status=released", "?status=released"] {
        let only = t.browse(&keeper, &format!("/boms{link}")).await.text();
        assert_eq!(bom_rows(&only), 1, "{link}");
        assert!(only.contains("PANEL"));
    }
    let drafts = t
        .browse(&keeper, "/boms?status=draft&per_page=50")
        .await
        .text();
    assert_eq!(bom_rows(&drafts), 12);
    // Sorted by the item made.
    let up = t
        .browse(&keeper, "/boms?sort=item_code&per_page=50")
        .await
        .text();
    let up = up
        .split("<table class=\"rx-grid__table\"")
        .nth(1)
        .unwrap_or_default();
    assert!(up.find("FG-00").unwrap() < up.find("FG-11").unwrap());
    let down = t
        .browse(&keeper, "/boms?sort=-item_code&per_page=50")
        .await
        .text();
    let down = down
        .split("<table class=\"rx-grid__table\"")
        .nth(1)
        .unwrap_or_default();
    assert!(down.find("PANEL").unwrap() < down.find("FG-00").unwrap());
    // The cost is a column only for who sees what materials cost.
    assert!(first.contains("Materials per unit (AED)"));
    let plain = t.browse(&viewer, "/boms").await.text();
    assert!(!plain.contains("Materials per unit"), "{plain}");
}
