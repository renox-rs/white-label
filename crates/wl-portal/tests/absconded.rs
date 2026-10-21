//! HRM-07 (#197), `HRM.RUL.14`: the portal is switched off for an employee
//! reported as absconded. Their grants stay as they are, but no portal action
//! and no portal page works for them while it lasts.

mod support;

use renox::prelude::*;

use support::{SARA_PARTY, boot};

/// Sara works for the company and is reported as absconded: her actions and
/// her pages are refused, and a user of another account is not affected.
#[renox::test]
async fn hrm07_an_absconded_employee_cannot_use_the_portal() {
    let t = boot().await;
    let (sara, grant) = t.portal_user("Sara", 70, SARA_PARTY).await;
    let (omar, _) = t.portal_user("Omar", 71, 72).await;

    t.act(&sara, "maintenance_request", SARA_PARTY, "en")
        .await
        .assert_ok();
    // No section is registered under this key: the grant let her through.
    t.app.acting_as(&sara);
    t.app
        .get(&format!("/portal/{SARA_PARTY}/nothing"))
        .await
        .assert_status(404);

    t.absent.disable(sara.id);

    t.act(&sara, "maintenance_request", SARA_PARTY, "en")
        .await
        .assert_forbidden();
    t.act(&sara, "pay_invoice", SARA_PARTY, "en")
        .await
        .assert_forbidden();
    t.app.acting_as(&sara);
    t.app
        .get(&format!("/portal/{SARA_PARTY}/nothing"))
        .await
        .assert_forbidden();
    assert_eq!(t.done.all(), ["maintenance_request:70"]);

    // The grant itself is untouched.
    let still = wl_portal::AccessGrant::find_or_404(t.db(), grant.id)
        .await
        .unwrap();
    assert_eq!(still.status, wl_portal::GrantStatus::Active);

    // Another employee's account works as before.
    t.act(&omar, "maintenance_request", 72, "en")
        .await
        .assert_ok();
}
