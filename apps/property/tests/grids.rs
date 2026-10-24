//! #531: the Property lists are standard grids that fill the page: the
//! buildings, leases, owner statements, management agreements, enquiries and
//! the non-compliant buildings. Each searches, sorts, pages and changes its
//! page size through the query string, and the grid is the page's content
//! right under its header (no stack, card or table around it).

mod support;

use property::agreement::{self, NewAgreement};
use property::leases::create_draft;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::tickets::ComplianceFlagRaised;

use support::{COMPANY, MANAGER, TENANT, aed, boot, day};

/// The page at `path`, 200.
async fn load(t: &support::Booted, path: &str) -> String {
    let page = t.app.get(path).await;
    assert_eq!(page.status.as_u16(), 200, "{path}");
    page.text()
}

/// The footer of the grid in `html` reads `from–to of total`.
fn shows_range(html: &str, from: usize, to: usize, total: usize) -> bool {
    html.contains(&format!("{from}–{to} of {total}"))
}

/// The grid of `html` sits right under the page header: nothing but the
/// header's end between them, no old table, and nothing listed after it.
fn fills_the_page(html: &str, path: &str) {
    let header = html
        .find("class=\"rx-page-header\"")
        .unwrap_or_else(|| panic!("{path}: a page header"));
    let grid = html
        .find("<form class=\"rx-grid")
        .unwrap_or_else(|| panic!("{path}: a grid"));
    assert!(header < grid, "{path}: the header comes first");
    let between = &html[header..grid];
    for wrapper in ["rx-stack", "rx-card", "class=\"card", "rx-panes"] {
        assert!(!between.contains(wrapper), "{path}: the grid is wrapped in {wrapper}");
    }
    assert!(!html.contains("rx-table-wrap"), "{path}: an old table is left");
    assert_eq!(html.matches("<form class=\"rx-grid").count(), 1, "{path}: one grid");
}

/// Every list of the vertical is a grid right under its header, with the
/// search box and the page sizes.
#[renox::test]
async fn every_property_list_is_a_grid_that_fills_the_page() {
    let t = boot().await;
    let building = t.building().await;
    let manager = t.user_with("Mona", MANAGER).await;
    t.app.acting_as(&manager);
    for path in [
        "/property",
        "/property/leases",
        "/property/statements",
        "/property/agreements",
        "/property/enquiries",
        "/property/compliance",
    ] {
        let html = load(&t, path).await;
        fills_the_page(&html, path);
        assert!(html.contains("name=\"search\""), "{path}: a search box");
        assert!(html.contains("per_page"), "{path}: a page size");
    }
    let portfolio = load(&t, "/property?search=TWR").await;
    assert!(portfolio.contains(&building.name_en));
}

/// The leases grid finds a lease by its tenant's name, sorts on the annual
/// rent, pages ten at a time and keeps the *New lease* nowhere but on the
/// unit (the building page).
#[renox::test]
async fn the_leases_grid_searches_the_tenant_sorts_and_pages() {
    let t = boot().await;
    let building = t.building().await;
    let manager = t.user_with("Mona", MANAGER).await;
    for n in 0..11 {
        let unit = t.unit(&building, &format!("10{n:02}")).await;
        t.lease(&unit, 10 + n).await;
    }
    let unit = t.unit(&building, "2001").await;
    create_draft(
        t.state(),
        None,
        property::NewLease::new(unit.id, COMPANY, day(5), day(369), aed(250_000), 4)
            .deposit(aed(5_000)),
    )
    .await
    .unwrap();
    t.app.acting_as(&manager);

    // Twelve leases, ten to a page: the second page holds two.
    let first = load(&t, "/property/leases?per_page=10&sort=id").await;
    assert!(shows_range(&first, 1, 10, 12));
    let second = load(&t, "/property/leases?per_page=10&sort=id&page=2").await;
    assert!(shows_range(&second, 11, 12, 12));
    assert!(second.contains("TWR-A / 2001"));
    assert!(!second.contains("TWR-A / 1000"));
    // Twenty-five to a page: all of them.
    let all = load(&t, "/property/leases?per_page=25").await;
    assert!(shows_range(&all, 1, 12, 12));

    // The tenant's name finds the company's lease only.
    let found = load(&t, "/property/leases?search=Blue%20Dunes").await;
    assert!(shows_range(&found, 1, 1, 1));
    assert!(found.contains("TWR-A / 2001"));
    let tariq = load(&t, "/property/leases?search=Tariq&per_page=25").await;
    assert!(shows_range(&tariq, 1, 11, 11));

    // The highest rent first.
    let by_rent = load(&t, "/property/leases?sort=-annual_rent").await;
    let top = by_rent.find("TWR-A / 2001").unwrap();
    let other = by_rent.find("TWR-A / 1000").unwrap();
    assert!(top < other, "the company's AED 250,000 lease comes first");
    assert!(by_rent.contains("250,000.00"));

    // The status filter: a draft is not active.
    let active = load(&t, "/property/leases?in.status=active").await;
    assert!(!active.contains("TWR-A / 2001"));
}

/// The agreements grid finds an agreement by its owner's name.
#[renox::test]
async fn the_agreements_grid_searches_the_owner() {
    let t = boot().await;
    let manager = t.user_with("Mona", MANAGER).await;
    let tariq = agreement::create_draft(t.state(), None, NewAgreement::new(TENANT, day(0), 500))
        .await
        .unwrap();
    let blue = agreement::create_draft(t.state(), None, NewAgreement::new(COMPANY, day(0), 500))
        .await
        .unwrap();
    t.app.acting_as(&manager);
    let all = load(&t, "/property/agreements").await;
    assert!(shows_range(&all, 1, 2, 2));
    let found = load(&t, "/property/agreements?search=Blue").await;
    assert!(shows_range(&found, 1, 1, 1));
    assert!(found.contains(&format!("/property/agreements/{}", blue.id)));
    assert!(!found.contains(&format!("/property/agreements/{}\"", tariq.id)));
}

/// A non-compliant building's row opens the building; a flag of a building
/// the manager does not hold is not found.
#[renox::test]
async fn a_compliance_row_opens_its_building() {
    let t = boot().await;
    let a = t.building_coded("TWR-A").await;
    let b = t.building_coded("TWR-B").await;
    let manager = t.user_in("Sami", MANAGER, a.id).await;
    for building in [&a, &b] {
        t.state()
            .emit(ComplianceFlagRaised {
                subject: Subject::new("property.building", building.id),
                category: "lifts".to_owned(),
                reason: "no_contract".to_owned(),
                contract: None,
            })
            .await
            .unwrap();
    }
    let flags = property::compliance::BuildingFlag::open(t.db()).await.unwrap();
    let of = |building: i64| flags.iter().find(|f| f.building_id == building).unwrap().id;
    t.app.acting_as(&manager);
    let html = load(&t, "/property/compliance").await;
    assert!(html.contains(&format!("/property/compliance/{}", of(a.id))));
    let open = t.app.get(&format!("/property/compliance/{}", of(a.id))).await;
    assert_eq!(open.status.as_u16(), 303);
    let location = open.header("location").unwrap_or_default().to_owned();
    assert!(
        location.ends_with(&format!("/property/buildings/{}", a.id)),
        "{location}"
    );
    let other = t.app.get(&format!("/property/compliance/{}", of(b.id))).await;
    assert_eq!(other.status.as_u16(), 404);
}
