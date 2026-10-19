//! The override pages (#50 A10): the request form, the list with its two
//! parts, and the details page with the approval card and its access rule.

mod support;

use renox::prelude::*;
use wl_documents::{BlockOverride, BlockOverrideStatus, KindSeed, permissions as perm};
use wl_kernel::approvals;
use wl_kernel::documents::BlockMode;

use support::{Options, RAVI, boot_options, day, ravi};

const ACTIVATE: &str = "deployment.activate";

fn health_card() -> KindSeed {
    KindSeed::new("health_card", "Health card")
        .subject_type("hr.employee")
        .expires()
        .blocks(ACTIVATE, BlockMode::Block)
}

#[renox::test]
async fn the_pages_ask_list_show_and_guard_an_override() {
    let t = boot_options(Options {
        kernel: true,
        ..Options::default()
    })
    .await;
    t.install(&[health_card()]).await;
    let rita = t.reviewer().await;
    t.accepted(&rita, ravi(), "health_card", Some(day(-3)))
        .await;
    let a = t.user("Alice", &[]).await;
    let b = t.user("Bea", &[perm::BLOCK_OVERRIDE]).await;
    let d = t.user("Dan", &[]).await;

    // The form is prefilled from a refusal's link.
    t.app.acting_as(&a);
    t.app
        .get(&format!(
            "/documents/overrides/new?kind=health_card&guarded={ACTIVATE}&record={RAVI}"
        ))
        .await
        .assert_ok()
        .assert_see(ACTIVATE)
        .assert_see("Health card");

    // A blank reason is a 422, and nothing is stored.
    t.app
        .htmx()
        .post(
            "/documents/overrides",
            &[
                ("kind", "health_card"),
                ("guarded", ACTIVATE),
                ("record", &RAVI.to_string()),
                ("reason", "  "),
            ],
        )
        .await
        .assert_status(422);
    assert_eq!(BlockOverride::query().count(t.db()).await.unwrap(), 0);

    // A asks; the override is stored and A sees it in "Your requests".
    t.app
        .post(
            "/documents/overrides",
            &[
                ("kind", "health_card"),
                ("guarded", ACTIVATE),
                ("record", &RAVI.to_string()),
                ("reason", "renewal booked 2026-12-03"),
            ],
        )
        .await;
    let row = BlockOverride::query().first(t.db()).await.unwrap().unwrap();
    assert_eq!(row.status, BlockOverrideStatus::Requested);
    assert_eq!(row.requested_by, a.id);
    let mine = t.app.get("/documents/overrides").await;
    mine.assert_ok()
        .assert_see("Your requests")
        .assert_see("Health card")
        .assert_see("Waiting for approval");
    t.app
        .get(&format!("/documents/overrides/{}", row.id))
        .await
        .assert_ok()
        .assert_see("renewal booked 2026-12-03");

    // B sees it waiting for approval, and the approval card on its page.
    t.app.acting_as(&b);
    let waiting = t.app.get("/documents/overrides").await;
    waiting
        .assert_ok()
        .assert_see(&format!("/documents/overrides/{}", row.id));
    t.app
        .get(&format!("/documents/overrides/{}", row.id))
        .await
        .assert_ok()
        .assert_see("renewal booked 2026-12-03");

    // Anyone else is refused.
    t.app.acting_as(&d);
    t.app
        .get(&format!("/documents/overrides/{}", row.id))
        .await
        .assert_forbidden();

    // After B approves, the page shows it approved with both users.
    approvals::approve(t.state(), row.approval_request_id.unwrap(), &b, "")
        .await
        .unwrap();
    t.app.acting_as(&a);
    t.app
        .get(&format!("/documents/overrides/{}", row.id))
        .await
        .assert_ok()
        .assert_see("Approved")
        .assert_see("Bea");
}
