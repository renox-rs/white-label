//! BL-TKT-03 (#148): which categories are compliance-critical is the
//! vertical's setting, changed by `tickets.manage` on the category page.

mod support;

use wl_kernel::Subject;
use wl_kernel::tickets::OpenTicket;
use wl_ticketing::flags::ComplianceFlag;
use wl_ticketing::visit_check;
use wl_ticketing::{Category, Tickets, permissions as perm};

use support::{boot, dubai};

#[renox::test]
async fn tkt03_a_manager_marks_a_category_compliance_critical_and_flags_follow() {
    let t = boot().await;
    let manager = t.user("Mina", &[perm::VIEW, perm::MANAGE]).await;
    let subject = Subject::new("property.building_system", 4);
    let due = renox::chrono::NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
    let night = renox::chrono::NaiveDate::from_ymd_opt(2026, 11, 4).unwrap();
    let visit = |due| OpenTicket::new(subject.clone(), "general", "Lift visit").planned_due(due);

    // Not compliance-critical yet: a missed visit raises no flag.
    Tickets::open_at(t.state(), visit(due), dubai("2026-10-25", 8, 0))
        .await
        .unwrap();
    let report = visit_check::run(t.state(), night).await.unwrap();
    assert_eq!((report.missed, report.flags), (1, 0));
    assert!(ComplianceFlag::open(t.db()).await.unwrap().is_empty());

    t.app.acting_as(&manager);
    t.app
        .htmx()
        .post(
            "/ticket-categories/general/compliance",
            &[("compliance_critical", "1")],
        )
        .await
        .assert_ok();
    assert!(
        Category::by_key(t.db(), "general")
            .await
            .unwrap()
            .unwrap()
            .compliance_critical
    );

    // A visit missed after that raises the flag.
    Tickets::open_at(t.state(), visit(due), dubai("2026-10-26", 8, 0))
        .await
        .unwrap();
    let report = visit_check::run(t.state(), renox::chrono::NaiveDate::from_ymd_opt(2026, 11, 5).unwrap())
        .await
        .unwrap();
    assert_eq!((report.missed, report.flags), (1, 1));
    assert_eq!(ComplianceFlag::open(t.db()).await.unwrap().len(), 1);

    let viewer = t.user("Vera", &[perm::VIEW]).await;
    t.app.acting_as(&viewer);
    t.app
        .htmx()
        .post(
            "/ticket-categories/general/compliance",
            &[("compliance_critical", "0")],
        )
        .await
        .assert_forbidden();
}
