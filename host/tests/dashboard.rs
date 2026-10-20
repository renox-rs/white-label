//! The dashboard of the Property client: figures read from the real modules
//! (units, leases, cheques, invoices, tickets, compliance), each a link to
//! its page, and a tile left out for a person without the permission.

mod support;

use host::roles::{ACCOUNTANT, ADMIN, MEMBER, PROPERTY_MANAGER};
use property::UnitUse;
use renox::prelude::*;
use renox::testing::TestApp;
use support::*;
use wl_kernel::Subject;
use wl_kernel::cheques::{RegisterCheque, cheques};
use wl_kernel::invoicing::VatCategory;
use wl_kernel::tickets::{Assignee, OpenTicket, TASK_CATEGORY};

/// The text of the figure under `label` on the page, and its link.
fn tile(html: &str, label: &str) -> Option<(String, String)> {
    let at = html.find(&format!("<p class=\"rx-stat__label\">{label}</p>"))?;
    let before = &html[..at];
    let link = before.rfind("href=\"")?;
    let href = before[link + 6..].split('"').next()?.to_owned();
    let after = &html[at..];
    let value = after.split("rx-stat__value\">").nth(1)?;
    let value = value.split("</p>").next()?.trim().to_owned();
    Some((value, href))
}

struct Scene {
    app: TestApp,
    admin: User,
    building_a: i64,
}

/// Two buildings: A with a unit under an active lease that ends in 64 days
/// and an unpaid rent invoice 10 days past its due date, B with a free unit;
/// two cheques of the tenant (one due for deposit, one bounced) and a task
/// ticket assigned to the admin.
async fn scene() -> Scene {
    let app = booted().await;
    let admin = person(&app, "Rana", ADMIN).await;
    let tenant = tenant(&app, "Tara Nasser").await;
    let unit_a = unit_of(&app, "TWR-A", "1204", UnitUse::Residential).await;
    let _unit_b = unit_of(&app, "TWR-B", "7", UnitUse::Residential).await;
    let lease = active_lease(&app, &unit_a, tenant, false).await;
    let clerk = person(&app, "Clara", host::roles::RECEIVABLES).await;
    let rent = invoiced_charge(
        &app,
        &clerk,
        &lease,
        2_000_000,
        VatCategory::Exempt,
        day(-10),
        1,
    )
    .await;
    let moved = wl_invoicing::overdue::run(app.state()).await.unwrap();
    assert!(moved >= 1, "the rent invoice is overdue now");
    let accountant = person(&app, "Amal", ACCOUNTANT).await;
    let entity = entity(&app, "PGP").await;
    let mut ids = Vec::new();
    for (number, date_in) in [("1001", 0), ("1002", 0)] {
        let request =
            RegisterCheque::new(tenant, entity.id, "ENBD", number, day(date_in), 2_000_000)
                .link(rent.subject())
                .agreement(lease.subject());
        let summary = cheques(app.state())
            .register(app.state(), request, Some(accountant.id))
            .await
            .unwrap();
        ids.push(summary.id.0);
    }
    // The second is deposited and returned unpaid.
    app.acting_as(&accountant);
    for (action, pairs) in [
        ("deposit", vec![]),
        (
            "bounce",
            vec![
                ("reason", "insufficient_funds"),
                ("certificate", "CERT-1"),
                ("honoured", ""),
            ],
        ),
    ] {
        let done = app
            .htmx()
            .post(&format!("/cheques/{}/{action}", ids[1]), &pairs)
            .await;
        assert_eq!(done.status.as_u16(), 200, "{action}");
    }
    // The first is on the deposit list.
    wl_payments::cheque_deposit::flag_due_run(app.state(), day(0))
        .await
        .unwrap();
    wl_ticketing::Tickets::open_at(
        app.state(),
        OpenTicket::new(
            Subject::new("property.building", unit_a.building_id),
            TASK_CATEGORY,
            "Replace the gate remote",
        )
        .assign_to(Assignee::user(admin.id)),
        renox::db::now(),
    )
    .await
    .unwrap();
    Scene {
        app,
        admin,
        building_a: unit_a.building_id,
    }
}

#[renox::test]
async fn the_admin_sees_the_figures_of_every_module_each_linking_to_its_page() {
    let scene = scene().await;
    scene.app.acting_as(&scene.admin);
    let page = scene.app.get("/dashboard").await;
    page.assert_ok();
    let html = page.text();
    let figure = |label: &str| tile(&html, label).unwrap_or_else(|| panic!("no tile {label}"));
    assert_eq!(figure("Units available"), ("1".into(), "/property".into()));
    assert_eq!(figure("Units leased"), ("1".into(), "/property".into()));
    assert_eq!(
        figure("Leases ending in 90 days"),
        ("1".into(), "/property/leases".into())
    );
    assert_eq!(
        figure("Cheques due for deposit"),
        ("1".into(), "/cheques?in.status=due_for_deposit".into())
    );
    assert_eq!(
        figure("Bounced cheques"),
        ("1".into(), "/cheques?in.status=bounced".into())
    );
    assert_eq!(
        figure("Overdue invoices"),
        ("AED 20,000.00".into(), "/invoices?in.status=overdue".into())
    );
    assert!(html.contains("1 invoice"));
    let (open, link) = figure("Open tickets");
    assert!(open.parse::<u32>().unwrap() >= 1, "{open}");
    assert!(link.starts_with("/tickets?in.status=open"));
    assert_eq!(figure("My tasks").0, "1");
    assert_eq!(
        figure("Buildings flagged"),
        ("0".into(), "/property/compliance".into())
    );
    // The lists: the lease that ends soonest and the person's task.
    assert!(html.contains("Leases ending soon"));
    assert!(html.contains("Replace the gate remote"));
    // The links lead to pages that open for the admin.
    for url in [
        "/property",
        "/property/leases",
        "/cheques",
        "/invoices",
        "/tickets",
    ] {
        scene.app.get(url).await.assert_ok();
    }
    // The raised flag of a building counts on the tile.
    property::compliance::BuildingFlag::create(
        scene.app.db(),
        property::compliance::BuildingFlag {
            building_id: scene.building_a,
            cause: "visit_missed".into(),
            category: "lift".into(),
            raised_on: day(-1),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let html = scene.app.get("/dashboard").await.text();
    assert_eq!(tile(&html, "Buildings flagged").unwrap().0, "1");
}

#[renox::test]
async fn a_person_without_the_permissions_sees_no_tile() {
    let scene = scene().await;
    let member = person(&scene.app, "Mina", MEMBER).await;
    scene.app.acting_as(&member);
    let page = scene.app.get("/dashboard").await;
    page.assert_ok()
        .assert_dont_see("rx-stat__label")
        .assert_dont_see("Units available")
        .assert_dont_see("Overdue invoices")
        .assert_dont_see("Leases ending soon")
        .assert_dont_see("Replace the gate remote");
}

#[renox::test]
async fn a_manager_of_one_building_counts_the_units_of_that_building_only() {
    let scene = scene().await;
    let scope = property::Building::scope_of(scene.building_a);
    let manager = scoped_person(&scene.app, "Mona2", PROPERTY_MANAGER, &scope).await;
    scene.app.acting_as(&manager);
    let html = scene.app.get("/dashboard").await.text();
    // Building A's unit is leased; the free unit of building B is not hers.
    assert_eq!(tile(&html, "Units available").unwrap().0, "0");
    assert_eq!(tile(&html, "Units leased").unwrap().0, "1");
    // The shared modules' tiles need their page's permission.
    assert!(tile(&html, "Overdue invoices").is_none());
}
